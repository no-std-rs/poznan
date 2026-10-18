use crate::{
    Error,
    html::{self, Html, Token},
};

/// Parses the small HTML lists returned by all three autocomplete endpoints.
/// `tag` holds one relevant tag; `value` holds one decoded result. Items are
/// borrowed only for the callback. Building groups and punctuation are retained.
pub struct LookupParser<'a> {
    html: Html<'a>,
    state: State<'a>,
    closed: bool,
}

struct State<'a> {
    value: &'a mut [u8],
    len: usize,
    in_list: bool,
    in_item: bool,
    seen_list: bool,
    nonempty: bool,
    count: u32,
}

impl<'a> LookupParser<'a> {
    pub fn new(tag: &'a mut [u8], value: &'a mut [u8]) -> Self {
        Self {
            html: Html::new(tag),
            state: State {
                value,
                len: 0,
                in_list: false,
                in_item: false,
                seen_list: false,
                nonempty: false,
                count: 0,
            },
            closed: false,
        }
    }

    pub fn feed(
        &mut self,
        input: &[u8],
        mut item: impl FnMut(&str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.state.nonempty |= input.iter().any(|b| !b.is_ascii_whitespace());
        let state = &mut self.state;
        let result = self.html.feed(input, |token| state.token(token, &mut item));
        if result.is_err() {
            self.closed = true;
        }
        result
    }

    /// An empty body or the city's no-match item is a successful zero-item list.
    pub fn finish(&mut self) -> Result<u32, Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        self.closed = true;
        self.html.finish()?;
        if self.state.in_item || self.state.in_list {
            return Err(Error::UnexpectedEof);
        }
        if self.state.nonempty && !self.state.seen_list {
            return Err(Error::SchemaChanged);
        }
        Ok(self.state.count)
    }
}

impl State<'_> {
    fn token(
        &mut self,
        token: Token<'_>,
        item: &mut impl FnMut(&str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        match token {
            Token::Byte(b) => {
                self.nonempty |= !b.is_ascii_whitespace();
                if self.in_item {
                    html::push(self.value, &mut self.len, b)?;
                } else if !b.is_ascii_whitespace() {
                    return Err(Error::SchemaChanged);
                }
            }
            Token::Tag(tag) if tag.is("ul") => {
                self.nonempty = true;
                if tag.closing() {
                    if !self.in_list || self.in_item {
                        return Err(Error::InvalidHtml);
                    }
                    self.in_list = false;
                } else {
                    if self.seen_list {
                        return Err(Error::SchemaChanged);
                    }
                    self.seen_list = true;
                    self.in_list = true;
                }
            }
            Token::Tag(tag) if tag.is("li") => {
                if !self.in_list {
                    return Err(Error::InvalidHtml);
                }
                if tag.closing() {
                    if !self.in_item {
                        return Err(Error::InvalidHtml);
                    }
                    let value = html::text(self.value, self.len)?;
                    if value.is_empty() {
                        return Err(Error::SchemaChanged);
                    }
                    if !value.starts_with("Brak dopasowania") && !value.starts_with("Brak wyników")
                    {
                        item(value)?;
                        self.count = self.count.checked_add(1).ok_or(Error::CountOverflow)?;
                    }
                    self.in_item = false;
                } else {
                    if self.in_item {
                        return Err(Error::InvalidHtml);
                    }
                    self.in_item = true;
                    self.len = 0;
                }
            }
            Token::Tag(tag) if tag.is("br") && self.in_item => {
                html::push(self.value, &mut self.len, b' ')?
            }
            Token::Tag(_) => {}
        }
        Ok(())
    }
}
