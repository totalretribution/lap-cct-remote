//! One pairing attempt, shared by the worker (which runs it), the tray (icon and menu) and the
//! pairing window (which shows it).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;
use light_protocol::RemoteId;

use crate::controller::{Cancel, PAIR_PRESSES, PAIR_TIMEOUT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Turning on Bluetooth scanning.
    Starting,
    /// Waiting for POWER presses; `heard` so far from the leading remote.
    Listening {
        heard: Option<RemoteId>,
        presses: usize,
    },
    Paired(RemoteId),
    NothingHeard,
    Cancelled,
    Failed(String),
}

impl Phase {
    #[must_use]
    pub fn in_progress(&self) -> bool {
        matches!(self, Phase::Starting | Phase::Listening { .. })
    }
}

#[derive(Clone, Debug)]
pub struct Session(Arc<Mutex<State>>);

#[derive(Debug)]
struct State {
    phase: Phase,
    listening_since: Option<Instant>,
    cancel: Cancel,
    repaint: Option<egui::Context>,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Session(Arc::new(Mutex::new(State {
            phase: Phase::Starting,
            listening_since: None,
            cancel: Cancel::new(),
            repaint: None,
        })))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        self.state().phase.clone()
    }

    /// How long listening has left, while listening.
    #[must_use]
    pub fn time_left(&self) -> Option<Duration> {
        let state = self.state();
        match state.phase {
            Phase::Listening { .. } => state
                .listening_since
                .map(|t| PAIR_TIMEOUT.saturating_sub(t.elapsed())),
            _ => None,
        }
    }

    pub fn set_phase(&self, phase: Phase) {
        let mut state = self.state();
        if matches!(phase, Phase::Listening { .. }) && state.listening_since.is_none() {
            state.listening_since = Some(Instant::now());
        }
        state.phase = phase;
        if let Some(ctx) = &state.repaint {
            ctx.request_repaint();
        }
    }

    /// Records a POWER press heard from `id`, the `presses`-th from that remote.
    pub fn heard(&self, id: RemoteId, presses: usize) {
        self.set_phase(Phase::Listening {
            heard: Some(id),
            presses: presses.min(PAIR_PRESSES),
        });
    }

    /// Stops the attempt, if it's still running.
    pub fn cancel(&self) {
        let state = self.state();
        if state.phase.in_progress() {
            state.cancel.cancel();
        }
    }

    #[must_use]
    pub fn cancel_handle(&self) -> Cancel {
        self.state().cancel.clone()
    }

    /// Repaints this window whenever the phase changes.
    pub fn repaint_with(&self, ctx: &egui::Context) {
        self.state().repaint = Some(ctx.clone());
    }
}
