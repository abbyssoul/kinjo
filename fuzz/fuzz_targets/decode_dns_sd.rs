#![no_main]

//! Fuzz the DNS-SD decimal-escape decoder used to render discovered service
//! names (e.g. `HP\032OfficeJet` -> `HP OfficeJet`). Beyond not panicking, two
//! semantic properties must hold: escape-free input decodes to itself, and a
//! reference-encoded byte string decodes back to the original bytes — so a
//! space or any other special character can never be dropped or reshaped.

use kinjo::discovery::decode_dns_sd_escapes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let decoded = decode_dns_sd_escapes(text);
        // Without a backslash there is nothing to decode: identity.
        if !text.contains('\\') {
            assert_eq!(decoded, text);
        }
    }

    // Round-trip: encode every byte the way Avahi escapes special characters
    // (`\DDD`, three decimal digits) and decode it back. The original bytes
    // must come out exactly.
    let encoded: String = data.iter().map(|b| format!("\\{b:03}")).collect();
    assert_eq!(
        decode_dns_sd_escapes(&encoded),
        String::from_utf8_lossy(data)
    );
});
