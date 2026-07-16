//! What a caller asks discovery for, and what discovery agrees to do.
//!
//! [`DiscoveryConfig`] is a *request*: plain fields projected from the CLI or
//! built by a library caller. [`DiscoveryOptions`] is that request **validated
//! and canonicalized** — the only thing [`start`](super::start) accepts.
//!
//! The two types exist so the guarantee is structural rather than remembered.
//! An adapter cannot be handed a malformed service type or a domain it cannot
//! honor, because there is no way to build the value that reaches it except
//! through [`DiscoveryConfig::validate`]. That is also why validation lives
//! here, at the start seam shared by CLI startup, refresh, and library callers,
//! instead of inside each browse loop: a check repeated per adapter is a check
//! each new adapter can forget.
//!
//! The rule the whole module serves: **a configured option is either honored
//! exactly or rejected.** Quietly turning "browse `_ssh._tcp`" into "browse
//! everything", or "browse `corp`" into "browse `local`", widens what the
//! program observes beyond what the user asked for.

use std::fmt;

/// The DNS-SD domain browsed when the caller does not name another. Empty,
/// `local`, and `local.` all mean this (see [`canonical_domain`]).
pub const DEFAULT_DOMAIN: &str = "local";

/// Longest DNS-SD service name, per RFC 6763 §7.
const MAX_SERVICE_NAME_LEN: usize = 15;

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

impl DiscoveryBackend {
    /// The name this backend is selected by, and is reported as.
    pub fn name(self) -> &'static str {
        match self {
            DiscoveryBackend::MdnsSd => "mdns-sd",
            #[cfg(feature = "zeroconf")]
            DiscoveryBackend::Zeroconf => "zeroconf",
        }
    }

    /// Whether this backend can browse a domain other than [`DEFAULT_DOMAIN`].
    ///
    /// This is a real capability difference, not a policy: upstream
    /// `zeroconf`'s browser exposes no domain setter at all, so the adapter has
    /// nowhere to put a custom domain. Browsing `local` while the user asked
    /// for `corp` would answer a question they did not ask, so the option is
    /// rejected instead (see [`DiscoveryOptionError::UnsupportedDomain`]).
    fn supports_custom_domain(self) -> bool {
        match self {
            DiscoveryBackend::MdnsSd => true,
            #[cfg(feature = "zeroconf")]
            DiscoveryBackend::Zeroconf => false,
        }
    }
}

impl fmt::Display for DiscoveryBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The transport a DNS-SD service type names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportProtocol {
    Tcp,
    Udp,
}

impl TransportProtocol {
    /// The label as it appears in a service type, without the leading `_`.
    pub fn as_str(self) -> &'static str {
        match self {
            TransportProtocol::Tcp => "tcp",
            TransportProtocol::Udp => "udp",
        }
    }
}

impl fmt::Display for TransportProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A validated DNS-SD service type: proof that some `_<name>._tcp` /
/// `_<name>._udp` text was well-formed, in the canonical (lower-case) spelling.
///
/// Only [`parse`](Self::parse) constructs one, so an adapter that holds this
/// value never has to ask whether the filter is usable — which is what lets the
/// adapters drop their "if it does not parse, browse everything instead"
/// fallbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTypeFilter {
    /// The service name without its leading `_`, lower-cased.
    name: String,
    protocol: TransportProtocol,
}

impl ServiceTypeFilter {
    /// Validate and canonicalize an explicit DNS-SD service type.
    ///
    /// Accepts `_<name>._tcp` and `_<name>._udp`, where `<name>` is 1–15 ASCII
    /// letters, digits, and internal hyphens, begins and ends alphanumeric, has
    /// no consecutive hyphens, and contains at least one letter (RFC 6763 §7).
    /// DNS-SD types are case-insensitive, so the value is lower-cased.
    pub fn parse(value: &str) -> Result<Self, DiscoveryOptionError> {
        Self::parse_parts(value).map_err(|reason| DiscoveryOptionError::ServiceType {
            value: value.to_string(),
            reason,
        })
    }

    fn parse_parts(value: &str) -> Result<Self, &'static str> {
        let rest = value
            .strip_prefix('_')
            .ok_or("a service type begins with `_`")?;
        // The name cannot contain `.`, so the first dot always separates the
        // name from the protocol. A trailing dot or an extra label therefore
        // lands in the protocol and is rejected there rather than ignored.
        let (name, protocol) = rest
            .split_once('.')
            .ok_or("expected a name and a transport, as in `_ssh._tcp`")?;
        let protocol = protocol
            .strip_prefix('_')
            .ok_or("the transport begins with `_`, as in `._tcp`")?;

