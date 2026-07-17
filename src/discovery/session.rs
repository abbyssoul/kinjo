//! The owned lifetime of a running discovery adapter.
//!
//! A [`DiscoverySession`] is the one thing a caller needs to hold to run
//! discovery: it owns the event receiver *and* the producer behind it, so the
//! two can never be separated, replaced independently, or outlive each other.
//! Dropping the session stops the producer.
//!
//! It also answers the question a bare `mpsc::Receiver` cannot: **why** the
//! events stopped. `try_recv` reports `Disconnected` identically for "the
//! adapter never started", "the browse died", and "the sample stream finished",
//! and reports `Empty` forever afterwards — indistinguishable from a quiet
//! network. The session pairs the disconnect with the producer's own account of
//! its ending and turns it into a typed, persistent [`SessionState`].

use std::sync::mpsc;

use super::DiscoveryEvent;
use super::worker::{BrowseOutcome, DiscoveryWorker};

/// Why discovery is no longer running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The adapter never started browsing: no runtime, no browser, no browse.
    /// Nothing was ever discovered, so there is nothing to distrust.
    Startup,
    /// The adapter was browsing and its producer went away unexpectedly.
    /// Anything it had reported is now unverifiable.
    Stopped,
}

/// A discovery failure with its provenance and actionable cause text.
///
/// The cause is carried by the value rather than left on a status line, so it
/// survives any number of later events instead of being overwritten by the next
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryFailure {
    pub kind: FailureKind,
    pub cause: String,
}

impl DiscoveryFailure {
    /// Short summary of what happened, without the cause.
    pub fn headline(&self) -> &'static str {
        match self.kind {
            FailureKind::Startup => "discovery failed to start",
            FailureKind::Stopped => "discovery stopped",
        }
    }

    /// One-line, user-facing rendering: what happened and why.
    pub fn message(&self) -> String {
        format!("{}: {}", self.headline(), self.cause)
    }
}

/// What a session is doing, and what its records are worth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// The producer is running and the channel is open.
    Listening,
    /// A finite sample stream finished normally. Only explicit fake discovery
    /// ends this way. Its records stay valid: they were never claims about the
    /// network, and nothing is expected to arrive after them.
    Complete,
    /// The producer stopped without completing. Real records gathered by this
    /// session are no longer being confirmed and must not be presented as live.
    Failed(DiscoveryFailure),
}

impl SessionState {
    /// Whether the session is still expected to produce events.
    pub fn is_listening(&self) -> bool {
        matches!(self, SessionState::Listening)
    }
}

/// One drain step of a session.
#[derive(Debug)]
pub enum SessionPoll {
    /// The producer sent an event.
    Event(DiscoveryEvent),
    /// No event is available right now. This says nothing about whether the
    /// session is still listening — ask [`DiscoverySession::state`].
    Idle,
    /// The producer finished. Reported exactly once, on the transition, so a
    /// caller can react to the ending rather than re-apply it every poll. The
    /// outcome remains readable from [`DiscoverySession::state`] afterwards.
    Ended(SessionState),
}

/// A running discovery adapter: its events, its producer, and its ending.
///
/// Construct one with [`start`](super::start). The session is the unit of
/// replacement: a refresh builds a new one and drops the old, which cancels the
/// old producer and takes its receiver with it. Events from a replaced session
/// therefore cannot reach the new one's list — the guarantee is structural
/// rather than a rule callers must remember.
pub struct DiscoverySession {
    receiver: mpsc::Receiver<DiscoveryEvent>,
    /// The producer. `None` only for a [`detached`](Self::detached) session,
    /// whose events come from a channel the caller drives itself.
    producer: Option<DiscoveryWorker>,
    /// Holds an [`inert`](Self::inert) session's channel open so it stays
    /// `Listening` with nothing feeding it. Never read: being alive is the
    /// entire job, and dropping it is what would end the session.
    #[cfg(test)]
    #[allow(dead_code)]
    keepalive: Option<mpsc::Sender<DiscoveryEvent>>,
    state: SessionState,
}

impl DiscoverySession {
    /// A session fed by a worker-backed adapter.
    pub(super) fn from_worker(
        receiver: mpsc::Receiver<DiscoveryEvent>,
        worker: DiscoveryWorker,
    ) -> Self {
        Self {
            receiver,
            producer: Some(worker),
            #[cfg(test)]
            keepalive: None,
            state: SessionState::Listening,
        }
    }

    /// A session whose events come from a caller-owned channel and whose
    /// producer this session does not manage. Dropping the sender ends the
    /// session exactly as a real producer going away does, which is what makes
    /// this the way to drive endings in a test.
    #[cfg(test)]
    pub(crate) fn detached(receiver: mpsc::Receiver<DiscoveryEvent>) -> Self {
        Self {
            receiver,
            producer: None,
            keepalive: None,
            state: SessionState::Listening,
        }
    }

