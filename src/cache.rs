//! Bounded, two-bank NOR flash cache. No heap, filesystem, or response buffering.
//!
//! [`Update::finish`] validates the parser and returns an unpublished [`Prepared`].
//! Call [`Prepared::commit`] only after the HTTP body AND transport have completed
//! successfully. Dropping either value leaves the previous snapshot available.
//! Banks must be reserved for this cache; do not share them with other writers.
use crate::{Date, Event, Fraction, Object, ScheduleParser, Stats};
use embedded_storage::nor_flash::{NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash};

const HEADER: usize = 64;
const MAGIC: &[u8; 8] = b"PZNW0001";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Flash(NorFlashErrorKind),
    Parse(crate::Error),
    Geometry,
    Full,
    Corrupt,
    BufferTooSmall,
    GenerationExhausted,
    Closed,
    Sink,
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for Error {}
fn io<E: NorFlashError>(e: E) -> Error {
    Error::Flash(e.kind())
}

/// Two nonoverlapping, equally sized, erase-aligned banks. Each contains one
/// complete schedule (all service objects returned by a single query).
#[derive(Debug, Clone, Copy)]
pub struct Cache {
    banks: [u32; 2],
    bank_size: u32,
}

/// Validated header. Reopen after reboot to verify both header and payload CRCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    bank: u32,
    generation: u64,
    fetched_at: u64,
    bytes: u32,
    stats: Stats,
    crc: u32,
}
impl Snapshot {
    pub fn generation(self) -> u64 {
        self.generation
    }
    /// Application-supplied Unix seconds, for freshness decisions while offline.
    pub fn fetched_at(self) -> u64 {
        self.fetched_at
    }
    /// Encoded payload bytes; excludes the 64-byte header and write padding.
    pub fn bytes(self) -> u32 {
        self.bytes
    }
    pub fn stats(self) -> Stats {
        self.stats
    }

    /// Replay the same events as the HTML parser, using bounded read and object
    /// buffers. A callback failure stops replay. Check the final result before
    /// publishing replayed data: CRC and counts are checked again at the end.
    pub fn replay<F: ReadNorFlash>(
        self,
        flash: &mut F,
        read: &mut [u8],
        metadata: &mut [u8],
        mut event: impl FnMut(Event<'_>) -> Result<(), crate::Error>,
    ) -> Result<Stats, Error> {
        read_geometry::<F>(read)?;
        let mut cursor = Reader {
            flash,
            buffer: read,
            snapshot: self,
            position: 0,
            start: 0,
            filled: 0,
            crc: !0,
        };
        let mut fields = [0; 4];
        let mut active = false;
        let mut stats = Stats::default();
        while cursor.position < self.bytes {
            let tag = cursor.byte()?;
            match tag {
                1 if !active => {
                    let mut total = 0usize;
                    for field in &mut fields {
                        let mut bytes = [0; 2];
                        cursor.exact(&mut bytes)?;
                        total = total
                            .checked_add(u16::from_le_bytes(bytes) as usize)
                            .ok_or(Error::Corrupt)?;
                        *field = total;
                    }
                    if total > metadata.len() {
                        return Err(Error::BufferTooSmall);
                    }
                    cursor.exact(&mut metadata[..total])?;
                    let object = object(metadata, fields)?;
                    event(Event::BeginObject(object)).map_err(|_| Error::Sink)?;
                    stats.objects = stats.objects.checked_add(1).ok_or(Error::Corrupt)?;
                    active = true;
                }
                2..=8 if active => {
                    let mut bytes = [0; 4];
                    cursor.exact(&mut bytes)?;
                    let date =
                        Date::new(u16::from_le_bytes([bytes[0], bytes[1]]), bytes[2], bytes[3])
                            .map_err(|_| Error::Corrupt)?;
                    let fraction = FRACTIONS[usize::from(tag - 2)];
                    event(Event::Pickup {
                        object: object(metadata, fields)?,
                        fraction,
                        date,
                    })
                    .map_err(|_| Error::Sink)?;
                    stats.pickups = stats.pickups.checked_add(1).ok_or(Error::Corrupt)?;
                }
                9 if active => {
                    event(Event::EndObject(object(metadata, fields)?)).map_err(|_| Error::Sink)?;
                    active = false;
                }
                _ => return Err(Error::Corrupt),
            }
        }
        if active || stats != self.stats || !cursor.crc != self.crc {
            return Err(Error::Corrupt);
        }
        Ok(stats)
    }
}

const FRACTIONS: [Fraction; 7] = [
    Fraction::Mixed,
    Fraction::Paper,
    Fraction::MetalsAndPlastics,
    Fraction::Glass,
    Fraction::Bio,
    Fraction::Bulky,
    Fraction::ChristmasTrees,
];
fn object(metadata: &[u8], fields: [usize; 4]) -> Result<Object<'_>, Error> {
    let field = |a, b| core::str::from_utf8(&metadata[a..b]).map_err(|_| Error::Corrupt);
    Ok(Object {
        address: field(0, fields[0])?,
        sector: field(fields[0], fields[1])?,
        property_kind: field(fields[1], fields[2])?,
        id: field(fields[2], fields[3])?,
    })
}

