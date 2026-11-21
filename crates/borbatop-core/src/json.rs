//! A minimal JSON value and writer, so snapshots can be printed without a serialization crate.
//!
//! Numbers are integers or fixed two-decimal values: the program never uses floating point.

use std::error::Error;
use std::fmt::{self, Write as _};

const HUNDRED: u32 = 100;
const MAX_DEPTH: usize = 32;
const MAX_FRACTION_DIGITS: usize = 2;
const UNICODE_ESCAPE_DIGITS: usize = 4;
const HEX_RADIX: u32 = 16;
const CONTROL_LIMIT: char = '\u{20}';

/// A JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    /// `null`: a reading that is not available.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An unsigned integer.
    Unsigned(u64),
    /// A signed integer.
    Signed(i64),
    /// A value in hundredths, written with two decimals (`123` becomes `1.23`).
    Hundredths(u32),
    /// A string; quotes, backslashes and control characters are escaped when written.
    Text(String),
    /// An ordered list.
    Array(Vec<Json>),
    /// An object whose keys keep their insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// An object from `(key, value)` pairs.
    ///
    /// # Arguments
    ///
    /// * `fields` - the members, in the order they are written
    #[must_use]
    pub fn object<const N: usize>(fields: [(&str, Self); N]) -> Self {
        Self::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    /// A string value.
    ///
    /// # Arguments
    ///
    /// * `text` - the text, unescaped
    #[must_use]
    pub fn text(text: &str) -> Self {
        Self::Text(text.to_owned())
    }

    /// A value, or `null` when there is none.
    ///
    /// # Arguments
    ///
    /// * `value` - the optional value
    #[must_use]
    pub fn or_null(value: Option<Self>) -> Self {
        value.unwrap_or(Self::Null)
    }

    /// Writes the value as compact JSON.
    ///
    /// # Arguments
    ///
    /// * `output` - text the value is appended to
    pub fn write_to(&self, output: &mut String) {
        match self {
            Self::Null => output.push_str("null"),
            Self::Bool(true) => output.push_str("true"),
            Self::Bool(false) => output.push_str("false"),
            Self::Unsigned(value) => push_display(output, value),
            Self::Signed(value) => push_display(output, value),
            Self::Hundredths(raw) => {
                let whole = raw.div_euclid(HUNDRED);
                let fraction = raw.rem_euclid(HUNDRED);
                push_display(output, &format_args!("{whole}.{fraction:02}"));
            }
            Self::Text(text) => write_string(text, output),
            Self::Array(items) => write_list(items.iter(), output, '[', ']', Self::write_to),
            Self::Object(fields) => {
                write_list(fields.iter(), output, '{', '}', |(key, value), output| {
                    write_string(key, output);
                    output.push(':');
                    value.write_to(output);
                });
            }
        }
    }

    /// The value as compact JSON text.
    #[must_use]
    pub fn render(&self) -> String {
        let mut output = String::new();
        self.write_to(&mut output);
        output
    }
}

fn push_display(output: &mut String, value: &dyn std::fmt::Display) {
    // Writing to a `String` cannot fail; the `Result` exists for other writers.
    let _infallible = write!(output, "{value}");
}

fn write_list<'a, T: 'a>(
    items: impl Iterator<Item = T>,
    output: &mut String,
    open: char,
    close: char,
    mut write_item: impl FnMut(T, &mut String),
) {
    output.push(open);
    for (position, item) in items.enumerate() {
        if position > 0 {
            output.push(',');
        }
        write_item(item, output);
    }
    output.push(close);
}

fn write_string(text: &str, output: &mut String) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control < CONTROL_LIMIT => {
                push_display(output, &format_args!("\\u{:04x}", u32::from(control)));
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

/// Why a text could not be read as JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonError {
    /// The text ended inside a value.
    UnexpectedEnd,
    /// A character that cannot start or continue the value at this byte offset.
    Unexpected {
        /// The character found.
        found: char,
        /// Byte offset in the text.
        position: usize,
    },
    /// A number this reader does not support: only integers and decimals with up to two
    /// fraction digits are accepted (that is all [`Json::write_to`] writes).
    InvalidNumber {
        /// Byte offset in the text.
        position: usize,
    },
    /// A string escape that is not valid JSON, or a surrogate half.
    InvalidEscape {
        /// Byte offset in the text.
        position: usize,
    },
    /// Arrays or objects nested deeper than the reader allows.
    TooDeep,
    /// Text after the end of the value.
    TrailingData {
        /// Byte offset in the text.
        position: usize,
    },
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => write!(formatter, "json ends too early"),
            Self::Unexpected { found, position } => {
                write!(formatter, "unexpected {found:?} at byte {position}")
            }
            Self::InvalidNumber { position } => {
                write!(formatter, "unsupported number at byte {position}")
            }
            Self::InvalidEscape { position } => {
                write!(formatter, "invalid string escape at byte {position}")
            }
            Self::TooDeep => write!(formatter, "json is nested too deeply"),
            Self::TrailingData { position } => {
                write!(formatter, "data after the json value at byte {position}")
            }
        }
    }
}

