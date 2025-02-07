use futures::StreamExt;
use papaya::Compute;
use pingora::{server::ShutdownWatch, services::background::BackgroundService};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    pin::pin,
    sync::Arc,
};
use tokio::{select, sync::watch};

use grcache_shared::{
    config::crd::GrcacheService,
    resource_change::{resource_changes, ResourceChange},
    service::{
        descriptor_set::{self, DummyPanicContext},
        qualified_service::QualifiedService,
        ServiceSpec,
    },
};
use kube::{
    runtime::{reflector::ObjectRef, watcher, WatchStreamExt as _},
    Api, Client,
};

use crate::discovery::{self, ServiceBackendsHandle};

#[derive(Clone)]
pub struct ServiceData {
    // public for tests
    pub generation: usize,
    pub service_spec: Option<Arc<ServiceSpec>>,
    pub load_balancer: Option<Arc<ServiceBackendsHandle>>,
}

pub struct ServiceConfigInner {
    pub ready: watch::Receiver<bool>,
    pub services: papaya::HashMap<String, ServiceData>,
}

impl ServiceConfigInner {
    /// Swap in new spec IF we are not already on a newer generation.
    /// This prevents races.
    fn update_service_if_newer(&self, service_name: String, data: ServiceData) {
        let services_map = self.services.pin();
        let result: Compute<_, _, ()> = services_map.compute(service_name.clone(), |kv| {
            if let Some((_key, value)) = kv {
                if value.generation < data.generation {
                    // Old generation, replace value.
                    papaya::Operation::Insert(data.clone())
                } else {
                    // We raced, abort.
                    papaya::Operation::Abort(())
                }
            } else {
                // No element in map, insert.
                papaya::Operation::Insert(data.clone())
            }
        });

        match result {
            Compute::Inserted(k, _v) => {
                log::info!(
                    "loaded spec for gRPC service: {} has_descriptor: {}",
                    k,
                    !data.service_spec.as_ref().unwrap().passthrough
                );
            }
            Compute::Updated { new: (k, _v), .. } => {
                log::info!(
                    "loaded updated spec for gRPC service: {} has_descriptor: {}",
                    k,
                    !data.service_spec.as_ref().unwrap().passthrough
                );
            }
            Compute::Removed(_k, _v) => unreachable!(),
            Compute::Aborted(_t) => (),
        }
    }
}

pub type ServiceConfig = Arc<ServiceConfigInner>;

//pub struct ServiceState {
//    /// The ready state for the service data.
//    /// The state is concidered ready if:
//    /// * Service resources are fetched from k8s store and up to date (as far as we know).
//    /// * Service descriptor sets are resolved.
//    ///
//    /// Goes through the following states:
//    /// * Starting - Not yet ready to accept requests.
//    /// * Ready - Ready to receive requests, we have needed data
//    /// * ReadyUpdating - Ready to receive requests, but data is possibly
//    ///   outdated. Rationale is we do not want to stop accepting requests
//    ///   because data us outdated.
//    ///
//    /// At boot, state stays in `Starting` state until BOTH:
//    /// * We have fetched data from k8s CRDs. Once we have this data, we
//    ///   have everything we need in order to proxy requests to upstreams.
//    /// * A reasonable effort has been made to fetch descriptors from
//    ///   their respective sources.
//    /// This has been chosen in order to satisfy these goals:
//    /// * When we are ready we are ALWAYS in a position to serve requests.
//    /// * When we are ready, we are LIKELY in a position to cache requests.
//    ///   When fetching descriptors fails persistently, we can instead just
//    ///   proxy directly to upstream.
//    /// * When we fail to fetch descriptors, we don't block startup forever.
//    ready: bool,
//
//    /// The state of our service descriptors from k8s.
//    services: HashMap<ObjectRef<GrcacheService>, GrcacheService>,
//}

pub struct K8SConfigService {
    dns_service_handle: discovery::dns::Handle,
    ready_sender: watch::Sender<bool>,
    config: ServiceConfig,
}

impl K8SConfigService {
    pub fn new(dns_service_handle: discovery::dns::Handle) -> (Self, ServiceConfig) {
        let (ready_sender, ready_receiver) = watch::channel(false);

        let config = Arc::new(ServiceConfigInner {
            ready: ready_receiver,
            services: papaya::HashMap::new(),
        });

        let service = Self {
            dns_service_handle,
            ready_sender,
            config: config.clone(),
        };

        (service, config)
    }
}

struct ServiceConfigState {
    spec_generation: usize,
    config: ServiceConfig,
    raw_services: HashMap<ObjectRef<GrcacheService>, GrcacheService>,
    ref_by_grpc_service: BTreeMap<String, HashSet<ObjectRef<GrcacheService>>>,
    dns_service_handle: discovery::dns::Handle,
}

impl ServiceConfigState {
    fn remove(&mut self, object_ref: &ObjectRef<GrcacheService>) {
        if let Some(previous) = self.remove_noupdate(object_ref) {
            //for name in previous.spec.service_names.iter() {
            let name = &previous.spec.service_name;
            self.update_service(name);
        }
    }

    fn remove_noupdate(
        &mut self,
        object_ref: &ObjectRef<GrcacheService>,
    ) -> Option<GrcacheService> {
        let previous = self.raw_services.remove(object_ref);

        if let Some(previous) = &previous {
            //for name in previous.spec.service_names.iter() {
            let name = &previous.spec.service_name;
            self.ref_by_grpc_service
                .get_mut(name)
                .unwrap()
                .remove(object_ref);
        }

        previous
    }

