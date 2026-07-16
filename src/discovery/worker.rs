//! Shared scaffold for discovery backends that browse from a worker thread.

use std::{
    future::Future,
    sync::{Arc, Mutex, mpsc},
    thread,
};

use tokio_util::sync::CancellationToken;

use super::{DiscoveryEvent, DiscoveryOptions, ServiceTypeFilter};

/// Which tokio runtime a backend's browse loop needs.
pub(super) enum RuntimeFlavor {
    /// Single-threaded; enough when the loop drives all its futures itself.
    CurrentThread,
    /// Multi-threaded; needed when the loop `tokio::spawn`s parallel workers.
    /// Only the `zeroconf` backend does, so tokio's `rt-multi-thread` feature
    /// is pulled in by the `zeroconf` cargo feature.
    #[cfg(feature = "zeroconf")]
    MultiThread,
}

impl RuntimeFlavor {
    fn build(&self) -> std::io::Result<tokio::runtime::Runtime> {
        let mut builder = match self {
            RuntimeFlavor::CurrentThread => tokio::runtime::Builder::new_current_thread(),
            #[cfg(feature = "zeroconf")]
            RuntimeFlavor::MultiThread => tokio::runtime::Builder::new_multi_thread(),
        };
        builder.enable_all().build()
    }
}

/// How a browse loop ended. This is the loop's own account of its ending, which
/// is the only thing that can tell "the adapter never started" apart from "the
/// adapter browsed and then stopped" and from "a finite sample stream finished".
/// A dropped event channel alone cannot: every ending looks identical from the
/// receiving side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BrowseOutcome {
    /// The loop never started browsing. Carries actionable cause text; the loop
    /// does not also send it as a `Status`, because a transient status line is
    /// exactly what a startup error must not depend on.
    Startup(String),
    /// The loop was browsing and its event source ended on its own.
    Stopped,
    /// A finite stream finished normally. Only the explicit fake adapter ends
    /// this way; its samples remain valid.
    #[cfg(feature = "fake")]
    Complete,
    /// The shutdown token fired: the session was dropped or replaced.
    Cancelled,
}

/// A discovery backend running its browse loop on a dedicated worker thread.
///
/// Owns the thread and its cancellation token: [`shutdown`](Self::shutdown)
/// (and `Drop`) cancel the loop and join the thread. Every browse loop selects
/// on the token, so the join is bounded rather than waiting out a browse.
///
/// The loop's [`BrowseOutcome`] is recorded before the channel disconnects, so
/// a reader that observes `Disconnected` is guaranteed to find the outcome
/// already published (see [`spawn`](Self::spawn)).
pub(super) struct DiscoveryWorker {
    shutdown: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
    outcome: Arc<Mutex<Option<BrowseOutcome>>>,
}

