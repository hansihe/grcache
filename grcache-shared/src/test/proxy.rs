fn proxy_server() -> ProxyServerTestContext {
    let (s0, r0) = watch::channel(true);

    let service_config = Arc::new(ServiceConfigInner {
        ready: r0,
        services: papaya::HashMap::new(),
    });

    let cache = Box::leak(Box::new(MockStorage {}));

    let proxy = GrpcProxy {
        service_config: service_config.clone(),
        cache,
    };

    let conf = ServerConf::default();
    let mut http_proxy = http_proxy_service(&Arc::new(conf), proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    http_proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    http_proxy.add_tcp("[::1]:12345");

    let (s, r) = watch::channel(false);
    let proxy_handle = tokio::spawn(async move {
        http_proxy.start_service(None, r).await;
    });

    ProxyServerTestContext {
        ready: s0,
        shutdown: s,
        proxy_handle,
        service_config,
    }
}
