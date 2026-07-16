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
mod options;
mod session;
mod worker;
#[cfg(feature = "zeroconf")]
mod zeroconf;

pub use entry::{
    BrowseMode, ChildService, Entry, EntryGroup, EntryGroupId, EntryId, GroupFacts, GroupingMode,
    HostAggregate, HostKey, LogicalService, OccurrenceId, Registration, RowHost, RowServiceType,
    ServiceTypeAggregate, TxtValue, UNRESOLVED_HOST_LABEL, browse_groups, browse_row_count,
    decode_dns_sd_escapes,
};
pub use options::{
    DEFAULT_DOMAIN, DiscoveryBackend, DiscoveryConfig, DiscoveryOptionError, DiscoveryOptions,
    ServiceTypeFilter, TransportProtocol,
};
pub use session::{DiscoveryFailure, DiscoverySession, FailureKind, SessionPoll, SessionState};

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

/// Start the discovery adapter selected by `options` and return the session
/// that owns it. Dropping the session stops the adapter.
///
/// Sample records come back only when `options.fake()` is set. A real adapter
/// that fails reports a [`SessionState::Failed`] and emits no entries:
/// fabricating plausible LAN endpoints out of a failure would let a user act on
/// a device that does not exist.
///
/// Taking [`DiscoveryOptions`] rather than a raw [`DiscoveryConfig`] is what
/// makes "an option is honored exactly or rejected" structural: the checking
/// happened in [`DiscoveryConfig::validate`], once, and no adapter below can be
/// reached with a value it would have to quietly reinterpret.
pub fn start(options: &DiscoveryOptions) -> DiscoverySession {
    if options.fake() {
        return fake::start(options);
    }
    match options.backend() {
        DiscoveryBackend::MdnsSd => mdns::start(options),
        #[cfg(feature = "zeroconf")]
        DiscoveryBackend::Zeroconf => zeroconf::start(options),
    }
}