        let protocol = match protocol.to_ascii_lowercase().as_str() {
            "tcp" => TransportProtocol::Tcp,
            "udp" => TransportProtocol::Udp,
            _ => return Err("the transport must be `_tcp` or `_udp`"),
        };

        Ok(Self {
            name: validate_service_name(name)?.to_ascii_lowercase(),
            protocol,
        })
    }

    /// The service name without its leading `_`, lower-cased.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The transport this type names.
    pub fn protocol(&self) -> TransportProtocol {
        self.protocol
    }
}

impl fmt::Display for ServiceTypeFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}._{}", self.name, self.protocol)
    }
}

/// Checks a service name against RFC 6763 §7, returning it unchanged.
fn validate_service_name(name: &str) -> Result<&str, &'static str> {
    if name.is_empty() {
        return Err("the service name is empty");
    }
    if !name.is_ascii() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("the service name may contain only ASCII letters, digits, and hyphens");
    }
    if name.len() > MAX_SERVICE_NAME_LEN {
        return Err("the service name may be at most 15 characters");
    }
    // `name` is non-empty ASCII, so first/last bytes are whole characters.
    let (first, last) = (name.as_bytes()[0], name.as_bytes()[name.len() - 1]);
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err("the service name must begin and end with a letter or digit");
    }
    if name.contains("--") {
        return Err("the service name must not contain consecutive hyphens");
    }
    if !name.bytes().any(|b| b.is_ascii_alphabetic()) {
        return Err("the service name must contain at least one letter");
    }
    Ok(name)
}

/// Canonicalize a browse domain. Empty, `local`, and `local.` are all the
/// default domain spelled differently; any other value is the caller's and is
/// passed through untouched.
fn canonical_domain(domain: &str) -> String {
    if domain.is_empty()
        || domain.eq_ignore_ascii_case(DEFAULT_DOMAIN)
        || domain.eq_ignore_ascii_case("local.")
    {
        return DEFAULT_DOMAIN.to_string();
    }
    domain.to_string()
}

/// A discovery option that cannot be honored as written.
///
/// Every variant names the offending value and the way out, because these
/// surface as the first thing a user sees when a run refuses to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryOptionError {
    /// The requested service type is not a DNS-SD service type.
    ServiceType {
        value: String,
        /// Which rule it broke, phrased for the person who typed it.
        reason: &'static str,
    },
    /// The selected backend cannot browse the requested domain.
    UnsupportedDomain {
        backend: DiscoveryBackend,
        domain: String,
    },
}

impl fmt::Display for DiscoveryOptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryOptionError::ServiceType { value, reason } => write!(
                f,
                "`{value}` is not a DNS-SD service type: {reason}. \
                 Use a type such as `_ssh._tcp` or `_dns-sd._udp`, \
                 or omit it to browse every service type",
            ),
            DiscoveryOptionError::UnsupportedDomain { backend, domain } => write!(
                f,
                "the `{backend}` backend cannot browse the `{domain}` domain: \
                 it can only browse the default `{DEFAULT_DOMAIN}` domain. \
                 Browse `{DEFAULT_DOMAIN}`, or select the `mdns-sd` backend, \
                 which supports custom domains",
            ),
        }
    }
}

impl std::error::Error for DiscoveryOptionError {}

/// Inputs a discovery backend needs, as requested. Keeps the discovery layer
/// decoupled from the CLI/UI layer (see [`crate::ui::cli::Cli::discovery_config`]).
///
/// This is an unchecked request: [`validate`](Self::validate) turns it into the
/// [`DiscoveryOptions`] an adapter can actually be started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryConfig {
    /// Use the built-in sample records instead of real mDNS discovery.
    pub fake: bool,
    /// Which mDNS/DNS-SD library to discover with when `fake` is unset.
    pub backend: DiscoveryBackend,
    /// DNS-SD domain to browse. Empty means the default domain.
    pub domain: String,
    /// Limit discovery to a single DNS-SD service type, if set. `None` means
    /// browse every supported/default type.
    pub service_type: Option<String>,
}

impl DiscoveryConfig {
    /// Check and canonicalize these inputs, once, before anything starts.
    ///
    /// Rejects a malformed service type, and a domain the selected backend
    /// cannot honor. Explicit fake discovery skips the *capability* check
    /// alone: it exercises no real adapter, so no adapter's limits apply to it
    /// — but its service type is still validated, because a filter that matches
    /// no sample is just as silently wrong as one that matches no device.
    pub fn validate(self) -> Result<DiscoveryOptions, DiscoveryOptionError> {
        let service_type = self
            .service_type
            .as_deref()
            .map(ServiceTypeFilter::parse)
            .transpose()?;
        let domain = canonical_domain(&self.domain);

        if !self.fake && domain != DEFAULT_DOMAIN && !self.backend.supports_custom_domain() {
            return Err(DiscoveryOptionError::UnsupportedDomain {
                backend: self.backend,
                domain,
            });
        }

        Ok(DiscoveryOptions {
            fake: self.fake,
            backend: self.backend,
            domain,
            service_type,
        })
    }
}

