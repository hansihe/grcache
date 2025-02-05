use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, Weak},
};

use hickory_resolver::TokioAsyncResolver;
use pingora::{
    protocols::l4::socket::SocketAddr, server::ShutdownWatch,
    services::background::BackgroundService,
};
use pingora_load_balancing::{
    discovery::ServiceDiscovery, prelude::RoundRobin, selection::Consistent, Backend, Backends,
    Extensions, LoadBalancer,
};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    watch,
};

use super::ServiceBackendsHandle;

enum Command {
    StartDiscoveryLoop {
        host_port: (String, u16),
        ready_send: watch::Sender<bool>,
        backends_sender: watch::Sender<BTreeSet<Backend>>,
    },
}

struct HandleState {
    service_backends: HashMap<(String, u16), Weak<ServiceBackendsHandle>>,
}

struct HandleInner {
    state: Mutex<HandleState>,
    service_commands: UnboundedSender<Command>,
}

#[derive(Clone)]
pub struct Handle(Arc<HandleInner>);

impl Handle {
    fn new(service_commands: UnboundedSender<Command>) -> Self {
        Handle(Arc::new(HandleInner {
            state: Mutex::new(HandleState {
                service_backends: HashMap::new(),
            }),
            service_commands,
        }))
    }

    pub fn backends_for_hostname(&self, hostname: String, port: u16) -> Arc<ServiceBackendsHandle> {
        let mut state = self.0.state.lock().unwrap();

        let host_port = (hostname, port);

        if let Some(lb) = state
            .service_backends
            .get(&host_port)
            .and_then(|lb| lb.upgrade())
        {
            lb
        } else {
            let (ready_send, ready_recv) = watch::channel(false);
            let (backends_sender, backends_receiver) = watch::channel(BTreeSet::new());

            let discovery = Box::new(DNSServiceDiscovery {
                host_port: host_port.clone(),
                backends: backends_receiver.clone(),
                ready: ready_recv.clone(),
            });

            let load_balancer_rr: LoadBalancer<RoundRobin> =
                LoadBalancer::from_backends(Backends::new(discovery.clone()));
            let load_balancer_cons: LoadBalancer<Consistent> =
                LoadBalancer::from_backends(Backends::new(discovery));

            let data = Arc::new(ServiceBackendsHandle {
                backends: backends_receiver,
                load_balancer_round_robin: load_balancer_rr,
                load_balancer_consistent: load_balancer_cons,
                ready: ready_recv,
            });

            state
                .service_backends
                .insert(host_port.clone(), Arc::downgrade(&data));
            self.0
                .service_commands
                .send(Command::StartDiscoveryLoop {
                    host_port,
                    ready_send,
                    backends_sender,
                })
                .unwrap();

            data
        }
    }
}

pub struct Service {
    handle: Handle,
    commands: std::sync::Mutex<Option<UnboundedReceiver<Command>>>,
}

impl Service {
    pub fn new() -> (Service, Handle) {
        let (commands_sender, commands_receiver) = unbounded_channel();
        let handle = Handle::new(commands_sender);
        (
            Service {
                handle: handle.clone(),
                commands: std::sync::Mutex::new(Some(commands_receiver)),
            },
            handle.clone(),
        )
    }
}

async fn dns_discovery_loop(
    resolver: Arc<TokioAsyncResolver>,
    handle: Handle,
    backends_sender: watch::Sender<BTreeSet<Backend>>,
    ready_send: watch::Sender<bool>,
    host_port: (String, u16),
) {
    loop {
        // TODO ready timeout?
        // This can also be done in receiver.

        let response = match resolver.ipv4_lookup(&host_port.0).await {
            Ok(response) => response,
            Err(error) => {
                log::error!(
                    "error \"{}\" while looking up DNS for host `{}:{}`, retrying..",
                    error,
                    host_port.0,
                    host_port.1
                );
                // TODO backoff?
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut backends = BTreeSet::new();
        for addr in response.iter() {
            backends.insert(Backend {
                // TODO port addr
                addr: SocketAddr::Inet(std::net::SocketAddr::new(addr.0.into(), 11111)),
                weight: 1,
                ext: Extensions::new(),
            });
        }

        let lb = {
            let state = handle.0.state.lock().unwrap();
            //state.backends.insert(hostname.clone(), backends.clone());
            state
                .service_backends
                .get(&host_port)
                .and_then(|lb| lb.upgrade())
        };

        let _ = backends_sender.send(backends);

        if let Some(lb) = lb {
            let _ = lb.load_balancer_round_robin.update().await;
            let _ = lb.load_balancer_consistent.update().await;
            let _ = ready_send.send(true);

            // Sleep until TTL expire
            tokio::time::sleep_until(response.valid_until().into()).await;
        } else {
            // All references to lb gone, we stop refresh loop.
            break;
        }
    }
}

#[async_trait::async_trait]
impl BackgroundService for Service {
    async fn start(&self, _shutdown: ShutdownWatch) {
        let mut commands = self.commands.try_lock().unwrap().take().unwrap();
        let resolver = Arc::new(
            TokioAsyncResolver::tokio_from_system_conf().expect("failed to create DNS resolver"),
        );

        log::info!("DNS discovery service started");

        // TODO select on shutdown in loop
        // TODO handle discovery loop panic/shutdown
        loop {
            // Unwrap is safe since sender is held from handle, will
            // never go out of scope since it is referenced from `self`.
            let command = commands.recv().await.unwrap();

            match command {
                Command::StartDiscoveryLoop {
                    host_port,
                    ready_send,
                    backends_sender,
                } => {
                    let resolver = resolver.clone();
                    let handle = self.handle.clone();

                    // Spawn task to handle the discovery loop
                    tokio::spawn(dns_discovery_loop(
                        resolver,
                        handle,
                        backends_sender,
                        ready_send,
                        host_port,
                    ));
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct DNSServiceDiscovery {
    host_port: (String, u16),
    backends: watch::Receiver<BTreeSet<Backend>>,
    ready: watch::Receiver<bool>,
}

#[async_trait::async_trait]
impl ServiceDiscovery for DNSServiceDiscovery {
    async fn discover(&self) -> Result<(BTreeSet<Backend>, HashMap<u64, bool>), pingora::BError> {
        // Wait for initial DNS query to complete
        self.ready.clone().wait_for(|v| *v).await.unwrap();

        let backends = self.backends.borrow().clone();
        Ok((backends, HashMap::new()))
    }
}
