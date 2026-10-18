use poznan::{Date, Error, Event, Fraction, LookupParser, Query, ScheduleParser, Stats};

const FIXTURE: &[u8] = include_bytes!("fixtures/schedule.html");

#[derive(Debug, PartialEq, Eq)]
enum Owned {
    Begin(String, String, String),
    Pickup(String, Fraction, Date),
    End(String),
}

fn events(input: &[u8], chunk: usize) -> Result<(Stats, Vec<Owned>), Error> {
    let (mut tag, mut text, mut metadata) = ([0; 512], [0; 256], [0; 512]);
    let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
    let mut result = Vec::new();
    for part in input.chunks(chunk) {
        parser.feed(part, |event| {
            result.push(match event {
                Event::BeginObject(o) => {
                    Owned::Begin(o.id.into(), o.address.into(), o.property_kind.into())
                }
                Event::Pickup {
                    object,
                    fraction,
                    date,
                } => Owned::Pickup(object.id.into(), fraction, date),
                Event::EndObject(o) => Owned::End(o.id.into()),
            });
            Ok(())
        })?;
    }
    Ok((parser.finish()?, result))
}

#[test]
fn schedules_survive_every_chunk_size_and_preserve_distinct_objects() {
    let expected = events(FIXTURE, FIXTURE.len()).unwrap();
    assert_eq!(
        expected.0,
        Stats {
            objects: 2,
            pickups: 10
        }
    );
    assert_eq!(
        expected.1[0],
        Owned::Begin(
            "1/1".into(),
            "POZNAŃ, TESTOWA & PRÓBNA, 100,102,102A".into(),
            "MZ".into()
        )
    );
    assert!(expected.1.contains(&Owned::Pickup(
        "1/1".into(),
        Fraction::Mixed,
        Date::new(2024, 2, 29).unwrap()
    )));
    assert!(expected.1.contains(&Owned::Begin(
        "1/1.".into(),
        "POZNAŃ, TESTOWA & PRÓBNA, 100,102,102A".into(),
        "MN".into()
    )));
    for n in 1..=FIXTURE.len() {
        assert_eq!(events(FIXTURE, n).unwrap(), expected, "chunk {n}");
    }
}

#[test]
fn every_truncation_before_the_schedule_closes_is_rejected() {
    let end = FIXTURE
        .windows(b"</div><footer>".len())
        .position(|p| p == b"</div><footer>")
        .unwrap()
        + 6;
    for n in 0..end {
        assert!(events(&FIXTURE[..n], 11).is_err(), "accepted prefix {n}");
    }
}

fn replace(from: &str, to: &str) -> Vec<u8> {
    String::from_utf8(FIXTURE.to_vec())
        .unwrap()
        .replacen(from, to, 1)
        .into_bytes()
}

#[test]
fn bad_cells_and_layout_changes_do_not_publish_a_completed_result() {
    for (from, to, expected) in [
        ("01, 29", "01, 30", Error::InvalidDate),
        ("2.2024", "2.2023", Error::InvalidDate),
        ("01, 29", "01, 01", Error::InvalidDays),
        ("01, 29", "01,", Error::InvalidDays),
        ("01, 29", "01; 29", Error::InvalidDays),
        ("waste-ZM", "waste-UNKNOWN", Error::SchemaChanged),
        ("Sektor: II", "Region: II", Error::SchemaChanged),
        ("Numer obiektu: 1/1", "Numer obiektu:", Error::SchemaChanged),
        ("TESTOWA &amp;", "TESTOWA &bogus;", Error::InvalidEntity),
    ] {
        // The first 2.2024 belongs to an ignored header, so replace all for this case.
        let data = if from == "2.2024" {
            String::from_utf8(FIXTURE.to_vec())
                .unwrap()
                .replace(from, to)
                .into_bytes()
        } else {
            replace(from, to)
        };
        assert_eq!(events(&data, 1).unwrap_err(), expected, "{from} -> {to}");
    }
    assert_eq!(
        events(b"<div class='waste-schedule'></div>", 1).unwrap_err(),
        Error::NoSchedule
    );
    assert_eq!(
        events(b"<html>Maintenance</html>", 1).unwrap_err(),
        Error::SchemaChanged
    );
}

