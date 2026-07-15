use std::{collections::BTreeMap, net::IpAddr, str::FromStr, sync::mpsc};

use tokio_util::sync::CancellationToken;
use zeroconf_tokio::prelude::*;
use zeroconf_tokio::{
    BrowserEvent, MdnsBrowser, MdnsBrowserAsync, ServiceDiscovery, ServiceRemoval, ServiceType,
};

use super::session::DiscoverySession;
use super::worker::{BrowseOutcome, DiscoveryWorker, RuntimeFlavor};
use super::{DiscoveryConfig, DiscoveryEvent, Entry, Registration};

/// DNS-SD service types browsed when the user does not pass `--service-type`.
///
/// `zeroconf` (and the underlying Avahi/Bonjour APIs it wraps) browses one
/// concrete service type at a time; unlike `avahi-browse -a` there is no
/// meta-query that enumerates every type on the link. We therefore sweep a
/// curated set of the most common types in parallel.
const DEFAULT_SERVICE_TYPES: &[&str] = &[
    "_ssh._tcp",
    "_sftp-ssh._tcp",
    "_http._tcp",
    "_https._tcp",
    "_ipp._tcp",
    "_ipps._tcp",
    "_printer._tcp",
    "_smb._tcp",
    "_afpovertcp._tcp",
    "_nfs._tcp",
    "_webdav._tcp",
    "_ftp._tcp",
    "_workstation._tcp",
    "_device-info._tcp",
    "_rfb._tcp",
    "_airplay._tcp",
    "_raop._tcp",
    "_googlecast._tcp",
    "_homekit._tcp",
    "_spotify-connect._tcp",
];

/// Start the mDNS/Avahi discovery backend built on the `zeroconf` crate: it
/// sweeps the link for DNS-SD services and streams them as [`DiscoveryEvent`]s.
/// When no browser can be started it emits no entries and reports a
/// [`BrowseOutcome::Startup`] carrying the cause.
///
/// Unlike the [`mdns`](super::mdns) backend, `zeroconf` wraps the native
/// Avahi/Bonjour APIs which browse one concrete service type at a time, so this
/// backend sweeps [`DEFAULT_SERVICE_TYPES`] in parallel (on a multi-threaded
/// runtime) when no filter is given.
pub(super) fn start(config: &DiscoveryConfig) -> DiscoverySession {
    let service_types = resolve_service_types(config.service_type.as_deref());
    let (worker, rx) = DiscoveryWorker::spawn(
        config,
        RuntimeFlavor::MultiThread,
        move |domain, service_type_filter, tx, shutdown| {
            browse_loop(service_types, domain, service_type_filter, tx, shutdown)
        },
    );
    DiscoverySession::from_worker(rx, worker)
}

async fn browse_loop(
    service_types: Vec<ServiceType>,
    _domain: String,
    _service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) -> BrowseOutcome {
    let mut workers = Vec::new();
    for service_type in service_types {
        let label = format_service_type(&service_type);
        match MdnsBrowserAsync::new(MdnsBrowser::new(service_type)) {
            Ok(mut browser) => match browser.start().await {
                Ok(()) => {
                    let tx = tx.clone();
                    let token = shutdown.clone();
                    workers.push(tokio::spawn(browse_one(browser, tx, token)));
                }
                Err(err) => {
                    let _ = tx.send(DiscoveryEvent::Status(format!(
                        "could not browse {label} ({err})"
                    )));
                }
            },
            Err(err) => {
                let _ = tx.send(DiscoveryEvent::Status(format!(
                    "could not create browser for {label} ({err})"
                )));
            }
        }
    }

    // Not one service type could be browsed: the adapter never started. The
    // cause is returned rather than sent as a `Status`, so it becomes a
    // persistent failure state instead of a status line the next event erases.
    if workers.is_empty() {
        return BrowseOutcome::Startup(
            "mDNS discovery unavailable; try --fake-discovery for sample records, or refresh to retry"
                .to_string(),
        );
    }

    let _ = tx.send(DiscoveryEvent::Status(format!(
        "browsing {} service type(s) over mDNS",
        workers.len()
    )));

    for worker in workers {
        let _ = worker.await;
    }

    // Every per-type browser finished. That is either the shutdown they all
    // select on, or the browsers ending by themselves — which means discovery
    // is over and nothing further will arrive.
    if shutdown.is_cancelled() {
        BrowseOutcome::Cancelled
    } else {
        BrowseOutcome::Stopped
    }
}

async fn browse_one(
    mut browser: MdnsBrowserAsync,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = browser.shutdown().await;
                break;
            }
            event = browser.next() => {
                match event {
                    Some(Ok(event)) => {
                        if tx.send(to_discovery_event(event)).is_err() {
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        let _ = tx.send(DiscoveryEvent::Status(format!("mDNS browse error: {err}")));
                    }
                    None => break,
                }
            }
        }
    }
}

fn to_discovery_event(event: BrowserEvent) -> DiscoveryEvent {
    match event {
        BrowserEvent::Add(discovery) => DiscoveryEvent::Upsert(record_from_discovery(&discovery)),
        BrowserEvent::Remove(removal) => {
            DiscoveryEvent::RemoveRegistration(registration_from_removal(&removal))
        }
    }
}

