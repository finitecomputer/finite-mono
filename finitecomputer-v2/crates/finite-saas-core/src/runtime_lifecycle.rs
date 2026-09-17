//! The canonical runtime-control lifecycle state machine (2026-08 audit H1).
//!
//! Every transition the store can write is a typed, consuming method here:
//! legal orderings are the only expressible programs, so an illegal
//! transition is unrepresentable rather than guarded against at runtime.
//! `Succeeded` exists only as the successor of `Ready`, which exists only as
//! the successor of `ComputeUp`: `succeeded` can never again be written for
//! "compute exists". Rehydrating from a persisted row goes through each
//! phase's `from_status`, the one honest runtime boundary.
//!
//! Kind-consistency of completions (Stop confirms with `Plain`, Destroy with
//! a receipt, Upgrade with artifact facts) is enforced upstream by
//! [`RuntimeControlCompletion::parse`], which is keyed on the request kind.

use super::{RuntimeControlCompletion, RuntimeControlRequestStatus, RuntimeLifecycleStage};

/// Phase markers. `Failed` carries its named stage; every other marker
/// is a zero-sized proof of position in the machine.
pub mod phase;

/// The persisted status a phase marker stands for.
pub trait LifecyclePhase {
    const STATUS: RuntimeControlRequestStatus;
}

macro_rules! lifecycle_phase {
    ($($phase:ty => $status:expr),+ $(,)?) => {$(
        impl LifecyclePhase for $phase {
            const STATUS: RuntimeControlRequestStatus = $status;
        }
    )+};
}

lifecycle_phase! {
    phase::Requested => RuntimeControlRequestStatus::Requested,
    phase::Launching => RuntimeControlRequestStatus::Launching,
    phase::ComputeUp => RuntimeControlRequestStatus::ComputeUp,
    phase::Ready => RuntimeControlRequestStatus::Ready,
    phase::Succeeded => RuntimeControlRequestStatus::Succeeded,
    phase::Stopped => RuntimeControlRequestStatus::Stopped,
    phase::Failed => RuntimeControlRequestStatus::Failed,
}

/// A runtime-control request at lifecycle phase `S`. Constructing one
/// requires either starting at [`phase::Requested`] or proving the
/// persisted status matches the phase via `from_status`.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLifecycle<S: LifecyclePhase> {
    phase: S,
}

impl<S: LifecyclePhase> RuntimeLifecycle<S> {
    fn next<T: LifecyclePhase>(phase: T) -> RuntimeLifecycle<T> {
        RuntimeLifecycle { phase }
    }

    pub fn status(&self) -> RuntimeControlRequestStatus {
        S::STATUS
    }
}

impl RuntimeLifecycle<phase::Requested> {
    pub fn enqueue() -> Self {
        Self::next(phase::Requested)
    }

    pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
        (status == RuntimeControlRequestStatus::Requested).then(Self::enqueue)
    }

    /// The Runner leased the request and owns the launch.
    pub fn lease(self) -> RuntimeLifecycle<phase::Launching> {
        Self::next(phase::Launching)
    }

    pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
        Self::next(phase::Failed { stage })
    }
}

impl RuntimeLifecycle<phase::Launching> {
    pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
        (status == RuntimeControlRequestStatus::Launching).then(|| Self::next(phase::Launching))
    }

    /// The Runner reports compute exists, carrying the kind-checked
    /// completion. This proves the runtime is up; it never proves the
    /// runtime is ready.
    pub fn compute_up(
        self,
        completion: &RuntimeControlCompletion,
    ) -> RuntimeLifecycle<phase::ComputeUp> {
        let _ = completion;
        Self::next(phase::ComputeUp)
    }

    /// Stop and Destroy confirm directly into their own terminal; a
    /// stopped runtime has no readiness phase.
    pub fn confirm_stopped(
        self,
        completion: &RuntimeControlCompletion,
    ) -> RuntimeLifecycle<phase::Stopped> {
        let _ = completion;
        Self::next(phase::Stopped)
    }

    /// Retirement requeues itself (Destroy only; the kind gate stays in
    /// the store, which owns the request row).
    pub fn retry(self) -> RuntimeLifecycle<phase::Requested> {
        Self::next(phase::Requested)
    }

    pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
        Self::next(phase::Failed { stage })
    }
}

impl RuntimeLifecycle<phase::ComputeUp> {
    pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
        (status == RuntimeControlRequestStatus::ComputeUp).then(|| Self::next(phase::ComputeUp))
    }

    /// The runtime's readiness probe fired. This is the only edge into
    /// `Ready`.
    pub fn ready(self) -> RuntimeLifecycle<phase::Ready> {
        Self::next(phase::Ready)
    }

    pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
        Self::next(phase::Failed { stage })
    }
}

impl RuntimeLifecycle<phase::Ready> {
    pub fn from_status(status: RuntimeControlRequestStatus) -> Option<Self> {
        (status == RuntimeControlRequestStatus::Ready).then(|| Self::next(phase::Ready))
    }

    /// Terminal success. Only reachable from `Ready`.
    pub fn succeed(self) -> RuntimeLifecycle<phase::Succeeded> {
        Self::next(phase::Succeeded)
    }

    pub fn fail(self, stage: RuntimeLifecycleStage) -> RuntimeLifecycle<phase::Failed> {
        Self::next(phase::Failed { stage })
    }
}

impl RuntimeLifecycle<phase::Failed> {
    pub fn stage(&self) -> RuntimeLifecycleStage {
        self.phase.stage
    }
}