#[test]
fn overflows_and_sink_failures_poison_the_parser() {
    for (t, x, m) in [(2, 256, 512), (512, 2, 512), (512, 256, 2)] {
        let (mut tag, mut text, mut metadata) = (vec![0; t], vec![0; x], vec![0; m]);
        let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
        assert_eq!(parser.feed(FIXTURE, |_| Ok(())), Err(Error::BufferTooSmall));
        assert_eq!(parser.finish(), Err(Error::Closed));
    }
    let (mut tag, mut text, mut metadata) = ([0; 512], [0; 256], [0; 512]);
    let mut parser = ScheduleParser::new(&mut tag, &mut text, &mut metadata);
    assert_eq!(parser.feed(FIXTURE, |_| Err(Error::Sink)), Err(Error::Sink));
    assert_eq!(parser.feed(&[], |_| Ok(())), Err(Error::Closed));
    assert_eq!(parser.finish(), Err(Error::Closed));
}

#[test]
fn irrelevant_large_tags_comments_and_raw_text_do_not_require_large_buffers() {
    let prelude = format!(
        "<!-- {} --><svg><path d='{}'></path></svg><SCRIPT>\"<td>fake</td>\";</SCRIPT><style>{}</style>",
        "x".repeat(10000),
        "x".repeat(10000),
        "x".repeat(10000)
    );
    let mut page = prelude.into_bytes();
    page.extend_from_slice(FIXTURE);
    assert_eq!(events(&page, 1).unwrap(), events(FIXTURE, 1).unwrap());
}

fn lookup(input: &[u8], chunk: usize) -> Result<Vec<String>, Error> {
    let (mut tag, mut value) = ([0; 128], [0; 256]);
    let mut parser = LookupParser::new(&mut tag, &mut value);
    let mut values = Vec::new();
    for p in input.chunks(chunk) {
        parser.feed(p, |s| {
            values.push(s.into());
            Ok(())
        })?;
    }
    assert_eq!(parser.finish()?, values.len() as u32);
    Ok(values)
}

#[test]
fn lookup_keeps_building_groups_unicode_and_trailing_dots() {
    let html = " <ul><li>JANA HENRYKA DĄBROWSKIEGO</li><li>104-114/DŁUGOSZA 1-5</li><li>1/2275.</li><li>A &amp; B</li></ul> ";
    for chunk in 1..=html.len() {
        assert_eq!(
            lookup(html.as_bytes(), chunk).unwrap(),
            [
                "JANA HENRYKA DĄBROWSKIEGO",
                "104-114/DŁUGOSZA 1-5",
                "1/2275.",
                "A & B"
            ]
        );
    }
    for html in [
        "",
        " \n",
        "<ul></ul>",
        "<ul><li>Brak dopasowania, sprawdź podany numer</li></ul>",
    ] {
        assert!(lookup(html.as_bytes(), 1).unwrap().is_empty());
    }
    for html in [
        "<ul><li>X",
        "<ul><li>X</li>",
        "<html></html>",
        "<ul><li></li></ul>",
    ] {
        assert!(lookup(html.as_bytes(), 1).is_err(), "{html}");
    }
}

#[test]
fn query_encoding_is_exact_and_overflow_does_not_modify_the_buffer() {
    let mut buffer = [0; 1024];
    let path = Query::Schedule {
        street: "DĄB & ?",
        number: "104-114/DŁUGOSZA 1-5",
        object: Some("1/2275."),
    }
    .path(&mut buffer)
    .unwrap();
    assert_eq!(
        path,
        "/mim/odpady/harmonogramy.html?action=search&co=waste_schedule&ws_street=D%C4%84B%20%26%20%3F&ws_number=104-114%2FD%C5%81UGOSZA%201-5&ws_object=1%2F2275."
    );
    let mut short = [42; 4];
    assert_eq!(
        Query::Streets { contains: "Dąb" }.path(&mut short),
        Err(Error::BufferTooSmall)
    );
    assert_eq!(short, [42; 4]);
    assert_eq!(
        Query::Streets { contains: "Dą" }.path(&mut buffer),
        Err(Error::InvalidQuery)
    );
    assert_eq!(
        Query::Numbers {
            street: "x",
            contains: "\n"
        }
        .path(&mut buffer),
        Err(Error::InvalidQuery)
    );
    assert!(
        Query::Schedule {
            street: "",
            number: "",
            object: Some("1/1.")
        }
        .path(&mut buffer)
        .unwrap()
        .ends_with("ws_object=1%2F1.")
    );
}

#[test]
fn calendar_rules() {
    assert!(Date::new(2000, 2, 29).is_ok());
    for (year, month, day) in [
        (1900, 2, 29),
        (2023, 2, 29),
        (2024, 4, 31),
        (0, 1, 1),
        (2024, 13, 1),
        (2024, 1, 0),
    ] {
        assert_eq!(Date::new(year, month, day), Err(Error::InvalidDate));
    }
}
