//! A minimal JSON value and writer, so snapshots can be printed without a serialization crate.
//!
//! Numbers are integers or fixed two-decimal values: the program never uses floating point.

use std::fmt::Write as _;

const HUNDRED: u32 = 100;
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

#[cfg(test)]
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
}
