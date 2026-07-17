//! Terminal presentation for text written by Kinjo itself.
//!
//! Values stay raw in discovery, matching, configuration, and command argv.
//! Every renderer and process-owned output path crosses this module only when
//! it is ready to turn a value into terminal bytes.

use std::fmt::Write as _;
use unicode_width::UnicodeWidthStr;

/// Return `value` with Unicode control and bidi-formatting characters replaced
/// by visible escapes.
///
/// C0, DEL, and C1 controls fit in a byte and use `\xNN`; any future control
/// outside that range uses Rust-like `\u{...}` notation. Bidi controls use the
/// same notation so untrusted labels cannot visually reorder adjacent trusted
/// UI text. Other printable Unicode, including combining characters, is
/// unchanged.
pub(crate) fn text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if !character.is_control() && !is_bidi_formatting(character) {
            output.push(character);
            continue;
        }

        let codepoint = character as u32;
        if codepoint <= u8::MAX as u32 {
            // Writing to a String cannot fail.
            write!(output, "\\x{codepoint:02X}").expect("writing to a String cannot fail");
        } else {
            write!(output, "\\u{{{codepoint:X}}}").expect("writing to a String cannot fail");
        }
    }
    output
}

fn is_bidi_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Number of terminal display columns occupied by already-safe text.
pub(crate) fn width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_bidi_formatting_are_visible() {
        assert_eq!(
            text("\0\u{7}\t\n\r\u{1b}\u{7f}\u{80}\u{85}\u{9f}"),
            "\\x00\\x07\\x09\\x0A\\x0D\\x1B\\x7F\\x80\\x85\\x9F"
        );

        assert_eq!(
            text("a\u{061c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069}z"),
            "a\\u{61C}\\u{200E}\\u{200F}\\u{202A}\\u{202E}\\u{2066}\\u{2069}z"
        );

        let printable = "café 界 e\u{301} 🙂";
        assert_eq!(text(printable), printable);
    }
}
