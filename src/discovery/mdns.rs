use std::{collections::BTreeMap, net::IpAddr, sync::mpsc, thread};

use mdns_sd_discovery::{
    BrowseEvent, DiscoveredService, RemovedService, ServiceBrowserBuilder, TxtRecord,
};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;

use super::fake;
use super::{Discovery, DiscoveryConfig, DiscoveryEvent, Entry, EntryId};

/// mDNS/Avahi discovery backend: browses the link for DNS-SD services via the
/// `mdns-sd-discovery` crate and streams them as [`DiscoveryEvent`]s. Falls back
/// to the [`fake`] backend when the browse cannot be started.
///
/// Unlike the previous `zeroconf` backend, `mdns-sd-discovery` exposes the
/// native DNS-SD service-type enumeration meta-query, so a single browser
/// discovers every service type on the network — there is no need to sweep a
/// curated list of types in parallel.
pub struct MdnsDiscovery {
    receiver: Option<mpsc::Receiver<DiscoveryEvent>>,
    shutdown: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
}

impl MdnsDiscovery {
    pub fn start(config: &DiscoveryConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();
        let worker = spawn_browser(config, tx, shutdown.clone());
        Self {
            receiver: Some(rx),
            shutdown,
            worker: Some(worker),
        }
    }
}

impl Discovery for MdnsDiscovery {
    fn events(&mut self) -> mpsc::Receiver<DiscoveryEvent> {
        self.receiver
            .take()
            .expect("discovery receiver can only be taken once")
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn spawn_browser(
    config: &DiscoveryConfig,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) -> thread::JoinHandle<()> {
    let domain = config.domain.clone();
    let service_type_filter = config.service_type.clone();

    thread::spawn(move || {
        let runtime = match Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send(DiscoveryEvent::Status(format!(
                    "failed to start mDNS runtime ({err}); using sample records"
                )));
                fake::spawn(domain, service_type_filter, tx);
                return;
            }
        };

        runtime.block_on(browse_loop(domain, service_type_filter, tx, shutdown));
    })
}

async fn browse_loop(
    domain: String,
    service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) {
    let mut builder = ServiceBrowserBuilder::new();
    if let Some(service_type) = &service_type_filter {
        builder.service_type(service_type);
    }
    // An empty or `local` domain means "use the default browse domain", which
    // the crate handles when no domain is set.
    if !domain.is_empty() && domain != "local" {
        builder.domain(&domain);
    }

    let mut browser = match builder.browse().await {
        Ok(browser) => browser,
        Err(err) => {
            let _ = tx.send(DiscoveryEvent::Status(format!(
                "mDNS discovery unavailable ({err}); using sample records"
            )));
            fake::spawn(domain, service_type_filter, tx);
            return;
        }
    };

    let _ = tx.send(DiscoveryEvent::Status(match &service_type_filter {
        Some(service_type) => format!("browsing {service_type} over mDNS"),
        None => "browsing all DNS-SD service types over mDNS".to_string(),
    }));

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            event = browser.recv() => match event {
                Some(Ok(event)) => {
                    if !emit_event(event, &tx) {
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
    // Dropping `browser` stops the underlying native browse operation.
}

/// Translates a [`BrowseEvent`] into [`DiscoveryEvent`]s and sends them.
/// Returns `false` once the receiver has been dropped so the caller can stop.
fn emit_event(event: BrowseEvent, tx: &mpsc::Sender<DiscoveryEvent>) -> bool {
    match event {
        BrowseEvent::Found(service) => tx
            .send(DiscoveryEvent::Upsert(record_from_service(&service)))
            .is_ok(),
        BrowseEvent::Removed(service) => {
            tx.send(DiscoveryEvent::Remove(id_from_removal(&service)))
                .is_ok()
        }
    }
}

/// Builds the resolved [`Entry`] for a discovered service. A service may resolve
/// to several IP addresses (IPv4/IPv6, or DNS load-balanced records); they are
/// all carried on the single logical-service entry — consumers pick among them
/// when a specific endpoint is needed.
fn record_from_service(service: &DiscoveredService) -> Entry {
    upsert_record(
        &service.name,
        &service.service_type,
        &service.domain,
        Some(service.host_name.as_str()),
        service.addresses.clone(),
        Some(service.port),
        txt_map(&service.txt_records),
    )
}

fn id_from_removal(removal: &RemovedService) -> EntryId {
    Entry::new(&removal.name, &removal.service_type, &removal.domain)
        .with_instance_id()
        .id
}

/// Collapses DNS-SD TXT records into the string map [`Entry`] carries. Binary
/// values are decoded lossily; a key-only entry maps to an empty value.
fn txt_map(records: &[TxtRecord]) -> BTreeMap<String, String> {
    records
        .iter()
        .map(|record| {
            let value = record
                .value
                .as_deref()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            (record.key.clone(), value)
        })
        .collect()
}

/// Builds a resolved [`Entry`] from the individual fields reported by a browse
/// event. Kept separate from the `mdns-sd-discovery` types so it can be unit
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

#[cfg(test)]
mod tests {
    use super::*;

    fn txt_record(key: &str, value: Option<&str>) -> TxtRecord {
        TxtRecord {
            key: key.to_string(),
            value: value.map(|v| v.as_bytes().to_vec()),
        }
    }

    fn service(name: &str, service_type: &str, addresses: Vec<IpAddr>) -> DiscoveredService {
        DiscoveredService {
            name: name.to_string(),
            service_type: service_type.to_string(),
            domain: "local".to_string(),
            host_name: format!("{name}.local"),
            port: 8080,
            addresses,
            txt_records: vec![txt_record("path", Some("/admin"))],
            interface_index: None,
        }
    }

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
        let record =
            upsert_record("pending", "_ipp._tcp", "local", Some(""), Vec::new(), Some(0), BTreeMap::new());

        assert_eq!(record.hostname, None);
        assert!(record.addresses.is_empty());
        assert_eq!(record.port, None);
        assert!(!record.has_instance_data());
    }

    #[test]
    fn all_addresses_land_on_one_record() {
        let svc = service(
            "workstation",
            "_ssh._tcp",
            vec![
                "192.168.1.20".parse().unwrap(),
                "192.168.1.21".parse().unwrap(),
            ],
        );

        let record = record_from_service(&svc);
        assert_eq!(record.addresses.len(), 2);
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
    }

    #[test]
    fn service_without_address_is_still_one_record() {
        let svc = service("pending-printer", "_ipp._tcp", Vec::new());

        let record = record_from_service(&svc);
        assert!(record.addresses.is_empty());
        assert_eq!(record.hostname.as_deref(), Some("pending-printer.local"));
    }

    #[test]
    fn txt_map_decodes_values_and_key_only_entries() {
        let records = vec![
            txt_record("path", Some("/admin")),
            txt_record("secure", None),
        ];

        let map = txt_map(&records);
        assert_eq!(map.get("path").map(String::as_str), Some("/admin"));
        assert_eq!(map.get("secure").map(String::as_str), Some(""));
    }

    #[test]
    fn removal_id_matches_pending_entry() {
        let removal = RemovedService {
            name: "nas".to_string(),
            service_type: "_http._tcp".to_string(),
            domain: "local".to_string(),
            interface_index: None,
        };

        let expected = Entry::new("nas", "_http._tcp", "local").pending_id();
        assert_eq!(id_from_removal(&removal), expected);
    }
}