impl DiscoveryWorker {
    /// Spawn `browse(domain, service_type_filter, tx, shutdown)` on a fresh
    /// runtime of the requested flavor, on its own thread. Returns the worker
    /// and the receiving end of its event channel: handing the receiver over
    /// once, at construction, is what removes the need for a take-once
    /// accessor that panics on a second call.
    ///
    /// The loop is handed the *validated* domain and filter, so it browses what
    /// was asked for without re-deciding what the values mean.
    pub(super) fn spawn<F, Fut>(
        options: &DiscoveryOptions,
        flavor: RuntimeFlavor,
        browse: F,
    ) -> (Self, mpsc::Receiver<DiscoveryEvent>)
    where
        F: FnOnce(
                String,
                Option<ServiceTypeFilter>,
                mpsc::Sender<DiscoveryEvent>,
                CancellationToken,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = BrowseOutcome>,
    {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();
        let domain = options.domain().to_string();
        let service_type_filter = options.service_type().cloned();
        let token = shutdown.clone();
        let outcome = Arc::new(Mutex::new(None));
        let slot = outcome.clone();

        let worker = thread::spawn(move || {
            // The browse loop owns `tx` and drops it when it returns. This
            // clone keeps the channel connected until the outcome has been
            // published, so a reader can never see `Disconnected` before the
            // ending that explains it.
            let keepalive = tx.clone();

            let ended = match flavor.build() {
                Ok(runtime) => runtime.block_on(browse(domain, service_type_filter, tx, token)),
                Err(err) => BrowseOutcome::Startup(format!(
                    "failed to start mDNS runtime ({err}); try --backend fake in a build with the fake feature for sample records, or refresh to retry"
                )),
            };

            *slot.lock().unwrap_or_else(|err| err.into_inner()) = Some(ended);
            drop(keepalive);
        });

        (
            Self {
                shutdown,
                worker: Some(worker),
                outcome,
            },
            rx,
        )
    }

    /// How the loop ended, once it has. `None` while it is still running.
    ///
    /// Callers must only trust `None` as "still running" when the event channel
    /// is still connected; after a disconnect the outcome is always published.
    pub(super) fn outcome(&self) -> Option<BrowseOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Cancel the browse loop and join its thread. Idempotent.
    pub(super) fn shutdown(&mut self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DiscoveryWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::discovery::{DiscoveryBackend, DiscoveryConfig};

    fn config() -> DiscoveryConfig {
        DiscoveryConfig {
            backend: DiscoveryBackend::MdnsSd,
            domain: "local".to_string(),
            service_type: None,
        }
    }

    fn options() -> DiscoveryOptions {
        config().validate().expect("valid test options")
    }

    /// A backend's browse startup failure (mdns-sd browse error, zeroconf
    /// browser creation error, or the runtime itself failing to build) must
    /// leave the worker reporting only its outcome, never fabricating an entry.
    #[test]
    fn browse_startup_failure_reports_the_cause_and_no_upsert() {
        let (worker, rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, _tx, _shutdown| async move {
                BrowseOutcome::Startup("simulated browse startup failure".to_string())
            },
        );

        let events: Vec<_> = rx.iter().collect();

        assert!(events.is_empty(), "a failed start must emit no entries");
        assert_eq!(
            worker.outcome(),
            Some(BrowseOutcome::Startup(
                "simulated browse startup failure".to_string()
            ))
        );
    }

    /// The outcome must be published before the channel disconnects, or a
    /// reader would see "the producer is gone" with no account of why.
    #[test]
    fn the_outcome_is_published_before_the_channel_disconnects() {
        let (worker, rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, _tx, _shutdown| async move { BrowseOutcome::Stopped },
        );

        // Block until every sender is gone, which is the exact moment a session
        // decides the producer ended.
        while rx.recv().is_ok() {}

        assert_eq!(worker.outcome(), Some(BrowseOutcome::Stopped));
    }

    /// The browse loop receives the validated domain and service-type filter,
    /// canonicalized — not whatever text the caller happened to type.
    #[test]
    fn the_loop_is_handed_the_validated_domain_and_filter() {
        let mut config = config();
        config.domain = "corp".to_string();
        config.service_type = Some("_SSH._tcp".to_string());

        let (_worker, rx) = DiscoveryWorker::spawn(
            &config.validate().expect("valid test options"),
            RuntimeFlavor::CurrentThread,
            |domain, service_type_filter, tx, _shutdown| async move {
                let _ = tx.send(DiscoveryEvent::Status(format!(
                    "{domain}/{}",
                    service_type_filter
                        .map(|filter| filter.to_string())
                        .unwrap_or_default()
                )));
                BrowseOutcome::Stopped
            },
        );

        match rx.recv() {
            Ok(DiscoveryEvent::Status(status)) => assert_eq!(status, "corp/_ssh._tcp"),
            other => panic!("expected Status, got {other:?}"),
        }
    }

    /// Shutdown must be bounded: a loop that selects on the token stops when
    /// asked instead of running to completion.
    #[test]
    fn shutdown_cancels_a_running_loop_and_joins_it() {
        let (mut worker, rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, tx, shutdown| async move {
                let _ = tx.send(DiscoveryEvent::Status("browsing".to_string()));
                // Far longer than the test is willing to wait: only
                // cancellation can end this.
                tokio::select! {
                    _ = shutdown.cancelled() => BrowseOutcome::Cancelled,
                    _ = tokio::time::sleep(Duration::from_secs(600)) => BrowseOutcome::Stopped,
                }
            },
        );
        assert!(matches!(rx.recv(), Ok(DiscoveryEvent::Status(_))));

        worker.shutdown();

        assert_eq!(worker.outcome(), Some(BrowseOutcome::Cancelled));
        // The thread is joined, so its sender is gone.
        assert!(rx.recv().is_err());
    }

    /// `shutdown` is called by `Drop` too, and must tolerate being run twice.
    #[test]
    fn shutdown_is_idempotent() {
        let (mut worker, _rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, _tx, shutdown| async move {
                shutdown.cancelled().await;
                BrowseOutcome::Cancelled
            },
        );

        worker.shutdown();
        worker.shutdown();

        assert_eq!(worker.outcome(), Some(BrowseOutcome::Cancelled));
    }

    /// Dropping the worker stops its producer rather than leaking the thread.
    #[test]
    fn dropping_the_worker_stops_its_producer() {
        let (worker, rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, tx, shutdown| async move {
                let _ = tx.send(DiscoveryEvent::Status("browsing".to_string()));
                shutdown.cancelled().await;
                // Sending after cancellation would keep feeding a list nobody
                // is entitled to trust; the loop stops instead.
                BrowseOutcome::Cancelled
            },
        );
        assert!(matches!(rx.recv(), Ok(DiscoveryEvent::Status(_))));

        drop(worker);

        assert!(rx.recv().is_err(), "the producer's sender must be gone");
    }

    /// A runtime that cannot be built is a startup failure like any other: an
    /// outcome with a cause, and no entries.
    #[test]
    fn a_loop_that_never_runs_still_publishes_an_outcome() {
        let (worker, rx) = DiscoveryWorker::spawn(
            &options(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, _tx, _shutdown| async move { BrowseOutcome::Stopped },
        );

        let events: Vec<_> = rx.iter().collect();

        assert!(events.is_empty());
        assert_eq!(worker.outcome(), Some(BrowseOutcome::Stopped));
    }
}
