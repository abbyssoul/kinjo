//! Conversion of dynamic application data into inert terminal text.
//!
//! Discovery and configuration values stay raw everywhere else so matching and
//! command interpolation keep their original semantics. Renderers must cross
//! this seam before giving dynamic text to Ratatui.

use std::fmt::Write as _;

/// Return `value` with Unicode control characters replaced by visible escapes.
///
/// C0, DEL, and C1 controls fit in a byte and use `\xNN`; any future control
/// outside that range uses Rust-like `\u{...}` notation. Printable Unicode,
/// including combining and bidirectional-formatting characters, is unchanged.
pub(crate) fn text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if !character.is_control() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_visible_and_printable_unicode_is_unchanged() {
        assert_eq!(
            text("\0\u{7}\t\n\r\u{1b}\u{7f}\u{80}\u{85}\u{9f}"),
            "\\x00\\x07\\x09\\x0A\\x0D\\x1B\\x7F\\x80\\x85\\x9F"
        );

        let printable = "café 界 e\u{301} 🙂 \u{202e}";
        assert_eq!(text(printable), printable);
    }
}
