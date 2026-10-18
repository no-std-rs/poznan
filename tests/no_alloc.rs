use poznan::{Error, LookupParser, Query, ScheduleParser, Stats};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
mod support;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// Run without libtest: even one #[test] has a concurrent harness thread whose
// allocations can race a process-global allocation counter on hosted runners.
fn main() {
    let (mut tag, mut text, mut metadata, mut path) = ([0; 512], [0; 256], [0; 512], [0; 1024]);
    let skipped = [b'x'; 1024];
    let mut flash_bytes = [0xff; 8192];
    let mut flash = support::Flash::new(&mut flash_bytes);
    let mut write = [0; 256];
    let mut feature = [0; 2048];
    let mut strings = [0; 512];
    let region = Region::new(GLOBAL);
    Query::Schedule {
        street: "TESTOWA",
        number: "100,102,102A",
        object: Some("1/1."),
    }
    .path(&mut path)
    .unwrap();
    let mut schedule = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
    let mut count = 0;
    // Eight MiB passes through the same parser before the tiny schedule arrives.
    for _ in 0..8192 {
        schedule.feed(&skipped, |_| Err(Error::Sink)).unwrap();
    }
    for chunk in include_bytes!("fixtures/schedule.html").chunks(1) {
        schedule
            .feed(chunk, |_| {
                count += 1;
                Ok(())
            })
            .unwrap();
    }
    let stats = schedule.finish().unwrap();
    let mut lookup = LookupParser::new(&mut tag, &mut text);
    lookup
        .feed(b"<ul><li>100,102,102A</li><li>1/1.</li></ul>", |_| {
            count += 1;
            Ok(())
        })
        .unwrap();
    let items = lookup.finish().unwrap();
    let cache = poznan::cache::Cache::new([0, 4096], 4096);
    let mut update = cache
        .update(
            &mut flash,
            poznan::cache::Buffers {
                tag: &mut tag,
                text: &mut text,
                metadata: &mut metadata,
                write: &mut write,
            },
            123,
        )
        .unwrap();
    for _ in 0..8192 {
        update.feed(&skipped).unwrap();
    }
    for chunk in include_bytes!("fixtures/schedule.html").chunks(1) {
        update.feed(chunk).unwrap();
    }
    let snapshot = update.finish().unwrap().commit().unwrap();
    assert_eq!(cache.open(&mut flash, &mut write).unwrap(), Some(snapshot));
    assert_eq!(
        snapshot
            .replay(&mut flash, &mut write, &mut metadata, |_| Ok(()))
            .unwrap(),
        stats
    );
    let mut mobility =
        poznan::mobility::Parser::new(poznan::mobility::Feed::Stops, &mut feature, &mut strings);
    mobility
        .feed(
            b"{\"type\":\"FeatureCollection\",\"features\":[",
            |_| Ok(()),
        )
        .unwrap();
    const STOP: &[u8] = br#"{"type":"Feature","id":"TEST01","geometry":{"type":"Point","coordinates":[16.9,52.4]},"properties":{"stop_name":"Pr\u00f3ba","zone":"A","headsigns":"1, 2","route_type":"3"}}"#;
    for i in 0..20000 {
        if i > 0 {
            mobility.feed(b",", |_| Ok(())).unwrap();
        }
        for chunk in STOP.chunks(1) {
            mobility.feed(chunk, |_| Ok(())).unwrap();
        }
    }
    mobility.feed(b"]}", |_| Ok(())).unwrap();
    assert_eq!(mobility.finish(), Ok(20000));
    let allocations = region.change();
    assert_eq!(
        stats,
        Stats {
            objects: 2,
            pickups: 10
        }
    );
    assert_eq!((items, count), (2, 16));
    assert_eq!(allocations.allocations, 0);
    assert_eq!(allocations.reallocations, 0);
    assert_eq!(allocations.bytes_allocated, 0);
    println!(
        "allocation probe: 8 MiB parse + 8 MiB cache/replay + 20,000 mobility records; zero heap allocations"
    );
}