/// Validated, canonical discovery inputs: what [`start`](super::start) runs.
///
/// Holding one is proof that the service type parses, that the domain is
/// canonical, and that the selected backend can honor it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryOptions {
    fake: bool,
    backend: DiscoveryBackend,
    domain: String,
    service_type: Option<ServiceTypeFilter>,
}

impl DiscoveryOptions {
    /// Whether the caller explicitly asked for sample records.
    pub fn fake(&self) -> bool {
        self.fake
    }

    /// The selected backend. Meaningless when [`fake`](Self::fake) is set.
    pub fn backend(&self) -> DiscoveryBackend {
        self.backend
    }

    /// The canonical browse domain; never empty.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The single type to browse, or `None` to browse every supported type.
    pub fn service_type(&self) -> Option<&ServiceTypeFilter> {
        self.service_type.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(domain: &str, service_type: Option<&str>) -> DiscoveryConfig {
        DiscoveryConfig {
            fake: false,
            backend: DiscoveryBackend::MdnsSd,
            domain: domain.to_string(),
            service_type: service_type.map(str::to_string),
        }
    }

    fn service_type_error(value: &str) -> String {
        let err = ServiceTypeFilter::parse(value).unwrap_err();
        assert!(
            matches!(&err, DiscoveryOptionError::ServiceType { value: v, .. } if v == value),
            "the error must quote the offending value: {err:?}"
        );
        err.to_string()
    }

    #[test]
    fn valid_tcp_and_udp_service_types_are_accepted() {
        for (input, name, protocol) in [
            ("_ssh._tcp", "ssh", TransportProtocol::Tcp),
            ("_dns-sd._udp", "dns-sd", TransportProtocol::Udp),
            // 15 characters, digits and internal hyphens, ends alphanumeric.
            (
                "_a1-b2-c3-d4-e5._tcp",
                "a1-b2-c3-d4-e5",
                TransportProtocol::Tcp,
            ),
            ("_x._udp", "x", TransportProtocol::Udp),
        ] {
            let filter = ServiceTypeFilter::parse(input).expect(input);

            assert_eq!(filter.name(), name);
            assert_eq!(filter.protocol(), protocol);
            assert_eq!(filter.to_string(), input);
        }
    }

    /// DNS-SD types are case-insensitive, so a shouted type is the same browse
    /// as a quiet one and canonicalizes to it.
    #[test]
    fn service_types_are_canonicalized_to_lower_case() {
        let filter = ServiceTypeFilter::parse("_SSH._TCP").expect("case-insensitive");

        assert_eq!(filter.to_string(), "_ssh._tcp");
        assert_eq!(filter, ServiceTypeFilter::parse("_ssh._tcp").unwrap());
    }

    /// Each malformed value is rejected, and the message says which rule it
    /// broke rather than a bare "invalid value".
    #[test]
    fn malformed_service_types_are_rejected_with_the_rule_they_broke() {
        for (input, expected) in [
            ("not a service type", "begins with `_`"),
            ("ssh._tcp", "begins with `_`"),
            ("_ssh", "expected a name and a transport"),
            ("_ssh.tcp", "the transport begins with `_`"),
            ("_ssh._sctp", "must be `_tcp` or `_udp`"),
            // A trailing dot lands in the transport: `_tcp.` is not `_tcp`.
            ("_ssh._tcp.", "must be `_tcp` or `_udp`"),
            ("_ssh._tcp._x", "must be `_tcp` or `_udp`"),
            ("__tcp", "expected a name and a transport"),
            ("_._tcp", "the service name is empty"),
            ("_s h._tcp", "only ASCII letters, digits, and hyphens"),
            ("_s_h._tcp", "only ASCII letters, digits, and hyphens"),
            ("_sshé._tcp", "only ASCII letters, digits, and hyphens"),
            ("_abcdefghijklmnop._tcp", "at most 15 characters"),
            ("_-ssh._tcp", "begin and end with a letter or digit"),
            ("_ssh-._tcp", "begin and end with a letter or digit"),
            ("_s--h._tcp", "consecutive hyphens"),
            ("_123._tcp", "at least one letter"),
            ("_1-2._tcp", "at least one letter"),
        ] {
            let message = service_type_error(input);

            assert!(
                message.contains(expected),
                "`{input}` should be rejected for {expected:?}, got: {message}"
            );
        }
    }

    /// The remedy is part of the message: a rejected type must not leave the
    /// user guessing at the shape of a good one.
    #[test]
    fn a_rejected_service_type_names_the_value_and_the_remedy() {
        let message = service_type_error("bogus");

        assert!(message.contains("`bogus`"));
        assert!(message.contains("_ssh._tcp"));
        assert!(message.contains("omit it to browse every service type"));
    }

    /// A boundary the grammar draws: 15 characters is the longest legal name.
    #[test]
    fn the_service_name_length_limit_is_fifteen() {
        assert!(ServiceTypeFilter::parse("_abcdefghijklmno._tcp").is_ok());
        assert!(ServiceTypeFilter::parse("_abcdefghijklmnop._tcp").is_err());
    }

    /// `None` is not a malformed filter; it is the request to browse all types.
    #[test]
    fn no_service_type_means_browse_every_type() {
        let options = config("local", None).validate().unwrap();

        assert_eq!(options.service_type(), None);
    }

    /// Empty, `local`, and `local.` are the default domain spelled differently.
    #[test]
    fn the_default_domain_is_canonicalized_from_its_spellings() {
        for spelling in ["", "local", "local.", "LOCAL", "Local.", "LoCaL"] {
            let options = config(spelling, None).validate().unwrap();

            assert_eq!(
                options.domain(),
                DEFAULT_DOMAIN,
                "`{spelling}` names the default domain"
            );
        }
    }

    /// A domain that is not the default is the caller's, and survives intact:
    /// canonicalization must not quietly redirect a browse.
    #[test]
    fn a_custom_domain_is_passed_through_unchanged() {
        for domain in ["corp", "corp.example.com", "Corp"] {
            let options = config(domain, None).validate().unwrap();

            assert_eq!(options.domain(), domain);
        }
    }

    /// mdns-sd can set a browse domain, so a custom domain is honored.
    #[test]
    fn mdns_sd_accepts_a_custom_domain() {
        let options = config("corp", Some("_ssh._tcp")).validate().unwrap();

        assert_eq!(options.backend(), DiscoveryBackend::MdnsSd);
        assert_eq!(options.domain(), "corp");
        assert_eq!(options.service_type().unwrap().to_string(), "_ssh._tcp");
    }

    /// The zeroconf browser has no domain setter, so a custom domain is
    /// rejected here — before anything spawns — rather than silently becoming a
    /// browse of `local`.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn zeroconf_rejects_a_custom_domain_with_actionable_text() {
        let mut config = config("corp", None);
        config.backend = DiscoveryBackend::Zeroconf;

        let err = config.validate().unwrap_err();

        assert_eq!(
            err,
            DiscoveryOptionError::UnsupportedDomain {
                backend: DiscoveryBackend::Zeroconf,
                domain: "corp".to_string(),
            }
        );
        let message = err.to_string();
        assert!(message.contains("`zeroconf`"));
        assert!(message.contains("`corp`"));
        assert!(
            message.contains("mdns-sd"),
            "the remedy must name a backend that can: {message}"
        );
    }

