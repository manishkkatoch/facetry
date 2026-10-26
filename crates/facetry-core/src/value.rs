//! A tiny JSON-shaped value tree and a small JSON parser for it.
//!
//! The spec reader ([`crate::spec_read`]) turns a [`Value`] into a [`crate::ChartSpec`]. The
//! tree has two producers: [`parse_json`] (JSON text: tests, tools) and, in `facetry-wasm`,
//! a walk over a JS object. Going through one plain tree is what lets the shipped wasm
//! skip serde entirely (see `docs/wasm-size.md`).

/// One JSON-like value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null` (and, from JS, `undefined`).
    Null,
    Bool(bool),
    /// An integer that fits `i64`. Kept apart from [`Value::Float`] because `1` and `1.0` are
    /// different to the reader: integer-only fields such as `decimals` accept the first.
    Int(i64),
    /// An integer above `i64::MAX`.
    Uint(u64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    /// Entries in input order. A key may repeat; the reader decides what that means.
    Object(Vec<(String, Value)>),
    /// Something a host passed that is not data (a JS function, symbol or BigInt). It matches
    /// no type, so the reader reports it at its path; the text names it.
    Other(&'static str),
}

impl Value {
    /// How serde-style errors name this value: `null`, ``integer `5` ``, `sequence`, ...
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => format!("boolean `{b}`"),
            Self::Int(i) => format!("integer `{i}`"),
            Self::Uint(u) => format!("integer `{u}`"),
            Self::Float(f) => format!("floating point `{f:?}`"),
            Self::Str(s) => format!("string {s:?}"),
            Self::Array(_) => "sequence".into(),
            Self::Object(_) => "map".into(),
            Self::Other(what) => (*what).into(),
        }
    }
}

/// Containers nested deeper than this are rejected, like `serde_json` does, so hostile input
/// cannot overflow the stack (and a cyclic JS object ends in an error instead of a trap).
pub const MAX_DEPTH: usize = 128;

/// A syntax error in JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    /// JSON pointer to where the parser was: the value being read, or for a broken separator
    /// the container around it.
    pub path: String,
    /// What went wrong, ending in ` at line L column C`.
    pub message: String,
}

/// Parses the first JSON value in `text`. Anything after it is ignored, as the serde-based
/// parser this replaced did. Error messages and positions follow `serde_json`'s wording.
pub fn parse_json(text: &str) -> Result<Value, JsonError> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
        path: Vec::new(),
    };
    p.value(0)
}

struct Parser<'a> {
    b: &'a [u8],
    /// Bytes consumed so far.
    i: usize,
    /// Keys and indices of the containers being read, as pointer segments.
    path: Vec<String>,
}

type Res<T> = Result<T, JsonError>;