impl Cache {
    pub const fn new(banks: [u32; 2], bank_size: u32) -> Self {
        Self { banks, bank_size }
    }

    fn geometry<F: ReadNorFlash>(&self, flash: &F, buffer: &[u8]) -> Result<(), Error> {
        read_geometry::<F>(buffer)?;
        if self.bank_size < HEADER as u32 || !self.bank_size.is_multiple_of(F::READ_SIZE as u32) {
            return Err(Error::Geometry);
        }
        let mut ends = [0; 2];
        for (i, start) in self.banks.iter().enumerate() {
            ends[i] = start.checked_add(self.bank_size).ok_or(Error::Geometry)?;
            if !start.is_multiple_of(F::READ_SIZE as u32) || ends[i] as usize > flash.capacity() {
                return Err(Error::Geometry);
            }
        }
        if self.banks[0] < ends[1] && self.banks[1] < ends[0] {
            return Err(Error::Geometry);
        }
        Ok(())
    }

    /// Scan both banks with streaming CRC validation. A torn/corrupt bank is
    /// ignored; a storage I/O error is reported. Blank flash returns `None`.
    pub fn open<F: ReadNorFlash>(
        &self,
        flash: &mut F,
        scratch: &mut [u8],
    ) -> Result<Option<Snapshot>, Error> {
        self.geometry(flash, scratch)?;
        let mut best: Option<Snapshot> = None;
        for bank in self.banks {
            let mut header = [0; HEADER];
            flash.read(bank, &mut header).map_err(io)?;
            if &header[..8] != MAGIC || crc(&header[..40]) != u32_at(&header, 40) {
                continue;
            }
            let snapshot = Snapshot {
                bank,
                generation: u64_at(&header, 8),
                fetched_at: u64_at(&header, 16),
                bytes: u32_at(&header, 24),
                stats: Stats {
                    objects: u32_at(&header, 28),
                    pickups: u32_at(&header, 32),
                },
                crc: u32_at(&header, 36),
            };
            if snapshot.generation == 0
                || snapshot.stats.objects == 0
                || snapshot.bytes == 0
                || snapshot.bytes > self.bank_size - HEADER as u32
            {
                continue;
            }
            let mut reader = Reader {
                flash,
                buffer: scratch,
                snapshot,
                position: 0,
                start: 0,
                filled: 0,
                crc: !0,
            };
            while reader.position < snapshot.bytes {
                reader.byte()?;
            }
            if !reader.crc == snapshot.crc {
                match best {
                    Some(old) if old.generation == snapshot.generation => {
                        return Err(Error::Corrupt);
                    }
                    Some(old) if old.generation > snapshot.generation => (),
                    _ => best = Some(snapshot),
                }
            }
        }
        Ok(best)
    }

