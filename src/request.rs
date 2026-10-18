use crate::Error;

pub const ORIGIN: &str = "https://www.poznan.pl";

/// Values are UTF-8, unencoded, and opaque: do not split building groups or
/// remove punctuation from an object ID. Street names should come from lookup.
#[derive(Debug, Clone, Copy)]
pub enum Query<'a> {
    Streets {
        contains: &'a str,
    },
    Numbers {
        street: &'a str,
        contains: &'a str,
    },
    Objects {
        street: &'a str,
        number: &'a str,
        contains: &'a str,
    },
    Schedule {
        street: &'a str,
        number: &'a str,
        object: Option<&'a str>,
    },
}

impl Query<'_> {
    /// Write the request target for a GET on [`ORIGIN`]. Validates capacity
    /// before modifying `out`. An object-only schedule uses empty street/number.
    pub fn path(self, out: &mut [u8]) -> Result<&str, Error> {
        let mut params = [("", ""); 5];
        let (base, n) = match self {
            Self::Streets { contains } => {
                if contains.chars().count() < 3 {
                    return Err(Error::InvalidQuery);
                }
                params[0] = ("ws_street", contains);
                ("/mim/public/odpady/waste_street_lookup.html", 1)
            }
            Self::Numbers { street, contains } => {
                nonempty(street)?;
                nonempty(contains)?;
                params[..2].copy_from_slice(&[("ws_street", street), ("ws_number", contains)]);
                ("/mim/public/odpady/waste_number_lookup.html", 2)
            }
            Self::Objects {
                street,
                number,
                contains,
            } => {
                nonempty(street)?;
                nonempty(number)?;
                params[..3].copy_from_slice(&[
                    ("ws_street", street),
                    ("ws_number", number),
                    ("ws_object", contains),
                ]);
                ("/mim/public/odpady/waste_object_lookup.html", 3)
            }
            Self::Schedule {
                street,
                number,
                object,
            } => {
                if let Some(id) = object {
                    nonempty(id)?;
                }
                if object.is_none() || !street.is_empty() || !number.is_empty() {
                    nonempty(street)?;
                    nonempty(number)?;
                }
                params.copy_from_slice(&[
                    ("action", "search"),
                    ("co", "waste_schedule"),
                    ("ws_street", street),
                    ("ws_number", number),
                    ("ws_object", object.unwrap_or("")),
                ]);
                ("/mim/odpady/harmonogramy.html", 5)
            }
        };
        let mut len = base.len();
        for (key, value) in &params[..n] {
            if value.chars().any(char::is_control) {
                return Err(Error::InvalidQuery);
            }
            len = len
                .checked_add(key.len() + 2)
                .ok_or(Error::BufferTooSmall)?;
            for b in value.bytes() {
                len = len
                    .checked_add(if unreserved(b) { 1 } else { 3 })
                    .ok_or(Error::BufferTooSmall)?;
            }
        }
        if len > out.len() {
            return Err(Error::BufferTooSmall);
        }
        out[..base.len()].copy_from_slice(base.as_bytes());
        let mut pos = base.len();
        for (i, (key, value)) in params[..n].iter().enumerate() {
            out[pos] = if i == 0 { b'?' } else { b'&' };
            pos += 1;
            out[pos..pos + key.len()].copy_from_slice(key.as_bytes());
            pos += key.len();
            out[pos] = b'=';
            pos += 1;
            for b in value.bytes() {
                if unreserved(b) {
                    out[pos] = b;
                    pos += 1;
                } else {
                    out[pos..pos + 3].copy_from_slice(&[b'%', hex(b >> 4), hex(b & 15)]);
                    pos += 3;
                }
            }
        }
        core::str::from_utf8(&out[..pos]).map_err(|_| Error::InvalidUtf8)
    }
}

fn nonempty(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidQuery)
    } else {
        Ok(())
    }
}
fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}
fn hex(b: u8) -> u8 {
    b"0123456789ABCDEF"[usize::from(b)]
}
