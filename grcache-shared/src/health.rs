use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};

use tokio::sync::watch;

pub struct State {
    ready_blocks: AtomicU64,
    ready: watch::Sender<bool>,
}

/// Handle to the health check subsystem.
#[derive(Clone)]
pub struct Health {
    state: Arc<State>,
}

impl Health {
    /// Creates a new health tracker.
    /// An initial `HealthEndpoint` is created, and the expectation
    /// is that the user will add other endpoints before indicating
    /// the root `HealthEndpoint` as ready.
    pub fn new() -> (Self, HealthEndpoint) {
        let state = Arc::new(State {
            ready_blocks: AtomicU64::new(1),
            ready: watch::channel(false).0,
        });
        let health = Health {
            state: state.clone(),
        };
        let endpoint = HealthEndpoint {
            can_indicate_ready: AtomicBool::new(true),
            state,
        };
        (health, endpoint)
    }

    /// Adds a new health endpoint.
    /// A health tracker can have an arbitrary number of health
    /// endpoints.
    pub fn add(&self, blocks_ready: bool) -> HealthEndpoint {
        if blocks_ready {
            let prev = self
                .state
                .ready_blocks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            if prev == 0 {
                log::warn!("added ready block, but we have already indicated readiness!");
            }
        }
        HealthEndpoint {
            can_indicate_ready: AtomicBool::new(blocks_ready),
            state: self.state.clone(),
        }
    }
}

pub struct HealthEndpoint {
    can_indicate_ready: AtomicBool,
    state: Arc<State>,
}

impl Drop for HealthEndpoint {
    fn drop(&mut self) {
        // Indicate readiness on drop.
        // We might want another behaviour?
        // At least this prevents us form going into a state
        // where we never indicate ready even if there are no
        // non ready endpoints.
        self.ready();
    }
}

impl HealthEndpoint {
    /// Sets a name for the health endpoint.
    /// This can be used to print diagnostics.
    pub fn name(&mut self, _name: impl Into<String>) {}

    /// Returns a reference to the health tracker object.
    pub fn health(&self) -> Health {
        Health {
            state: self.state.clone(),
        }
    }

    /// Indicates readiness.
    /// This function is indepotent, and can be called as many
    /// times as convenient. It is also expected to be very cheap
    /// to call.
    pub fn ready(&self) {
        if self
            .can_indicate_ready
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            let before_count = self
                .state
                .ready_blocks
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

            // If count == 1, then we are the last.
            // Signal readiness.
            if before_count == 1 {
                self.state.ready.send_if_modified(|v| {
                    let ready = *v;
                    *v = true;
                    !ready
                });
            }
        }
    }
}
