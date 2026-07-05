//! Thin binary entry point. All logic lives in the `kinjo` library so it can
//! be reused and fuzzed; see [`kinjo::run`].

fn main() -> color_eyre::eyre::Result<()> {
    kinjo::run()
}
