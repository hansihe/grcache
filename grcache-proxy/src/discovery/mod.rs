use std::{collections::BTreeSet, sync::Arc};

use pingora_load_balancing::{prelude::RoundRobin, selection::Consistent, Backend, LoadBalancer};
use tokio::sync::watch;

pub mod dns;

/// Handle to actively maintained set of backends for a service
/// maintained through a service discovery mechanism.
pub struct ServiceBackendsHandle {
    /// Watch which allows you to listen for changes to the set of
    /// backends.
    pub backends: watch::Receiver<BTreeSet<Backend>>,
    /// Pingora `LoadBalancer` for the backends.
    /// It is usually recommended to use this for backend selection
    /// as it may also take any potential health checks into account.
    pub load_balancer_round_robin: LoadBalancer<RoundRobin>,
    pub load_balancer_consistent: LoadBalancer<Consistent>,
    /// When service discovery first starts, it might take some amount
    /// of time before it is considered finished.
    /// This watch signals `true` when it is ready.
    /// Once it has transitioned into `true`, it will never go back
    /// to false.
    pub ready: watch::Receiver<bool>,
}
