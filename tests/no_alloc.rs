use poznan::{Error, LookupParser, Query, ScheduleParser, Stats};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// Run without libtest: even one #[test] has a concurrent harness thread whose
// allocations can race a process-global allocation counter on hosted runners.
fn main() {
    let (mut tag, mut text, mut metadata, mut path) = ([0; 512], [0; 256], [0; 512], [0; 1024]);
    let skipped = [b'x'; 1024];
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
    println!("allocation probe: 8 MiB streamed; zero heap allocations");
}
