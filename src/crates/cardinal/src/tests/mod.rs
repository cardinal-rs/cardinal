pub mod http;

#[cfg(test)]
mod tests {
    use crate::tests::http::http::{create_server_with, Route, TestHttpServer};
    use crate::Cardinal;
    use async_trait::async_trait;
    use cardinal_base::context::CardinalContext;
    use cardinal_base::destinations::container::DestinationWrapper;
    use cardinal_base::provider::ProviderScope;
    use cardinal_config::{
        load_config, CardinalConfig, Destination, DestinationMatch, DestinationMatchValue,
        DestinationRetry, DestinationRetryBackoffType, DestinationTimeouts, ServerConfig,
    };
    use cardinal_errors::CardinalError;
    use cardinal_plugins::container::{PluginBuiltInType, PluginContainer, PluginHandler};
    use cardinal_plugins::headers::CARDINAL_PARAMS_HEADER_BASE;
    use cardinal_plugins::plugin_executor::CardinalPluginExecutor;
    use cardinal_plugins::request_context::{RequestContext, RequestContextBase};
    use cardinal_plugins::runner::{MiddlewareResult, RequestMiddleware, ResponseMiddleware};
    use cardinal_proxy::context_provider::CardinalContextProvider;
    use cardinal_proxy::req::ReqCtx;
    use cardinal_wasm_plugins::plugin::WasmPlugin;
    use cardinal_wasm_plugins::wasmer::AsStoreRef;
    use pingora::proxy::Session;
    use std::collections::{BTreeMap, HashMap};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use tiny_http::{Method, Response};
    use tokio::sync::OnceCell;
    use ureq::http::{HeaderName, HeaderValue};
    use ureq::Error as UreqError;

    static SERVER: OnceLock<Mutex<std::thread::JoinHandle<()>>> = OnceLock::new();

    static TEST_HTTP_SERVER: OnceCell<Servers> = OnceCell::const_new();

    struct Servers {
        posts_api: TestHttpServer,
        auth_api: TestHttpServer,
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

    fn spawn_backend(address: impl Into<String>, routes: Vec<Route>) -> TestHttpServer {
        create_server_with(address.into(), routes)
    }

    fn config_with_destinations(
        server_addr: &str,
        force_path_parameter: bool,
        destinations: Vec<Destination>,
    ) -> CardinalConfig {
        let mut map = BTreeMap::new();
        for destination in destinations {
            map.insert(destination.name.clone(), destination);
        }

        CardinalConfig {
            server: ServerConfig {
                address: server_addr.to_string(),
                force_path_parameter,
                log_upstream_response: true,
                global_request_middleware: vec![],
                global_response_middleware: vec![],
            },
            destinations: map,
            plugins: vec![],
        }
    }

    fn retry_test_config(
        server_addr: &str,
        backend_addr: &str,
        retry: DestinationRetry,
    ) -> CardinalConfig {
        let mut destination = destination_with_match("retry", backend_addr, None, true);
        destination.retry = Some(retry);

        config_with_destinations(server_addr, true, vec![destination])
    }

    fn timeout_test_config(
        server_addr: &str,
        backend_addr: &str,
        timeouts: DestinationTimeouts,
    ) -> CardinalConfig {
        let mut destination = destination_with_match("timeout", backend_addr, None, true);
        destination.timeout = Some(timeouts);

        config_with_destinations(server_addr, true, vec![destination])
    }

    fn destination_with_match(
        name: &str,
        url: &str,
        matcher: Option<Vec<DestinationMatch>>,
        default: bool,
    ) -> Destination {
        Destination {
            name: name.to_string(),
            url: url.to_string(),
            health_check: None,
            default,
            r#match: matcher,
            routes: vec![],
            middleware: vec![],
            timeout: None,
            retry: None,
        }
    }

    fn single_match(matcher: DestinationMatch) -> Option<Vec<DestinationMatch>> {
        Some(vec![matcher])
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
        Ok(create_server_with(
            "127.0.0.1:9995".to_string(),
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
        Ok(create_server_with(
            "127.0.0.1:9992".to_string(),
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
    async fn plugin_executor_denies_global_request_middleware() {
        let config = load_test_config("plugin_executor_denies.toml");
        let builder = Cardinal::builder(config);
        let context = builder.context();
        let server_addr = context.config.server.address.clone();
        let backend_addr = context
            .config
            .destinations
            .get("posts")
            .expect("posts destination")
            .url
            .clone();

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("backend-ok");
                let _ = request.respond(response).unwrap();
            })],
        );

        let plugin_hits = Arc::new(AtomicUsize::new(0));
        let plugin_hits_for_container = plugin_hits.clone();

        let builder = builder.register_provider_with_factory::<PluginContainer, _>(
            ProviderScope::Singleton,
            move |_ctx| {
                const PLUGIN_NAME: &str = "ShortCircuitDenied";
                let mut container = PluginContainer::new_empty();
                container.add_plugin(
                    PLUGIN_NAME.to_string(),
                    PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                        TestRequestShortCircuitMiddleware {
                            hits: plugin_hits_for_container.clone(),
                        },
                    ))),
                );
                Ok(container)
            },
        );