    fn apply(&mut self, object: GrcacheService) {
        // Remove old if present, this handles updates properly.
        let object_ref = ObjectRef::from_obj(&object);
        self.remove_noupdate(&object_ref);

        self.raw_services.insert(object_ref.clone(), object.clone());

        //for grpc_service in object.spec.service_names.iter() {
        let grpc_service = &object.spec.service_name;
        self.ref_by_grpc_service
            .entry(grpc_service.to_owned())
            .or_default()
            .insert(object_ref.clone());

        self.update_service(&grpc_service);
    }

    fn update_service(&mut self, grpc_service: &str) {
        let objects = self.ref_by_grpc_service.get(grpc_service).unwrap();

        if objects.is_empty() {
            let generation = self.spec_generation;
            self.spec_generation += 1;

            let services = self.config.services.pin();
            services.insert(
                grpc_service.to_string(),
                ServiceData {
                    generation,
                    service_spec: None,
                    load_balancer: None,
                },
            );

            log::info!("unloaded spec for gRPC service: {}", grpc_service);
        } else {
            // When we have duplicate objects, we select an object deterministically
            // so that `grcache` replicas behave identically.
            let object_ref = objects
                .iter()
                .min_by_key(|v| (&v.namespace, &v.name))
                .unwrap();

            let object = &self.raw_services[object_ref];

            // Having more than one Service object per GrcacheService object is an
            // error. We behave gracefully but complain.
            if objects.len() > 1 {
                log::error!(
                    "Duplicate service resource for gRPC service {}! Make sure each gRPC service only has a single `GrcacheService` object. Selected: {} Objects: {:?}",
                    grpc_service,
                    object_ref.name,
                    objects.iter().map(|v| v.name.clone()).collect::<Vec<_>>()
                );
            }

            let generation = self.spec_generation;
            self.spec_generation += 1;

            // TODO spec generation
            // Right now this is racy

            let mut services = Vec::new();
            //for service_name in object.spec.service_names.iter() {
            let service_name = &object.spec.service_name;
            // TODO error
            let service = QualifiedService::parse(&service_name).unwrap();
            services.push((service_name.to_owned(), service));

            let load_balancer = match &object.spec.upstream {
                grcache_shared::config::crd::Upstream::Dns { url, port } => {
                    // Since we create the new handle before we remove
                    // the old entry from the service map, this only
                    // has the cost of reference counting for the case
                    // where the hostname is identical.
                    self.dns_service_handle
                        .backends_for_hostname(url.clone(), *port)
                }
            };

            if let Some(source) = object.spec.descriptor_set_source.clone() {
                let config = self.config.clone();

                tokio::spawn(async move {
                    // TODO error
                    let descriptor_set = descriptor_set::from_source(&DummyPanicContext, &source)
                        .await
                        .unwrap();

                    for (service_name, service) in services.iter() {
                        // TODO error
                        let (spec, _validation_errors) =
                            ServiceSpec::build(&descriptor_set, &service).unwrap();
                        let spec = Arc::new(spec);

                        let service_data = ServiceData {
                            generation,
                            service_spec: Some(spec.clone()),
                            load_balancer: Some(load_balancer.clone()),
                        };

                        config.update_service_if_newer(service_name.to_owned(), service_data);
                    }

                    // TODO mark as ready
                });
            } else {
                for (_service_name, service) in services.iter() {
                    let spec = ServiceSpec::build_passthrough(service);
                    let spec = Arc::new(spec);

                    let service_data = ServiceData {
                        generation,
                        service_spec: Some(spec.clone()),
                        load_balancer: Some(load_balancer.clone()),
                    };

                    self.config
                        .update_service_if_newer(service_name.to_owned(), service_data);
                }

                // TODO mark as ready
            }
        }
    }
}

#[async_trait::async_trait]
impl BackgroundService for K8SConfigService {
    async fn start(&self, _shutdown: ShutdownWatch) {
        let k8s_client = Client::try_default()
            .await
            .expect("failed to create kubernetes client");

        let nodes: Api<GrcacheService> = Api::default_namespaced(k8s_client.clone());

        // TODO pull from config
        // TODO filter by cluster
        //let node_filter = watcher::Config::default().fields("spec.cluster=default");
        let node_filter = watcher::Config::default();

        let watcher = watcher(nodes, node_filter);
        let mut changes = pin!(resource_changes(watcher.default_backoff()));

        let mut state = ServiceConfigState {
            spec_generation: 0,
            config: self.config.clone(),
            raw_services: HashMap::new(),
            ref_by_grpc_service: BTreeMap::new(),
            dns_service_handle: self.dns_service_handle.clone(),
        };

        log::info!("loading initial service specs..");

        loop {
            select! {
                change = changes.next() => {
                    match change {
                        Some(Ok(ResourceChange::Ready)) => {
                            log::info!("k8s service spec watch caught up");

                            // TODO also wait for service discovery
                            let _ = self.ready_sender.send(true);
                        },
                        Some(Ok(ResourceChange::Apply(object))) => {
                            let object_ref = ObjectRef::from_obj(&object);
                            log::info!("k8s service spec watch apply: {:?}", object_ref.name);
                            state.apply(object);

                        },
                        Some(Ok(ResourceChange::Delete(object_ref))) => {
                            log::info!("k8s service spec watch delete: {:?}", object_ref.name);
                            state.remove(&object_ref);
                        },
                        Some(Err(err)) => {
                            log::error!("got error in service CRD watcher: {:?}, retrying..", err);
                        },
                        None => {
                            unreachable!("changes stream exited!");
                        },
                    }
                }
            }
        }
    }
}
