use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, Weak},
    time::Instant,
};

use hickory_resolver::TokioAsyncResolver;
use pingora::{
    protocols::l4::socket::SocketAddr, server::ShutdownWatch,
    services::background::BackgroundService,
};
use pingora_load_balancing::{
    prelude::RoundRobin, selection::Consistent, Backend, Extensions, LoadBalancer,
};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    watch,
};

use super::{ServiceBackendsHandle, WatchServiceDiscovery};

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

            let discovery = WatchServiceDiscovery::new(backends_receiver.clone());

            let load_balancer_rr: LoadBalancer<RoundRobin> =
                LoadBalancer::from_backends(discovery.clone().into_backends());
            let load_balancer_cons: LoadBalancer<Consistent> =
                LoadBalancer::from_backends(discovery.clone().into_backends());

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
    resolver: Arc<TokioAsyncResolver>,
}

impl Service {
    pub fn new(resolver: Arc<TokioAsyncResolver>) -> (Service, Handle) {
        let (commands_sender, commands_receiver) = unbounded_channel();
        let handle = Handle::new(commands_sender);
        (
            Service {
                handle: handle.clone(),
                commands: std::sync::Mutex::new(Some(commands_receiver)),
                resolver,
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
    log::info!(
        "started DNS discovery loop for `{}:{}`",
        host_port.0,
        host_port.1
    );

    let handle_weak = {
        let state = handle.0.state.lock().unwrap();
        state.service_backends.get(&host_port).unwrap().clone()
    };

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

                if handle_weak.strong_count() == 0 {
                    // All references to lb gone, we stop refresh loop.
                    break;
                }

                // TODO backoff?
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut backends = BTreeSet::new();
        for addr in response.iter() {
            backends.insert(Backend {
                // TODO port addr
                addr: SocketAddr::Inet(std::net::SocketAddr::new(addr.0.into(), host_port.1)),
                weight: 1,
                ext: Extensions::new(),
            });
        }

        // Send updated backends to watch
        let has_changed = backends_sender.send_if_modified(|val| {
            if val != &backends {
                *val = backends;
                true
            } else {
                false
            }
        });

        if has_changed {
            if let Some(lb) = handle_weak.upgrade() {
                let _ = lb.load_balancer_round_robin.update().await;
                let _ = lb.load_balancer_consistent.update().await;
            }
        }

        if handle_weak.strong_count() == 0 {
            // All references to lb gone, we stop refresh loop.
            break;
        }

        let _ = ready_send.send(true);

        let valid_sec = response
            .valid_until()
            .duration_since(Instant::now())
            .as_secs();
        log::debug!(
            "got result for DNS `{}:{}` valid for {}s",
            host_port.0,
            host_port.1,
            valid_sec
        );

        // Sleep until TTL expire
        tokio::time::sleep_until(response.valid_until().into()).await;
    }

    log::info!(
        "stopped DNS discovery loop for `{}:{}`",
        host_port.0,
        host_port.1
    );
}

#[async_trait::async_trait]
impl BackgroundService for Service {
    async fn start(&self, _shutdown: ShutdownWatch) {
        let mut commands = self.commands.try_lock().unwrap().take().unwrap();

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
                    let resolver = self.resolver.clone();
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
