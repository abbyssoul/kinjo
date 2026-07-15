use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU32,
    sync::mpsc,
    time::Duration,
};

use futures::future::join_all;
use mdns_sd_discovery::{
    BrowseEvent, DiscoveredService, RemovedService, ServiceBrowserBuilder, ServiceResolverBuilder,
    TxtRecord,
};
use tokio_util::sync::CancellationToken;

use super::worker::{DiscoveryWorker, RuntimeFlavor};
use super::{DiscoveryConfig, DiscoveryEvent, Entry, EntryId, OccurrenceId, Registration};

/// How often known services are re-confirmed with a one-shot resolve.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound for a single liveness resolve.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Consecutive failed probes after which a service is reported as removed.
const PROBE_FAILURE_THRESHOLD: u32 = 3;

/// The occurrence a browse event reported: the registration it announced, plus
/// the network interface the announcement arrived on when the browser named
/// one. Two interfaces carrying the same registration are two occurrences and
/// are tracked, probed, and removed independently.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServiceKey {
    registration: Registration,
    interface_index: Option<NonZeroU32>,
}

impl ServiceKey {
    /// The adapter's occurrence name for this key: the interface index, when
    /// the browser reported one.
    fn occurrence(&self) -> Option<OccurrenceId> {
        self.interface_index.map(OccurrenceId)
    }

    /// How this occurrence goes away. An interface index names exactly one
    /// occurrence; without one the adapter cannot tell its occurrences apart
    /// and must remove the whole registration (see
    /// [`DiscoveryEvent::RemoveRegistration`]).
    fn removal_event(&self) -> DiscoveryEvent {
        match self.occurrence() {
            Some(occurrence) => {
                DiscoveryEvent::Remove(EntryId::named(self.registration.clone(), occurrence))
            }
            None => DiscoveryEvent::RemoveRegistration(self.registration.clone()),
        }
    }
}

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
    /// A browse event (re-)announced the occurrence: it is alive.
    fn note_found(&mut self, key: ServiceKey) {
        self.services.insert(key, TrackedService::default());
    }

    /// Avahi reported the occurrence removed: it is authoritatively gone, and a
    /// reappearance will produce a fresh `Found` event.
    fn note_removed(&mut self, key: &ServiceKey) {
        self.services.remove(key);
    }

    /// Avahi reported a removal it could not attribute to an interface: every
    /// occurrence of the registration is gone, so stop probing all of them.
    fn note_registration_removed(&mut self, registration: &Registration) {
        self.services
            .retain(|key, _| key.registration != *registration);
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

/// Start the mDNS/Avahi discovery backend: it browses the link for DNS-SD
/// services via the `mdns-sd-discovery` crate and streams them as
/// [`DiscoveryEvent`]s. Reports a [`DiscoveryEvent::Status`] and emits no
/// entries when the browse cannot be started.
///
/// Unlike the `zeroconf` backend, `mdns-sd-discovery` exposes the native
/// DNS-SD service-type enumeration meta-query, so a single browser discovers
/// every service type on the network — there is no need to sweep a curated
/// list of types in parallel. The probe futures are not `Send` on every
/// platform (Windows holds raw pointers across awaits), so the loop runs on a
/// current-thread runtime and drives them itself.
pub(super) fn start(config: &DiscoveryConfig) -> DiscoveryWorker {
    DiscoveryWorker::spawn(config, RuntimeFlavor::CurrentThread, browse_loop)
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
                "mDNS discovery unavailable ({err}); try --fake-discovery for sample records, or refresh to retry"
            )));
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
        BrowseEvent::Found(service) => tracker.note_found(key_from_service(service)),
        BrowseEvent::Removed(removal) => {
            let key = key_from_removal(removal);
            // An unattributed removal is registration-wide (see
            // `ServiceKey::removal_event`); forget every occurrence with it, not
            // just the one keyed by "no interface".
            match key.interface_index {
                Some(_) => tracker.note_removed(&key),
                None => tracker.note_registration_removed(&key.registration),
            }
        }
    }
}

/// Probes each occurrence with a bounded one-shot resolve, concurrently.
/// Returns each key with the resolved data on success or `None` when it did not
/// answer in time.
async fn probe_services(keys: Vec<ServiceKey>) -> Vec<(ServiceKey, Option<DiscoveredService>)> {
    // Joined in-task rather than spawned: the resolve future is not `Send` on
    // Windows (it holds raw PWSTR pointers across an await), and the browse
    // loop runs on a current-thread runtime anyway.
    let probes = keys.into_iter().map(|key| async move {
        let mut builder = ServiceResolverBuilder::new(
            &key.registration.name,
            &key.registration.service_type,
            &key.registration.domain,
        );
        builder.timeout(PROBE_TIMEOUT);
        // Confine the probe to the interface this occurrence was announced on.
        // An unconfined resolve answers from any interface still carrying the
        // registration, which would report a dead occurrence as alive for as
        // long as one sibling survives.
        if let Some(index) = key.interface_index {
            builder.interface_index(index);
        }
        let result = builder.resolve().await;
        (key, result.ok())
    });
    join_all(probes).await
}

