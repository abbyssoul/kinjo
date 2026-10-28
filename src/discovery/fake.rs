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

/// A resolved sample service: `name` of `service_type`, served by `host` at
/// `addresses` on `port`, advertising the given TXT pairs.
fn sample(
    domain: &str,
    name: &str,
    service_type: &str,
    host: &str,
    addresses: &[&str],
    port: u16,
    txt: &[(&str, &str)],
) -> Entry {
    let mut entry = Entry::new(name, service_type, domain);
    entry.hostname = Some(format!("{host}.local"));
    entry.addresses = addresses
        .iter()
        .map(|address| address.parse().expect("sample address"))
        .collect();
    entry.port = Some(port);
    entry.txt = txt
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    entry
}

/// A tongue-in-cheek network of famous fictional machines. Every name,
/// address, and hardware id here is made up, so the sample set is safe to show
/// in screenshots and demo recordings.
fn fake_records(domain: &str) -> Vec<Entry> {
    vec![
        // A single logical SSH service reachable at two addresses (dual-stack),
        // kept together on one entry.
        sample(
            domain,
            "Mainframe",
            "_ssh._tcp",
            "mainframe",
            &["192.168.1.64", "fd00:dead:beef::64"],
            22,
            &[("os", "z/OS"), ("uptime", "41y")],
        ),
        // A second SSH service, on its own host. The `_ssh._tcp` service-type
        // row therefore aggregates two hosts that do not agree on a hostname,
        // which is what makes a rule like `ssh {hostname}` ask which host to
        // act on. Without it the sample set can only ever produce rows whose
        // children all prepare the same command, and that question never
        // arises.
        sample(
            domain,
            "WOPR",
            "_ssh._tcp",
            "wopr",
            &["192.168.1.83"],
            22,
            &[("motd", "shall-we-play-a-game"), ("defcon", "5")],
        ),
        // The same host again under another service type, so the host view and
        // same-host filter have something to group.
        sample(
            domain,
            "Mainframe [de:ad:be:ef:ca:fe]",
            "_workstation._tcp",
            "mainframe",
            &["192.168.1.64"],
            9,
            &[],
        ),
        sample(
            domain,
            "GLaDOS",
            "_http._tcp",
            "glados",
            &["192.168.1.98"],
            8080,
            &[("path", "/test-chamber/19"), ("cake", "lie")],
        ),
        sample(
            domain,
            "Skynet Defense Grid",
            "_https._tcp",
            "skynet",
            &["192.168.1.97"],
            443,
            &[("status", "self-aware"), ("judgment-day", "1997-08-29")],
        ),
        sample(
            domain,
            "Deep Thought",
            "_smb._tcp",
            "deep-thought",
            &["192.168.1.42"],
            445,
            &[("answer", "42"), ("eta", "7.5M-years")],
        ),
        sample(
            domain,
            "HAL 9000",
            "_raop._tcp",
            "hal9000",
            &["192.168.1.200"],
            7000,
            &[("pod-bay-doors", "closed"), ("mood", "sorry-dave")],
        ),
        sample(
            domain,
            "Multivac",
            "_http._tcp",
            "multivac",
            &["192.168.1.56"],
            80,
            &[("path", "/last-question"), ("answer", "insufficient-data")],
        ),
        sample(
            domain,
            "Flux Capacitor",
            "_hap._tcp",
            "delorean",
            &["192.168.1.85"],
            1955,
            &[("power", "1.21GW"), ("speed", "88mph")],
        ),
        sample(
            domain,
            "Holodeck 3",
            "_googlecast._tcp",
            "holodeck",
            &["192.168.1.101"],
            8009,
            &[("program", "sherlock-holmes"), ("safety", "off")],
        ),
        sample(
            domain,
            "Marvin",
            "_spotify-connect._tcp",
            "marvin",
            &["192.168.1.142"],
            4070,
            &[("brain", "planet-sized"), ("mood", "depressed")],
        ),
        // Announced but never resolved — it is, after all, a lie.
        Entry::new("Cake Dispenser", "_ipp._tcp", domain),
    ]
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

        // The `Mainframe` service is one logical entry carrying both of its
        // addresses, so address selection stays demonstrable.
        let mainframe = ssh
            .iter()
            .find(|record| record.name == "Mainframe")
            .expect("the multi-address SSH service");
        assert_eq!(mainframe.addresses.len(), 2);

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
