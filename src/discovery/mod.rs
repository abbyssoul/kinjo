//! Discovery: the producer of [`Entry`] values.
//!
//! A discovery backend streams [`DiscoveryEvent`]s as entries appear, disappear,
//! or status changes. [`Entry`] is the contract type shared with the rest of the
//! program.
//!
//! [`start`] hands back a [`DiscoverySession`]: the owned lifetime of a running
//! adapter. The session is the whole interface — events, state, and shutdown —
//! so callers never hold a receiver whose producer they cannot see, and never
//! mistake a dead adapter for a quiet network.
//!
//! The adapter seam lives *inside* this module, at the browse loop: `mdns-sd`,
//! `zeroconf`, and the explicit fake genuinely differ in how they browse, but
//! they do not differ in how a caller runs and stops them. That is why there is
//! one concrete session type rather than a trait over receivers.

mod entry;
mod fake;
mod mdns;
mod session;
mod worker;
#[cfg(feature = "zeroconf")]
mod zeroconf;

pub use entry::{
    Entry, EntryGroup, EntryId, GroupingMode, OccurrenceId, Registration, decode_dns_sd_escapes,
    group_entries,
};
pub use session::{DiscoveryFailure, DiscoverySession, FailureKind, SessionPoll, SessionState};

/// The mDNS/DNS-SD library used to discover services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoveryBackend {
    /// `mdns-sd-discovery`: a single browser enumerates every service type via
    /// the native DNS-SD meta-query. This is the default.
    #[default]
    MdnsSd,
    /// `zeroconf`: wraps the system Avahi/Bonjour stack, sweeping a curated set
    /// of common service types in parallel. Only available when the crate is
    /// built with the `zeroconf` feature.
    #[cfg(feature = "zeroconf")]
    Zeroconf,
}

/// An event emitted by a discovery backend.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// An occurrence appeared or its attributes changed.
    Upsert(Entry),
    /// Exactly one occurrence went away. Other occurrences of the same
    /// registration are unaffected and stay live.
    Remove(EntryId),
    /// Every occurrence of a registration went away.
    ///
    /// This is the honest event for an adapter that cannot tell which
    /// occurrence it lost: the zeroconf backend, whose removals carry no
    /// discriminator at all, and an mdns-sd removal that arrives without an
    /// interface index. Such an adapter must say "all of them" rather than
    /// guess at one and silently drop a live sibling.
    RemoveRegistration(Registration),
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

/// Start the discovery adapter selected by `config` and return the session that
/// owns it. Dropping the session stops the adapter.
///
/// Sample records come back only when `config.fake` is set. A real adapter that
/// fails reports a [`SessionState::Failed`] and emits no entries: fabricating
/// plausible LAN endpoints out of a failure would let a user act on a device
/// that does not exist.
pub fn start(config: &DiscoveryConfig) -> DiscoverySession {
    if config.fake {
        return fake::start(config);
    }
    match config.backend {
        DiscoveryBackend::MdnsSd => mdns::start(config),
        #[cfg(feature = "zeroconf")]
        DiscoveryBackend::Zeroconf => zeroconf::start(config),
    }
}
