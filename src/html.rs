//! Small tokenizer for this source. Irrelevant tags and raw script/style bodies
//! are skipped with constant state, including arbitrarily long attributes.
use crate::Error;

pub(crate) enum Token<'a> {
    Tag(Tag<'a>),
    Byte(u8),
}

#[derive(Clone, Copy)]
pub(crate) struct Tag<'a> {
    raw: &'a str,
}

impl<'a> Tag<'a> {
    pub fn closing(self) -> bool {
        self.raw.starts_with('/')
    }
    pub fn name(self) -> &'a str {
        self.raw
            .trim_start_matches('/')
            .split(|c: char| c.is_ascii_whitespace() || c == '/')
            .next()
            .unwrap_or("")
    }
    pub fn is(self, name: &str) -> bool {
        self.name().eq_ignore_ascii_case(name)
    }
    pub fn attr(self, wanted: &str) -> Result<Option<&'a str>, Error> {
        let mut rest = &self.raw[self.name().len() + usize::from(self.closing())..];
        let mut found = None;
        loop {
            rest = rest.trim_ascii_start();
            if rest.is_empty() || rest == "/" {
                return Ok(found);
            }
            let n = rest
                .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '/'))
                .unwrap_or(rest.len());
            if n == 0 {
                return Err(Error::InvalidHtml);
            }
            let key = &rest[..n];
            rest = rest[n..].trim_ascii_start();
            let value = if let Some(tail) = rest.strip_prefix('=') {
                rest = tail.trim_ascii_start();
                if let Some(q) = rest.chars().next().filter(|c| matches!(c, '\'' | '"')) {
                    rest = &rest[1..];
                    let n = rest.find(q).ok_or(Error::InvalidHtml)?;
                    let value = &rest[..n];
                    rest = &rest[n + 1..];
                    value
                } else {
                    let n = rest
                        .find(|c: char| c.is_ascii_whitespace())
                        .unwrap_or(rest.len());
                    if n == 0 {
                        return Err(Error::InvalidHtml);
                    }
                    let value = &rest[..n];
                    rest = &rest[n..];
                    value
                }
            } else {
                ""
            };
            if key.eq_ignore_ascii_case(wanted) {
                if found.is_some() {
                    return Err(Error::InvalidHtml);
                }
                found = Some(value);
            }
        }
    }
    pub fn class(self, name: &str) -> Result<bool, Error> {
        Ok(self
            .attr("class")?
            .is_some_and(|v| v.split_ascii_whitespace().any(|c| c == name)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Tag,
    Comment,
    Raw,
    RawEnd,
}

pub(crate) struct Html<'a> {
    tag: &'a mut [u8],
    len: usize,
    mode: Mode,
    quote: u8,
    named: bool,
    skip: bool,
    raw: &'static [u8],
    matched: usize,
    dashes: u8,
}

impl<'a> Html<'a> {
    pub fn new(tag: &'a mut [u8]) -> Self {
        Self {
            tag,
            len: 0,
            mode: Mode::Text,
            quote: 0,
            named: false,
            skip: false,
            raw: b"",
            matched: 0,
            dashes: 0,
        }
    }
    pub fn feed(
        &mut self,
        input: &[u8],
        mut emit: impl FnMut(Token<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for &b in input {
            match self.mode {
                Mode::Text if b == b'<' => {
                    self.mode = Mode::Tag;
                    self.len = 0;
                    self.quote = 0;
                    self.named = false;
                    self.skip = false;
                }
                Mode::Text => emit(Token::Byte(b))?,
                Mode::Comment => {
                    if b == b'>' && self.dashes >= 2 {
                        self.mode = Mode::Text;
                    }
                    self.dashes = if b == b'-' {
                        self.dashes.saturating_add(1)
                    } else {
                        0
                    };
                }
                Mode::Raw => {
                    if b.eq_ignore_ascii_case(&self.raw[self.matched]) {
                        self.matched += 1;
                        if self.matched == self.raw.len() {
                            self.mode = Mode::RawEnd;
                        }
                    } else {
                        self.matched = usize::from(b == b'<');
                    }
                }
                Mode::RawEnd => {
                    if b == b'>' {
                        self.mode = Mode::Text;
                        self.matched = 0;
                    } else if !b.is_ascii_whitespace() && b != b'/' {
                        self.mode = Mode::Raw;
                        self.matched = usize::from(b == b'<');
                    }
                }
                Mode::Tag => {
                    if self.quote == 0 && b == b'>' {
                        if !self.named {
                            self.choose_tag()?;
                        }
                        self.mode = Mode::Text;
                        if !self.skip {
                            let raw = core::str::from_utf8(&self.tag[..self.len])
                                .map_err(|_| Error::InvalidUtf8)?;
                            let tag = Tag { raw };
                            if !tag.closing() && (tag.is("script") || tag.is("style")) {
                                self.raw = if tag.is("script") {
                                    b"</script"
                                } else {
                                    b"</style"
                                };
                                self.matched = 0;
                                self.mode = Mode::Raw;
                            } else {
                                emit(Token::Tag(tag))?;
                            }
                        }
                        continue;
                    }
                    if !self.named && b.is_ascii_whitespace() {
                        self.choose_tag()?;
                    }
                    if !self.skip {
                        push(self.tag, &mut self.len, b)?;
                        if &self.tag[..self.len] == b"!--" {
                            self.mode = Mode::Comment;
                            self.dashes = 0;
                        }
                    }
                    if matches!(b, b'\'' | b'"') {
                        if self.quote == b {
                            self.quote = 0;
                        } else if self.quote == 0 {
                            self.quote = b;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn choose_tag(&mut self) -> Result<(), Error> {
        let name = core::str::from_utf8(&self.tag[..self.len])
            .map_err(|_| Error::InvalidUtf8)?
            .trim_start_matches('/');
        self.skip = ![
            "div", "table", "tr", "td", "li", "ul", "br", "script", "style",
        ]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n));
        self.named = true;
        Ok(())
    }
    pub fn finish(&self) -> Result<(), Error> {
        if self.mode == Mode::Text {
            Ok(())
        } else {
            Err(Error::UnexpectedEof)
        }
    }
}

pub(crate) fn push(out: &mut [u8], len: &mut usize, byte: u8) -> Result<(), Error> {
    *out.get_mut(*len).ok_or(Error::BufferTooSmall)? = byte;
    *len += 1;
    Ok(())
}

/// Decode relevant text in place. Validate UTF-8 even when no entities occur.
pub(crate) fn text(buf: &mut [u8], len: usize) -> Result<&str, Error> {
    let mut src = 0;
    let mut dst = 0;
    while src < len {
        if buf[src] != b'&' {
            buf[dst] = buf[src];
            dst += 1;
            src += 1;
            continue;
        }
        let end = buf[src + 1..len]
            .iter()
            .position(|b| *b == b';')
            .ok_or(Error::InvalidEntity)?
            + src
            + 1;
        let entity = core::str::from_utf8(&buf[src + 1..end]).map_err(|_| Error::InvalidUtf8)?;
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => ' ',
            "oacute" => 'ó',
            "Oacute" => 'Ó',
            v if v.starts_with('#') => {
                let (digits, radix) = if v
                    .as_bytes()
                    .get(1)
                    .is_some_and(|b| matches!(b, b'x' | b'X'))
                {
                    (&v[2..], 16)
                } else {
                    (&v[1..], 10)
                };
                let code = u32::from_str_radix(digits, radix).map_err(|_| Error::InvalidEntity)?;
                char::from_u32(code)
                    .filter(|c| *c != '\0')
                    .ok_or(Error::InvalidEntity)?
            }
            _ => return Err(Error::InvalidEntity),
        };
        let mut encoded = [0; 4];
        let value = ch.encode_utf8(&mut encoded).as_bytes();
        buf[dst..dst + value.len()].copy_from_slice(value);
        dst += value.len();
        src = end + 1;
    }
    core::str::from_utf8(&buf[..dst])
        .map(str::trim)
        .map_err(|_| Error::InvalidUtf8)
}
