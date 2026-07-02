use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::mpsc,
    thread,
    time::Duration,
};

use mdns_sd_discovery::{
    BrowseEvent, DiscoveredService, RemovedService, ServiceBrowserBuilder, ServiceResolverBuilder,
    TxtRecord,
};
use futures::future::join_all;
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;

use super::fake;
use super::{Discovery, DiscoveryConfig, DiscoveryEvent, Entry, EntryId};

/// How often known services are re-confirmed with a one-shot resolve.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound for a single liveness resolve.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Consecutive failed probes after which a service is reported as removed.
const PROBE_FAILURE_THRESHOLD: u32 = 3;

/// The identity triple (name, service type, domain) a browse event reports.
type ServiceKey = (String, String, String);

#[derive(Debug, Default)]
struct TrackedService {
    /// Consecutive failed liveness probes.
    failures: u32,
    /// Whether a `Remove` has been emitted for this service. It stays tracked
    /// so a service that was merely unreachable can be re-announced when a
    /// probe succeeds again — Avahi does not repeat its `ItemNew` while the
    /// browse-driving PTR record is still cached.
    removed: bool,
}

/// Decides when a quiet service is declared dead, and when it has come back.
///
/// mDNS browsing is edge-triggered: a service that dies *without* multicasting
/// goodbye packets produces no `Removed` event until Avahi's cached PTR record
/// expires (typically 75 minutes). The tracker keeps the set of services the
/// browser has reported so the browse loop can re-confirm them periodically
/// with one-shot resolves; a service whose probes fail
/// [`PROBE_FAILURE_THRESHOLD`] times in a row is reported removed within a few
/// probe cycles instead.
#[derive(Debug, Default)]
struct LivenessTracker {
    services: HashMap<ServiceKey, TrackedService>,
}

impl LivenessTracker {
    /// A browse event (re-)announced the service: it is alive.
    fn note_found(&mut self, key: ServiceKey) {
        self.services.insert(key, TrackedService::default());
    }

    /// Avahi reported the service removed: it is authoritatively gone, and a
    /// reappearance will produce a fresh `Found` event.
    fn note_removed(&mut self, key: &ServiceKey) {
        self.services.remove(key);
    }

    /// The services the next probe cycle should re-confirm.
    fn probe_keys(&self) -> Vec<ServiceKey> {
        self.services.keys().cloned().collect()
    }

    /// Records a successful probe. Returns `true` when the service had been
    /// reported removed and should be re-announced.
    fn record_success(&mut self, key: &ServiceKey) -> bool {
        let Some(service) = self.services.get_mut(key) else {
            return false;
        };
        let reappeared = service.removed;
        service.failures = 0;
        service.removed = false;
        reappeared
    }

