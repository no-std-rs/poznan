mod arena;
mod flash;
mod mobility;
use embedded_io_async::{ErrorKind, ErrorType, Read, Write};
use embedded_tls::TlsClock;
use poznan::{Query, cache::Cache};
use poznan_https::{HOST, Workspace, refresh};
use std::{
    env,
    future::Future,
    net::TcpStream,
    path::Path,
    task::{Context, Poll, Waker},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[global_allocator]
static ALLOC: arena::Allocator = arena::Allocator;
struct Clock;
impl TlsClock for Clock {
    fn now() -> Option<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    }
}
struct Socket(TcpStream);
impl ErrorType for Socket {
    type Error = ErrorKind;
}
impl Read for Socket {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        std::io::Read::read(&mut self.0, out).map_err(|_| ErrorKind::Other)
    }
}
impl Write for Socket {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize, Self::Error> {
        std::io::Write::write(&mut self.0, bytes).map_err(|_| ErrorKind::Other)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        std::io::Write::flush(&mut self.0).map_err(|_| ErrorKind::Other)
    }
}
// Socket operations above block, so this probe needs no executor allocation.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("blocking probe unexpectedly suspended"),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|s| {
        matches!(
            s.as_str(),
            "stops" | "meters" | "parse-stops" | "parse-meters"
        )
    }) {
        return mobility::run(&args);
    }
    let cache = Cache::new([0, 32768], 32768);
    let usage = "Mobility: stops|meters CACHE_NDJSON; parse-stops|parse-meters INPUT_JSON CACHE_NDJSON\nUsage: poznan-live-probe refresh CACHE_FILE STREET BUILDING_GROUP [OBJECT_ID]\n       poznan-live-probe read CACHE_FILE\n       poznan-live-probe import CACHE_FILE HTML_FILE";
    let Some(file) = args.get(1) else {
        return Err(usage.into());
    };
    let mut flash = flash::Flash::open(Path::new(file))?;
    match args[0].as_str() {
        "refresh" if (4..=5).contains(&args.len()) => {
            let query = Query::Schedule {
                street: &args[2],
                number: &args[3],
                object: args.get(4).map(String::as_str),
            };
            let socket = TcpStream::connect((HOST, 443))?;
            socket.set_read_timeout(Some(Duration::from_secs(30)))?;
            socket.set_write_timeout(Some(Duration::from_secs(30)))?;
            let mut workspace = Workspace::<Clock>::new();
            let mut rng = rand::rngs::OsRng;
            let (result, peak, misses) = arena::measure(|| {
                block_on(refresh(
                    Socket(socket),
                    &mut flash,
                    cache,
                    query,
                    &mut rng,
                    &mut workspace,
                ))
            });
            eprintln!(
                "HTTPS pipeline heap: {peak} / {} B; arena misses: {misses}",
                poznan_https::RSA_HEAP_BYTES
            );
            assert_eq!(misses, 0, "pipeline exceeded its bounded heap");
            let snapshot = result.map_err(|e| format!("refresh failed: {e:?}"))?;
            println!(
                "committed generation={} bytes={} objects={} pickups={}",
                snapshot.generation(),
                snapshot.bytes(),
                snapshot.stats().objects,
                snapshot.stats().pickups
            );
        }
        "read" if args.len() == 2 => {
            let snapshot = cache
                .open(&mut flash, &mut [0; 256])?
                .ok_or("no valid cache")?;
            snapshot.replay(&mut flash, &mut [0; 256], &mut [0; 512], |event| {
                println!("{event:?}");
                Ok(())
            })?;
            eprintln!(
                "cache generation={} fetched_at={} bytes={} objects={} pickups={}",
                snapshot.generation(),
                snapshot.fetched_at(),
                snapshot.bytes(),
                snapshot.stats().objects,
                snapshot.stats().pickups
            );
        }
        "import" if args.len() == 3 => {
            let mut input = std::fs::File::open(&args[2])?;
            let (mut tag, mut text, mut metadata, mut write) =
                ([0; 512], [0; 256], [0; 512], [0; 256]);
            let mut update = cache.update(
                &mut flash,
                poznan::cache::Buffers {
                    tag: &mut tag,
                    text: &mut text,
                    metadata: &mut metadata,
                    write: &mut write,
                },
                Clock::now().ok_or("clock")?,
            )?;
            let mut bytes = [0; 512];
            loop {
                let n = std::io::Read::read(&mut input, &mut bytes)?;
                if n == 0 {
                    break;
                }
                update.feed(&bytes[..n])?;
            }
            let s = update.finish()?.commit()?;
            println!(
                "committed generation={} bytes={} objects={} pickups={}",
                s.generation(),
                s.bytes(),
                s.stats().objects,
                s.stats().pickups
            );
        }
        _ => return Err(usage.into()),
    }
    Ok(())
}
