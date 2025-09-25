pub mod http;

#[cfg(test)]
mod tests {
    use crate::tests::http::http::{create_server_with, Route, TestHttpServer};
    use crate::Cardinal;
    use async_trait::async_trait;
    use cardinal_base::context::CardinalContext;
    use cardinal_base::destinations::container::DestinationWrapper;
    use cardinal_base::provider::ProviderScope;
    use cardinal_config::{load_config, CardinalConfig};
    use cardinal_errors::CardinalError;
    use cardinal_plugins::container::{PluginBuiltInType, PluginContainer, PluginHandler};
    use cardinal_plugins::headers::CARDINAL_PARAMS_HEADER_BASE;
    use cardinal_plugins::runner::{MiddlewareResult, RequestMiddleware, ResponseMiddleware};
    use cardinal_plugins::utils::parse_query_string_multi;
    use cardinal_wasm_plugins::plugin::WasmPlugin;
    use cardinal_wasm_plugins::runner::WasmRunner;
    use cardinal_wasm_plugins::ExecutionContext;
    use pingora::proxy::Session;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::Duration;
    use tiny_http::{Method, Response};
    use tokio::runtime::{Handle, Runtime};
    use tokio::sync::OnceCell;
    use ureq::Error as UreqError;

    static GLOBAL_RT: OnceCell<Runtime> = OnceCell::const_new();
    static SERVER: OnceLock<Mutex<std::thread::JoinHandle<()>>> = OnceLock::new();

    static TEST_HTTP_SERVER: OnceCell<Servers> = OnceCell::const_new();

    struct Servers {
        posts_api: TestHttpServer,
        auth_api: TestHttpServer,
    }

    async fn runtime_handle() -> Handle {
        GLOBAL_RT
            .get_or_try_init(|| async {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
            })
            .await
            .expect("failed to initialize test runtime")
            .handle()
            .clone()
    }

