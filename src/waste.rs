use crate::{
    Date, Error, date,
    html::{self, Html, Token},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fraction {
    Mixed,
    Paper,
    MetalsAndPlastics,
    Glass,
    Bio,
    Bulky,
    ChristmasTrees,
}

impl Fraction {
    pub fn code(self) -> &'static str {
        match self {
            Self::Mixed => "ZM",
            Self::Paper => "PAP",
            Self::MetalsAndPlastics => "TWS",
            Self::Glass => "SZK",
            Self::Bio => "BIO",
            Self::Bulky => "WYST",
            Self::ChristmasTrees => "CHO",
        }
    }
    fn from_class(class: &str) -> Result<Self, Error> {
        let mut result = None;
        for word in class.split_ascii_whitespace() {
            let fraction = match word {
                "waste-ZM" => Self::Mixed,
                "waste-PAP" => Self::Paper,
                "waste-TWS" => Self::MetalsAndPlastics,
                "waste-SZK" => Self::Glass,
                "waste-BIO" => Self::Bio,
                "waste-WYST" => Self::Bulky,
                "waste-CHO" => Self::ChristmasTrees,
                "waste-cell" | "waste-type" | "waste-column" => continue,
                v if v.starts_with("waste-") => return Err(Error::SchemaChanged),
                _ => continue,
            };
            if result.replace(fraction).is_some() {
                return Err(Error::SchemaChanged);
            }
        }
        result.ok_or(Error::SchemaChanged)
    }
}

/// One city's service object. IDs and the address are opaque strings. In
/// particular `1/2275` and `1/2275.` must never be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object<'a> {
    pub address: &'a str,
    pub sector: &'a str,
    pub property_kind: &'a str,
    pub id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    BeginObject(Object<'a>),
    Pickup {
        object: Object<'a>,
        fraction: Fraction,
        date: Date,
    },
    EndObject(Object<'a>),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub objects: u32,
    pub pickups: u32,
}

/// Incremental parser for the city's waste schedule tables. Suggested starting
/// capacities: 512 bytes for `tag`, 256 for `text`, 512 for `metadata`.
/// These are explicit bounds, not assumptions about every future source record.
pub struct ScheduleParser<'a> {
    html: Html<'a>,
    state: State<'a>,
    closed: bool,
}

#[derive(Clone, Copy)]
enum Cell {
    Kind,
    Dates(u16, u8),
}

struct State<'a> {
    text: &'a mut [u8],
    text_len: usize,
    metadata: &'a mut [u8],
    metadata_len: usize,
    fields: [(usize, usize); 4],
    field_count: usize,
    meta_depth: u32,
    ready: bool,
    scope: u32,
    seen_scope: bool,
    closed_scope: bool,
    table: bool,
    row: bool,
    cell: Option<Cell>,
    fraction: Option<Fraction>,
    month_cells: u32,
    waste_rows: u32,
    stats: Stats,
}

impl<'a> ScheduleParser<'a> {
    pub fn new(tag: &'a mut [u8], text: &'a mut [u8], metadata: &'a mut [u8]) -> Self {
        Self {
            html: Html::new(tag),
            state: State {
                text,
                text_len: 0,
                metadata,
                metadata_len: 0,
                fields: [(0, 0); 4],
                field_count: 0,
                meta_depth: 0,
                ready: false,
                scope: 0,
                seen_scope: false,
                closed_scope: false,
                table: false,
                row: false,
                cell: None,
                fraction: None,
                month_cells: 0,
                waste_rows: 0,
                stats: Stats::default(),
            },
            closed: false,
        }
    }

    pub fn feed(
        &mut self,
        input: &[u8],
        mut event: impl FnMut(Event<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let state = &mut self.state;
        let result = self
            .html
            .feed(input, |token| state.token(token, &mut event));
        if result.is_err() {
            self.closed = true;
        }
        result
    }

    /// Only after this succeeds AND the HTTP transfer succeeds can events be
    /// committed. An unmatched address is `NoSchedule`, not a blank calendar.
    pub fn finish(&mut self) -> Result<Stats, Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.closed = true;
        self.html.finish()?;
        let s = &self.state;
        if s.scope != 0 || s.meta_depth != 0 || s.table || s.row || s.cell.is_some() || s.ready {
            return Err(Error::UnexpectedEof);
        }
        if !s.seen_scope {
            return Err(Error::SchemaChanged);
        }
        if !s.closed_scope {
            return Err(Error::UnexpectedEof);
        }
        if s.stats.objects == 0 {
            return Err(Error::NoSchedule);
        }
        Ok(s.stats)
    }
}

