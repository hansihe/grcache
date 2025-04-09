use std::sync::OnceLock;

use grcache_shared::config::TracingConfig;
use http::{HeaderMap, Request};
use opentelemetry::{
    global,
    propagation::{TextMapCompositePropagator, TextMapPropagator},
    Context,
};
use opentelemetry_datadog::DatadogPropagator;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider, Resource};

fn get_resource() -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| {
            Resource::builder()
                .with_service_name("grcache-proxy")
                .build()
        })
        .clone()
}

pub struct Tracing {
    tracer_provider: SdkTracerProvider,
}

impl Tracing {
    pub fn init(config: &TracingConfig) -> Self {
        let mut propagators: Vec<Box<dyn TextMapPropagator + Send + Sync + 'static>> = Vec::new();
        if config.opentelemetry_propagator {
            propagators.push(Box::new(TraceContextPropagator::new()));
        }
        if config.datadog_propagator {
            propagators.push(Box::new(DatadogPropagator::new()));
        }
        global::set_text_map_propagator(TextMapCompositePropagator::new(propagators));

        let exporter = SpanExporter::builder()
            .with_http()
            .build()
            .expect("Failed to create span exporter");
        let tracer_provider = SdkTracerProvider::builder()
            .with_resource(get_resource())
            .with_batch_exporter(exporter)
            .build();
        global::set_tracer_provider(tracer_provider.clone());

        Tracing { tracer_provider }
    }

    pub fn shutdown(self) {
        self.tracer_provider.shutdown().unwrap();
    }
}

pub fn extract_context_from_headers(headers: &HeaderMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}
