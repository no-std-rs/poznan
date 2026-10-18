use crate::Error;

/// A calendar date, independent of time zone. Waste dates are local civil dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, Error> {
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year.is_multiple_of(4)
                && (!year.is_multiple_of(100) || year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            _ => return Err(Error::InvalidDate),
        };
        if year == 0 || year > 9999 || day == 0 || day > days {
            return Err(Error::InvalidDate);
        }
        Ok(Self { year, month, day })
    }

    pub fn year(self) -> u16 {
        self.year
    }
    pub fn month(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }
}

impl core::fmt::Display for Date {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

pub(crate) fn month(value: &str) -> Result<(u16, u8), Error> {
    let (m, y) = value.split_once('.').ok_or(Error::InvalidDate)?;
    if !(1..=2).contains(&m.len())
        || y.len() != 4
        || !m.bytes().chain(y.bytes()).all(|b| b.is_ascii_digit())
    {
        return Err(Error::InvalidDate);
    }
    let year = y.parse().map_err(|_| Error::InvalidDate)?;
    let month = m.parse().map_err(|_| Error::InvalidDate)?;
    Date::new(year, month, 1)?;
    Ok((year, month))
}
