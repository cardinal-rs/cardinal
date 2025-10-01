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
    use cardinal_plugins::request_context::RequestContext;
    use cardinal_plugins::runner::{MiddlewareResult, RequestMiddleware, ResponseMiddleware};
    use cardinal_proxy::CardinalContextProvider;
    use cardinal_wasm_plugins::plugin::WasmPlugin;
    use cardinal_wasm_plugins::wasmer::AsStoreRef;
    use pingora::proxy::Session;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::Duration;
    use tiny_http::{Method, Response};
    use tokio::sync::OnceCell;
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
                                cardinal_wasm_plugins::ExecutionContext,
                            >,
                                  ptr: i32,
                                  len: i32|
                                  -> i32 {
                                let len = len.max(0);
                                signal.store(len, Ordering::SeqCst);

                                ctx.data_mut()
                                    .response_mut()
                                    .headers_mut()
                                    .insert("x-env-signal".into(), "from-host".into());

                                if let Some(memory) = ctx.data().memory() {
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
                                cardinal_wasm_plugins::ExecutionContext,
                            >,
                                  ptr: i32,
                                  len: i32|
                                  -> i32 {
                                let len = len.max(0);
                                signal.store(len, Ordering::SeqCst);
                                touched.store(true, Ordering::SeqCst);

                                ctx.data_mut()
                                    .response_mut()
                                    .headers_mut()
                                    .insert("x-env-signal".into(), "from-host".into());

                                if let Some(memory) = ctx.data().memory() {
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

    fn spawn_cardinal(cardinal: Cardinal) -> JoinHandle<()> {
        std::thread::spawn(move || {
            cardinal.run().unwrap();
        })
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
        fn resolve(&self, _session: &Session) -> Option<Arc<CardinalContext>> {
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
