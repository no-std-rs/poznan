use poznan::{
    Error,
    mobility::{Feed, Parser, Record},
};
const STOPS: &[u8] = include_bytes!("fixtures/stops.json");
const METERS: &[u8] = include_bytes!("fixtures/meters.json");
fn parse(feed: Feed, data: &[u8], chunk: usize) -> Result<Vec<String>, Error> {
    let (mut feature, mut strings) = ([0; 2048], [0; 512]);
    let mut parser = Parser::new(feed, &mut feature, &mut strings);
    let mut records = Vec::new();
    for part in data.chunks(chunk) {
        parser.feed(part, |r| {
            records.push(format!("{r:?}"));
            Ok(())
        })?;
    }
    assert_eq!(parser.finish()?, records.len() as u32);
    assert_eq!(parser.feed(b"", |_| Ok(())), Err(Error::Closed));
    assert_eq!(parser.finish(), Err(Error::Closed));
    Ok(records)
}
#[test]
fn all_chunk_boundaries_preserve_unicode_coordinates_and_payment_unknowns() {
    for (feed, data) in [(Feed::Stops, STOPS), (Feed::ParkingMeters, METERS)] {
        let expected = parse(feed, data, data.len()).unwrap();
        for chunk in 1..=data.len() {
            assert_eq!(parse(feed, data, chunk).unwrap(), expected, "chunk {chunk}");
        }
    }
    let stops = parse(Feed::Stops, STOPS, 1).unwrap();
    assert!(stops[0].contains("TEST42"));
    assert!(stops[1].contains("Próba 🚋"));
    assert!(stops[1].contains("longitude: 16.95"));
    assert!(stops[1].contains("latitude: 52.39"));
    let meters = parse(Feed::ParkingMeters, METERS, 1).unwrap();
    assert!(
        meters[0].contains("coins: Some(true), card: Some(false), blik: Some(true), peka: None")
    );
    assert!(meters[1].contains("coins: None, card: None, blik: Some(false), peka: None"));
}
#[test]
fn every_truncated_collection_is_rejected_and_errors_poison_the_parser() {
    let last = STOPS.iter().rposition(|b| *b == b'}').unwrap();
    for end in 0..=last {
        assert!(
            parse(Feed::Stops, &STOPS[..end], 7).is_err(),
            "prefix {end}"
        );
    }
    let mut feature = [0; 2048];
    let mut strings = [0; 512];
    let mut parser = Parser::new(Feed::Stops, &mut feature, &mut strings);
    assert_eq!(parser.feed(STOPS, |_| Err(Error::Sink)), Err(Error::Sink));
    assert_eq!(parser.feed(STOPS, |_| Ok(())), Err(Error::Closed));
    assert_eq!(parser.finish(), Err(Error::Closed));
}
#[test]
fn reject_schema_changes_wrong_crs_bad_json_and_nonfinite_coordinates() {
    let stops = core::str::from_utf8(STOPS).unwrap();
    for (from, to) in [
        ("FeatureCollection", "Collection"),
        ("\"4326\"", "\"2177\""),
        ("\"Point\"", "\"LineString\""),
        ("stop_name", "name"),
        ("16.931123", "181"),
        ("16.931123", "1e999"),
        ("52.408456", "-91"),
        ("16.931123,52.408456", "16,52,100"),
        ("\"id\":\"TEST42\"", "\"id\":null"),
        ("\"id\":\"TEST42\"", "\"id\":\"\""),
        ("\"zone\":\"A\"", "\"zone\":\"A\",\"zone\":\"B\""),
        ("\"geometry\":{", "\"geometry\":null,\"geometry\":{"),
    ] {
        let bad = stops.replace(from, to);
        assert_ne!(bad, stops, "fixture lacks {from}");
        assert!(
            parse(Feed::Stops, bad.as_bytes(), 1).is_err(),
            "{from} => {to}"
        );
    }
    for data in [
        br#"{"error_msg":"bad request"}"#.as_slice(),
        br#"{"type":"FeatureCollection","features":[],"features":[]}"#,
        br#"{"type":"FeatureCollection","features":[],}"#,
        br#"{"type":"FeatureCollection","features":[{} ,]}"#,
        br#"{"type":"FeatureCollection","features":[]} true"#,
        br#"{"type":"FeatureCollection","features":[],"x":01}"#,
        br#"{"type":"FeatureCollection","features":[],"x":"\ud800"}"#,
        b"{\"type\":\"FeatureCollection\",\"features\":[],\"x\":\"\xff\"}",
    ] {
        assert!(parse(Feed::Stops, data, 1).is_err(), "{data:?}");
    }
    let bad = core::str::from_utf8(METERS)
        .unwrap()
        .replace("TAK", "maybe");
    assert_eq!(
        parse(Feed::ParkingMeters, bad.as_bytes(), 1),
        Err(Error::SchemaChanged)
    );
    assert_eq!(
        parse(
            Feed::Stops,
            br#"{"features":[],"type":"FeatureCollection"}"#,
            1
        )
        .unwrap(),
        Vec::<String>::new()
    );
}
#[test]
fn feature_string_and_depth_limits_are_explicit() {
    let mut small = [0; 32];
    let mut strings = [0; 512];
    let mut parser = Parser::new(Feed::Stops, &mut small, &mut strings);
    assert_eq!(parser.feed(STOPS, |_| Ok(())), Err(Error::BufferTooSmall));
    let mut feature = [0; 2048];
    let mut small = [0; 2];
    let mut parser = Parser::new(Feed::Stops, &mut feature, &mut small);
    assert_eq!(parser.feed(STOPS, |_| Ok(())), Err(Error::BufferTooSmall));
    let deep = format!("{{\"x\":{}0{}}}", "[".repeat(17), "]".repeat(17));
    assert_eq!(
        parse(Feed::Stops, deep.as_bytes(), 1),
        Err(Error::DepthExceeded)
    );
}
#[test]
fn many_features_use_one_reusable_record_buffer() {
    let start = core::str::from_utf8(METERS).unwrap();
    let record = start
        .split("\"features\":[")
        .nth(1)
        .unwrap()
        .split("},{")
        .next()
        .unwrap()
        .to_owned()
        + "}";
    let (mut feature, mut strings) = ([0; 512], [0; 128]);
    let mut parser = Parser::new(Feed::ParkingMeters, &mut feature, &mut strings);
    parser
        .feed(
            b"{\"type\":\"FeatureCollection\",\"features\":[",
            |_| Ok(()),
        )
        .unwrap();
    let mut count = 0;
    for i in 0..20000 {
        if i > 0 {
            parser.feed(b",", |_| Ok(())).unwrap();
        }
        parser
            .feed(record.as_bytes(), |r| {
                assert!(matches!(r, Record::ParkingMeter(_)));
                count += 1;
                Ok(())
            })
            .unwrap();
    }
    parser.feed(b"]}", |_| Ok(())).unwrap();
    assert_eq!(parser.finish(), Ok(20000));
    assert_eq!(count, 20000);
}
