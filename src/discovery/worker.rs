//! Shared scaffold for discovery backends that browse from a worker thread.

use std::{future::Future, sync::mpsc, thread};

use tokio_util::sync::CancellationToken;

use super::{Discovery, DiscoveryConfig, DiscoveryEvent};

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

/// A discovery backend running its browse loop on a dedicated worker thread.
/// Owns the thread and its cancellation token: dropping the worker cancels the
/// loop and joins the thread. Reports a [`DiscoveryEvent::Status`] and emits no
/// entries when the async runtime cannot be started (browse-time failures
/// remain each loop's concern).
pub(super) struct DiscoveryWorker {
    receiver: Option<mpsc::Receiver<DiscoveryEvent>>,
    shutdown: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
}

impl DiscoveryWorker {
    /// Spawn `browse(domain, service_type_filter, tx, shutdown)` on a fresh
    /// runtime of the requested flavor, on its own thread.
    pub(super) fn spawn<F, Fut>(config: &DiscoveryConfig, flavor: RuntimeFlavor, browse: F) -> Self
    where
        F: FnOnce(String, Option<String>, mpsc::Sender<DiscoveryEvent>, CancellationToken) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = ()>,
    {
        let (tx, rx) = mpsc::channel();
        let shutdown = CancellationToken::new();
        let domain = config.domain.clone();
        let service_type_filter = config.service_type.clone();
        let token = shutdown.clone();

        let worker = thread::spawn(move || {
            let runtime = match flavor.build() {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = tx.send(DiscoveryEvent::Status(format!(
                        "failed to start mDNS runtime ({err}); try --fake-discovery for sample records, or refresh to retry"
                    )));
                    return;
                }
            };
            runtime.block_on(browse(domain, service_type_filter, tx, token));
        });

        Self {
            receiver: Some(rx),
            shutdown,
            worker: Some(worker),
        }
    }
}

impl Discovery for DiscoveryWorker {
    fn events(&mut self) -> mpsc::Receiver<DiscoveryEvent> {
        self.receiver
            .take()
            .expect("discovery receiver can only be taken once")
    }
}

impl Drop for DiscoveryWorker {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::DiscoveryBackend;

    fn config() -> DiscoveryConfig {
        DiscoveryConfig {
            fake: false,
            backend: DiscoveryBackend::MdnsSd,
            domain: "local".to_string(),
            service_type: None,
        }
    }

    /// A backend's browse startup failure (mdns-sd browse error, zeroconf
    /// browser creation error, or the runtime itself failing to build) must
    /// leave the worker reporting only a `Status`, never fabricating an entry.
    #[test]
    fn browse_startup_failure_relays_status_only_no_upsert() {
        let mut worker = DiscoveryWorker::spawn(
            &config(),
            RuntimeFlavor::CurrentThread,
            |_domain, _service_type_filter, tx, _shutdown| async move {
                let _ = tx.send(DiscoveryEvent::Status(
                    "simulated browse startup failure".to_string(),
                ));
            },
        );

        let events: Vec<_> = worker.events().iter().collect();

        assert!(matches!(events.as_slice(), [DiscoveryEvent::Status(_)]));
    }
}