/// Feeds probe outcomes into the tracker and emits the resulting events: a
/// removal for occurrences that crossed the failure threshold and `Upsert` for
/// previously-removed occurrences that answered again. Returns `false` once the
/// receiver has been dropped so the caller can stop.
fn apply_probe_results(
    results: Vec<(ServiceKey, Option<DiscoveredService>)>,
    tracker: &mut LivenessTracker,
    tx: &mpsc::Sender<DiscoveryEvent>,
) -> bool {
    for (key, outcome) in results {
        let sent = match outcome {
            Some(service) => {
                // The tracked key's occurrence, not the resolve's own: it is
                // what the browse event named this occurrence, so the upsert
                // lands on the record being probed rather than forking a new one.
                !tracker.record_success(&key)
                    || tx
                        .send(DiscoveryEvent::Upsert(
                            record_from_service(&service).with_occurrence(key.occurrence()),
                        ))
                        .is_ok()
            }
            None => !tracker.record_failure(&key) || tx.send(key.removal_event()).is_ok(),
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
    let event = match event {
        BrowseEvent::Found(service) => DiscoveryEvent::Upsert(record_from_service(&service)),
        BrowseEvent::Removed(removal) => key_from_removal(&removal).removal_event(),
    };
    tx.send(event).is_ok()
}

/// Builds the resolved [`Entry`] for a discovered service. A service may resolve
/// to several IP addresses (IPv4/IPv6, or DNS load-balanced records); they are
/// all carried on the single logical-service entry — consumers pick among them
/// when a specific endpoint is needed.
///
/// The interface the announcement arrived on names the occurrence, so the same
/// registration seen on two interfaces yields two entries that neither
/// overwrite nor remove each other.
fn record_from_service(service: &DiscoveredService) -> Entry {
    Entry::resolved(
        &service.name,
        &service.service_type,
        &service.domain,
        Some(service.host_name.as_str()),
        service.addresses.clone(),
        Some(service.port),
        txt_map(&service.txt_records),
    )
    .with_occurrence(service.interface_index.map(OccurrenceId))
}

fn key_from_service(service: &DiscoveredService) -> ServiceKey {
    ServiceKey {
        registration: Registration::new(&service.name, &service.service_type, &service.domain),
        interface_index: service.interface_index,
    }
}

fn key_from_removal(removal: &RemovedService) -> ServiceKey {
    ServiceKey {
        registration: Registration::new(&removal.name, &removal.service_type, &removal.domain),
        interface_index: removal.interface_index,
    }
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

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

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
        ServiceKey {
            registration: Registration::new(name, "_http._tcp", "local"),
            interface_index: None,
        }
    }

    /// The same registration as [`key`], but announced on one named interface.
    fn key_on(name: &str, index: u32) -> ServiceKey {
        ServiceKey {
            registration: Registration::new(name, "_http._tcp", "local"),
            interface_index: NonZeroU32::new(index),
        }
    }

    fn interface(index: u32) -> Option<NonZeroU32> {
        NonZeroU32::new(index)
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
        // This occurrence has no interface index, so the adapter cannot name
        // what it lost and removes the registration.
        match rx.try_recv() {
            Ok(DiscoveryEvent::RemoveRegistration(registration)) => {
                assert_eq!(
                    registration,
                    Registration::new("nas", "_http._tcp", "local")
                );
            }
            other => panic!("expected RemoveRegistration, got {other:?}"),
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

    fn removal(name: &str, interface_index: Option<NonZeroU32>) -> RemovedService {
        RemovedService {
            name: name.to_string(),
            service_type: "_http._tcp".to_string(),
            domain: "local".to_string(),
            interface_index,
        }
    }

    #[test]
    fn found_on_two_interfaces_yields_two_occurrences() {
        let mut wired = service("nas", "_http._tcp", vec!["192.168.1.30".parse().unwrap()]);
        wired.interface_index = interface(2);
        let mut wireless = service("nas", "_http._tcp", vec!["192.168.1.31".parse().unwrap()]);
        wireless.interface_index = interface(3);

        let wired = record_from_service(&wired);
        let wireless = record_from_service(&wireless);

        // Identical registration, host, and port: only the interface differs.
        assert_eq!(wired.registration(), wireless.registration());
        assert_eq!(wired.hostname, wireless.hostname);
        assert_eq!(wired.port, wireless.port);
        // …so they are distinct occurrences and cannot overwrite each other.
        assert_ne!(wired.id(), wireless.id());
        assert_eq!(
            wired.occurrence(),
            Some(OccurrenceId(interface(2).unwrap()))
        );
    }

    #[test]
    fn interface_specific_removal_names_exactly_that_occurrence() {
        let mut found = service("nas", "_http._tcp", vec!["192.168.1.30".parse().unwrap()]);
        found.interface_index = interface(2);
        let entry = record_from_service(&found);

        match key_from_removal(&removal("nas", interface(2))).removal_event() {
            // The removal must address the very record the Found event created.
            DiscoveryEvent::Remove(id) => assert_eq!(id, entry.id()),
            other => panic!("expected Remove, got {other:?}"),
        }

        // A sibling on another interface is a different occurrence, so this
        // removal leaves it alone.
        match key_from_removal(&removal("nas", interface(3))).removal_event() {
            DiscoveryEvent::Remove(id) => assert_ne!(id, entry.id()),
            other => panic!("expected Remove, got {other:?}"),
        }
    }

    #[test]
    fn removal_without_an_interface_falls_back_to_the_registration() {
        match key_from_removal(&removal("nas", None)).removal_event() {
            DiscoveryEvent::RemoveRegistration(registration) => {
                assert_eq!(
                    registration,
                    Registration::new("nas", "_http._tcp", "local")
                );
            }
            other => panic!("expected RemoveRegistration, got {other:?}"),
        }
    }

    #[test]
    fn liveness_transitions_track_each_interface_separately() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key_on("nas", 2));
        tracker.note_found(key_on("nas", 3));

        // Interface 2 goes quiet; interface 3 keeps answering.
        for _ in 1..PROBE_FAILURE_THRESHOLD {
            assert!(!tracker.record_failure(&key_on("nas", 2)));
        }
        assert!(tracker.record_failure(&key_on("nas", 2)));

        // The sibling's liveness is untouched: it is still probed, and its own
        // failure streak starts from zero.
        assert!(tracker.probe_keys().contains(&key_on("nas", 3)));
        assert!(!tracker.record_failure(&key_on("nas", 3)));
    }

    #[test]
    fn probe_failure_on_a_named_occurrence_removes_only_that_occurrence() {
        let mut tracker = LivenessTracker::default();
        let (tx, rx) = mpsc::channel();
        tracker.note_found(key_on("nas", 2));
        tracker.note_found(key_on("nas", 3));

        for _ in 0..PROBE_FAILURE_THRESHOLD {
            assert!(apply_probe_results(
                vec![(key_on("nas", 2), None)],
                &mut tracker,
                &tx
            ));
        }

        match rx.try_recv() {
            Ok(DiscoveryEvent::Remove(id)) => {
                assert_eq!(
                    id,
                    EntryId::named(
                        Registration::new("nas", "_http._tcp", "local"),
                        OccurrenceId(interface(2).unwrap()),
                    )
                );
            }
            other => panic!("expected Remove, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn recovered_probe_upserts_the_occurrence_that_was_probed() {
        let mut tracker = LivenessTracker::default();
        let (tx, rx) = mpsc::channel();
        tracker.note_found(key_on("nas", 2));
        for _ in 0..PROBE_FAILURE_THRESHOLD {
            tracker.record_failure(&key_on("nas", 2));
        }
        let _ = rx.try_recv();

        // The resolve answers without naming an interface; the upsert must
        // still carry the tracked occurrence, or it would land on a new record
        // and leave the removed one listed forever.
        let nas = service("nas", "_http._tcp", vec!["192.168.1.30".parse().unwrap()]);
        assert!(apply_probe_results(
            vec![(key_on("nas", 2), Some(nas))],
            &mut tracker,
            &tx
        ));

        match rx.try_recv() {
            Ok(DiscoveryEvent::Upsert(entry)) => {
                assert_eq!(
                    entry.occurrence(),
                    Some(OccurrenceId(interface(2).unwrap()))
                );
                assert_eq!(entry.hostname.as_deref(), Some("nas.local"));
            }
            other => panic!("expected Upsert, got {other:?}"),
        }
    }

    #[test]
    fn registration_wide_removal_forgets_every_tracked_occurrence() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key_on("nas", 2));
        tracker.note_found(key_on("nas", 3));
        tracker.note_found(key_on("printer", 2));

        track_event(&BrowseEvent::Removed(removal("nas", None)), &mut tracker);

        // Both `nas` occurrences are gone; the unrelated registration stays.
        assert_eq!(tracker.probe_keys(), vec![key_on("printer", 2)]);
    }

    #[test]
    fn interface_specific_removal_keeps_probing_the_sibling() {
        let mut tracker = LivenessTracker::default();
        tracker.note_found(key_on("nas", 2));
        tracker.note_found(key_on("nas", 3));

        track_event(
            &BrowseEvent::Removed(removal("nas", interface(2))),
            &mut tracker,
        );

        assert_eq!(tracker.probe_keys(), vec![key_on("nas", 3)]);
    }
}
