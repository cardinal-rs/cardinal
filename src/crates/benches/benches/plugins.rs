use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use benches::support::http_support::Route;
use benches::support::http_url;
use benches::support::{
    cardinal_with_plugin_factory, destination_url, load_test_config, spawn_backend, spawn_cardinal,
    wait_for_startup,
};
use cardinal_base::context::CardinalContext;
use cardinal_errors::CardinalError;
use cardinal_plugins::container::{PluginBuiltInType, PluginHandler};
use cardinal_plugins::request_context::RequestContext;
use cardinal_plugins::runner::{MiddlewareResult, RequestMiddleware};
use cardinal_rs::Cardinal;
use criterion::{criterion_group, criterion_main, Criterion};
use pingora::proxy::Session;
use tiny_http::{Method, Response};

fn bench_global_request_middleware_executes_before_backend(c: &mut Criterion) {
    let config = load_test_config("global_request_middleware_test.toml");
    let server_addr = config.server.address.clone();
    let backend_addr = destination_url(&config, "posts");

    let backend_hits = Arc::new(AtomicUsize::new(0));
    let request_hits = Arc::new(AtomicUsize::new(0));

    let backend_hits_clone = backend_hits.clone();
    let _backend_server = spawn_backend(
        backend_addr.clone(),
        vec![Route::new(Method::Get, "/post", move |request| {
            backend_hits_clone.fetch_add(1, Ordering::SeqCst);
            let response = Response::from_string("middleware-ok");
            let _ = request.respond(response);
        })],
    );

    let request_hits_clone = request_hits.clone();
    let plugin_name = "TestGlobalRequest".to_string();
    let plugin_name_for_container = plugin_name.clone();
    let cardinal = cardinal_with_plugin_factory(config, move |container| {
        container.add_plugin(
            plugin_name_for_container.clone(),
            PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                TestGlobalRequestMiddleware {
                    hits: request_hits_clone.clone(),
                },
            ))),
        );
    });

    let _cardinal_thread = spawn_cardinal(cardinal);
    wait_for_startup();

    let url = http_url(&server_addr, "/posts/post");

    let mut group = c.benchmark_group("plugins_simple");
    group.bench_function("global_request_middleware_executes_before_backend", |b| {
        b.iter(|| {
            let mut response = ureq::get(&url).call().expect("successful response");
            assert_eq!(response.status(), 200);
            let mut body = String::new();
            let body = response.body_mut().read_to_string().unwrap();
            assert_eq!(body, "middleware-ok");
        });
    });
    group.finish();

    assert!(backend_hits.load(Ordering::SeqCst) > 0);
    assert!(request_hits.load(Ordering::SeqCst) > 0);
}

fn bench_wasm_outbound_plugin_adds_response_headers(c: &mut Criterion) {
    let config = load_test_config("wasm_outbound_header_set.toml");
    let server_addr = config.server.address.clone();
    let backend_addr = destination_url(&config, "posts");

    let backend_hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = backend_hits.clone();

    let _backend_server = spawn_backend(
        backend_addr.clone(),
        vec![Route::new(Method::Get, "/post", move |request| {
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let response = Response::from_string("outbound-ok");
            let _ = request.respond(response);
        })],
    );

    let cardinal = Cardinal::new(config);
    let _cardinal_thread = spawn_cardinal(cardinal);
    wait_for_startup();

    let url = http_url(&server_addr, "/posts/post");

    let mut group = c.benchmark_group("plugins_heavy");
    group.bench_function("wasm_outbound_plugin_adds_response_headers", |b| {
        b.iter(|| {
            let mut response = ureq::get(&url)
                .header("x-set-response", "true")
                .call()
                .expect("outbound plugin response");
            assert_eq!(response.status(), 201);
            let tag = response.headers().get("x-wasm-response").map(|s| s.to_str().unwrap());
            assert_eq!(tag, Some("enabled"));
            let mut body = String::new();
            let body = response.body_mut().read_to_string().unwrap();
            assert_eq!(body, "outbound-ok");
        });
    });
    group.finish();

    assert!(backend_hits.load(Ordering::SeqCst) > 0);
}

#[derive(Clone)]
struct TestGlobalRequestMiddleware {
    hits: Arc<AtomicUsize>,
}

#[async_trait]
impl RequestMiddleware for TestGlobalRequestMiddleware {
    async fn on_request(
        &self,
        _session: &mut Session,
        _backend: &mut RequestContext,
        _cardinal: Arc<CardinalContext>,
    ) -> Result<MiddlewareResult, CardinalError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Ok(MiddlewareResult::Continue(HashMap::new()))
    }
}

criterion_group!(
    plugins,
    bench_global_request_middleware_executes_before_backend,
    bench_wasm_outbound_plugin_adds_response_headers
);
criterion_main!(plugins);
