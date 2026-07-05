#![no_main]

//! Fuzz the DNS-SD decimal-escape decoder used to render discovered service
//! names (e.g. `HP\032OfficeJet` -> `HP OfficeJet`). It must handle any input
//! without panicking and always return valid UTF-8.

use kinjo::discovery::decode_dns_sd_escapes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = decode_dns_sd_escapes(text);
    }
});