    pub fn run_cardinal() -> &'static Mutex<std::thread::JoinHandle<()>> {
        SERVER.get_or_init(create_cardinal_ins)
    }

    fn load_test_config(name: &str) -> CardinalConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/tests/configs")
            .join(name);
        let path_str = path.to_string_lossy().to_string();
        load_config(&[path_str]).expect("failed to load test config")
    }

    const STARTUP_WAIT_MS: u64 = 500;

    async fn wait_for_startup() {
        tokio::time::sleep(Duration::from_millis(STARTUP_WAIT_MS)).await;
    }

    fn http_url(address: &str, path: &str) -> String {
        format!("http://{}{}", address, path)
    }

    fn destination_url(config: &CardinalConfig, name: &str) -> String {
        config
            .destinations
            .get(name)
            .unwrap_or_else(|| panic!("missing destination {name}"))
            .url
            .clone()
    }

    fn spawn_backend(
        handle: &Handle,
        address: impl Into<String>,
        routes: Vec<Route>,
    ) -> TestHttpServer {
        create_server_with(address.into(), handle.clone(), routes)
    }

    fn cardinal_with_plugin_factory<F>(config: CardinalConfig, init: F) -> Cardinal
    where
        F: Fn(&mut PluginContainer) + Send + Sync + 'static,
    {
        let init = Arc::new(init);
        Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let init = Arc::clone(&init);
                    let mut container = PluginContainer::new_empty();
                    init(&mut container);
                    Ok(container)
                },
            )
            .build()
    }

    fn expect_status(err: UreqError, expected: u16) {
        match err {
            UreqError::StatusCode(code) => assert_eq!(code, expected),
            _ => panic!("unexpected error variant"),
        }
    }

    fn create_cardinal_ins() -> Mutex<std::thread::JoinHandle<()>> {
        let config = load_test_config("cardinal_default.toml");
        let cardinal = Cardinal::new(config);

        let handle = std::thread::spawn(move || {
            cardinal.run().unwrap();
        });

        Mutex::new(handle)
    }

    async fn create_posts_api() -> Result<TestHttpServer, ()> {
        let handle = runtime_handle().await;
        Ok(create_server_with(
            "127.0.0.1:9995".to_string(),
            handle.clone(),
            vec![
                Route::new(Method::Post, "/post", move |request| {
                    let response = Response::from_string("Hello World");
                    let _ = request.respond(response).unwrap();
                }),
                Route::new(Method::Get, "/post", move |request| {
                    let response = Response::from_string("Hello World");
                    let _ = request.respond(response).unwrap();
                }),
            ],
        ))
    }

    async fn create_auth_api() -> Result<TestHttpServer, ()> {
        let handle = runtime_handle().await;
        Ok(create_server_with(
            "127.0.0.1:9992".to_string(),
            handle.clone(),
            vec![Route::new(Method::Post, "/current", move |request| {
                let response = Response::from_string("Hello World");
                let _ = request.respond(response).unwrap();
            })],
        ))
    }

    async fn create_servers_collection() -> Result<Servers, ()> {
        Ok(Servers {
            posts_api: create_posts_api().await?,
            auth_api: create_auth_api().await?,
        })
    }

    async fn get_servers() -> &'static Servers {
        TEST_HTTP_SERVER
            .get_or_try_init(create_servers_collection)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_cardinal_default() {
        let _run_cardinal = run_cardinal();
        wait_for_startup().await;
        let _servers = get_servers().await;
        wait_for_startup().await;

        let mut response = ureq::post(&http_url("127.0.0.1:1704", "/posts/post"))
            .config()
            .timeout_send_request(Some(std::time::Duration::from_secs(1)))
            .build()
            .send_empty()
            .unwrap();

        let status = response.status();
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "Hello World");
    }

    #[tokio::test]
    async fn global_request_middleware_executes_before_backend() {
        let handle = runtime_handle().await;
        let config = load_test_config("global_request_middleware.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");
        let backend_hits = Arc::new(AtomicUsize::new(0));
        let request_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("middleware-ok");
                let _ = request.respond(response).unwrap();
            })],
        );

        let request_hits_clone = request_hits.clone();
        let plugin_name = "TestGlobalRequest".to_string();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = cardinal_with_plugin_factory(config, move |container| {
            container.add_plugin(
                plugin_name_clone.clone(),
                PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                    TestGlobalRequestMiddleware {
                        hits: request_hits_clone.clone(),
                    },
                ))),
            );
        });

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "middleware-ok");

        assert_eq!(request_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn global_response_middleware_decorates_response() {
        let handle = runtime_handle().await;
        let config = load_test_config("global_response_middleware.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");
        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("base-response");
                let _ = request.respond(response).unwrap();
            })],
        );

        let response_hits = Arc::new(AtomicUsize::new(0));
        let response_hits_clone = response_hits.clone();
        let plugin_name = "TestGlobalResponse".to_string();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = cardinal_with_plugin_factory(config, move |container| {
            container.add_plugin(
                plugin_name_clone.clone(),
                PluginHandler::Builtin(PluginBuiltInType::Outbound(Arc::new(
                    TestGlobalResponseMiddleware {
                        hits: response_hits_clone.clone(),
                        header_name: "x-global-response",
                        header_value: "applied",
                    },
                ))),
            );
        });

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let header_value = response
            .headers()
            .get("x-global-response")
            .and_then(|v| v.to_str().ok());
        assert_eq!(header_value, Some("applied"));
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "base-response");

        assert_eq!(response_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn destination_request_middleware_can_short_circuit() {
        let handle = runtime_handle().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let middleware_hits = Arc::new(AtomicUsize::new(0));

        let config = load_test_config("destination_short_circuit.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("should-not-see");
                let _ = request.respond(response).unwrap();
            })],
        );

        let middleware_hits_clone = middleware_hits.clone();
        let plugin_name = "ShortCircuitInbound".to_string();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = cardinal_with_plugin_factory(config, move |container| {
            container.add_plugin(
                plugin_name_clone.clone(),
                PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                    TestRequestShortCircuitMiddleware {
                        hits: middleware_hits_clone.clone(),
                    },
                ))),
            );
        });

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let err = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .expect_err("expected short-circuit response");

        expect_status(err, 418);

        assert_eq!(middleware_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn restricted_route_middleware_enforces_routes_and_injects_params() {
        let handle = runtime_handle().await;

        let config = load_test_config("restricted_route_middleware.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = config
            .destinations
            .get("posts")
            .expect("missing posts destination")
            .url
            .clone();

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let header_value: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let backend_hits_clone = backend_hits.clone();
        let header_value_clone = header_value.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/123/detail", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let expected_header = format!("{}id", CARDINAL_PARAMS_HEADER_BASE);
                let header = request.headers().iter().find_map(|h| {
                    let field = h.field.as_str().as_str();
                    if field.eq_ignore_ascii_case(expected_header.as_str()) {
                        Some(h.value.to_string())
                    } else {
                        None
                    }
                });
                *header_value_clone.lock().unwrap() = header;

                let response = Response::from_string("restricted-ok");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut allowed = ureq::get(&http_url(&server_addr, "/posts/123/detail"))
            .call()
            .unwrap();
        assert_eq!(allowed.status(), 200);
        let body = allowed.body_mut().read_to_string().unwrap();
        assert_eq!(body, "restricted-ok");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
        assert_eq!(header_value.lock().unwrap().as_deref(), Some("123"));

        let err = ureq::get(&http_url(&server_addr, "/posts/123"))
            .call()
            .expect_err("expected restricted route middleware to block request");

        expect_status(err, 402);

        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn restricted_route_middleware_blocks_unconfigured_route() {
        let handle = runtime_handle().await;

        let config = load_test_config("restricted_route_middleware_negative.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = config
            .destinations
            .get("posts")
            .expect("missing posts destination")
            .url
            .clone();

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/123", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("should-not-hit");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let err = ureq::get(&http_url(&server_addr, "/posts/123"))
            .call()
            .expect_err("expected restricted route middleware to block request");

        expect_status(err, 402);

        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wasm_response_plugin_injects_headers() {
        let handle = runtime_handle().await;

        let config = load_test_config("wasm_response_plugin.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("wasm-backend");
                let _ = request.respond(response).unwrap();
            })],
        );

        let cardinal = Cardinal::new(config);

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&format!(
            "{}?tenant=cardinal",
            http_url(&server_addr, "/posts/post")
        ))
        .header("Authorization", "Bearer wasm")
        .call();

        println!("{:?}", response);

        let mut response = response.unwrap();

        assert_eq!(response.status(), 200);
        let headers = response.headers();
        println!("{:?}", headers);
        let decision = headers
            .get("x-decision")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        let auth = headers
            .get("x-auth")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        let tenant = headers
            .get("x-tenant")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());

        assert_eq!(decision.as_deref(), Some("allow"));
        assert_eq!(auth.as_deref(), Some("Bearer wasm"));
        assert_eq!(tenant.as_deref(), Some("cardinal"));

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "wasm-backend");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wasm_inbound_plugin_allows_request_when_trigger_present() {
        let handle = runtime_handle().await;
        let config = load_test_config("wasm_inbound_header_set.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let header = request.headers().iter().find_map(|h| {
                    if h.field.equiv("x-allow") {
                        Some(h.value.as_str().to_string())
                    } else {
                        None
                    }
                });

                let response = if let Some(value) = header {
                    Response::from_string(value)
                } else {
                    Response::from_string("missing").with_status_code(500)
                };
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .header("x-allow", "true")
            .call()
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "true");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wasm_inbound_plugin_blocks_request_without_trigger() {
        let handle = runtime_handle().await;
        let config = load_test_config("wasm_inbound_header_skip.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("unexpected");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let err = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .expect_err("expected backend rejection when plugin not triggered");

        expect_status(err, 403);

        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wasm_outbound_plugin_adds_response_headers_for_client() {
        let handle = runtime_handle().await;
        let config = load_test_config("wasm_outbound_header_set.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("outbound-ok");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .header("x-set-response", "true")
            .call()
            .unwrap();

        assert_eq!(response.status(), 201);
        let headers = response.headers();
        let tag = headers
            .get("x-wasm-response")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        assert_eq!(tag.as_deref(), Some("enabled"));
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "outbound-ok");
    }

    #[tokio::test]
    async fn wasm_outbound_plugin_skips_headers_when_not_triggered() {
        let handle = runtime_handle().await;
        let config = load_test_config("wasm_outbound_header_skip.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
            &handle,
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("outbound-skip");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .unwrap();

        assert_eq!(response.status(), 200);
        let headers = response.headers();
        assert!(headers.get("x-wasm-response").is_none());
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "outbound-skip");
    }

    fn spawn_cardinal(cardinal: Cardinal) -> JoinHandle<()> {
        std::thread::spawn(move || {
            cardinal.run().unwrap();
        })
    }

    struct TestGlobalRequestMiddleware {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl RequestMiddleware for TestGlobalRequestMiddleware {
        async fn on_request(
            &self,
            _session: &mut Session,
            _backend: Arc<DestinationWrapper>,
            _cardinal: Arc<CardinalContext>,
        ) -> Result<MiddlewareResult, CardinalError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(MiddlewareResult::Continue)
        }
    }

    struct TestGlobalResponseMiddleware {
        hits: Arc<AtomicUsize>,
        header_name: &'static str,
        header_value: &'static str,
    }

    #[async_trait]
    impl ResponseMiddleware for TestGlobalResponseMiddleware {
        async fn on_response(
            &self,
            _session: &mut Session,
            _backend: Arc<DestinationWrapper>,
            response: &mut pingora::http::ResponseHeader,
            _cardinal: Arc<CardinalContext>,
        ) {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let _ = response.insert_header(self.header_name, self.header_value);
        }
    }

    struct TestRequestShortCircuitMiddleware {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl RequestMiddleware for TestRequestShortCircuitMiddleware {
        async fn on_request(
            &self,
            session: &mut Session,
            _backend: Arc<DestinationWrapper>,
            _cardinal: Arc<CardinalContext>,
        ) -> Result<MiddlewareResult, CardinalError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let _ = session.respond_error(418).await;
            Ok(MiddlewareResult::Responded)
        }
    }
}
