//! Shared scaffold for discovery backends that browse from a worker thread.

use std::{future::Future, sync::mpsc, thread};

use tokio_util::sync::CancellationToken;

use super::{Discovery, DiscoveryConfig, DiscoveryEvent, fake};

/// Which tokio runtime a backend's browse loop needs.
pub(super) enum RuntimeFlavor {
    /// Single-threaded; enough when the loop drives all its futures itself.
    CurrentThread,
    /// Multi-threaded; needed when the loop `tokio::spawn`s parallel workers.
    MultiThread,
}

impl RuntimeFlavor {
    fn build(&self) -> std::io::Result<tokio::runtime::Runtime> {
        let mut builder = match self {
            RuntimeFlavor::CurrentThread => tokio::runtime::Builder::new_current_thread(),
            RuntimeFlavor::MultiThread => tokio::runtime::Builder::new_multi_thread(),
        };
        builder.enable_all().build()
    }
}

/// A discovery backend running its browse loop on a dedicated worker thread.
/// Owns the thread and its cancellation token: dropping the worker cancels the
/// loop and joins the thread. Falls back to the [`fake`] backend when the async
/// runtime cannot be started (browse-time failures remain each loop's concern).
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
                        "failed to start mDNS runtime ({err}); using sample records"
                    )));
                    fake::spawn(domain, service_type_filter, tx);
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