    /// The capability check is about the *canonical* domain: `local.` is the
    /// default domain, so zeroconf can honor it exactly.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn zeroconf_accepts_every_spelling_of_the_default_domain() {
        for spelling in ["", "local", "local.", "LOCAL"] {
            let mut config = config(spelling, None);
            config.backend = DiscoveryBackend::Zeroconf;

            let options = config.validate().expect(spelling);

            assert_eq!(options.domain(), DEFAULT_DOMAIN);
        }
    }

    /// Explicit fake discovery exercises no real adapter, so no adapter's
    /// capability limits apply: sample records can carry any domain.
    #[cfg(feature = "zeroconf")]
    #[test]
    fn explicit_fake_mode_accepts_a_domain_no_real_backend_could() {
        let mut config = config("corp", None);
        config.backend = DiscoveryBackend::Zeroconf;
        config.fake = true;

        let options = config.validate().expect("fake bypasses capability checks");

        assert_eq!(options.domain(), "corp");
    }

    /// …but a malformed service type is still malformed in fake mode. It is not
    /// a backend capability: a filter that silently matches no sample is the
    /// same confusing outcome as one that matches no device.
    #[test]
    fn explicit_fake_mode_still_validates_the_service_type() {
        let mut config = config("corp", Some("not a service type"));
        config.fake = true;

        let err = config.validate().unwrap_err();

        assert!(matches!(err, DiscoveryOptionError::ServiceType { .. }));
    }
}