    /// A session that has already ended in `state`.
    ///
    /// For tests about how an ending is *presented* rather than how it is
    /// reached. Reaching each ending for real is this module's own business and
    /// is covered below; this lets a caller cover every terminal state without a
    /// live producer — including [`SessionState::Complete`], which otherwise
    /// only a finite fake stream can produce, and then only under its feature.
    ///
    /// `state` must be an ending. [`SessionState::Listening`] here would be a
    /// session that has ended and is still running: use [`Self::inert`] for a
    /// live one.
    #[cfg(test)]
    pub(crate) fn ended(state: SessionState) -> Self {
        debug_assert!(
            !state.is_listening(),
            "`ended` takes an ending; use `inert` for a listening session"
        );
        // No sender: the channel is disconnected, exactly as it is after a real
        // producer has gone. `poll` leaves an already-ended state alone, so the
        // session stays in the ending it was given.
        let (_, receiver) = mpsc::channel();
        Self {
            receiver,
            producer: None,
            keepalive: None,
            state,
        }
    }

    /// A session that stays listening and never produces anything, for tests
    /// about behaviour that has nothing to do with discovery.
    #[cfg(test)]
    pub(crate) fn inert() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            receiver: rx,
            producer: None,
            keepalive: Some(tx),
            state: SessionState::Listening,
        }
    }

    /// Take the next available event, if any, and notice when the producer ends.
    pub fn poll(&mut self) -> SessionPoll {
        match self.receiver.try_recv() {
            Ok(event) => SessionPoll::Event(event),
            Err(mpsc::TryRecvError::Empty) => SessionPoll::Idle,
            // Disconnected is only reported once the buffered events have been
            // drained, so no event is ever lost to the ending.
            Err(mpsc::TryRecvError::Disconnected) => {
                if !self.state.is_listening() {
                    // Already reported; the caller reacted to it once.
                    return SessionPoll::Idle;
                }
                self.state = self.ending();
                SessionPoll::Ended(self.state.clone())
            }
        }
    }

    /// What the session is doing, and what its records are worth.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Stop the producer and join it. Bounded: every browse loop selects on the
    /// cancellation token. Idempotent, and also run by `Drop`.
    pub fn shutdown(&mut self) {
        if let Some(producer) = &mut self.producer {
            producer.shutdown();
        }
    }

    /// Translate the producer's account of its ending into a session state.
    ///
    /// Only called once the channel has disconnected, which guarantees the
    /// worker published its outcome first.
    fn ending(&self) -> SessionState {
        match self.producer.as_ref().and_then(DiscoveryWorker::outcome) {
            #[cfg(feature = "fake")]
            Some(BrowseOutcome::Complete) => SessionState::Complete,
            Some(BrowseOutcome::Startup(cause)) => SessionState::Failed(DiscoveryFailure {
                kind: FailureKind::Startup,
                cause,
            }),
            Some(BrowseOutcome::Cancelled) => SessionState::Failed(DiscoveryFailure {
                kind: FailureKind::Stopped,
                cause: "discovery was stopped".to_string(),
            }),
            Some(BrowseOutcome::Overloaded(cause)) => SessionState::Failed(DiscoveryFailure {
                kind: FailureKind::Stopped,
                cause,
            }),
            // A detached session has no producer to ask, and a worker whose
            // thread died without publishing cannot account for itself. Both
            // are an unexplained stop, which is the honest answer.
            Some(BrowseOutcome::Stopped) | None => SessionState::Failed(DiscoveryFailure {
                kind: FailureKind::Stopped,
                cause: "the browse ended unexpectedly; refresh to retry".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Entry;
    #[cfg(feature = "fake")]
    use crate::discovery::{DiscoveryBackend, DiscoveryConfig, DiscoveryOptions, start};

    #[cfg(feature = "fake")]
    fn fake_options(service_type: Option<&str>) -> DiscoveryOptions {
        DiscoveryConfig {
            backend: DiscoveryBackend::Fake,
            domain: "local".to_string(),
            service_type: service_type.map(str::to_string),
        }
        .validate()
        .expect("valid test options")
    }

    /// Drain a session to its ending, collecting the events it produced.
    fn drain_to_end(session: &mut DiscoverySession) -> (Vec<DiscoveryEvent>, SessionState) {
        let mut events = Vec::new();
        loop {
            match session.poll() {
                SessionPoll::Event(event) => events.push(event),
                SessionPoll::Idle => std::thread::yield_now(),
                SessionPoll::Ended(state) => return (events, state),
            }
        }
    }

    #[cfg(feature = "fake")]
    fn upserts(events: &[DiscoveryEvent]) -> Vec<Entry> {
        events
            .iter()
            .filter_map(|event| match event {
                DiscoveryEvent::Upsert(entry) => Some(entry.clone()),
                _ => None,
            })
            .collect()
    }

    /// A quiet network and a dead producer must not look alike: an empty
    /// channel is `Idle` while listening, and a dropped one is an ending.
    #[test]
    fn an_empty_channel_is_idle_and_a_dropped_one_ends_the_session() {
        let (tx, rx) = mpsc::channel();
        let mut session = DiscoverySession::detached(rx);

        assert!(matches!(session.poll(), SessionPoll::Idle));
        assert!(session.state().is_listening());

        drop(tx);

        match session.poll() {
            SessionPoll::Ended(SessionState::Failed(failure)) => {
                assert_eq!(failure.kind, FailureKind::Stopped);
            }
            other => panic!("expected a failed ending, got {other:?}"),
        }
    }

    /// The ending is reported once so a caller can act on the transition, but
    /// the state persists: it must never decay back to "listening".
    #[test]
    fn the_ending_is_reported_once_and_the_state_persists() {
        let (tx, rx) = mpsc::channel();
        let mut session = DiscoverySession::detached(rx);
        drop(tx);

        assert!(matches!(session.poll(), SessionPoll::Ended(_)));

        // Every later poll is quiet, and the verdict stands.
        assert!(matches!(session.poll(), SessionPoll::Idle));
        assert!(matches!(session.poll(), SessionPoll::Idle));
        assert!(matches!(session.state(), SessionState::Failed(_)));
        assert!(!session.state().is_listening());
    }

    /// Events buffered before the producer went away are still delivered; the
    /// ending comes after them, not instead of them.
    #[test]
    fn buffered_events_are_delivered_before_the_ending() {
        let (tx, rx) = mpsc::channel();
        let mut session = DiscoverySession::detached(rx);
        tx.send(DiscoveryEvent::Status("browsing".to_string()))
            .unwrap();
        tx.send(DiscoveryEvent::Upsert(Entry::new(
            "nas",
            "_http._tcp",
            "local",
        )))
        .unwrap();
        drop(tx);

        let (events, state) = drain_to_end(&mut session);

        assert_eq!(events.len(), 2);
        assert!(matches!(state, SessionState::Failed(_)));
    }

    /// Explicit fake discovery streams the documented samples and then reports
    /// a *normal* completion: a finite stream running out is not a failure, and
    /// its records stay trustworthy.
    #[cfg(feature = "fake")]
    #[test]
    fn the_fake_session_completes_normally_after_its_finite_stream() {
        let mut session = start(&fake_options(Some("_ssh._tcp")));

        let (events, state) = drain_to_end(&mut session);

        assert_eq!(state, SessionState::Complete);
        assert!(!state.is_listening());
        let records = upserts(&events);
        // The sample set advertises SSH on two hosts.
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .all(|record| record.service_type == "_ssh._tcp"),
            "the filter must admit only the requested type"
        );
    }

    /// Dropping a session must stop its producer even mid-stream — including
    /// the fake adapter, whose sample stream sleeps between records.
    #[cfg(feature = "fake")]
    #[test]
    fn dropping_a_fake_session_cancels_its_delayed_stream() {
        let mut session = start(&fake_options(None));

        // Wait for the stream to actually start, so the cancellation lands
        // during one of its inter-record sleeps rather than before it began.
        loop {
            match session.poll() {
                SessionPoll::Event(DiscoveryEvent::Upsert(_)) => break,
                SessionPoll::Ended(state) => panic!("stream ended early: {state:?}"),
                _ => std::thread::yield_now(),
            }
        }

        // Drop must be bounded: it cancels the sleep rather than waiting out
        // the rest of the samples. The test hanging here is the failure.
        drop(session);
    }

    /// Shutting a session down stops its producer and is safe to repeat.
    #[cfg(feature = "fake")]
    #[test]
    fn shutdown_stops_the_producer_and_is_idempotent() {
        let mut session = start(&fake_options(None));

        session.shutdown();
        session.shutdown();

        // The producer is gone, so the session can only report its ending.
        loop {
            match session.poll() {
                SessionPoll::Idle => std::thread::yield_now(),
                SessionPoll::Ended(state) => {
                    assert!(!state.is_listening());
                    break;
                }
                SessionPoll::Event(_) => {}
            }
        }
    }

    /// A cancelled producer is a stop, not a completion: its records were cut
    /// short and must not be labelled as a finished stream.
    #[cfg(feature = "fake")]
    #[test]
    fn a_cancelled_producer_ends_as_stopped_not_complete() {
        let mut session = start(&fake_options(None));
        session.shutdown();

        let (_events, state) = drain_to_end(&mut session);

        match state {
            SessionState::Failed(failure) => assert_eq!(failure.kind, FailureKind::Stopped),
            other => panic!("expected a failed ending, got {other:?}"),
        }
    }

    /// The failure's cause travels with the value, so it is still available
    /// after any number of later polls.
    #[test]
    fn a_failure_carries_its_own_cause_text() {
        let failure = DiscoveryFailure {
            kind: FailureKind::Startup,
            cause: "mDNS discovery unavailable (no such device)".to_string(),
        };

        assert_eq!(failure.headline(), "discovery failed to start");
        assert_eq!(
            failure.message(),
            "discovery failed to start: mDNS discovery unavailable (no such device)"
        );
    }
}
