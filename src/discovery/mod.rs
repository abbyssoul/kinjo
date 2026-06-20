//! Discovery: the producer of [`Entry`] values.
//!
//! A discovery backend streams [`DiscoveryEvent`]s as entries appear, disappear,
//! or status changes. [`Entry`] is the contract type shared with the rest of the
//! program — swapping in a different backend (e.g. a non-mDNS DNS-SD source)
//! only requires implementing [`Discovery`] and producing `Entry` values.

mod entry;
mod fake;
mod mdns;
mod zeroconf;

use std::sync::mpsc;

pub use entry::{Entry, EntryGroup, EntryId, GroupingMode, decode_dns_sd_escapes, group_entries};

/// The mDNS/DNS-SD library used to discover services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoveryBackend {
    /// `mdns-sd-discovery`: a single browser enumerates every service type via
    /// the native DNS-SD meta-query. This is the default.
    #[default]
    MdnsSd,
    /// `zeroconf`: wraps the system Avahi/Bonjour stack, sweeping a curated set
    /// of common service types in parallel.
    Zeroconf,
}

/// An event emitted by a discovery backend.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// An entry appeared or its attributes changed.
    Upsert(Entry),
    /// An entry went away.
    Remove(EntryId),
    /// A human-readable status update about the discovery process.
    Status(String),
}

/// Inputs a discovery backend needs. Keeps the discovery layer decoupled from
/// the CLI/UI layer (see [`crate::ui::cli::Cli::discovery_config`]).
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Use the built-in sample records instead of real mDNS discovery.
    pub fake: bool,
    /// Which mDNS/DNS-SD library to discover with when `fake` is unset.
    pub backend: DiscoveryBackend,
    /// DNS-SD domain to browse.
    pub domain: String,
    /// Limit discovery to a single DNS-SD service type, if set.
    pub service_type: Option<String>,
}

/// A swappable source of [`DiscoveryEvent`]s.
pub trait Discovery {
    /// Take the event receiver. Callable once.
    fn events(&mut self) -> mpsc::Receiver<DiscoveryEvent>;
}

/// Construct the discovery backend selected by `config`.
pub fn start(config: &DiscoveryConfig) -> Box<dyn Discovery> {
    if config.fake {
        return Box::new(fake::FakeDiscovery::start(config));
    }
    match config.backend {
        DiscoveryBackend::MdnsSd => Box::new(mdns::MdnsDiscovery::start(config)),
        DiscoveryBackend::Zeroconf => Box::new(zeroconf::ZeroconfDiscovery::start(config)),
    }
}
