//! Portable DNS-SD TXT normalization shared by discovery adapters.
//!
//! Kinjo's public entry, predicate, template, and process-argument interfaces
//! are text-based. This module is the one place adapters turn their native TXT
//! representation into that portable model: valid DNS-SD keys are canonical,
//! duplicates obey first-wins semantics, and binary values are never decoded
//! lossily into text that was not advertised.

use std::collections::{BTreeMap, BTreeSet};

/// DNS-SD keys are non-empty printable US-ASCII excluding `=` (RFC 6763 6.4).
/// Nine bytes is only a recommendation and real printers exceed it. 255 is the
/// protocol's limit for the whole length-prefixed TXT string, so no legal key
/// can be longer.
fn canonical_key(key: &str) -> Option<String> {
    let bytes = key.as_bytes();
    if !(1..=255).contains(&bytes.len())
        || !bytes
            .iter()
            .all(|byte| matches!(byte, 0x20..=0x7e) && *byte != b'=')
    {
        return None;
    }
    Some(key.to_ascii_lowercase())
}

/// Builds the text TXT map carried by an [`Entry`](super::Entry).
///
/// `seen` is separate from `values` deliberately: an invalid-UTF-8 first value
/// still owns its case-insensitive key under DNS-SD first-wins semantics. A
/// later duplicate must not become actionable merely because it is valid text.
#[derive(Debug, Default)]
pub(super) struct TextTxtMap {
    seen: BTreeSet<String>,
    values: BTreeMap<String, String>,
}

impl TextTxtMap {
    pub(super) fn observe_bytes(&mut self, key: &str, value: Option<&[u8]>) {
        let Some(key) = canonical_key(key) else {
            return;
        };
        if !self.seen.insert(key.clone()) {
            return;
        }
        let bytes = value.unwrap_or_default();
        let Ok(value) = std::str::from_utf8(bytes) else {
            return;
        };
        self.values.insert(key, value.to_string());
    }

    #[cfg(any(feature = "zeroconf", test))]
    pub(super) fn observe_text(&mut self, key: &str, value: &str) {
        self.observe_bytes(key, Some(value.as_bytes()));
    }

    pub(super) fn into_values(self) -> BTreeMap<String, String> {
        self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_validated_and_canonicalized() {
        let mut txt = TextTxtMap::default();
        txt.observe_text("Path", "/admin");
        txt.observe_text("", "empty");
        txt.observe_text("printer-type", "3");
        txt.observe_text("printer-state", "idle");
        txt.observe_text("mopria-certified", "2.1");
        txt.observe_text("bad=key", "equals");
        txt.observe_text("bad\nkey", "control");
        txt.observe_text("non-ascii-é", "unicode");

        assert_eq!(
            txt.into_values(),
            BTreeMap::from([
                ("mopria-certified".to_string(), "2.1".to_string()),
                ("path".to_string(), "/admin".to_string()),
                ("printer-state".to_string(), "idle".to_string()),
                ("printer-type".to_string(), "3".to_string()),
            ])
        );
    }

    #[test]
    fn duplicate_keys_are_case_insensitive_and_first_wins() {
        let mut txt = TextTxtMap::default();
        txt.observe_text("Printer-Type", "first");
        txt.observe_text("printer-type", "second");

        assert_eq!(
            txt.into_values().get("printer-type").map(String::as_str),
            Some("first")
        );
    }

    #[test]
    fn protocol_sized_key_bound_accepts_255_bytes_and_rejects_256() {
        let accepted = "a".repeat(255);
        let rejected = "b".repeat(256);
        let mut txt = TextTxtMap::default();
        txt.observe_text(&accepted, "yes");
        txt.observe_text(&rejected, "no");

        let values = txt.into_values();
        assert_eq!(values.get(&accepted).map(String::as_str), Some("yes"));
        assert!(!values.contains_key(&rejected));
    }

    #[test]
    fn invalid_utf8_is_not_lossy_and_still_owns_the_key() {
        let mut txt = TextTxtMap::default();
        txt.observe_bytes("path", Some(&[0xff]));
        txt.observe_text("PATH", "replacement");

        assert!(txt.into_values().is_empty());
    }
}