impl State<'_> {
    fn object(&self) -> Object<'_> {
        let field = |i: usize| {
            let (start, end) = self.fields[i];
            core::str::from_utf8(&self.metadata[start..end]).expect("validated metadata")
        };
        Object {
            address: field(0),
            sector: field(1),
            property_kind: field(2),
            id: field(3),
        }
    }

    fn metadata_field(&mut self) -> Result<(), Error> {
        if self.field_count >= 4 {
            return Err(Error::SchemaChanged);
        }
        let text = html::text(self.text, self.text_len)?;
        let value = match self.field_count {
            0 => text,
            1 => text
                .strip_prefix("Sektor:")
                .ok_or(Error::SchemaChanged)?
                .trim(),
            2 => text
                .strip_prefix("Rodzaj nieruchomości:")
                .ok_or(Error::SchemaChanged)?
                .trim(),
            3 => text
                .strip_prefix("Numer obiektu:")
                .ok_or(Error::SchemaChanged)?
                .trim(),
            _ => unreachable!(),
        };
        if value.is_empty() {
            return Err(Error::SchemaChanged);
        }
        let end = self
            .metadata_len
            .checked_add(value.len())
            .ok_or(Error::BufferTooSmall)?;
        self.metadata
            .get_mut(self.metadata_len..end)
            .ok_or(Error::BufferTooSmall)?
            .copy_from_slice(value.as_bytes());
        self.fields[self.field_count] = (self.metadata_len, end);
        self.metadata_len = end;
        self.field_count += 1;
        Ok(())
    }

    fn token(
        &mut self,
        token: Token<'_>,
        emit: &mut impl FnMut(Event<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let tag = match token {
            Token::Byte(b) => {
                if self.meta_depth >= 2 || matches!(self.cell, Some(Cell::Dates(..))) {
                    html::push(self.text, &mut self.text_len, b)?;
                }
                return Ok(());
            }
            Token::Tag(tag) => tag,
        };
        if tag.is("div") {
            if !tag.closing() {
                if tag.class("waste-schedule")? {
                    if self.seen_scope {
                        return Err(Error::SchemaChanged);
                    }
                    self.seen_scope = true;
                    self.scope = 1;
                } else if self.scope != 0 {
                    self.scope = self.scope.checked_add(1).ok_or(Error::CountOverflow)?;
                    if tag.class("address-container")? {
                        if self.table || self.meta_depth != 0 || self.ready {
                            return Err(Error::SchemaChanged);
                        }
                        self.meta_depth = 1;
                        self.metadata_len = 0;
                        self.field_count = 0;
                    } else if self.meta_depth != 0 {
                        self.meta_depth =
                            self.meta_depth.checked_add(1).ok_or(Error::CountOverflow)?;
                        if self.meta_depth == 2 {
                            self.text_len = 0;
                        }
                    }
                }
            } else if self.scope != 0 {
                if self.meta_depth == 2 {
                    self.metadata_field()?;
                }
                if self.meta_depth != 0 {
                    self.meta_depth -= 1;
                    if self.meta_depth == 0 {
                        if self.field_count != 4 {
                            return Err(Error::SchemaChanged);
                        }
                        self.ready = true;
                    }
                }
                self.scope -= 1;
                if self.scope == 0 {
                    if self.table || self.ready {
                        return Err(Error::SchemaChanged);
                    }
                    self.closed_scope = true;
                }
            }
            return Ok(());
        }
        if self.scope == 0 {
            return Ok(());
        }
        if tag.is("br") && self.meta_depth >= 2 {
            html::push(self.text, &mut self.text_len, b' ')?;
        }
        if tag.is("table") {
            if tag.closing() {
                if self.table {
                    if self.row || self.cell.is_some() || self.waste_rows == 0 {
                        return Err(Error::SchemaChanged);
                    }
                    emit(Event::EndObject(self.object()))?;
                    self.stats.objects = self
                        .stats
                        .objects
                        .checked_add(1)
                        .ok_or(Error::CountOverflow)?;
                    self.table = false;
                    self.ready = false;
                }
            } else if tag
                .attr("id")?
                .is_some_and(|id| id.starts_with("schedule_"))
            {
                let id = tag.attr("id")?.ok_or(Error::SchemaChanged)?;
                let suffix = &id["schedule_".len()..];
                if suffix.is_empty()
                    || !suffix.bytes().all(|b| b.is_ascii_digit())
                    || !self.ready
                    || self.table
                    || self.meta_depth != 0
                {
                    return Err(Error::SchemaChanged);
                }
                self.table = true;
                self.waste_rows = 0;
                emit(Event::BeginObject(self.object()))?;
            } else if self.table {
                return Err(Error::SchemaChanged);
            }
            return Ok(());
        }
        if !self.table {
            return Ok(());
        }
        if tag.is("tr") {
            if tag.closing() {
                if !self.row || self.cell.is_some() {
                    return Err(Error::InvalidHtml);
                }
                if self.fraction.is_some() {
                    if self.month_cells == 0 {
                        return Err(Error::SchemaChanged);
                    }
                    self.waste_rows = self.waste_rows.checked_add(1).ok_or(Error::CountOverflow)?;
                }
                self.row = false;
            } else {
                if self.row {
                    return Err(Error::InvalidHtml);
                }
                self.row = true;
                self.fraction = None;
                self.month_cells = 0;
            }
        } else if tag.is("td") {
            if !self.row {
                return Err(Error::InvalidHtml);
            }
            if tag.closing() {
                match self.cell.take().ok_or(Error::InvalidHtml)? {
                    Cell::Kind => {}
                    Cell::Dates(year, month) => self.days(year, month, emit)?,
                }
            } else {
                if self.cell.is_some() {
                    return Err(Error::InvalidHtml);
                }
                self.text_len = 0;
                if tag.class("waste-type")? {
                    if self.fraction.is_some() || self.month_cells != 0 {
                        return Err(Error::SchemaChanged);
                    }
                    self.fraction = Some(Fraction::from_class(
                        tag.attr("class")?.ok_or(Error::SchemaChanged)?,
                    )?);
                    self.cell = Some(Cell::Kind);
                } else if tag.class("waste-date")? {
                    if self.fraction.is_none() {
                        return Err(Error::SchemaChanged);
                    }
                    let (year, month) =
                        date::month(tag.attr("data-value")?.ok_or(Error::SchemaChanged)?)?;
                    self.cell = Some(Cell::Dates(year, month));
                    self.month_cells = self
                        .month_cells
                        .checked_add(1)
                        .ok_or(Error::CountOverflow)?;
                } else {
                    return Err(Error::SchemaChanged);
                }
            }
        }
        Ok(())
    }

    fn days(
        &mut self,
        year: u16,
        month: u8,
        emit: &mut impl FnMut(Event<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let text = html::text(self.text, self.text_len)?;
        let mut mask = 0u32;
        if !text.is_empty() {
            // Validate the complete cell before emitting any of its dates.
            for part in text.split(',') {
                let part = part.trim();
                if part.is_empty() || part.len() > 2 || !part.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Error::InvalidDays);
                }
                let day = part.parse::<u8>().map_err(|_| Error::InvalidDays)?;
                Date::new(year, month, day)?;
                let bit = 1u32 << day;
                if mask & bit != 0 {
                    return Err(Error::InvalidDays);
                }
                mask |= bit;
            }
        }
        let fraction = self.fraction.ok_or(Error::SchemaChanged)?;
        for day in 1..=31 {
            if mask & (1 << day) != 0 {
                emit(Event::Pickup {
                    object: self.object(),
                    fraction,
                    date: Date::new(year, month, day)?,
                })?;
                self.stats.pickups = self
                    .stats
                    .pickups
                    .checked_add(1)
                    .ok_or(Error::CountOverflow)?;
            }
        }
        Ok(())
    }
}
