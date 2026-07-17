//! Bounded, non-blocking delivery from discovery adapters to a session.
//!
//! Adapter threads must never block while publishing: a full synchronous send
//! could prevent the browse loop observing cancellation and make session
//! shutdown wait forever. A full inbox therefore fails closed. The producer is
//! cancelled, the reason is retained outside the queue, and the session later
//! reports a visible failure after draining the already accepted events.

use std::sync::{Arc, Mutex, mpsc};

use tokio_util::sync::CancellationToken;

use super::DiscoveryEvent;

/// Enough headroom for a large real LAN while bounding burst memory.
pub(super) const EVENT_CAPACITY: usize = 4_096;

#[derive(Debug, Default)]
struct State {
    overload_reason: Mutex<Option<String>>,
}

/// Producer half used by every concrete discovery adapter.
#[derive(Debug, Clone)]
pub(super) struct EventSender {
    sender: mpsc::SyncSender<DiscoveryEvent>,
    state: Arc<State>,
    shutdown: CancellationToken,
    capacity: usize,
}

/// Worker-owned view of failure state that does not keep the event channel
/// connected after every sender has gone away.
#[derive(Debug, Clone)]
pub(super) struct InboxControl {
    state: Arc<State>,
}

impl EventSender {
    pub(super) fn send(&self, event: DiscoveryEvent) -> Result<(), ()> {
        match self.sender.try_send(event) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                self.overload(format!(
                    "discovery produced more than {} pending events; records were cleared; refresh to retry",
                    self.capacity
                ));
                Err(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => Err(()),
        }
    }

    /// Fail the whole session before publishing state Kinjo cannot continue to
    /// track honestly. The first reason wins so a queue overflow cannot obscure
    /// an earlier, more specific resource failure.
    pub(super) fn overload(&self, reason: impl Into<String>) {
        let mut slot = self
            .state
            .overload_reason
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if slot.is_none() {
            *slot = Some(reason.into());
        }
        drop(slot);
        self.shutdown.cancel();
    }
}

impl InboxControl {
    pub(super) fn overload_reason(&self) -> Option<String> {
        self.state
            .overload_reason
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

pub(super) fn channel(
    shutdown: &CancellationToken,
) -> (EventSender, mpsc::Receiver<DiscoveryEvent>, InboxControl) {
    channel_with_capacity(shutdown, EVENT_CAPACITY)
}

fn channel_with_capacity(
    shutdown: &CancellationToken,
    capacity: usize,
) -> (EventSender, mpsc::Receiver<DiscoveryEvent>, InboxControl) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let state = Arc::new(State::default());
    (
        EventSender {
            sender,
            state: state.clone(),
            shutdown: shutdown.clone(),
            capacity,
        },
        receiver,
        InboxControl { state },
    )
}

#[cfg(test)]
pub(super) fn test_channel(
    shutdown: &CancellationToken,
) -> (EventSender, mpsc::Receiver<DiscoveryEvent>) {
    let (sender, receiver, _) = channel(shutdown);
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_is_bounded_non_blocking_and_cancels_the_producer() {
        let shutdown = CancellationToken::new();
        let (sender, receiver, control) = channel_with_capacity(&shutdown, 2);

        assert!(
            sender
                .send(DiscoveryEvent::Status("one".to_string()))
                .is_ok()
        );
        assert!(
            sender
                .send(DiscoveryEvent::Status("two".to_string()))
                .is_ok()
        );
        assert!(
            sender
                .send(DiscoveryEvent::Status("three".to_string()))
                .is_err()
        );

        assert!(shutdown.is_cancelled());
        assert!(
            control
                .overload_reason()
                .unwrap()
                .contains("2 pending events")
        );
        assert_eq!(receiver.try_iter().count(), 2);
    }
}