fn record_from_discovery(discovery: &ServiceDiscovery) -> Entry {
    let txt = discovery
        .txt()
        .as_ref()
        .map(|txt| txt.iter().collect::<BTreeMap<String, String>>())
        .unwrap_or_default();

    // `zeroconf` resolves a service to a single address; carry it as the sole
    // element of the multi-address list the rest of the program expects.
    let addresses = discovery
        .address()
        .parse::<IpAddr>()
        .ok()
        .into_iter()
        .collect();

    Entry::resolved(
        discovery.name(),
        &format_service_type(discovery.service_type()),
        discovery.domain(),
        Some(discovery.host_name()),
        addresses,
        Some(*discovery.port()),
        txt,
    )
}

/// `zeroconf` reports a removal as a bare name/type/domain: it exposes nothing
/// that would tell two occurrences of one registration apart, so the adapter
/// cannot narrow this to a single occurrence and says the registration is gone.
fn registration_from_removal(removal: &ServiceRemoval) -> Registration {
    Registration::new(removal.name(), removal.kind(), removal.domain())
}

fn format_service_type(service_type: &ServiceType) -> String {
    format!("_{}._{}", service_type.name(), service_type.protocol())
}

fn resolve_service_types(filter: Option<&str>) -> Vec<ServiceType> {
    if let Some(filter) = filter
        && let Ok(service_type) = ServiceType::from_str(filter)
    {
        return vec![service_type];
    }

    DEFAULT_SERVICE_TYPES
        .iter()
        .filter_map(|kind| ServiceType::from_str(kind).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_filter_browses_single_type() {
        let types = resolve_service_types(Some("_ssh._tcp"));
        assert_eq!(types.len(), 1);
        assert_eq!(format_service_type(&types[0]), "_ssh._tcp");
    }

    #[test]
    fn missing_filter_falls_back_to_default_sweep() {
        let types = resolve_service_types(None);
        assert_eq!(types.len(), DEFAULT_SERVICE_TYPES.len());
    }

    #[test]
    fn unparseable_filter_falls_back_to_default_sweep() {
        let types = resolve_service_types(Some("not a service type"));
        assert_eq!(types.len(), DEFAULT_SERVICE_TYPES.len());
    }

    /// `zeroconf` reports a removal with nothing that distinguishes one
    /// occurrence of a registration from another, so the adapter must say the
    /// whole registration is gone rather than guess at a single occurrence.
    #[test]
    fn removal_is_an_explicit_registration_wide_removal() {
        let removal = ServiceRemoval::builder()
            .name("nas".to_string())
            .kind("_http._tcp".to_string())
            .domain("local".to_string())
            .build()
            .expect("service removal");

        match to_discovery_event(BrowserEvent::Remove(removal)) {
            DiscoveryEvent::RemoveRegistration(registration) => {
                assert_eq!(
                    registration,
                    Registration::new("nas", "_http._tcp", "local")
                );
            }
            other => panic!("expected RemoveRegistration, got {other:?}"),
        }
    }

    /// No configured service type can start a browser (simulated here with an
    /// empty type list, deterministic regardless of Avahi availability): the
    /// loop must report a startup failure with its cause and never fabricate an
    /// entry. A user whose Avahi is down must not be handed sample devices they
    /// could act on.
    #[tokio::test]
    async fn no_started_workers_is_a_startup_failure_with_no_upsert() {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();

        let outcome = browse_loop(Vec::new(), "local".to_string(), None, tx, shutdown).await;

        match outcome {
            BrowseOutcome::Startup(cause) => {
                assert!(cause.contains("mDNS discovery unavailable"));
                assert!(cause.contains("refresh to retry"));
            }
            other => panic!("expected a startup failure, got {other:?}"),
        }
        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events.is_empty(),
            "a failed zeroconf start must emit no events at all, and above all no sample Upsert: {events:?}"
        );
    }

    /// The whole `zeroconf` session, not just its loop: a failed start must
    /// surface as a typed failure carrying the cause, with no sample records.
    #[test]
    fn a_failed_zeroconf_session_reports_a_typed_failure_and_no_samples() {
        use crate::discovery::{FailureKind, SessionPoll, SessionState};

        // An unparseable service type would fall back to the default sweep, so
        // drive the no-browser case through the loop the session runs.
        let (worker, rx) = DiscoveryWorker::spawn(
            &DiscoveryConfig {
                fake: false,
                backend: crate::discovery::DiscoveryBackend::Zeroconf,
                domain: "local".to_string(),
                service_type: None,
            },
            RuntimeFlavor::MultiThread,
            move |domain, service_type_filter, tx, shutdown| {
                browse_loop(Vec::new(), domain, service_type_filter, tx, shutdown)
            },
        );
        let mut session = DiscoverySession::from_worker(rx, worker);

        let mut events = Vec::new();
        let state = loop {
            match session.poll() {
                SessionPoll::Event(event) => events.push(event),
                SessionPoll::Idle => std::thread::yield_now(),
                SessionPoll::Ended(state) => break state,
            }
        };

        match state {
            SessionState::Failed(failure) => {
                assert_eq!(failure.kind, FailureKind::Startup);
                assert!(failure.cause.contains("mDNS discovery unavailable"));
            }
            other => panic!("expected a failed session, got {other:?}"),
        }
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, DiscoveryEvent::Upsert(_))),
            "no sample Upsert may reach the UI on a real adapter failure"
        );
    }
}