impl Error for JsonError {}

impl Json {
    /// Reads one JSON value.
    ///
    /// The reader accepts what [`Json::render`] writes plus ordinary whitespace: integers,
    /// decimals with up to two fraction digits (read as [`Json::Hundredths`]), strings with the
    /// standard escapes, arrays and objects. Exponents and longer decimals are refused rather
    /// than rounded.
    ///
    /// # Arguments
    ///
    /// * `text` - one JSON value, nothing after it
    ///
    /// # Errors
    ///
    /// Returns a [`JsonError`] with the byte offset of the first problem.
    ///
    /// # Examples
    ///
    /// ```
    /// use borbatop_core::Json;
    ///
    /// let value = Json::parse(r#"{"load": 1.25, "name": "init"}"#)?;
    /// assert_eq!(value.get("load").and_then(Json::as_hundredths), Some(125));
    /// assert_eq!(value.get("name").and_then(Json::as_str), Some("init"));
    /// # Ok::<(), borbatop_core::JsonError>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self, JsonError> {
        let mut parser = Parser { text, position: 0 };
        let value = parser.value(0)?;
        parser.skip_whitespace();
        match parser.peek() {
            None => Ok(value),
            Some(_) => Err(JsonError::TrailingData {
                position: parser.position,
            }),
        }
    }

    /// The member `key` of an object.
    ///
    /// # Arguments
    ///
    /// * `key` - member name
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .find_map(|(name, value)| (name == key).then_some(value)),
            Self::Null
            | Self::Bool(_)
            | Self::Unsigned(_)
            | Self::Signed(_)
            | Self::Hundredths(_)
            | Self::Text(_)
            | Self::Array(_) => None,
        }
    }

    /// Whether the value is `null`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The value as an unsigned integer.
    #[must_use]
    pub const fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(*value),
            Self::Null
            | Self::Bool(_)
            | Self::Signed(_)
            | Self::Hundredths(_)
            | Self::Text(_)
            | Self::Array(_)
            | Self::Object(_) => None,
        }
    }

    /// The value as a signed integer.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Signed(value) => Some(*value),
            Self::Unsigned(value) => i64::try_from(*value).ok(),
            Self::Null
            | Self::Bool(_)
            | Self::Hundredths(_)
            | Self::Text(_)
            | Self::Array(_)
            | Self::Object(_) => None,
        }
    }

    /// The value in hundredths; a whole number counts as that many hundreds.
    #[must_use]
    pub fn as_hundredths(&self) -> Option<u32> {
        match self {
            Self::Hundredths(raw) => Some(*raw),
            Self::Unsigned(whole) => u32::try_from(*whole).ok()?.checked_mul(HUNDRED),
            Self::Null
            | Self::Bool(_)
            | Self::Signed(_)
            | Self::Text(_)
            | Self::Array(_)
            | Self::Object(_) => None,
        }
    }

    /// The value as text.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Null
            | Self::Bool(_)
            | Self::Unsigned(_)
            | Self::Signed(_)
            | Self::Hundredths(_)
            | Self::Array(_)
            | Self::Object(_) => None,
        }
    }

    /// The items of an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            Self::Null
            | Self::Bool(_)
            | Self::Unsigned(_)
            | Self::Signed(_)
            | Self::Hundredths(_)
            | Self::Text(_)
            | Self::Object(_) => None,
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text.get(self.position..)?.chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position = self.position.saturating_add(character.len_utf8());
        Some(character)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn unexpected(&self, found: char) -> JsonError {
        JsonError::Unexpected {
            found,
            position: self.position,
        }
    }

    fn expect(&mut self, wanted: char) -> Result<(), JsonError> {
        match self.peek() {
            Some(found) if found == wanted => {
                self.bump();
                Ok(())
            }
            Some(found) => Err(self.unexpected(found)),
            None => Err(JsonError::UnexpectedEnd),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.skip_whitespace();
        let Some(first) = self.peek() else {
            return Err(JsonError::UnexpectedEnd);
        };
        match first {
            'n' => self.word("null", Json::Null),
            't' => self.word("true", Json::Bool(true)),
            'f' => self.word("false", Json::Bool(false)),
            '"' => self.string().map(Json::Text),
            '[' => self.array(depth),
            '{' => self.object(depth),
            '-' | '0'..='9' => self.number(),
            other => Err(self.unexpected(other)),
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        for wanted in word.chars() {
            self.expect(wanted)?;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.position;
        let negative = self.peek() == Some('-');
        if negative {
            self.bump();
        }
        let whole = self.digits();
        if whole.is_empty() {
            return Err(JsonError::InvalidNumber { position: start });
        }
        let fraction = if self.peek() == Some('.') {
            self.bump();
            let digits = self.digits();
            if digits.is_empty() || digits.len() > MAX_FRACTION_DIGITS {
                return Err(JsonError::InvalidNumber { position: start });
            }
            Some(digits)
        } else {
            None
        };
        let invalid = JsonError::InvalidNumber { position: start };

        match (negative, fraction) {
            (true, None) => {
                let text = self.text.get(start..self.position).ok_or(invalid)?;
                text.parse().map(Json::Signed).map_err(|_| invalid)
            }
            (false, None) => whole.parse().map(Json::Unsigned).map_err(|_| invalid),
            (false, Some(fraction)) => {
                let scale = match fraction.len() {
                    1 => HUNDRED / 10,
                    _ => 1,
                };
                let whole: u32 = whole.parse().map_err(|_| invalid)?;
                let fraction: u32 = fraction.parse().map_err(|_| invalid)?;
                whole
                    .checked_mul(HUNDRED)
                    .and_then(|hundreds| hundreds.checked_add(fraction.checked_mul(scale)?))
                    .map(Json::Hundredths)
                    .ok_or(invalid)
            }
            (true, Some(_)) => Err(invalid),
        }
    }

    fn digits(&mut self) -> String {
        let mut digits = String::new();
        while let Some(digit) = self.peek().filter(char::is_ascii_digit) {
            digits.push(digit);
            self.bump();
        }
        digits
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect('"')?;
        let mut text = String::new();
        loop {
            let at = self.position;
            match self.bump().ok_or(JsonError::UnexpectedEnd)? {
                '"' => return Ok(text),
                '\\' => text.push(self.escape(at)?),
                control if control < '\u{20}' => return Err(self.unexpected(control)),
                other => text.push(other),
            }
        }
    }

    fn escape(&mut self, at: usize) -> Result<char, JsonError> {
        let invalid = JsonError::InvalidEscape { position: at };
        match self.bump().ok_or(JsonError::UnexpectedEnd)? {
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            '/' => Ok('/'),
            'b' => Ok('\u{8}'),
            'f' => Ok('\u{c}'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'u' => {
                let mut code = 0_u32;
                for _ in 0..UNICODE_ESCAPE_DIGITS {
                    let digit = self
                        .bump()
                        .and_then(|character| character.to_digit(HEX_RADIX))
                        .ok_or(invalid)?;
                    code = code
                        .checked_mul(HEX_RADIX)
                        .ok_or(invalid)?
                        .saturating_add(digit);
                }
                char::from_u32(code).ok_or(invalid)
            }
            _ => Err(invalid),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        let depth = depth
            .checked_add(1)
            .filter(|d| *d <= MAX_DEPTH)
            .ok_or(JsonError::TooDeep)?;
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.skip_whitespace();
            match self.bump().ok_or(JsonError::UnexpectedEnd)? {
                ',' => {}
                ']' => return Ok(Json::Array(items)),
                other => return Err(self.unexpected(other)),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        let depth = depth
            .checked_add(1)
            .filter(|d| *d <= MAX_DEPTH)
            .ok_or(JsonError::TooDeep)?;
        self.expect('{')?;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(':')?;
            fields.push((key, self.value(depth)?));
            self.skip_whitespace();
            match self.bump().ok_or(JsonError::UnexpectedEnd)? {
                ',' => {}
                '}' => return Ok(Json::Object(fields)),
                other => return Err(self.unexpected(other)),
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scalars_render_as_json_literals() {
        assert_eq!(Json::Null.render(), "null");
        assert_eq!(Json::Bool(true).render(), "true");
        assert_eq!(Json::Bool(false).render(), "false");
        assert_eq!(Json::Unsigned(u64::MAX).render(), "18446744073709551615");
        assert_eq!(Json::Signed(-7).render(), "-7");
    }

    #[test]
    fn hundredths_keep_two_decimals() {
        assert_eq!(Json::Hundredths(0).render(), "0.00");
        assert_eq!(Json::Hundredths(5).render(), "0.05");
        assert_eq!(Json::Hundredths(123).render(), "1.23");
        assert_eq!(Json::Hundredths(1_000).render(), "10.00");
    }

    #[test]
    fn strings_escape_quotes_backslashes_and_controls() {
        assert_eq!(Json::text("a\"b\\c").render(), r#""a\"b\\c""#);
        assert_eq!(Json::text("l1\nl2\t\r").render(), r#""l1\nl2\t\r""#);
        assert_eq!(Json::text("\u{1b}[31m").render(), r#""\u001b[31m""#);
        assert_eq!(Json::text("ação").render(), "\"ação\"");
    }

    #[test]
    fn containers_keep_order_and_separate_with_commas() {
        let value = Json::object([
            ("b", Json::Array(vec![Json::Unsigned(1), Json::Null])),
            ("a", Json::object([])),
            ("c", Json::Array(Vec::new())),
        ]);
        assert_eq!(value.render(), r#"{"b":[1,null],"a":{},"c":[]}"#);
    }

    #[test]
    fn or_null_maps_absence_to_null() {
        assert_eq!(Json::or_null(None).render(), "null");
        assert_eq!(Json::or_null(Some(Json::Unsigned(3))).render(), "3");
    }
    #[test]
    fn what_render_writes_parse_reads_back() {
        let value = Json::object([
            ("n", Json::Null),
            ("yes", Json::Bool(true)),
            ("no", Json::Bool(false)),
            ("big", Json::Unsigned(u64::MAX)),
            ("neg", Json::Signed(-42)),
            ("load", Json::Hundredths(1_234)),
            ("text", Json::text("a\"b\\c\nd\u{1b}é")),
            (
                "list",
                Json::Array(vec![Json::Unsigned(1), Json::Array(Vec::new())]),
            ),
            ("empty", Json::object([])),
        ]);
        assert_eq!(Json::parse(&value.render()), Ok(value));
    }

    #[test]
    fn whitespace_and_escapes_are_accepted() {
        let value = Json::parse(" { \"a\" : [ 1 , 2.5 , \"\\u00e9\\/\" ] }\n").unwrap();
        let items = value.get("a").and_then(Json::as_array).unwrap();
        assert_eq!(items.first().and_then(Json::as_u64), Some(1));
        assert_eq!(items.get(1).and_then(Json::as_hundredths), Some(250));
        assert_eq!(items.get(2).and_then(Json::as_str), Some("é/"));
    }

    #[test]
    fn a_single_fraction_digit_means_tenths() {
        assert_eq!(Json::parse("0.5"), Ok(Json::Hundredths(50)));
        assert_eq!(Json::parse("0.05"), Ok(Json::Hundredths(5)));
        assert_eq!(Json::parse("3.00"), Ok(Json::Hundredths(300)));
    }

    #[test]
    fn broken_or_unsupported_text_is_an_error_with_a_position() {
        for (text, error) in [
            ("", JsonError::UnexpectedEnd),
            ("[1,", JsonError::UnexpectedEnd),
            ("\"abc", JsonError::UnexpectedEnd),
            ("tru", JsonError::UnexpectedEnd),
            ("1.234", JsonError::InvalidNumber { position: 0 }),
            ("1.", JsonError::InvalidNumber { position: 0 }),
            ("-1.5", JsonError::InvalidNumber { position: 0 }),
            ("-", JsonError::InvalidNumber { position: 0 }),
            ("1e5", JsonError::TrailingData { position: 1 }),
            ("[1] x", JsonError::TrailingData { position: 4 }),
            ("\"\\q\"", JsonError::InvalidEscape { position: 1 }),
            ("\"\\ud800\"", JsonError::InvalidEscape { position: 1 }),
            (
                "18446744073709551616",
                JsonError::InvalidNumber { position: 0 },
            ),
        ] {
            assert_eq!(Json::parse(text), Err(error), "{text:?}");
        }
        assert!(matches!(
            Json::parse("{1:2}"),
            Err(JsonError::Unexpected { found: '1', .. })
        ));
        assert!(matches!(
            Json::parse("\"a\nb\""),
            Err(JsonError::Unexpected { .. })
        ));
    }

    #[test]
    fn deep_nesting_is_refused_not_a_stack_overflow() {
        let deep = format!("{}{}", "[".repeat(1_000), "]".repeat(1_000));
        assert_eq!(Json::parse(&deep), Err(JsonError::TooDeep));
        let fine = format!("{}{}", "[".repeat(10), "]".repeat(10));
        assert!(Json::parse(&fine).is_ok());
    }

    #[test]
    fn accessors_return_none_for_the_wrong_kind() {
        let value = Json::parse(r#"{"a": 1, "b": "x", "c": null, "d": -2}"#).unwrap();
        assert_eq!(value.get("a").and_then(Json::as_str), None);
        assert_eq!(value.get("b").and_then(Json::as_u64), None);
        assert!(value.get("c").is_some_and(Json::is_null));
        assert_eq!(value.get("d").and_then(Json::as_i64), Some(-2));
        assert_eq!(value.get("a").and_then(Json::as_i64), Some(1));
        assert_eq!(value.get("zzz"), None);
        assert_eq!(Json::Unsigned(1).get("a"), None);
        assert_eq!(value.get("a").and_then(Json::as_hundredths), Some(100));
    }
}
