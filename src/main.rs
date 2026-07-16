//! Thin binary entry point. All logic lives in the `kinjo` library so it can
//! be reused and fuzzed; see [`kinjo::process_main`].

fn main() -> std::process::ExitCode {
    kinjo::process_main()
}
