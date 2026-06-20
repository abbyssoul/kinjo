use std::{collections::BTreeMap, net::IpAddr, str::FromStr, sync::mpsc, thread};

use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use zeroconf_tokio::prelude::*;
use zeroconf_tokio::{
    BrowserEvent, MdnsBrowser, MdnsBrowserAsync, ServiceDiscovery, ServiceRemoval, ServiceType,
};

use super::fake;
use super::{Discovery, DiscoveryConfig, DiscoveryEvent, Entry, EntryId};

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

/// mDNS/Avahi discovery backend built on the `zeroconf` crate: sweeps the link
/// for DNS-SD services and streams them as [`DiscoveryEvent`]s. Falls back to the
/// [`fake`] backend when no browser can be started.
///
/// Unlike the [`mdns`](super::mdns) backend, `zeroconf` wraps the native
/// Avahi/Bonjour APIs which browse one concrete service type at a time, so this
/// backend sweeps [`DEFAULT_SERVICE_TYPES`] in parallel when no filter is given.
pub struct ZeroconfDiscovery {
    receiver: Option<mpsc::Receiver<DiscoveryEvent>>,
    shutdown: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
}

impl ZeroconfDiscovery {
    pub fn start(config: &DiscoveryConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();
        let worker = spawn_zeroconf(config, tx, shutdown.clone());
        Self {
            receiver: Some(rx),
            shutdown,
            worker: Some(worker),
        }
    }
}

impl Discovery for ZeroconfDiscovery {
    fn events(&mut self) -> mpsc::Receiver<DiscoveryEvent> {
        self.receiver
            .take()
            .expect("discovery receiver can only be taken once")
    }
}

impl Drop for ZeroconfDiscovery {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn spawn_zeroconf(
    config: &DiscoveryConfig,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) -> thread::JoinHandle<()> {
    let domain = config.domain.clone();
    let service_type_filter = config.service_type.clone();
    let service_types = resolve_service_types(service_type_filter.as_deref());

    thread::spawn(move || {
        let runtime = match Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send(DiscoveryEvent::Status(format!(
                    "failed to start mDNS runtime ({err}); using sample records"
                )));
                fake::spawn(domain, service_type_filter, tx);
                return;
            }
        };

        runtime.block_on(browse_loop(
            service_types,
            domain,
            service_type_filter,
            tx,
            shutdown,
        ));
    })
}

async fn browse_loop(
    service_types: Vec<ServiceType>,
    domain: String,
    service_type_filter: Option<String>,
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
            "mDNS discovery unavailable; using sample records".to_string(),
        ));
        fake::spawn(domain, service_type_filter, tx);
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

    upsert_record(
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
    Entry::new(removal.name(), removal.kind(), removal.domain())
        .with_instance_id()
        .id
}

/// Builds a resolved [`Entry`] from the individual fields reported by a
/// browse event. Kept separate from the `zeroconf` types so it can be unit
/// tested without standing up the mDNS stack.
fn upsert_record(
    name: &str,
    service_type: &str,
    domain: &str,
    hostname: Option<&str>,
    addresses: Vec<IpAddr>,
    port: Option<u16>,
    txt: BTreeMap<String, String>,
) -> Entry {
    let mut record = Entry::new(name, service_type, domain);
    record.hostname = hostname
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    record.addresses = addresses;
    record.port = port.filter(|port| *port != 0);
    record.txt = txt;
    record.with_instance_id()
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
    fn builds_resolved_record_from_browse_fields() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/admin".to_string());

        let record = upsert_record(
            "nas",
            "_http._tcp",
            "local",
            Some("nas.local"),
            vec!["192.168.1.30".parse().unwrap()],
            Some(8080),
            txt,
        );

        assert_eq!(record.name, "nas");
        assert_eq!(record.hostname.as_deref(), Some("nas.local"));
        assert_eq!(record.primary_address(), Some("192.168.1.30".parse().unwrap()));
        assert_eq!(record.port, Some(8080));
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
        assert!(record.has_instance_data());
    }

    #[test]
    fn blank_host_and_zero_port_become_unresolved() {
        let record = upsert_record(
            "pending",
            "_ipp._tcp",
            "local",
            Some(""),
            Vec::new(),
            Some(0),
            BTreeMap::new(),
        );

        assert_eq!(record.hostname, None);
        assert!(record.addresses.is_empty());
        assert_eq!(record.port, None);
        assert!(!record.has_instance_data());
    }

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
}