    /// Erase the inactive bank and begin a bounded streaming update. A 256-byte
    /// write buffer works with ESP32's 4-byte write / 4096-byte erase geometry.
    /// Overflow, parser failure or dropping the update never publishes it.
    pub fn update<'a, F: NorFlash>(
        &self,
        flash: &'a mut F,
        buffers: Buffers<'a>,
        fetched_at: u64,
    ) -> Result<Update<'a, F>, Error> {
        self.geometry(flash, buffers.write)?;
        if F::WRITE_SIZE == 0
            || F::ERASE_SIZE == 0
            || F::ERASE_SIZE > u32::MAX as usize
            || !HEADER.is_multiple_of(F::WRITE_SIZE)
            || !buffers.write.len().is_multiple_of(F::WRITE_SIZE)
            || !self.bank_size.is_multiple_of(F::ERASE_SIZE as u32)
            || !self.bank_size.is_multiple_of(F::WRITE_SIZE as u32)
            || self.banks.iter().any(|b| {
                !b.is_multiple_of(F::ERASE_SIZE as u32) || !b.is_multiple_of(F::WRITE_SIZE as u32)
            })
        {
            return Err(Error::Geometry);
        }
        let old = self.open(flash, buffers.write)?;
        let bank = match old {
            Some(s) if s.bank == self.banks[0] => self.banks[1],
            _ => self.banks[0],
        };
        let generation = old
            .map_or(Some(1), |s| s.generation.checked_add(1))
            .ok_or(Error::GenerationExhausted)?;
        flash.erase(bank, bank + self.bank_size).map_err(io)?;
        let snapshot = Snapshot {
            bank,
            generation,
            fetched_at,
            bytes: 0,
            stats: Stats::default(),
            crc: 0,
        };
        Ok(Update {
            parser: ScheduleParser::new(buffers.tag, buffers.text, buffers.metadata),
            stage: Stage {
                flash,
                buffer: buffers.write,
                used: 0,
                written: 0,
                limit: self.bank_size - HEADER as u32,
                crc: !0,
                snapshot,
            },
            closed: false,
        })
    }
}

/// All variable RAM is supplied by the caller. Suggested capacities total 1536 B.
pub struct Buffers<'a> {
    pub tag: &'a mut [u8],
    pub text: &'a mut [u8],
    pub metadata: &'a mut [u8],
    pub write: &'a mut [u8],
}

pub struct Update<'a, F: NorFlash> {
    parser: ScheduleParser<'a>,
    stage: Stage<'a, F>,
    closed: bool,
}
impl<'a, F: NorFlash> Update<'a, F> {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let mut sink_error = None;
        let stage = &mut self.stage;
        let parsed = self.parser.feed(bytes, |event| {
            stage.event(event).map_err(|e| {
                sink_error = Some(e);
                crate::Error::Sink
            })
        });
        if let Err(e) = parsed {
            self.closed = true;
            return Err(sink_error.unwrap_or(Error::Parse(e)));
        }
        Ok(())
    }
    /// Validate EOF and flush the payload, leaving its header unpublished.
    /// The caller must still check the transport result before committing.
    pub fn finish(mut self) -> Result<Prepared<'a, F>, Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.stage.snapshot.stats = self.parser.finish().map_err(Error::Parse)?;
        self.stage.flush()?;
        self.stage.snapshot.crc = !self.stage.crc;
        Ok(Prepared {
            flash: self.stage.flash,
            snapshot: self.stage.snapshot,
        })
    }
}

