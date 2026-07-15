use std::{collections::BTreeMap, net::IpAddr, str::FromStr, sync::mpsc};

use tokio_util::sync::CancellationToken;
use zeroconf_tokio::prelude::*;
use zeroconf_tokio::{
    BrowserEvent, MdnsBrowser, MdnsBrowserAsync, ServiceDiscovery, ServiceRemoval, ServiceType,
};

use super::worker::{DiscoveryWorker, RuntimeFlavor};
use super::{DiscoveryConfig, DiscoveryEvent, Entry, EntryId};

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
/// Reports a [`DiscoveryEvent::Status`] and emits no entries when no browser
/// can be started.
///
/// Unlike the [`mdns`](super::mdns) backend, `zeroconf` wraps the native
/// Avahi/Bonjour APIs which browse one concrete service type at a time, so this
/// backend sweeps [`DEFAULT_SERVICE_TYPES`] in parallel (on a multi-threaded
/// runtime) when no filter is given.
pub(super) fn start(config: &DiscoveryConfig) -> DiscoveryWorker {
    let service_types = resolve_service_types(config.service_type.as_deref());
    DiscoveryWorker::spawn(
        config,
        RuntimeFlavor::MultiThread,
        move |domain, service_type_filter, tx, shutdown| {
            browse_loop(service_types, domain, service_type_filter, tx, shutdown)
        },
    )
}

async fn browse_loop(
    service_types: Vec<ServiceType>,
    _domain: String,
    _service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) {
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

    if workers.is_empty() {
        let _ = tx.send(DiscoveryEvent::Status(
            "mDNS discovery unavailable; try --fake-discovery for sample records, or refresh to retry"
                .to_string(),
        ));
        return;
    }

    let _ = tx.send(DiscoveryEvent::Status(format!(
        "browsing {} service type(s) over mDNS",
        workers.len()
    )));

    for worker in workers {
        let _ = worker.await;
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
        BrowserEvent::Remove(removal) => DiscoveryEvent::Remove(id_from_removal(&removal)),
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

fn id_from_removal(removal: &ServiceRemoval) -> EntryId {
    EntryId::registration(removal.name(), removal.kind(), removal.domain())
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

    /// No configured service type can start a browser (simulated here with an
    /// empty type list, deterministic regardless of Avahi availability): the
    /// loop must report only a `Status`, never fabricate an entry.
    #[tokio::test]
    async fn no_started_workers_emits_status_only_no_upsert() {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();

        browse_loop(Vec::new(), "local".to_string(), None, tx, shutdown).await;

        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(events.as_slice(), [DiscoveryEvent::Status(_)]));
    }
}
