mod support;
use poznan::{
    Event, ScheduleParser, Stats,
    cache::{Buffers, Cache, Error, Snapshot},
};
use support::Flash;

const HTML: &[u8] = include_bytes!("fixtures/schedule.html");
const CACHE: Cache = Cache::new([0, 4096], 4096);
fn refresh(cache: Cache, flash: &mut Flash<'_>, html: &[u8], time: u64) -> Result<Snapshot, Error> {
    let (mut tag, mut text, mut metadata, mut write) = ([0; 512], [0; 256], [0; 512], [0; 256]);
    let mut update = cache.update(
        flash,
        Buffers {
            tag: &mut tag,
            text: &mut text,
            metadata: &mut metadata,
            write: &mut write,
        },
        time,
    )?;
    for chunk in html.chunks(7) {
        update.feed(chunk)?;
    }
    update.finish()?.commit()
}
fn key(event: Event<'_>) -> String {
    format!("{event:?}")
}

#[test]
fn reboots_replay_exact_events_and_keep_distinct_object_ids() {
    let mut bytes = [0xff; 8192];
    let mut flash = Flash::new(&mut bytes);
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), None);
    let snapshot = refresh(CACHE, &mut flash, HTML, 12345).unwrap();
    let reopened = CACHE.open(&mut flash, &mut [0; 4]).unwrap().unwrap();
    assert_eq!(reopened, snapshot);
    assert_eq!((snapshot.generation(), snapshot.fetched_at()), (1, 12345));
    assert!(snapshot.bytes() < 256);
    let mut expected = Vec::new();
    let (mut tag, mut text, mut metadata) = ([0; 512], [0; 256], [0; 512]);
    let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
    parser
        .feed(HTML, |e| {
            expected.push(key(e));
            Ok(())
        })
        .unwrap();
    let mut actual = Vec::new();
    let stats = reopened
        .replay(&mut flash, &mut [0; 4], &mut [0; 512], |e| {
            actual.push(key(e));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        stats,
        Stats {
            objects: 2,
            pickups: 10
        }
    );
    assert_eq!(actual, expected);
    let second = refresh(CACHE, &mut flash, HTML, 23456).unwrap();
    assert_eq!(second.generation(), 2);
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(second));
    let third = refresh(CACHE, &mut flash, HTML, 34567).unwrap();
    assert_eq!(third.generation(), 3);
}

#[test]
fn parser_failure_poison_and_failed_transport_preserve_old_cache() {
    let mut bytes = [0xff; 8192];
    let mut flash = Flash::new(&mut bytes);
    let old = refresh(CACHE, &mut flash, HTML, 1).unwrap();
    for end in 0..HTML.len() {
        let result = refresh(CACHE, &mut flash, &HTML[..end], 2);
        // Trailing whitespace/footer can be omitted, but the complete schedule cannot.
        if result.is_ok() {
            break;
        }
        assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(old));
    }
    let old = CACHE.open(&mut flash, &mut [0; 256]).unwrap().unwrap();
    let (mut tag, mut text, mut metadata, mut write) = ([0; 512], [0; 256], [0; 512], [0; 256]);
    let mut update = CACHE
        .update(
            &mut flash,
            Buffers {
                tag: &mut tag,
                text: &mut text,
                metadata: &mut metadata,
                write: &mut write,
            },
            3,
        )
        .unwrap();
    update.feed(HTML).unwrap();
    let prepared = update.finish().unwrap();
    // Model a failed HTTP completion after a syntactically valid body.
    drop(prepared);
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(old));
    let bad = String::from_utf8(HTML.to_vec())
        .unwrap()
        .replace("waste-ZM", "waste-NEW");
    let mut update = CACHE
        .update(
            &mut flash,
            Buffers {
                tag: &mut tag,
                text: &mut text,
                metadata: &mut metadata,
                write: &mut write,
            },
            3,
        )
        .unwrap();
    assert!(update.feed(bad.as_bytes()).is_err());
    assert_eq!(update.feed(HTML), Err(Error::Closed));
    assert!(matches!(update.finish(), Err(Error::Closed)));
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(old));
}

#[test]
fn every_interrupted_mutation_keeps_a_complete_generation() {
    let mut original = [0xff; 8192];
    let mut flash = Flash::new(&mut original);
    refresh(CACHE, &mut flash, HTML, 1).unwrap();
    let old = refresh(CACHE, &mut flash, HTML, 2).unwrap();
    // Both banks contain data: this refresh must erase a previous generation.
    let mut saved = [0xff; 8192];
    saved.copy_from_slice(flash.bytes);
    let before = flash.operations;
    let new = refresh(CACHE, &mut flash, HTML, 3).unwrap();
    let operations = flash.operations - before;
    for operation in 0..operations {
        let max = if operation == 0 { 4096 } else { 256 };
        for prefix in 0..=max {
            let mut image = saved;
            let mut flash = Flash::new(&mut image);
            flash.fail = Some((operation, prefix));
            assert!(refresh(CACHE, &mut flash, HTML, 3).is_err());
            flash.fail = None;
            let recovered = CACHE.open(&mut flash, &mut [0; 256]).unwrap().unwrap();
            // A completely written header may be durable even if write reports error.
            assert!(recovered == old || recovered == new);
            assert_eq!(
                recovered
                    .replay(&mut flash, &mut [0; 256], &mut [0; 512], |_| Ok(()))
                    .unwrap(),
                old.stats()
            );
        }
    }
}

#[test]
fn corrupt_newer_bank_falls_back_and_io_and_capacity_errors_are_visible() {
    let mut bytes = [0xff; 8192];
    let mut flash = Flash::new(&mut bytes);
    let old = refresh(CACHE, &mut flash, HTML, 1).unwrap();
    refresh(CACHE, &mut flash, HTML, 2).unwrap();
    flash.bytes[4096 + 65] ^= 1;
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(old));
    assert_eq!(CACHE.open(&mut flash, &mut [0; 3]), Err(Error::Geometry));
    assert_eq!(
        Cache::new([0, 0], 4096).open(&mut flash, &mut [0; 256]),
        Err(Error::Geometry)
    );
    assert_eq!(
        Cache::new([0, 8192], 4096).open(&mut flash, &mut [0; 256]),
        Err(Error::Geometry)
    );
    assert_eq!(
        old.replay(&mut flash, &mut [0; 256], &mut [0; 1], |_| Ok(())),
        Err(Error::BufferTooSmall)
    );
    assert_eq!(
        old.replay(&mut flash, &mut [0; 256], &mut [0; 512], |_| Err(
            poznan::Error::Sink
        )),
        Err(Error::Sink)
    );
    let html = String::from_utf8(HTML.to_vec()).unwrap();
    let row = "<tr><td class='waste-type waste-ZM'>Z</td><td data-value='1.2026' class='waste-date'>1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31</td></tr>";
    let huge = html.replacen("</table>", &(row.repeat(40) + "</table>"), 1);
    assert_eq!(
        refresh(CACHE, &mut flash, huge.as_bytes(), 3),
        Err(Error::Full)
    );
    assert_eq!(CACHE.open(&mut flash, &mut [0; 256]).unwrap(), Some(old));
}