#[must_use = "commit only after successful HTTP completion, or drop to abort"]
pub struct Prepared<'a, F: NorFlash> {
    flash: &'a mut F,
    snapshot: Snapshot,
}
impl<F: NorFlash> Prepared<'_, F> {
    /// Publish last, after a successful transfer. Each flash word is written
    /// once between erases. Header/payload CRCs detect interrupted writes on
    /// reopen; the old bank is untouched. This is integrity, not authentication.
    pub fn commit(self) -> Result<Snapshot, Error> {
        let s = self.snapshot;
        let mut h = [0xff; HEADER];
        h[..8].copy_from_slice(MAGIC);
        h[8..16].copy_from_slice(&s.generation.to_le_bytes());
        h[16..24].copy_from_slice(&s.fetched_at.to_le_bytes());
        h[24..28].copy_from_slice(&s.bytes.to_le_bytes());
        h[28..32].copy_from_slice(&s.stats.objects.to_le_bytes());
        h[32..36].copy_from_slice(&s.stats.pickups.to_le_bytes());
        h[36..40].copy_from_slice(&s.crc.to_le_bytes());
        let checksum = crc(&h[..40]);
        h[40..44].copy_from_slice(&checksum.to_le_bytes());
        self.flash.write(s.bank, &h).map_err(io)?;
        Ok(s)
    }
}
struct Stage<'a, F> {
    flash: &'a mut F,
    buffer: &'a mut [u8],
    used: usize,
    written: u32,
    limit: u32,
    crc: u32,
    snapshot: Snapshot,
}
impl<F: NorFlash> Stage<'_, F> {
    fn event(&mut self, event: Event<'_>) -> Result<(), Error> {
        match event {
            Event::BeginObject(o) => {
                self.push(&[1])?;
                let fields = [o.address, o.sector, o.property_kind, o.id];
                for field in fields {
                    let n = u16::try_from(field.len()).map_err(|_| Error::BufferTooSmall)?;
                    self.push(&n.to_le_bytes())?;
                }
                for field in fields {
                    self.push(field.as_bytes())?;
                }
            }
            Event::Pickup { fraction, date, .. } => {
                let tag = FRACTIONS
                    .iter()
                    .position(|f| *f == fraction)
                    .ok_or(Error::Corrupt)? as u8
                    + 2;
                let y = date.year().to_le_bytes();
                self.push(&[tag, y[0], y[1], date.month(), date.day()])?;
            }
            Event::EndObject(_) => self.push(&[9])?,
        }
        Ok(())
    }
    fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for &b in bytes {
            if self.snapshot.bytes >= self.limit {
                return Err(Error::Full);
            }
            self.buffer[self.used] = b;
            self.used += 1;
            self.snapshot.bytes += 1;
            self.crc = crc_byte(self.crc, b);
            if self.used == self.buffer.len() {
                self.flush()?;
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        if self.used == 0 {
            return Ok(());
        }
        let aligned = self.used.next_multiple_of(F::WRITE_SIZE);
        if aligned as u64 + u64::from(self.written) > u64::from(self.limit) {
            return Err(Error::Full);
        }
        self.buffer[self.used..aligned].fill(0xff);
        self.flash
            .write(
                self.snapshot.bank + HEADER as u32 + self.written,
                &self.buffer[..aligned],
            )
            .map_err(io)?;
        self.written += aligned as u32;
        self.used = 0;
        Ok(())
    }
}

fn read_geometry<F: ReadNorFlash>(buffer: &[u8]) -> Result<(), Error> {
    if F::READ_SIZE == 0
        || !HEADER.is_multiple_of(F::READ_SIZE)
        || buffer.is_empty()
        || !buffer.len().is_multiple_of(F::READ_SIZE)
        || buffer.len() > u32::MAX as usize
    {
        return Err(Error::Geometry);
    }
    Ok(())
}
struct Reader<'a, F> {
    flash: &'a mut F,
    buffer: &'a mut [u8],
    snapshot: Snapshot,
    position: u32,
    start: u32,
    filled: usize,
    crc: u32,
}
impl<F: ReadNorFlash> Reader<'_, F> {
    fn byte(&mut self) -> Result<u8, Error> {
        if self.position >= self.snapshot.bytes {
            return Err(Error::Corrupt);
        }
        if self.filled == 0 || self.position - self.start >= self.filled as u32 {
            self.start = self.position;
            let left = (self.snapshot.bytes - self.position) as usize;
            self.filled = left.min(self.buffer.len()).next_multiple_of(F::READ_SIZE);
            self.flash
                .read(
                    self.snapshot.bank + HEADER as u32 + self.start,
                    &mut self.buffer[..self.filled],
                )
                .map_err(io)?;
        }
        let b = self.buffer[(self.position - self.start) as usize];
        self.position += 1;
        self.crc = crc_byte(self.crc, b);
        Ok(b)
    }
    fn exact(&mut self, out: &mut [u8]) -> Result<(), Error> {
        for b in out {
            *b = self.byte()?;
        }
        Ok(())
    }
}
fn u32_at(h: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(h[i..i + 4].try_into().expect("fixed header field"))
}
fn u64_at(h: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(h[i..i + 8].try_into().expect("fixed header field"))
}
fn crc(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0, |c, &b| crc_byte(c, b))
}
fn crc_byte(mut crc: u32, b: u8) -> u32 {
    crc ^= u32::from(b);
    for _ in 0..8 {
        crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
    }
    crc
}
