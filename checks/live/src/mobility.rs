//! Hosted streaming cache: stage NDJSON beside its destination, publish after
//! successful HTTP framing/parser EOF, and leave the old file on any failure.
use crate::{Clock, Socket, arena, block_on};
use embedded_tls::TlsClock;
use poznan::{
    Error,
    mobility::{Feed, Parser, Record},
};
use poznan_https::{HOST, HttpWorkspace};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};

struct Staged {
    file: File,
    temporary: PathBuf,
    destination: PathBuf,
}
impl Staged {
    fn new(destination: &Path) -> io::Result<Self> {
        let mut name = OsString::from(
            destination
                .file_name()
                .ok_or_else(|| io::Error::other("missing output filename"))?,
        );
        name.push(format!(".partial-{}", std::process::id()));
        let temporary = destination.with_file_name(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        Ok(Self {
            file,
            temporary,
            destination: destination.to_owned(),
        })
    }
    fn publish(self) -> io::Result<()> {
        self.file.sync_all()?;
        fs::rename(&self.temporary, &self.destination)?;
        let parent = self
            .destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        File::open(parent)?.sync_all()
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (feed, input, destination) = match args {
        [kind, file] if kind == "stops" || kind == "meters" => (
            if kind == "stops" {
                Feed::Stops
            } else {
                Feed::ParkingMeters
            },
            None,
            file,
        ),
        [kind, input, file] if kind == "parse-stops" || kind == "parse-meters" => (
            if kind == "parse-stops" {
                Feed::Stops
            } else {
                Feed::ParkingMeters
            },
            Some(input),
            file,
        ),
        _ => return Err(
            "Usage: stops|meters CACHE_NDJSON; parse-stops|parse-meters INPUT_JSON CACHE_NDJSON"
                .into(),
        ),
    };
    let mut stage = Staged::new(Path::new(destination))?;
    let (mut feature, mut strings) = ([0; 2048], [0; 512]);
    let fetched_at = Clock::now().ok_or("clock")?;
    let count = if let Some(input) = input {
        let mut input = File::open(input)?;
        let mut parser = Parser::new(feed, &mut feature, &mut strings);
        let mut bytes = [0; 512];
        loop {
            let n = input.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            parser.feed(&bytes[..n], |r| {
                write_record(&mut stage.file, r).map_err(|_| Error::Sink)
            })?;
        }
        parser.finish()?
    } else {
        let socket = TcpStream::connect((HOST, 443))?;
        socket.set_read_timeout(Some(Duration::from_secs(30)))?;
        socket.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut workspace = HttpWorkspace::<Clock>::new();
        let mut rng = rand::rngs::OsRng;
        let (result, peak, misses) = arena::measure(|| {
            block_on(poznan_https::mobility(
                Socket(socket),
                feed,
                &mut rng,
                &mut workspace,
                &mut feature,
                &mut strings,
                |r| write_record(&mut stage.file, r).map_err(|_| Error::Sink),
            ))
        });
        eprintln!(
            "mobility HTTPS heap: {peak} / {} B; arena misses: {misses}",
            poznan_https::RSA_HEAP_BYTES
        );
        assert_eq!(misses, 0, "mobility pipeline exceeded its bounded heap");
        result.map_err(|e| format!("mobility refresh failed: {e:?}"))?
    };
    writeln!(
        stage.file,
        "{{\"type\":\"complete\",\"items\":{count},\"fetched_at\":{fetched_at}}}"
    )?;
    stage.publish()?;
    println!("committed {count} records to {destination}");
    Ok(())
}
fn json(out: &mut impl Write, value: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    for c in value.chars() {
        match c {
            '"' => out.write_all(b"\\\"")?,
            '\\' => out.write_all(b"\\\\")?,
            c if c <= '\u{1f}' => write!(out, "\\u{:04x}", u32::from(c))?,
            c => {
                let mut buf = [0; 4];
                out.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    out.write_all(b"\"")
}
fn boolean(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    }
}
fn write_record(out: &mut impl Write, record: Record<'_>) -> io::Result<()> {
    match record {
        Record::Stop(stop) => {
            out.write_all(b"{\"type\":\"stop\",\"id\":")?;
            json(out, stop.id)?;
            out.write_all(b",\"name\":")?;
            json(out, stop.name)?;
            out.write_all(b",\"zone\":")?;
            json(out, stop.zone)?;
            out.write_all(b",\"route_type\":")?;
            json(out, stop.route_type)?;
            out.write_all(b",\"lines\":")?;
            json(out, stop.lines)?;
            writeln!(
                out,
                ",\"longitude\":{},\"latitude\":{}}}",
                stop.point.longitude, stop.point.latitude
            )
        }
        Record::ParkingMeter(meter) => {
            out.write_all(b"{\"type\":\"parking_meter\",\"id\":")?;
            json(out, meter.id)?;
            out.write_all(b",\"street\":")?;
            json(out, meter.street)?;
            out.write_all(b",\"zone\":")?;
            json(out, meter.zone)?;
            writeln!(
                out,
                ",\"longitude\":{},\"latitude\":{},\"coins\":{},\"card\":{},\"blik\":{},\"peka\":{}}}",
                meter.point.longitude,
                meter.point.latitude,
                boolean(meter.coins),
                boolean(meter.card),
                boolean(meter.blik),
                boolean(meter.peka)
            )
        }
    }
}