        let can_run_calls = Arc::new(AtomicUsize::new(0));
        let plugin_executor: Arc<dyn CardinalPluginExecutor> =
            Arc::new(DenyingPluginExecutor::new(can_run_calls.clone()));
        let cardinal = builder.with_plugin_executor(plugin_executor).build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "backend-ok");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
        assert_eq!(plugin_hits.load(Ordering::SeqCst), 0);
        assert_eq!(can_run_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn request_middleware_headers_are_applied_to_response() {
        let mut config = load_test_config("wasm_request_status_short_circuit.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("request-middleware-ok");
                let _ = request.respond(response).unwrap();
            })],
        );

        let middleware_hits = Arc::new(AtomicUsize::new(0));
        let middleware_hits_clone = middleware_hits.clone();
        let plugin_name = "HeaderPropagatingRequest".to_string();
        config.server.global_request_middleware = vec![plugin_name.clone()];
        let plugin_name_clone = plugin_name.clone();

        let cardinal = cardinal_with_plugin_factory(config, move |container| {
            let mut headers = HashMap::new();
            headers.insert("x-request-middleware".to_string(), "propagated".to_string());

            container.add_plugin(
                plugin_name_clone.clone(),
                PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                    TestRequestHeaderMiddleware {
                        hits: middleware_hits_clone.clone(),
                        headers,
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
            .get("x-request-middleware")
            .and_then(|v| v.to_str().ok());
        assert_eq!(header_value, Some("propagated"));

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "request-middleware-ok");

        assert_eq!(middleware_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wasm_request_middleware_propagates_headers() {
        let config = load_test_config("wasm_request_header_set.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("wasm-request-ok");
                let _ = request.respond(response).unwrap();
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
        let decision = headers.get("x-decision").and_then(|v| v.to_str().ok());
        assert_eq!(decision, Some("allowed"));

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "wasm-request-ok");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wasm_request_middleware_shared_state_persists_between_middleware() {
        let config = load_test_config("wasm_request_shared_state.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("shared-state-ok");
                let _ = request.respond(response).unwrap();
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
        let shared_header = headers.get("x-shared-token").and_then(|v| v.to_str().ok());
        assert_eq!(shared_header, Some("alpha"));

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "shared-state-ok");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wasm_request_middleware_missing_shared_state_is_ignored() {
        let config = load_test_config("wasm_request_shared_state_missing.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("shared-state-missing-ok");
                let _ = request.respond(response).unwrap();
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
        assert!(headers
            .get("x-shared-token")
            .and_then(|v| v.to_str().ok())
            .is_none());

        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "shared-state-missing-ok");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn global_response_middleware_decorates_response() {
        let config = load_test_config("global_response_middleware.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");
        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
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
        let backend_hits = Arc::new(AtomicUsize::new(0));
        let middleware_hits = Arc::new(AtomicUsize::new(0));

        let config = load_test_config("destination_short_circuit.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
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
    async fn wasm_host_import_invokes_custom_function() {
        use cardinal_wasm_plugins::wasmer::{Function, FunctionEnvMut, Store};

        let config = load_test_config("wasm_host_import.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "host");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr,
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("host-import-ok");
                let _ = request.respond(response).unwrap();
            })],
        );

        let host_signal_value = Arc::new(AtomicI32::new(0));
        let host_signal_for_factory = host_signal_value.clone();
        let host_plugin_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/wasm-plugins/host-signal/plugin.wasm");

        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let wasm_plugin = WasmPlugin::from_path(&host_plugin_path).map_err(|e| {
                        CardinalError::Other(format!("Failed to load host plugin: {}", e))
                    })?;

                    let plugin_arc = Arc::new(wasm_plugin);
                    let mut container = PluginContainer::new();

                    let host_signal_for_fn = host_signal_for_factory.clone();
                    container.add_host_function("env", "host_signal", move |store, env| {
                        let signal = host_signal_for_fn.clone();
                        Function::new_typed_with_env(
                            store,
                            env,
                            move |mut ctx: FunctionEnvMut<
                                cardinal_wasm_plugins::SharedExecutionContext,
                            >,
                                  ptr: i32,
                                  len: i32|
                                  -> i32 {
                                let len = len.max(0);
                                signal.store(len, Ordering::SeqCst);

                                {
                                    let mut inner = ctx.data_mut().write();
                                    inner.response_mut().headers_mut().insert(
                                        HeaderName::from_static("x-env-signal"),
                                        HeaderValue::from_static("from-host"),
                                    );
                                }

                                if let Some(memory) = ctx.data().read().memory() {
                                    let store_ref = ctx.as_store_ref();
                                    let view = memory.view(&store_ref);
                                    let sentinel = [0xAA, 0xBB, 0xCC, 0xDD];
                                    let len = len.min(sentinel.len() as i32) as usize;
                                    if len > 0 {
                                        let _ = view.write(ptr as u64, &sentinel[..len]);
                                    }
                                }

                                0
                            },
                        )
                    });
                    container
                        .add_plugin("host_plugin".to_string(), PluginHandler::Wasm(plugin_arc));

                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/host/post"))
            .call()
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "host-import-ok");

        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
        assert_eq!(host_signal_value.load(Ordering::SeqCst), 4);
        assert_eq!(
            response
                .headers()
                .get("x-host-signal")
                .and_then(|v| v.to_str().ok()),
            Some("called")
        );
        assert_eq!(
            response
                .headers()
                .get("x-host-memory")
                .and_then(|v| v.to_str().ok()),
            Some("170")
        );
        assert_eq!(
            response
                .headers()
                .get("x-env-signal")
                .and_then(|v| v.to_str().ok()),
            Some("from-host")
        );
    }

    #[tokio::test]
    async fn wasm_host_import_can_mutate_env_memory() {
        use cardinal_wasm_plugins::wasmer::{Function, FunctionEnvMut, Store};

        let config = load_test_config("wasm_host_import_env.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "host");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr,
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("host-import-env");
                let _ = request.respond(response).unwrap();
            })],
        );

        let host_signal_value = Arc::new(AtomicI32::new(0));
        let env_touched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let host_signal_for_factory = host_signal_value.clone();
        let env_touched_factory = env_touched.clone();
        let host_plugin_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/wasm-plugins/host-signal/plugin.wasm");

        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let wasm_plugin = WasmPlugin::from_path(&host_plugin_path).map_err(|e| {
                        CardinalError::Other(format!("Failed to load host plugin: {}", e))
                    })?;

                    let plugin_arc = Arc::new(wasm_plugin);
                    let mut container = PluginContainer::new();

                    let host_signal_for_fn = host_signal_for_factory.clone();
                    let env_touched_for_fn = env_touched_factory.clone();
                    container.add_host_function("env", "host_signal", move |store, env| {
                        let signal = host_signal_for_fn.clone();
                        let touched = env_touched_for_fn.clone();
                        Function::new_typed_with_env(
                            store,
                            env,
                            move |mut ctx: FunctionEnvMut<
                                cardinal_wasm_plugins::SharedExecutionContext,
                            >,
                                  ptr: i32,
                                  len: i32|
                                  -> i32 {
                                let len = len.max(0);
                                signal.store(len, Ordering::SeqCst);
                                touched.store(true, Ordering::SeqCst);

                                {
                                    let mut inner = ctx.data_mut().write();

                                    inner.response_mut().headers_mut().insert(
                                        HeaderName::from_static("x-env-signal"),
                                        HeaderValue::from_static("from-host"),
                                    );
                                }

                                if let Some(memory) = ctx.data().read().memory() {
                                    let store = ctx.as_store_ref();
                                    let view = memory.view(&store);
                                    let sentinel = [0xAA, 0xBB, 0xCC, 0xDD];
                                    let len = len.min(sentinel.len() as i32) as usize;
                                    if len > 0 {
                                        let _ = view.write(ptr as u64, &sentinel[..len]);
                                    }
                                }

                                0
                            },
                        )
                    });
                    container.add_plugin(
                        "host_plugin".to_string(),
                        PluginHandler::Wasm(plugin_arc.clone()),
                    );

                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(&server_addr, "/host/post"))
            .call()
            .unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
        assert_eq!(host_signal_value.load(Ordering::SeqCst), 4);
        assert!(env_touched.load(std::sync::atomic::Ordering::SeqCst));

        // Ensure headers from both the plugin and host import are present
        let headers = response.headers();
        assert_eq!(
            headers.get("x-host-signal").and_then(|v| v.to_str().ok()),
            Some("called")
        );
        assert_eq!(
            headers.get("x-host-memory").and_then(|v| v.to_str().ok()),
            Some("170")
        );
        assert_eq!(
            headers.get("x-env-signal").and_then(|v| v.to_str().ok()),
            Some("from-host")
        );
    }

    #[tokio::test]
    async fn wasm_response_plugin_injects_headers() {
        let config = load_test_config("wasm_response_plugin.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
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
        let config = load_test_config("wasm_inbound_header_set.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
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
        let config = load_test_config("wasm_inbound_header_skip.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
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

        let err = ureq::get(&http_url(&server_addr, "/posts/post")).call();

        println!("{:?}", err);

        let err = err.expect_err("expected backend rejection when plugin not triggered");

        expect_status(err, 403);

        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn request_middleware_status_override_short_circuits() {
        let mut config = load_test_config("global_request_middleware.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("should-not-hit");
                let _ = request.respond(response).unwrap();
            })],
        );

        let plugin_name = "WasmStatusShortCircuit".to_string();
        config.server.global_request_middleware = vec![plugin_name.clone()];
        let plugin_name_clone = plugin_name.clone();
        let wasm_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/wasm-plugins/inbound-tag/plugin.wasm");
        let wasm_path_clone = wasm_path.clone();

        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let wasm_plugin = WasmPlugin::from_path(&wasm_path_clone).map_err(|e| {
                        CardinalError::Other(format!(
                            "Failed to load wasm status middleware plugin: {}",
                            e
                        ))
                    })?;

                    let mut container = PluginContainer::new();
                    container.add_plugin(
                        plugin_name_clone.clone(),
                        PluginHandler::Wasm(Arc::new(wasm_plugin)),
                    );

                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let err = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .expect_err("expected request middleware to short circuit with status override");

        expect_status(err, 403);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wasm_outbound_plugin_adds_response_headers_for_client() {
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
        let config = load_test_config("wasm_outbound_header_skip.toml");
        let server_addr = config.server.address.clone();
        let backend_addr = destination_url(&config, "posts");

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = backend_hits.clone();

        let _backend_server = spawn_backend(
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

    #[tokio::test]
    async fn routes_path_exact_before_prefix() {
        let server_addr = "127.0.0.1:1840";
        let exact_backend_addr = "127.0.0.1:1841";
        let prefix_backend_addr = "127.0.0.1:1842";

        let exact_destination = destination_with_match(
            "status_exact",
            exact_backend_addr,
            single_match(DestinationMatch {
                host: Some(DestinationMatchValue::String("status.example.com".into())),
                path_prefix: None,
                path_exact: Some("/status".into()),
            }),
            false,
        );

        let prefix_destination = destination_with_match(
            "status_prefix",
            prefix_backend_addr,
            single_match(DestinationMatch {
                host: Some(DestinationMatchValue::String("status.example.com".into())),
                path_prefix: Some(DestinationMatchValue::String("/status".into())),
                path_exact: None,
            }),
            false,
        );

        let config = config_with_destinations(
            server_addr,
            false,
            vec![exact_destination.clone(), prefix_destination.clone()],
        );

        let _exact_backend = spawn_backend(
            exact_backend_addr,
            vec![Route::new(Method::Get, "/status", move |request| {
                let response = Response::from_string("exact");
                let _ = request.respond(response);
            })],
        );

        let _prefix_backend = spawn_backend(
            prefix_backend_addr,
            vec![Route::new(Method::Get, "/status/health", move |request| {
                let response = Response::from_string("prefix");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(server_addr, "/status"))
            .header("Host", "status.example.com")
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "exact");

        let mut response = ureq::get(&http_url(server_addr, "/status/health"))
            .header("Host", "status.example.com")
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "prefix");
    }

    #[tokio::test]
    async fn routes_regex_hosts_and_falls_back_to_default() {
        let server_addr = "127.0.0.1:1843";
        let v1_backend_addr = "127.0.0.1:1844";
        let v2_backend_addr = "127.0.0.1:1845";
        let fallback_backend_addr = "127.0.0.1:1846";

        let regex_match = |path: &str| DestinationMatch {
            host: Some(DestinationMatchValue::Regex {
                regex: "^api\\.(eu|us)\\.example\\.com$".into(),
            }),
            path_prefix: Some(DestinationMatchValue::String(path.into())),
            path_exact: None,
        };

        let config = config_with_destinations(
            server_addr,
            false,
            vec![
                destination_with_match(
                    "v1",
                    v1_backend_addr,
                    single_match(regex_match("/v1")),
                    false,
                ),
                destination_with_match(
                    "v2",
                    v2_backend_addr,
                    single_match(regex_match("/v2")),
                    false,
                ),
                destination_with_match("fallback", fallback_backend_addr, None, true),
            ],
        );

        let _v1_backend = spawn_backend(
            v1_backend_addr,
            vec![Route::new(Method::Get, "/v1/items", move |request| {
                let response = Response::from_string("v1");
                let _ = request.respond(response);
            })],
        );

        let _v2_backend = spawn_backend(
            v2_backend_addr,
            vec![Route::new(Method::Get, "/v2/items", move |request| {
                let response = Response::from_string("v2");
                let _ = request.respond(response);
            })],
        );

        let _fallback_backend = spawn_backend(
            fallback_backend_addr,
            vec![Route::new(Method::Get, "/v3/unknown", move |request| {
                let response = Response::from_string("fallback");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(server_addr, "/v2/items"))
            .header("Host", "api.eu.example.com")
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "v2");

        let mut response = ureq::get(&http_url(server_addr, "/v3/unknown"))
            .header("Host", "api.eu.example.com")
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "fallback");
    }

    #[tokio::test]
    async fn hostless_destinations_follow_configuration_order() {
        let server_addr = "127.0.0.1:1847";
        let first_backend_addr = "127.0.0.1:1848";
        let second_backend_addr = "127.0.0.1:1849";
        let fallback_backend_addr = "127.0.0.1:1850";

        let first_destination = destination_with_match(
            "reports_a_regex",
            first_backend_addr,
            single_match(DestinationMatch {
                host: None,
                path_prefix: Some(DestinationMatchValue::Regex {
                    regex: "^/reports/.*".into(),
                }),
                path_exact: None,
            }),
            false,
        );

        let second_destination = destination_with_match(
            "reports_b_prefix",
            second_backend_addr,
            single_match(DestinationMatch {
                host: None,
                path_prefix: Some(DestinationMatchValue::String("/reports".into())),
                path_exact: None,
            }),
            false,
        );

        let fallback_destination =
            destination_with_match("fallback", fallback_backend_addr, None, true);

        let config = config_with_destinations(
            server_addr,
            false,
            vec![first_destination, second_destination, fallback_destination],
        );

        let _first_backend = spawn_backend(
            first_backend_addr,
            vec![Route::new(Method::Get, "/reports/daily", move |request| {
                let response = Response::from_string("regex");
                let _ = request.respond(response);
            })],
        );

        let _second_backend = spawn_backend(
            second_backend_addr,
            vec![
                Route::new(Method::Get, "/reports", move |request| {
                    let response = Response::from_string("prefix-root");
                    let _ = request.respond(response);
                }),
                Route::new(Method::Get, "/reports/daily", move |request| {
                    let response = Response::from_string("prefix-daily");
                    let _ = request.respond(response);
                }),
            ],
        );

        let _fallback_backend = spawn_backend(
            fallback_backend_addr,
            vec![Route::new(Method::Get, "/other", move |request| {
                let response = Response::from_string("fallback");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(server_addr, "/reports/daily"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "regex");

        let mut response = ureq::get(&http_url(server_addr, "/reports"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "prefix-root");
    }

    #[tokio::test]
    async fn force_parameter_unknown_segment_uses_default() {
        let server_addr = "127.0.0.1:1851";
        let default_backend_addr = "127.0.0.1:1852";

        let default_destination =
            destination_with_match("fallback", default_backend_addr, None, true);

        let config = config_with_destinations(server_addr, true, vec![default_destination]);

        let _default_backend = spawn_backend(
            default_backend_addr,
            vec![Route::new(Method::Get, "/unknown/path", move |request| {
                let response = Response::from_string("default");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(server_addr, "/unknown/path"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "default");
    }

    #[tokio::test]
    async fn returns_404_when_no_default_matches() {
        let server_addr = "127.0.0.1:1853";
        let billing_backend_addr = "127.0.0.1:1854";
        let support_backend_addr = "127.0.0.1:1855";

        let billing_destination = destination_with_match(
            "billing",
            billing_backend_addr,
            single_match(DestinationMatch {
                host: Some(DestinationMatchValue::String("billing.example.com".into())),
                path_prefix: Some(DestinationMatchValue::String("/billing".into())),
                path_exact: None,
            }),
            false,
        );

        let support_destination = destination_with_match(
            "support",
            support_backend_addr,
            single_match(DestinationMatch {
                host: Some(DestinationMatchValue::String("support.example.com".into())),
                path_prefix: Some(DestinationMatchValue::String("/support".into())),
                path_exact: None,
            }),
            false,
        );

        let config = config_with_destinations(
            server_addr,
            false,
            vec![billing_destination, support_destination],
        );

        let _billing_backend = spawn_backend(
            billing_backend_addr,
            vec![Route::new(Method::Get, "/billing/orders", move |request| {
                let response = Response::from_string("billing");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let mut response = ureq::get(&http_url(server_addr, "/billing/orders"))
            .header("Host", "billing.example.com")
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "billing");

        let err = ureq::get(&http_url(server_addr, "/unknown"))
            .header("Host", "billing.example.com")
            .call()
            .expect_err("expected 404 when no destination matches");
        expect_status(err, 404);
    }

    #[tokio::test]
    async fn context_provider_resolve_allows_request() {
        let config = load_test_config("context_provider_missing.toml");
        let builder = Cardinal::builder(config);
        let context = builder.context();
        let server_addr = context.config.server.address.clone();
        let backend_addr = context
            .config
            .destinations
            .get("posts")
            .expect("posts destination")
            .url
            .clone();

        let resolves = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn CardinalContextProvider> = Arc::new(TestContextProvider::new(
            Some(context.clone()),
            resolves.clone(),
        ));
        let cardinal = builder.with_context_provider(provider.clone()).build();

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let header = request.headers().iter().find_map(|h| {
                    if h.field.equiv("x-allow") {
                        Some(h.value.as_str().to_string())
                    } else {
                        None
                    }
                });
                let value = header.unwrap_or_else(|| "missing".to_string());
                let response = Response::from_string(value);
                let _ = request.respond(response);
            })],
        );

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
        assert_eq!(resolves.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn context_provider_missing_returns_421() {
        let config = load_test_config("context_provider.toml");
        let builder = Cardinal::builder(config);
        let context = builder.context();
        let server_addr = context.config.server.address.clone();
        let backend_addr = context
            .config
            .destinations
            .get("posts")
            .expect("posts destination")
            .url
            .clone();

        let resolves = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn CardinalContextProvider> =
            Arc::new(TestContextProvider::new(None, resolves.clone()));
        let cardinal = builder.with_context_provider(provider.clone()).build();

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("should-not-hit");
                let _ = request.respond(response);
            })],
        );

        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let err = ureq::get(&http_url(&server_addr, "/posts/post"))
            .call()
            .expect_err("expected 421 when context provider returns None");

        expect_status(err, 421);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
        assert_eq!(resolves.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_exponential_backoff_eventually_succeeds() {
        let server_addr = "127.0.0.1:1950";
        let backend_addr = "127.0.0.1:9850";

        let config = retry_test_config(
            server_addr,
            backend_addr,
            DestinationRetry {
                max_attempts: 5,
                interval_ms: 100,
                backoff_type: DestinationRetryBackoffType::Exponential,
                max_interval: None,
            },
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_holder = Arc::new(Mutex::new(None));

        {
            let backend_addr = backend_addr.to_string();
            let backend_holder = backend_holder.clone();
            let backend_hits = backend_hits.clone();

            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1350));
                let route_hits = backend_hits.clone();

                let server = spawn_backend(
                    backend_addr,
                    vec![Route::new(Method::Get, "/resource", move |request| {
                        route_hits.fetch_add(1, Ordering::SeqCst);
                        let response = Response::from_string("retry-success");
                        let _ = request.respond(response).unwrap();
                    })],
                );

                *backend_holder.lock().unwrap() = Some(server);
            });
        }

        let start = Instant::now();
        let mut response = ureq::get(&http_url(server_addr, "/retry/resource"))
            .call()
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "retry-success");
        assert!(elapsed >= Duration::from_millis(1200));
        assert!(elapsed <= Duration::from_millis(2400));
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        {
            let mut guard = backend_holder.lock().unwrap();
            assert!(guard.is_some());
            guard.take();
        }
    }

    #[tokio::test]
    async fn retry_respects_max_attempts_and_fails_when_backend_unavailable() {
        let server_addr = "127.0.0.1:1951";
        let backend_addr = "127.0.0.1:9851";

        let config = retry_test_config(
            server_addr,
            backend_addr,
            DestinationRetry {
                max_attempts: 2,
                interval_ms: 150,
                backoff_type: DestinationRetryBackoffType::Exponential,
                max_interval: None,
            },
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let start = Instant::now();
        let result = ureq::get(&http_url(server_addr, "/retry/resource")).call();
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(140));
        assert!(elapsed <= Duration::from_millis(650));

        let err = result.expect_err("expected retry exhaustion error");
        assert!(
            matches!(
                err,
                UreqError::ConnectionFailed | UreqError::Io(_) | UreqError::StatusCode(502)
            ),
            "unexpected error variant: {err:?}"
        );
    }

    #[tokio::test]
    async fn retry_max_interval_caps_total_wait_time() {
        let server_addr = "127.0.0.1:1952";
        let backend_addr = "127.0.0.1:9852";

        let config = retry_test_config(
            server_addr,
            backend_addr,
            DestinationRetry {
                max_attempts: 5,
                interval_ms: 120,
                backoff_type: DestinationRetryBackoffType::Exponential,
                max_interval: Some(200),
            },
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_holder = Arc::new(Mutex::new(None));

        {
            let backend_addr = backend_addr.to_string();
            let backend_holder = backend_holder.clone();
            let backend_hits = backend_hits.clone();

            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(320));
                let route_hits = backend_hits.clone();

                let server = spawn_backend(
                    backend_addr,
                    vec![Route::new(Method::Get, "/resource", move |request| {
                        route_hits.fetch_add(1, Ordering::SeqCst);
                        let response = Response::from_string("retry-capped");
                        let _ = request.respond(response).unwrap();
                    })],
                );

                *backend_holder.lock().unwrap() = Some(server);
            });
        }

        let start = Instant::now();
        let mut response = ureq::get(&http_url(server_addr, "/retry/resource"))
            .call()
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "retry-capped");
        assert!(elapsed >= Duration::from_millis(320));
        assert!(elapsed <= Duration::from_millis(750));
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        {
            let mut guard = backend_holder.lock().unwrap();
            assert!(guard.is_some());
            guard.take();
        }
    }

    #[tokio::test]
    async fn retry_linear_backoff_eventually_succeeds() {
        let server_addr = "127.0.0.1:1953";
        let backend_addr = "127.0.0.1:9853";

        let config = retry_test_config(
            server_addr,
            backend_addr,
            DestinationRetry {
                max_attempts: 4,
                interval_ms: 60,
                backoff_type: DestinationRetryBackoffType::Linear,
                max_interval: None,
            },
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_holder = Arc::new(Mutex::new(None));

        {
            let backend_addr = backend_addr.to_string();
            let backend_holder = backend_holder.clone();
            let backend_hits = backend_hits.clone();

            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(260));
                let route_hits = backend_hits.clone();

                let server = spawn_backend(
                    backend_addr,
                    vec![Route::new(Method::Get, "/resource", move |request| {
                        route_hits.fetch_add(1, Ordering::SeqCst);
                        let response = Response::from_string("retry-linear");
                        let _ = request.respond(response).unwrap();
                    })],
                );

                *backend_holder.lock().unwrap() = Some(server);
            });
        }

        let start = Instant::now();
        let mut response = ureq::get(&http_url(server_addr, "/retry/resource"))
            .call()
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "retry-linear");
        assert!(elapsed >= Duration::from_millis(260));
        assert!(elapsed <= Duration::from_millis(800));
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        {
            let mut guard = backend_holder.lock().unwrap();
            assert!(guard.is_some());
            guard.take();
        }
    }

    #[tokio::test]
    async fn retry_without_backoff_retries_quickly() {
        let server_addr = "127.0.0.1:1954";
        let backend_addr = "127.0.0.1:9854";

        let config = retry_test_config(
            server_addr,
            backend_addr,
            DestinationRetry {
                max_attempts: 3,
                interval_ms: 80,
                backoff_type: DestinationRetryBackoffType::None,
                max_interval: None,
            },
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_holder = Arc::new(Mutex::new(None));

        {
            let backend_addr = backend_addr.to_string();
            let backend_holder = backend_holder.clone();
            let backend_hits = backend_hits.clone();

            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(130));
                let route_hits = backend_hits.clone();

                let server = spawn_backend(
                    backend_addr,
                    vec![Route::new(Method::Get, "/resource", move |request| {
                        route_hits.fetch_add(1, Ordering::SeqCst);
                        let response = Response::from_string("retry-none");
                        let _ = request.respond(response).unwrap();
                    })],
                );

                *backend_holder.lock().unwrap() = Some(server);
            });
        }

        let start = Instant::now();
        let mut response = ureq::get(&http_url(server_addr, "/retry/resource"))
            .call()
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "retry-none");
        assert!(elapsed >= Duration::from_millis(130));
        assert!(elapsed <= Duration::from_millis(500));
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);

        {
            let mut guard = backend_holder.lock().unwrap();
            assert!(guard.is_some());
            guard.take();
        }
    }

    #[tokio::test]
    async fn timeout_read_exceeded_returns_error() {
        let server_addr = "127.0.0.1:1960";
        let backend_addr = "127.0.0.1:9860";

        let config = timeout_test_config(
            server_addr,
            backend_addr,
            DestinationTimeouts {
                read: Some(150),
                connect: None,
                write: None,
                idle: None,
            },
        );

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr,
            vec![Route::new(Method::Get, "/resource", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(400));
                let response = Response::from_string("slow-response");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let start = Instant::now();
        let err = ureq::get(&http_url(server_addr, "/timeout/resource"))
            .call()
            .expect_err("expected upstream read timeout");
        let elapsed = start.elapsed();

        assert!(backend_hits.load(Ordering::SeqCst) >= 1);
        assert!(elapsed >= Duration::from_millis(120));
        assert!(elapsed <= Duration::from_millis(800));
        assert!(
            matches!(
                err,
                UreqError::StatusCode(504)
                    | UreqError::StatusCode(502)
                    | UreqError::ConnectionFailed
                    | UreqError::Io(_)
            ),
            "unexpected error variant: {err:?}"
        );
    }

    #[tokio::test]
    async fn timeout_read_within_limit_succeeds() {
        let server_addr = "127.0.0.1:1961";
        let backend_addr = "127.0.0.1:9861";

        let config = timeout_test_config(
            server_addr,
            backend_addr,
            DestinationTimeouts {
                read: Some(800),
                connect: None,
                write: None,
                idle: None,
            },
        );

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = spawn_backend(
            backend_addr,
            vec![Route::new(Method::Get, "/resource", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(200));
                let response = Response::from_string("timely-response");
                let _ = request.respond(response);
            })],
        );

        let cardinal = Cardinal::new(config);
        let _cardinal_thread = spawn_cardinal(cardinal);
        wait_for_startup().await;

        let start = Instant::now();
        let mut response = ureq::get(&http_url(server_addr, "/timeout/resource"))
            .call()
            .unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(200));
        assert!(elapsed < Duration::from_millis(600));
        assert_eq!(response.status(), 200);
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(body, "timely-response");
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
    }

    fn spawn_cardinal(cardinal: Cardinal) -> JoinHandle<()> {
        std::thread::spawn(move || {
            cardinal.run().unwrap();
        })
    }

    struct DenyingPluginExecutor {
        can_run_calls: Arc<AtomicUsize>,
    }

    impl DenyingPluginExecutor {
        fn new(can_run_calls: Arc<AtomicUsize>) -> Self {
            Self { can_run_calls }
        }
    }

    #[async_trait]
    impl CardinalPluginExecutor for DenyingPluginExecutor {
        async fn can_run_plugin(
            &self,
            _binding_id: &str,
            _session: &mut Session,
            _req_ctx: &mut RequestContext,
        ) -> Result<bool, pingora::BError> {
            self.can_run_calls.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }
    }

    struct TestContextProvider {
        context: Option<Arc<CardinalContext>>,
        resolve_count: Arc<AtomicUsize>,
    }

    impl TestContextProvider {
        fn new(context: Option<Arc<CardinalContext>>, resolve_count: Arc<AtomicUsize>) -> Self {
            Self {
                context,
                resolve_count,
            }
        }
    }

    impl CardinalContextProvider for TestContextProvider {
        fn resolve(&self, _session: &Session, ctx: &mut ReqCtx) -> Option<Arc<CardinalContext>> {
            self.resolve_count.fetch_add(1, Ordering::SeqCst);
            self.context.as_ref().map(Arc::clone)
        }
    }

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
            _backend: &mut RequestContext,
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
            _backend: &mut RequestContext,
            _cardinal: Arc<CardinalContext>,
        ) -> Result<MiddlewareResult, CardinalError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let _ = session.respond_error(418).await;
            Ok(MiddlewareResult::Responded)
        }
    }

    struct TestRequestHeaderMiddleware {
        hits: Arc<AtomicUsize>,
        headers: HashMap<String, String>,
    }

    #[async_trait]
    impl RequestMiddleware for TestRequestHeaderMiddleware {
        async fn on_request(
            &self,
            _session: &mut Session,
            _backend: &mut RequestContext,
            _cardinal: Arc<CardinalContext>,
        ) -> Result<MiddlewareResult, CardinalError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(MiddlewareResult::Continue(self.headers.clone()))
        }
    }
}
