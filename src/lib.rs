//! Streaming Poznań waste lookups and pickup schedules, without `std` or `alloc`.
//!
//! Transport is caller-owned. Feed decoded HTTP body bytes to [`LookupParser`]
//! or [`ScheduleParser`] in arbitrary chunks. All variable storage is borrowed
//! from the caller; insufficient capacity is an error. Events are provisional
//! until both the parser's `finish` and the transport succeed. A flash-backed
//! consumer should stage its update and commit only after both succeed.
//!
//! This parses the city's observed HTML, not a general HTML document. Source
//! layout changes must fail visibly rather than publish an empty calendar.

#![no_std]
#![forbid(unsafe_code)]

mod date;
mod html;
mod lookup;
mod request;
mod waste;

pub use date::Date;
pub use lookup::LookupParser;
pub use request::{ORIGIN, Query};
pub use waste::{Event, Fraction, Object, ScheduleParser, Stats};

/// Errors poison a parser. Discard it and its provisional output after failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    BufferTooSmall,
    InvalidUtf8,
    InvalidHtml,
    InvalidEntity,
    InvalidDate,
    InvalidDays,
    InvalidQuery,
    SchemaChanged,
    UnexpectedEof,
    NoSchedule,
    Sink,
    Closed,
    CountOverflow,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for Error {}
