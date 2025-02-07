use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use pingora_load_balancing::{
    discovery::ServiceDiscovery, prelude::RoundRobin, selection::Consistent, Backend, Backends,
    LoadBalancer,
};
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

impl ServiceBackendsHandle {
    pub fn new_test() -> (
        Arc<Self>,
        watch::Sender<bool>,
        watch::Sender<BTreeSet<Backend>>,
    ) {
        let (backend_sender, backend_receiver) = watch::channel(BTreeSet::new());
        let (ready_sender, ready_receiver) = watch::channel(true);
        let discovery = WatchServiceDiscovery::new(backend_receiver.clone());
        let handle = ServiceBackendsHandle {
            backends: backend_receiver,
            load_balancer_round_robin: LoadBalancer::from_backends(
                discovery.clone().into_backends(),
            ),
            load_balancer_consistent: LoadBalancer::from_backends(
                discovery.clone().into_backends(),
            ),
            ready: ready_receiver,
        };
        (Arc::new(handle), ready_sender, backend_sender)
    }
}

#[derive(Debug, Clone)]
struct WatchServiceDiscovery {
    backends: watch::Receiver<BTreeSet<Backend>>,
}

impl WatchServiceDiscovery {
    pub fn new(receiver: watch::Receiver<BTreeSet<Backend>>) -> Self {
        Self { backends: receiver }
    }

    pub fn into_backends(self) -> Backends {
        Backends::new(Box::new(self))
    }
}

#[async_trait::async_trait]
impl ServiceDiscovery for WatchServiceDiscovery {
    async fn discover(&self) -> Result<(BTreeSet<Backend>, HashMap<u64, bool>), pingora::BError> {
        let backends = self.backends.borrow().clone();
        Ok((backends, HashMap::new()))
    }
}
