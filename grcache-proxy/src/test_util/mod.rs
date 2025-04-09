use std::{collections::BTreeSet, net::SocketAddr, sync::Arc};

use grcache_shared::{
    config::crd::DescriptorSetSource,
    service::{
        descriptor_set::{self, DummyPanicContext},
        qualified_service::QualifiedService,
        ServiceSpec,
    },
};
use pingora_load_balancing::Backend;
use proxy::{proxy_server, ProxyServerTestContext};
use tokio::sync::watch;

use crate::{discovery::ServiceBackendsHandle, service_store::ServiceData};

pub mod mock_storage;
pub mod proxy;

pub struct ProxyTest {
    proxy_ctx: ProxyServerTestContext,
}

impl ProxyTest {
    pub async fn new() -> Self {
        let server_test_ctx = proxy_server().await;

        ProxyTest {
            proxy_ctx: server_test_ctx,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.proxy_ctx.listener_addr
    }

    pub fn add_service_passthrough(&mut self, service_name: &str) -> BackendsTest {
        let (backends_handle, ready_sender, backends_sender) = ServiceBackendsHandle::new_test();
        ready_sender.send(true).unwrap();

        self.proxy_ctx.service_config.services.pin().insert(
            service_name.into(),
            ServiceData {
                generation: 10,
                service_spec: Some(Arc::new(ServiceSpec::build_passthrough(
                    &QualifiedService::parse(service_name).unwrap(),
                ))),
                load_balancer: Some(backends_handle.clone()),
            },
        );

        BackendsTest {
            handle: backends_handle,
            sender: backends_sender,
        }
    }

    pub async fn add_service(
        &mut self,
        service_name: &str,
        descriptor_set_path: &str,
    ) -> BackendsTest {
        let descriptor_set = descriptor_set::from_source(
            &DummyPanicContext,
            &DescriptorSetSource::File {
                path: descriptor_set_path.into(),
            },
        )
        .await
        .unwrap();
        let (service_spec, _validation_errors) = ServiceSpec::build(
            &descriptor_set,
            &QualifiedService::parse(service_name).unwrap(),
        )
        .unwrap();

        let (backends_handle, ready_sender, backends_sender) = ServiceBackendsHandle::new_test();
        ready_sender.send(true).unwrap();

        self.proxy_ctx.service_config.services.pin().insert(
            service_name.into(),
            ServiceData {
                generation: 10,
                service_spec: Some(Arc::new(service_spec)),
                load_balancer: Some(backends_handle.clone()),
            },
        );

        BackendsTest {
            handle: backends_handle,
            sender: backends_sender,
        }
    }

    pub async fn shutdown(self) {
        self.proxy_ctx.shutdown().await;
    }
}

pub struct BackendsTest {
    handle: Arc<ServiceBackendsHandle>,
    sender: watch::Sender<BTreeSet<Backend>>,
}

impl BackendsTest {
    pub async fn set_backends(&mut self, backends: BTreeSet<Backend>) {
        self.sender.send(backends).unwrap();
        self.handle.load_balancer_consistent.update().await.unwrap();
        self.handle
            .load_balancer_round_robin
            .update()
            .await
            .unwrap();
    }

    pub async fn set_single_backend_addr(&mut self, addr: SocketAddr) {
        let mut backends = BTreeSet::new();
        backends.insert(Backend::new(&format!("{}", addr)).unwrap());
        self.set_backends(backends).await;
    }
}