    /// Records a failed probe. Returns `true` when this failure crossed the
    /// threshold and the service should be reported removed now.
    fn record_failure(&mut self, key: &ServiceKey) -> bool {
        let Some(service) = self.services.get_mut(key) else {
            return false;
        };
        if service.removed {
            return false;
        }
        service.failures += 1;
        if service.failures >= PROBE_FAILURE_THRESHOLD {
            service.removed = true;
            return true;
        }
        false
    }
}

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

    // Browse events alone leave silently-dead services listed until Avahi's
    // PTR cache expires; periodic liveness probes fill that gap (see
    // [`LivenessTracker`]).
    let mut tracker = LivenessTracker::default();
    let mut probe_timer = tokio::time::interval(PROBE_INTERVAL);
    probe_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = probe_timer.tick() => {
                let keys = tracker.probe_keys();
                if keys.is_empty() {
                    continue;
                }
                // A probe cycle is bounded by PROBE_TIMEOUT, but stay
                // responsive to shutdown while it runs.
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    results = probe_services(keys) => {
                        if !apply_probe_results(results, &mut tracker, &tx) {
                            break;
                        }
                    }
                }
            }
            event = browser.recv() => match event {
                Some(Ok(event)) => {
                    track_event(&event, &mut tracker);
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

/// Keeps the liveness tracker in sync with what the browser reports.
fn track_event(event: &BrowseEvent, tracker: &mut LivenessTracker) {
    match event {
        BrowseEvent::Found(service) => tracker.note_found((
            service.name.clone(),
            service.service_type.clone(),
            service.domain.clone(),
        )),
        BrowseEvent::Removed(removal) => tracker.note_removed(&(
            removal.name.clone(),
            removal.service_type.clone(),
            removal.domain.clone(),
        )),
    }
}

/// Probes each service with a bounded one-shot resolve, concurrently. Returns
/// each key with the resolved data on success or `None` when the service did
/// not answer in time.
async fn probe_services(keys: Vec<ServiceKey>) -> Vec<(ServiceKey, Option<DiscoveredService>)> {
    // Joined in-task rather than spawned: the resolve future is not `Send` on
    // Windows (it holds raw PWSTR pointers across an await), and the browse
    // loop runs on a current-thread runtime anyway.
    let probes = keys.into_iter().map(|key| async move {
        let result = ServiceResolverBuilder::new(&key.0, &key.1, &key.2)
            .timeout(PROBE_TIMEOUT)
            .resolve()
            .await;
        (key, result.ok())
    });
    join_all(probes).await
}

/// Feeds probe outcomes into the tracker and emits the resulting events:
/// `Remove` for services that crossed the failure threshold and `Upsert` for
/// previously-removed services that answered again. Returns `false` once the
/// receiver has been dropped so the caller can stop.
fn apply_probe_results(
    results: Vec<(ServiceKey, Option<DiscoveredService>)>,
    tracker: &mut LivenessTracker,
    tx: &mpsc::Sender<DiscoveryEvent>,
) -> bool {
    for (key, outcome) in results {
        let sent = match outcome {
            Some(service) => {
                !tracker.record_success(&key)
                    || tx
                        .send(DiscoveryEvent::Upsert(record_from_service(&service)))
                        .is_ok()
            }
            None => {
                !tracker.record_failure(&key)
                    || tx
                        .send(DiscoveryEvent::Remove(EntryId::registration(
                            key.0, key.1, key.2,
                        )))
                        .is_ok()
            }
        };
        if !sent {
            return false;
        }
    }
    true
}

/// Translates a [`BrowseEvent`] into [`DiscoveryEvent`]s and sends them.
/// Returns `false` once the receiver has been dropped so the caller can stop.
fn emit_event(event: BrowseEvent, tx: &mpsc::Sender<DiscoveryEvent>) -> bool {
    match event {
        BrowseEvent::Found(service) => tx
            .send(DiscoveryEvent::Upsert(record_from_service(&service)))
            .is_ok(),
        BrowseEvent::Removed(service) => tx
            .send(DiscoveryEvent::Remove(id_from_removal(&service)))
            .is_ok(),
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
    EntryId::registration(&removal.name, &removal.service_type, &removal.domain)
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
        assert_eq!(
            record.primary_address(),
            Some("192.168.1.30".parse().unwrap())
        );
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

    fn key(name: &str) -> ServiceKey {
        (
            name.to_string(),
            "_http._tcp".to_string(),
            "local".to_string(),
        )
    }

    #[test]
    fn tracker_reports_removal_only_at_the_failure_threshold() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key("nas"));

        for _ in 1..PROBE_FAILURE_THRESHOLD {
            assert!(!tracker.record_failure(&key("nas")));
        }
        assert!(tracker.record_failure(&key("nas")));
        // Already reported removed: further failures stay quiet.
        assert!(!tracker.record_failure(&key("nas")));
    }

    #[test]
    fn tracker_success_resets_the_failure_count() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key("nas"));

        for _ in 1..PROBE_FAILURE_THRESHOLD {
            assert!(!tracker.record_failure(&key("nas")));
        }
        assert!(!tracker.record_success(&key("nas")));

        // The streak starts over: the threshold is counted from scratch.
        for _ in 1..PROBE_FAILURE_THRESHOLD {
            assert!(!tracker.record_failure(&key("nas")));
        }
        assert!(tracker.record_failure(&key("nas")));
    }

    #[test]
    fn tracker_reannounces_a_service_that_answers_after_removal() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key("nas"));

        for _ in 0..PROBE_FAILURE_THRESHOLD {
            tracker.record_failure(&key("nas"));
        }
        // The service answers again: it must be re-announced (Avahi will not
        // repeat ItemNew while its cached PTR record lives).
        assert!(tracker.record_success(&key("nas")));
        // …but only once; it is alive from here on.
        assert!(!tracker.record_success(&key("nas")));
        assert!(tracker.probe_keys().contains(&key("nas")));
    }

    #[test]
    fn tracker_forgets_services_avahi_removed() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key("nas"));
        tracker.note_removed(&key("nas"));

        assert!(tracker.probe_keys().is_empty());
        // Stale probe results for a forgotten service are ignored.
        assert!(!tracker.record_failure(&key("nas")));
        assert!(!tracker.record_success(&key("nas")));
    }

    #[test]
    fn probe_results_emit_remove_after_threshold_and_upsert_on_recovery() {
        let mut tracker = LivenessTracker::default();
        let (tx, rx) = mpsc::channel();
        let nas = service("nas", "_http._tcp", vec!["192.168.1.30".parse().unwrap()]);
        tracker.note_found(key("nas"));

        // Failures up to the threshold: exactly one Remove comes out.
        for _ in 0..PROBE_FAILURE_THRESHOLD {
            assert!(apply_probe_results(
                vec![(key("nas"), None)],
                &mut tracker,
                &tx
            ));
        }
        match rx.try_recv() {
            Ok(DiscoveryEvent::Remove(id)) => {
                assert_eq!(id.registration_key(), ("nas", "_http._tcp", "local"));
            }
            other => panic!("expected Remove, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());

        // A successful probe re-announces the service with its resolved data.
        assert!(apply_probe_results(
            vec![(key("nas"), Some(nas))],
            &mut tracker,
            &tx
        ));
        match rx.try_recv() {
            Ok(DiscoveryEvent::Upsert(entry)) => {
                assert_eq!(entry.name, "nas");
                assert_eq!(entry.hostname.as_deref(), Some("nas.local"));
            }
            other => panic!("expected Upsert, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
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
