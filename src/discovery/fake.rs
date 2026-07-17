use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::inbox::EventSender;
use super::session::DiscoverySession;
use super::worker::{BrowseOutcome, DiscoveryWorker, RuntimeFlavor};
use super::{DiscoveryEvent, DiscoveryOptions, Entry, ServiceTypeFilter};

/// Pause between sample records, so the list populates visibly rather than
/// appearing all at once.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);

/// Start the built-in sample-records backend. Reached only when the user asks
/// for it with `--backend fake`: a real adapter's failure never falls back
/// here, because a plausible `192.168.1.x` endpoint the user did not ask for is
/// an actionable lie.
///
/// The sample stream is finite. It runs on the same worker scaffold as the real
/// adapters so that it is cancellable mid-stream and dropping its session stops
/// it, and so that running out of samples is reported as a normal
/// [`BrowseOutcome::Complete`] rather than as a discovery failure.
///
/// Sample records exercise no real adapter, so this backend accepts any
/// configured domain — including one no real backend could browse.
pub(super) fn start(options: &DiscoveryOptions) -> DiscoverySession {
    let (worker, rx) = DiscoveryWorker::spawn(options, RuntimeFlavor::CurrentThread, sample_loop);
    DiscoverySession::from_worker(rx, worker)
}

/// Stream the sample records, pausing between them and staying responsive to
/// cancellation while paused.
async fn sample_loop(
    domain: String,
    service_type_filter: Option<ServiceTypeFilter>,
    tx: EventSender,
    shutdown: CancellationToken,
) -> BrowseOutcome {
    if tx
        .send(DiscoveryEvent::Status(
            "using sample discovery records".to_string(),
        ))
        .is_err()
    {
        return BrowseOutcome::Stopped;
    }

    let mut records = fake_records(&domain);
    // The filter is canonical, and so are the sample types, so this compares
    // like with like instead of hoping the user matched the samples' spelling.
    if let Some(service_type) = service_type_filter.map(|filter| filter.to_string()) {
        records.retain(|record| record.service_type == service_type);
    }

    for record in records {
        if tx.send(DiscoveryEvent::Upsert(record)).is_err() {
            return BrowseOutcome::Stopped;
        }
        tokio::select! {
            _ = shutdown.cancelled() => return BrowseOutcome::Cancelled,
            _ = tokio::time::sleep(SAMPLE_INTERVAL) => {}
        }
    }

    BrowseOutcome::Complete
}

fn fake_records(domain: &str) -> Vec<Entry> {
    // A single logical SSH service reachable at two addresses (load-balanced /
    // dual-stack), kept together on one entry.
    let mut ssh = Entry::new("workstation", "_ssh._tcp", domain);
    ssh.hostname = Some("workstation.local".to_string());
    ssh.addresses = vec![
        "192.168.1.20".parse().unwrap(),
        "192.168.1.21".parse().unwrap(),
    ];
    ssh.port = Some(22);

    // A second SSH service, on its own host. The `_ssh._tcp` service-type row
    // therefore aggregates two hosts that do not agree on a hostname, which is
    // what makes a rule like `ssh {hostname}` ask which host to act on. Without
    // it the sample set can only ever produce rows whose children all prepare
    // the same command, and that question never arises.
    let mut ssh_pi = Entry::new("raspberry-pi", "_ssh._tcp", domain);
    ssh_pi.hostname = Some("raspberry-pi.local".to_string());
    ssh_pi.addresses = vec!["192.168.1.40".parse().unwrap()];
    ssh_pi.port = Some(22);

    let mut http = Entry::new("nas", "_http._tcp", domain);
    http.hostname = Some("nas.local".to_string());
    http.addresses = vec!["192.168.1.30".parse().unwrap()];
    http.port = Some(8080);
    http.txt.insert("path".to_string(), "/admin".to_string());

    let mut https = Entry::new("router", "_https._tcp", domain);
    https.hostname = Some("router.local".to_string());
    https.addresses = vec!["192.168.1.1".parse().unwrap()];
    https.port = Some(443);

    let unresolved = Entry::new("pending-printer", "_ipp._tcp", domain);

    vec![ssh, ssh_pi, http, https, unresolved]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::discovery::inbox;

    #[test]
    fn fake_records_carry_the_requested_domain_and_an_unresolved_entry() {
        let records = fake_records("corp");

        assert!(records.iter().all(|record| record.domain == "corp"));
        assert!(
            records.iter().any(|record| !record.has_instance_data()),
            "expected at least one pending/unresolved record"
        );
        let ssh: Vec<_> = records
            .iter()
            .filter(|record| record.service_type == "_ssh._tcp")
            .collect();

        // The `workstation` service is one logical entry carrying both of its
        // addresses, so address selection stays demonstrable.
        let workstation = ssh
            .iter()
            .find(|record| record.name == "workstation")
            .expect("the multi-address SSH service");
        assert_eq!(workstation.addresses.len(), 2);

        // Two SSH services on two hosts, so the `_ssh._tcp` service-type row
        // aggregates children whose `{hostname}` commands differ and a target
        // must be chosen.
        assert_eq!(ssh.len(), 2);
        let hosts: BTreeSet<_> = ssh
            .iter()
            .filter_map(|record| record.hostname.as_deref())
            .collect();
        assert_eq!(hosts.len(), 2, "the SSH services must be on distinct hosts");
    }

    #[tokio::test]
    async fn the_sample_loop_streams_status_then_filtered_records_and_completes() {
        let shutdown = CancellationToken::new();
        let (tx, rx) = inbox::test_channel(&shutdown);

        let outcome = sample_loop(
            "local".to_string(),
            Some(ServiceTypeFilter::parse("_ssh._tcp").unwrap()),
            tx,
            shutdown,
        )
        .await;

        // Running out of samples is a normal completion, not a failure.
        assert_eq!(outcome, BrowseOutcome::Complete);

        let mut statuses = 0;
        let mut upserts = Vec::new();
        while let Ok(event) = rx.recv() {
            match event {
                DiscoveryEvent::Status(_) => statuses += 1,
                DiscoveryEvent::Upsert(record) => upserts.push(record),
                DiscoveryEvent::Remove(_) | DiscoveryEvent::RemoveRegistration(_) => {}
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

    /// Cancellation must be honoured while the stream is sleeping between
    /// records, not only between sends: otherwise dropping a session would
    /// block for the rest of the samples.
    #[tokio::test]
    async fn the_sample_loop_stops_when_cancelled_mid_stream() {
        let shutdown = CancellationToken::new();
        let (tx, rx) = inbox::test_channel(&shutdown);
        // Cancelled up front: the loop must bail out at its first pause.
        shutdown.cancel();

        let outcome = sample_loop("local".to_string(), None, tx, shutdown).await;

        assert_eq!(outcome, BrowseOutcome::Cancelled);
        let upserts = rx
            .iter()
            .filter(|event| matches!(event, DiscoveryEvent::Upsert(_)))
            .count();
        assert_eq!(
            upserts, 1,
            "the loop stops at its first pause, having sent one record"
        );
    }

    /// A receiver that has gone away ends the loop instead of streaming into a
    /// channel nobody reads.
    #[tokio::test]
    async fn the_sample_loop_stops_when_the_receiver_is_gone() {
        let shutdown = CancellationToken::new();
        let (tx, rx) = inbox::test_channel(&shutdown);
        drop(rx);

        let outcome = sample_loop("local".to_string(), None, tx, shutdown).await;

        assert_eq!(outcome, BrowseOutcome::Stopped);
    }
}