impl Parser<'_> {
    /// An error whose reported position is just after byte `end` (a count of consumed bytes).
    fn fail<T>(&self, message: &str, end: usize) -> Res<T> {
        let end = end.min(self.b.len());
        let line_start = self.b[..end]
            .iter()
            .rposition(|&c| c == b'\n')
            .map_or(0, |n| n + 1);
        let line = 1 + self.b[..line_start].iter().filter(|&&c| c == b'\n').count();
        Err(JsonError {
            path: self.path.concat(),
            message: format!("{message} at line {line} column {}", end - line_start),
        })
    }

    /// An error at the byte just consumed.
    fn error<T>(&self, message: &str) -> Res<T> {
        self.fail(message, self.i)
    }

    /// An error at the byte about to be read.
    fn peek_error<T>(&self, message: &str) -> Res<T> {
        self.fail(message, self.i + 1)
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    /// Skips whitespace and returns the next byte without consuming it.
    fn skip_ws(&mut self) -> Option<u8> {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\t' | b'\r')) {
            self.i += 1;
        }
        self.peek()
    }

    /// Reads a value; `depth` is how many containers enclose it.
    fn value(&mut self, depth: usize) -> Res<Value> {
        let Some(c) = self.skip_ws() else {
            return self.peek_error("EOF while parsing a value");
        };
        match c {
            b'n' => self.ident("null", Value::Null),
            b't' => self.ident("true", Value::Bool(true)),
            b'f' => self.ident("false", Value::Bool(false)),
            b'-' | b'0'..=b'9' => self.number(),
            b'"' => {
                self.i += 1;
                self.string().map(Value::Str)
            }
            b'[' | b'{' => {
                if depth + 1 >= MAX_DEPTH {
                    return self.peek_error("recursion limit exceeded");
                }
                self.i += 1;
                if c == b'[' {
                    self.array(depth + 1)
                } else {
                    self.object(depth + 1)
                }
            }
            _ => self.peek_error("expected value"),
        }
    }

    /// `null`, `true` or `false`: the first byte was seen, the rest must match.
    fn ident(&mut self, word: &str, value: Value) -> Res<Value> {
        self.i += 1;
        for &want in &word.as_bytes()[1..] {
            match self.next() {
                None => return self.error("EOF while parsing a value"),
                Some(c) if c != want => return self.error("expected ident"),
                Some(_) => {}
            }
        }
        Ok(value)
    }

    fn array(&mut self, depth: usize) -> Res<Value> {
        let mut items = Vec::new();
        loop {
            // Container-level: a broken separator is reported at the array, not an element.
            match self.skip_ws() {
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                Some(b',') if !items.is_empty() => {
                    self.i += 1;
                    match self.skip_ws() {
                        Some(b']') => return self.peek_error("trailing comma"),
                        None => return self.peek_error("EOF while parsing a value"),
                        Some(_) => {}
                    }
                }
                Some(_) if items.is_empty() => {}
                Some(_) => return self.peek_error("expected `,` or `]`"),
                None => return self.peek_error("EOF while parsing a list"),
            }
            self.path.push(format!("/{}", items.len()));
            let item = self.value(depth)?;
            self.path.pop();
            items.push(item);
        }
    }

    fn object(&mut self, depth: usize) -> Res<Value> {
        let mut entries: Vec<(String, Value)> = Vec::new();
        loop {
            match self.skip_ws() {
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(entries));
                }
                Some(b',') if !entries.is_empty() => {
                    self.i += 1;
                    self.skip_ws();
                }
                Some(_) if entries.is_empty() => {}
                Some(_) => return self.peek_error("expected `,` or `}`"),
                None => return self.peek_error("EOF while parsing an object"),
            }
            match self.peek() {
                Some(b'"') => self.i += 1,
                Some(b'}') => return self.peek_error("trailing comma"),
                Some(_) => return self.peek_error("key must be a string"),
                None => return self.peek_error("EOF while parsing a value"),
            }
            let key = self.string()?;
            match self.skip_ws() {
                Some(b':') => self.i += 1,
                Some(_) => return self.peek_error("expected `:`"),
                None => return self.peek_error("EOF while parsing an object"),
            }
            self.path.push(crate::validate::pointer(&[&key]));
            let value = self.value(depth)?;
            self.path.pop();
            entries.push((key, value));
        }
    }

    /// The rest of a string whose opening quote was consumed.
    fn string(&mut self) -> Res<String> {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(c) = self.next() else {
                return self.error("EOF while parsing a string");
            };
            match c {
                b'"' => break,
                b'\\' => {
                    let ch = self.escape()?;
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                0..=0x1f => {
                    return self
                        .error("control character (\\u0000-\\u001F) found while parsing a string");
                }
                _ => out.push(c),
            }
        }
        // The input was a `&str` and escapes were encoded as UTF-8, so this holds.
        Ok(String::from_utf8(out).expect("valid UTF-8 in, valid UTF-8 out"))
    }

    /// One escape sequence, after its backslash.
    fn escape(&mut self) -> Res<char> {
        let Some(c) = self.next() else {
            return self.error("EOF while parsing a string");
        };
        Ok(match c {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let first = self.hex4()?;
                let code = match first {
                    0xDC00..=0xDFFF => {
                        return self.error("lone leading surrogate in hex escape");
                    }
                    // A UTF-16 surrogate pair spells one character above U+FFFF.
                    0xD800..=0xDBFF => {
                        for want in *b"\\u" {
                            match self.next() {
                                Some(c) if c == want => {}
                                Some(_) => return self.error("unexpected end of hex escape"),
                                None => return self.error("EOF while parsing a string"),
                            }
                        }
                        let second = self.hex4()?;
                        if !(0xDC00..=0xDFFF).contains(&second) {
                            return self.error("lone leading surrogate in hex escape");
                        }
                        (u32::from(first - 0xD800) << 10 | u32::from(second - 0xDC00)) + 0x1_0000
                    }
                    n => u32::from(n),
                };
                match char::from_u32(code) {
                    Some(c) => c,
                    None => return self.error("invalid unicode code point"),
                }
            }
            _ => return self.error("invalid escape"),
        })
    }

    fn hex4(&mut self) -> Res<u16> {
        let Some(digits) = self.b.get(self.i..self.i + 4) else {
            self.i = self.b.len();
            return self.error("EOF while parsing a string");
        };
        self.i += 4;
        let mut n = 0u16;
        for &d in digits {
            match (d as char).to_digit(16) {
                Some(v) => n = n << 4 | v as u16,
                None => return self.error("invalid escape"),
            }
        }
        Ok(n)
    }

    /// A number: `-`? int (`.` digits)? (`e` sign? digits)?. Integers stay integers.
    fn number(&mut self) -> Res<Value> {
        let start = self.i;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.i += 1;
        }
        match self.next() {
            None => return self.error("EOF while parsing a value"),
            // One leading zero only.
            Some(b'0') => {
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return self.peek_error("invalid number");
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            Some(_) => return self.error("invalid number"),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return match self.peek() {
                    Some(_) => self.peek_error("invalid number"),
                    None => self.peek_error("EOF while parsing a value"),
                };
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            match self.next() {
                None => return self.error("EOF while parsing a value"),
                Some(b'0'..=b'9') => {}
                Some(_) => return self.error("invalid number"),
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        // Only ASCII digits and signs were consumed.
        let text = std::str::from_utf8(&self.b[start..self.i]).expect("ASCII");
        if !is_float {
            // `-0` is a float (negative zero), like in serde_json.
            if negative {
                if let Ok(n) = text.parse::<i64>() {
                    if n != 0 {
                        return Ok(Value::Int(n));
                    }
                }
            } else if let Ok(n) = text.parse::<u64>() {
                return Ok(i64::try_from(n).map_or(Value::Uint(n), Value::Int));
            }
        }
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(Value::Float(f)),
            _ => self.error("number out of range"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Value {
        parse_json(text).unwrap()
    }

    fn err(text: &str) -> String {
        let e = parse_json(text).unwrap_err();
        format!("[{}] {}", e.path, e.message)
    }

    #[test]
    fn scalars_keep_their_kind() {
        assert_eq!(ok("null"), Value::Null);
        assert_eq!(ok(" true "), Value::Bool(true));
        assert_eq!(ok("5"), Value::Int(5));
        assert_eq!(ok("-5"), Value::Int(-5));
        assert_eq!(ok("5.0"), Value::Float(5.0));
        assert_eq!(ok("1e1"), Value::Float(10.0));
        assert_eq!(ok("18446744073709551615"), Value::Uint(u64::MAX));
        assert_eq!(
            ok("18446744073709551616"),
            Value::Float(18446744073709551616.0)
        );
        assert_eq!(ok("-9223372036854775808"), Value::Int(i64::MIN));
        let Value::Float(z) = ok("-0") else {
            panic!("-0 is a float")
        };
        assert!(z == 0.0 && z.is_sign_negative());
    }

    #[test]
    fn strings_decode_escapes_and_surrogate_pairs() {
        assert_eq!(ok(r#""a\n\"é😀""#), Value::Str("a\n\"é😀".into()));
    }

    #[test]
    fn objects_keep_order_and_duplicates() {
        assert_eq!(
            ok(r#"{"b": 1, "a": [2], "b": null}"#),
            Value::Object(vec![
                ("b".into(), Value::Int(1)),
                ("a".into(), Value::Array(vec![Value::Int(2)])),
                ("b".into(), Value::Null),
            ])
        );
    }

    #[test]
    fn text_after_the_first_value_is_ignored() {
        assert_eq!(ok("{} x"), Value::Object(vec![]));
    }

    #[test]
    fn syntax_errors_report_a_position_and_where_in_the_document() {
        assert_eq!(err(""), "[] EOF while parsing a value at line 1 column 0");
        assert_eq!(
            err("{\"a\": [1,\n2,]}"),
            "[/a] trailing comma at line 2 column 3"
        );
        assert_eq!(
            err(r#"{"a": {"b": tru}}"#),
            "[/a/b] expected ident at line 1 column 16"
        );
        assert_eq!(
            err(r#"{"a": 01}"#),
            "[/a] invalid number at line 1 column 8"
        );
        assert_eq!(err(r#"{"a" 1}"#), "[] expected `:` at line 1 column 6");
        assert_eq!(err(r#"[1 2]"#), "[] expected `,` or `]` at line 1 column 4");
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_stack_overflow() {
        let deep = "[".repeat(1000);
        assert!(err(&deep).contains("recursion limit exceeded"));
        let fine = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse_json(&fine).is_ok());
    }
}
