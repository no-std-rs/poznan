//! Streaming city GeoJSON feeds. Only one bounded feature is held in RAM.
//! Records are provisional until `finish` AND the HTTP transfer succeed.
//!
//! ```
//! use poznan::mobility::{Feed, Parser};
//! let (mut feature, mut strings) = ([0; 2048], [0; 512]);
//! let mut parser = Parser::new(Feed::Stops, &mut feature, &mut strings);
//! parser.feed(br#"{"features":[],"type":"FeatureCollection"}"#, |record| {
//!     // Consume now, or copy selected fields into bounded application storage.
//!     let _ = record;
//!     Ok(())
//! })?;
//! assert_eq!(parser.finish()?, 0);
//! # Ok::<(), poznan::Error>(())
//! ```
use crate::{
    Error,
    json::{JsonError, JsonString, Value},
};

/// Official city map feeds, in longitude/latitude (EPSG:4326) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Stops,
    ParkingMeters,
}
impl Feed {
    pub const fn path(self) -> &'static str {
        match self {
            Self::Stops => "/mim/plan/map_service.html?mtype=pub_transport&co=cluster",
            Self::ParkingMeters => {
                "/mim/plan/map_service.html?mtype=pub_transport&co=parking_meters"
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub longitude: f64,
    pub latitude: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub zone: &'a str,
    /// The source's opaque route-type code; no guessed mode mapping.
    pub route_type: &'a str,
    /// The source's comma-separated line labels, preserved verbatim.
    pub lines: &'a str,
    pub point: Point,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParkingMeter<'a> {
    pub id: &'a str,
    pub street: &'a str,
    pub zone: &'a str,
    pub point: Point,
    /// `None` means missing, null or blank; it never means "not accepted".
    pub coins: Option<bool>,
    pub card: Option<bool>,
    pub blik: Option<bool>,
    pub peka: Option<bool>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Record<'a> {
    Stop(Stop<'a>),
    ParkingMeter(ParkingMeter<'a>),
}

/// Suggested bounds: 2048 B `feature`, 512 B `strings`. Capacity errors are
/// explicit. Metadata and unknown top-level values share the feature buffer.
pub struct Parser<'a> {
    frame: Frame<'a>,
    strings: &'a mut [u8],
    feed: Feed,
    phase: Phase,
    field: Field,
    features: bool,
    collection: bool,
    crs: bool,
    count: u32,
    closed: bool,
}
#[derive(Clone, Copy)]
enum Phase {
    Start,
    KeyOrEnd,
    Key,
    Colon,
    Value,
    Array,
    ItemOrEnd,
    Item,
    ItemComma,
    Comma,
    Done,
}
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Type,
    Crs,
    Features,
    Other,
}
impl<'a> Parser<'a> {
    pub fn new(feed: Feed, feature: &'a mut [u8], strings: &'a mut [u8]) -> Self {
        Self {
            frame: Frame::new(feature),
            strings,
            feed,
            phase: Phase::Start,
            field: Field::Other,
            features: false,
            collection: false,
            crs: false,
            count: 0,
            closed: false,
        }
    }
    pub fn feed(
        &mut self,
        bytes: &[u8],
        mut record: impl FnMut(Record<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let result = self.input(bytes, &mut record);
        if result.is_err() {
            self.closed = true;
        }
        result
    }
    /// Empty feature collections are valid; an error envelope or missing
    /// collection marker is not. A CRS appearing after records is checked too.
    pub fn finish(&mut self) -> Result<u32, Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.closed = true;
        if !matches!(self.phase, Phase::Done) {
            return Err(Error::UnexpectedEof);
        }
        if !self.collection || !self.features {
            return Err(Error::SchemaChanged);
        }
        Ok(self.count)
    }
    fn input(
        &mut self,
        bytes: &[u8],
        record: &mut impl FnMut(Record<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for &b in bytes {
            let mut again = true;
            while again {
                again = false;
                match self.phase {
                    Phase::Start => {
                        if whitespace(b) {
                            continue;
                        }
                        if b != b'{' {
                            return Err(Error::InvalidJson);
                        }
                        self.phase = Phase::KeyOrEnd;
                    }
                    Phase::KeyOrEnd if self.frame.len == 0 && b == b'}' => self.phase = Phase::Done,
                    Phase::KeyOrEnd | Phase::Key => {
                        if self.frame.len == 0 && whitespace(b) {
                            continue;
                        }
                        if self.frame.len == 0 && b != b'"' {
                            return Err(Error::InvalidJson);
                        }
                        if self.frame.push(b)?.0 {
                            let key = Value::parse_document(self.frame.bytes())?.string()?;
                            self.field = if key.decoded_eq_ascii("features") {
                                Field::Features
                            } else if key.decoded_eq_ascii("type") {
                                Field::Type
                            } else if key.decoded_eq_ascii("crs") {
                                Field::Crs
                            } else {
                                Field::Other
                            };
                            self.frame.clear();
                            self.phase = Phase::Colon;
                        }
                    }
                    Phase::Colon => {
                        if whitespace(b) {
                            continue;
                        }
                        if b != b':' {
                            return Err(Error::InvalidJson);
                        }
                        if self.field == Field::Features {
                            if self.features {
                                return Err(Error::SchemaChanged);
                            }
                            self.features = true;
                            self.phase = Phase::Array;
                        } else {
                            self.phase = Phase::Value;
                        }
                    }
                    Phase::Array => {
                        if whitespace(b) {
                            continue;
                        }
                        if b != b'[' {
                            return Err(Error::SchemaChanged);
                        }
                        self.phase = Phase::ItemOrEnd;
                    }
                    Phase::ItemOrEnd if self.frame.len == 0 && b == b']' => {
                        self.phase = Phase::Comma
                    }
                    Phase::ItemOrEnd | Phase::Item | Phase::Value => {
                        if self.frame.len == 0 && whitespace(b) {
                            continue;
                        }
                        if matches!(self.phase, Phase::ItemOrEnd | Phase::Item)
                            && self.frame.len == 0
                            && b != b'{'
                        {
                            return Err(Error::SchemaChanged);
                        }
                        let (done, consumed) = self.frame.push(b)?;
                        if done {
                            let value = Value::parse_document(self.frame.bytes())?;
                            if matches!(self.phase, Phase::Value) {
                                match self.field {
                                    Field::Type => {
                                        if self.collection
                                            || !value
                                                .string()?
                                                .decoded_eq_ascii("FeatureCollection")
                                        {
                                            return Err(Error::SchemaChanged);
                                        }
                                        self.collection = true;
                                    }
                                    Field::Crs => {
                                        if self.crs {
                                            return Err(Error::SchemaChanged);
                                        }
                                        self.crs = true;
                                        let crs = value.object()?;
                                        if !crs.required("type")?.string()?.decoded_eq_ascii("EPSG")
                                            || !crs
                                                .required("properties")?
                                                .object()?
                                                .required("code")?
                                                .string()?
                                                .decoded_eq_ascii("4326")
                                        {
                                            return Err(Error::SchemaChanged);
                                        }
                                    }
                                    _ => (),
                                }
                                self.phase = Phase::Comma;
                            } else {
                                let decoded = decode(self.feed, value, self.strings)?;
                                record(decoded)?;
                                self.count =
                                    self.count.checked_add(1).ok_or(Error::CountOverflow)?;
                                self.phase = Phase::ItemComma;
                            }
                            self.frame.clear();
                            again = !consumed;
                        }
                    }
                    Phase::Comma => {
                        if whitespace(b) {
                            continue;
                        }
                        self.phase = match b {
                            b',' => Phase::Key,
                            b'}' => Phase::Done,
                            _ => return Err(Error::InvalidJson),
                        };
                    }
                    Phase::ItemComma => {
                        if whitespace(b) {
                            continue;
                        }
                        self.phase = match b {
                            b',' => Phase::Item,
                            b']' => Phase::Comma,
                            _ => return Err(Error::InvalidJson),
                        };
                    }
                    Phase::Done => {
                        if !whitespace(b) {
                            return Err(Error::InvalidJson);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
fn whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t')
}

// Frame one JSON value. The bounded decoder subsequently validates its complete
// grammar, UTF-8, escapes and numbers; balanced braces alone never validate it.
struct Frame<'a> {
    data: &'a mut [u8],
    len: usize,
    stack: [u8; 16],
    depth: usize,
    string: bool,
    escaped: bool,
    scalar: bool,
}
impl<'a> Frame<'a> {
    fn new(data: &'a mut [u8]) -> Self {
        Self {
            data,
            len: 0,
            stack: [0; 16],
            depth: 0,
            string: false,
            escaped: false,
            scalar: false,
        }
    }
    fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
    fn clear(&mut self) {
        self.len = 0;
        self.depth = 0;
        self.string = false;
        self.escaped = false;
        self.scalar = false;
    }
    fn push(&mut self, b: u8) -> Result<(bool, bool), Error> {
        if self.scalar && (whitespace(b) || matches!(b, b',' | b'}' | b']')) {
            return Ok((true, false));
        }
        let first = self.len == 0;
        *self.data.get_mut(self.len).ok_or(Error::BufferTooSmall)? = b;
        self.len += 1;
        if self.string {
            if self.escaped {
                self.escaped = false;
            } else if b == b'\\' {
                self.escaped = true;
            } else if b == b'"' {
                self.string = false;
                return Ok((self.depth == 0, true));
            }
            return Ok((false, true));
        }
        match b {
            b'"' => self.string = true,
            b'{' | b'[' => {
                *self.stack.get_mut(self.depth).ok_or(Error::DepthExceeded)? =
                    if b == b'{' { b'}' } else { b']' };
                self.depth += 1;
            }
            b'}' | b']' => {
                if self.depth == 0 || self.stack[self.depth - 1] != b {
                    return Err(Error::InvalidJson);
                }
                self.depth -= 1;
                return Ok((self.depth == 0, true));
            }
            _ if first => self.scalar = true,
            _ => (),
        }
        Ok((false, true))
    }
}
fn text(data: &mut [u8], used: &mut usize, value: JsonString<'_>) -> Result<(usize, usize), Error> {
    let start = *used;
    let len = value.decode_into(&mut data[start..])?.len();
    *used += len;
    Ok((start, *used))
}
fn string(data: &[u8], range: (usize, usize)) -> &str {
    core::str::from_utf8(&data[range.0..range.1]).expect("decoded UTF-8")
}
fn payment(value: Option<Value<'_>>, data: &mut [u8]) -> Result<Option<bool>, Error> {
    match value {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(None),
        Some(v) => match v.string()?.decode_into(data)? {
            "" => Ok(None),
            "TAK" => Ok(Some(true)),
            "NIE" => Ok(Some(false)),
            _ => Err(Error::SchemaChanged),
        },
    }
}
fn decode<'a>(feed: Feed, value: Value<'_>, data: &'a mut [u8]) -> Result<Record<'a>, Error> {
    let feature = value.object()?;
    if !feature
        .required("type")?
        .string()?
        .decoded_eq_ascii("Feature")
    {
        return Err(Error::SchemaChanged);
    }
    let geometry = feature.required("geometry")?.object()?;
    if !geometry
        .required("type")?
        .string()?
        .decoded_eq_ascii("Point")
    {
        return Err(Error::SchemaChanged);
    }
    let mut coords = geometry.required("coordinates")?.array()?.iter();
    let point = Point {
        longitude: coords.next().ok_or(Error::SchemaChanged)??.f64()?,
        latitude: coords.next().ok_or(Error::SchemaChanged)??.f64()?,
    };
    if coords.next().is_some()
        || !(-180.0..=180.0).contains(&point.longitude)
        || !(-90.0..=90.0).contains(&point.latitude)
    {
        return Err(Error::SchemaChanged);
    }
    let properties = feature.required("properties")?.object()?;
    let mut used = 0;
    let id = text(data, &mut used, feature.required("id")?.string()?)?;
    if string(data, id).is_empty() {
        return Err(Error::SchemaChanged);
    }
    match feed {
        Feed::Stops => {
            let name = text(data, &mut used, properties.required("stop_name")?.string()?)?;
            let zone = text(data, &mut used, properties.required("zone")?.string()?)?;
            let route = text(
                data,
                &mut used,
                properties.required("route_type")?.string()?,
            )?;
            let lines = text(data, &mut used, properties.required("headsigns")?.string()?)?;
            Ok(Record::Stop(Stop {
                id: string(data, id),
                name: string(data, name),
                zone: string(data, zone),
                route_type: string(data, route),
                lines: string(data, lines),
                point,
            }))
        }
        Feed::ParkingMeters => {
            let street = text(data, &mut used, properties.required("street")?.string()?)?;
            let zone = text(data, &mut used, properties.required("zone")?.string()?)?;
            let coins = payment(properties.get("bilon")?, &mut data[used..])?;
            let card = payment(properties.get("karta")?, &mut data[used..])?;
            let blik = payment(properties.get("blik")?, &mut data[used..])?;
            let peka = payment(properties.get("peka")?, &mut data[used..])?;
            Ok(Record::ParkingMeter(ParkingMeter {
                id: string(data, id),
                street: string(data, street),
                zone: string(data, zone),
                point,
                coins,
                card,
                blik,
                peka,
            }))
        }
    }
}
impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        match error {
            JsonError::InvalidUtf8 => Self::InvalidUtf8,
            JsonError::BufferTooSmall => Self::BufferTooSmall,
            JsonError::NestingTooDeep => Self::DepthExceeded,
            JsonError::MissingField(_) | JsonError::DuplicateField(_) | JsonError::WrongType => {
                Self::SchemaChanged
            }
            _ => Self::InvalidJson,
        }
    }
}
