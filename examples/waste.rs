//! Hosted adapter: curl owns HTTP/TLS; Rust streams the body into the no_std core.
use poznan::{Error, Event, LookupParser, ORIGIN, Query, ScheduleParser, Stats};
use std::{
    env,
    error::Error as StdError,
    fs::File,
    io::{self, Read, Write},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn StdError>>;

const USAGE: &str = "Usage:
  waste streets TEXT
  waste numbers STREET NUMBER_PREFIX
  waste objects STREET BUILDING_GROUP [OBJECT_PREFIX]
  waste schedule STREET BUILDING_GROUP [OBJECT_ID]
  waste object OBJECT_ID
  waste parse-schedule [FILE|-]
  waste parse-lookup [FILE|-]

Use the exact street and building group returned by lookup. Output is NDJSON.
Records are provisional until the final complete record AND successful exit.
Live commands require curl. Offline parsing does not require networking.";

#[derive(Clone, Copy)]
enum Summary {
    Lookup(u32),
    Schedule(Stats),
}

fn main() {
    if let Err(error) = run() {
        eprintln!("waste: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut out = io::BufWriter::with_capacity(1024, io::stdout().lock());
    let summary = match args.as_slice() {
        ["parse-schedule", rest @ ..] | ["parse-lookup", rest @ ..] if rest.len() <= 1 => {
            let schedule = args[0] == "parse-schedule";
            let path = rest.first().copied().unwrap_or("-");
            if path == "-" {
                parse(&mut io::stdin().lock(), schedule, &mut out)?
            } else {
                parse(&mut File::open(path)?, schedule, &mut out)?
            }
        }
        ["-h" | "--help"] | [] => {
            println!("{USAGE}");
            return Ok(());
        }
        values => {
            let query = match values {
                ["streets", contains] => Query::Streets { contains },
                ["numbers", street, contains] => Query::Numbers { street, contains },
                ["objects", street, number] => Query::Objects {
                    street,
                    number,
                    contains: "",
                },
                ["objects", street, number, contains] => Query::Objects {
                    street,
                    number,
                    contains,
                },
                ["schedule", street, number] => Query::Schedule {
                    street,
                    number,
                    object: None,
                },
                ["schedule", street, number, object] => Query::Schedule {
                    street,
                    number,
                    object: Some(object),
                },
                ["object", object] => Query::Schedule {
                    street: "",
                    number: "",
                    object: Some(object),
                },
                _ => return Err(USAGE.into()),
            };
            let mut path = [0; 4096];
            let path = query.path(&mut path)?;
            let mut child = Command::new("curl")
                .args([
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--globoff",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--connect-timeout",
                    "15",
                    "--max-time",
                    "120",
                    "--header",
                    "Accept-Encoding: identity",
                ])
                .arg(format!("{ORIGIN}{path}"))
                .stdout(Stdio::piped())
                .spawn()?;
            let result = match child.stdout.take() {
                Some(mut body) => {
                    parse(&mut body, matches!(query, Query::Schedule { .. }), &mut out)
                }
                None => Err("curl did not provide stdout".into()),
            };
            if result.is_err() {
                let _ = child.kill();
            }
            let status = child.wait()?;
            let summary = result?;
            if !status.success() {
                return Err(format!("HTTP transfer failed: {status}").into());
            }
            summary
        }
    };
    match summary {
        Summary::Lookup(items) => writeln!(out, "{{\"type\":\"complete\",\"items\":{items}}}")?,
        Summary::Schedule(Stats { objects, pickups }) => writeln!(
            out,
            "{{\"type\":\"complete\",\"objects\":{objects},\"pickups\":{pickups}}}"
        )?,
    }
    out.flush()?;
    Ok(())
}

fn parse(source: &mut impl Read, schedule: bool, out: &mut impl Write) -> Result<Summary> {
    let mut input = [0; 1024];
    let mut tag = [0; 512];
    let mut text = [0; 256];
    if schedule {
        let mut metadata = [0; 512];
        let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
        loop {
            let n = source.read(&mut input)?;
            if n == 0 {
                break;
            }
            parser.feed(&input[..n], |event| {
                write_event(out, event).map_err(|_| Error::Sink)
            })?;
        }
        Ok(Summary::Schedule(parser.finish()?))
    } else {
        let mut parser = LookupParser::new(&mut tag, &mut text);
        loop {
            let n = source.read(&mut input)?;
            if n == 0 {
                break;
            }
            parser.feed(&input[..n], |value| {
                (|| -> io::Result<()> {
                    out.write_all(b"{\"type\":\"match\",\"value\":")?;
                    json(out, value)?;
                    out.write_all(b"}\n")
                })()
                .map_err(|_| Error::Sink)
            })?;
        }
        Ok(Summary::Lookup(parser.finish()?))
    }
}

fn write_event(out: &mut impl Write, event: Event<'_>) -> io::Result<()> {
    match event {
        Event::BeginObject(object) => {
            out.write_all(b"{\"type\":\"object\",\"id\":")?;
            json(out, object.id)?;
            out.write_all(b",\"address\":")?;
            json(out, object.address)?;
            out.write_all(b",\"sector\":")?;
            json(out, object.sector)?;
            out.write_all(b",\"property_kind\":")?;
            json(out, object.property_kind)?;
            out.write_all(b"}\n")
        }
        Event::Pickup {
            object,
            fraction,
            date,
        } => {
            out.write_all(b"{\"type\":\"pickup\",\"object\":")?;
            json(out, object.id)?;
            writeln!(
                out,
                ",\"fraction\":\"{}\",\"date\":\"{date}\"}}",
                fraction.code()
            )
        }
        Event::EndObject(object) => {
            out.write_all(b"{\"type\":\"end_object\",\"id\":")?;
            json(out, object.id)?;
            out.write_all(b"}\n")
        }
    }
}

fn json(out: &mut impl Write, value: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    for ch in value.chars() {
        match ch {
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
