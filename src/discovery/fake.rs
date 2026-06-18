use std::{sync::mpsc, thread, time::Duration};

use super::{Discovery, DiscoveryConfig, DiscoveryEvent, Entry};

/// Built-in sample-records discovery backend. Used for demos, the
/// `--fake-discovery` flag, and as the mDNS backend's fallback.
pub struct FakeDiscovery {
    receiver: Option<mpsc::Receiver<DiscoveryEvent>>,
}

impl FakeDiscovery {
    pub fn start(config: &DiscoveryConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        spawn(config.domain.clone(), config.service_type.clone(), tx);
        Self { receiver: Some(rx) }
    }
}

impl Discovery for FakeDiscovery {
    fn events(&mut self) -> mpsc::Receiver<DiscoveryEvent> {
        self.receiver
            .take()
            .expect("discovery receiver can only be taken once")
    }
}

/// Stream the sample records over `tx`. Shared with the mDNS backend, which
/// falls back to it when no browser can be started.
pub(super) fn spawn(
    domain: String,
    service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
) {
    thread::spawn(move || {
        let _ = tx.send(DiscoveryEvent::Status(
            "using sample discovery records".to_string(),
        ));
        let mut records = fake_records(&domain);
        if let Some(service_type) = service_type_filter {
            records.retain(|record| record.service_type == service_type);
        }
        for record in records {
            let _ = tx.send(DiscoveryEvent::Upsert(record));
            thread::sleep(Duration::from_millis(150));
        }
    });
}

fn fake_records(domain: &str) -> Vec<Entry> {
    let mut ssh_a = Entry::new("workstation", "_ssh._tcp", domain);
    ssh_a.hostname = Some("workstation.local".to_string());
    ssh_a.address = Some("192.168.1.20".parse().unwrap());
    ssh_a.port = Some(22);

    let mut ssh_b = ssh_a.clone();
    ssh_b.address = Some("192.168.1.21".parse().unwrap());

    let mut http = Entry::new("nas", "_http._tcp", domain);
    http.hostname = Some("nas.local".to_string());
    http.address = Some("192.168.1.30".parse().unwrap());
    http.port = Some(8080);
    http.txt.insert("path".to_string(), "/admin".to_string());

    let mut https = Entry::new("router", "_https._tcp", domain);
    https.hostname = Some("router.local".to_string());
    https.address = Some("192.168.1.1".parse().unwrap());
    https.port = Some(443);

    let unresolved = Entry::new("pending-printer", "_ipp._tcp", domain);

    vec![
        ssh_a.with_instance_id(),
        ssh_b.with_instance_id(),
        http.with_instance_id(),
        https.with_instance_id(),
        unresolved.with_instance_id(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_records_carry_the_requested_domain_and_an_unresolved_entry() {
        let records = fake_records("corp");

        assert!(records.iter().all(|record| record.domain == "corp"));
        assert!(
            records.iter().any(|record| !record.has_instance_data()),
            "expected at least one pending/unresolved record"
        );
        // The two SSH instances differ only by address but keep distinct ids.
        let ssh_ids: std::collections::BTreeSet<_> = records
            .iter()
            .filter(|record| record.service_type == "_ssh._tcp")
            .map(|record| record.id.0.clone())
            .collect();
        assert_eq!(ssh_ids.len(), 2);
    }

    #[test]
    fn spawn_streams_status_then_filtered_records() {
        let (tx, rx) = mpsc::channel();
        spawn("local".to_string(), Some("_ssh._tcp".to_string()), tx);

        let mut statuses = 0;
        let mut upserts = Vec::new();
        while let Ok(event) = rx.recv() {
            match event {
                DiscoveryEvent::Status(_) => statuses += 1,
                DiscoveryEvent::Upsert(record) => upserts.push(record),
                DiscoveryEvent::Remove(_) => {}
            }
        }

        assert_eq!(statuses, 1);
        assert!(!upserts.is_empty());
        assert!(
            upserts
                .iter()
                .all(|record| record.service_type == "_ssh._tcp")
        );
    }
}
