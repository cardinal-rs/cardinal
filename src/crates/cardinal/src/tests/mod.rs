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
    use cardinal_plugins::runner::{MiddlewareResult, RequestMiddleware, ResponseMiddleware};
    use cardinal_plugins::utils::parse_query_string_multi;
    use cardinal_wasm_plugins::plugin::WasmPlugin;
    use cardinal_wasm_plugins::runner::WasmRunner;
    use cardinal_wasm_plugins::ExecutionContext;
    use pingora::proxy::Session;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
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

    fn get_running_folder() -> PathBuf {
        std::env::current_dir().unwrap_or(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }

    fn load_test_config(name: &str) -> CardinalConfig {
        let path = get_running_folder().join("src/tests/configs").join(name);

        let path_str = path.to_string_lossy().to_string();
        load_config(&[path_str]).expect("failed to load test config")
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
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _servers = get_servers().await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let mut response = ureq::post("http://127.0.0.1:1704/posts/post")
            .config()
            .timeout_send_request(Some(std::time::Duration::from_secs(1)))
            .build()
            .send_empty()
            .unwrap();

        println!("Hello");

        let status = response.status();
        let body = response.body_mut().read_to_string().unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "Hello World");
    }

    #[tokio::test]
    async fn global_request_middleware_executes_before_backend() {
        let handle = runtime_handle().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let request_hits = Arc::new(AtomicUsize::new(0));
        let backend_addr = "127.0.0.1:2905".to_string();
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = create_server_with(
            backend_addr.clone(),
            handle.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("middleware-ok");
                let _ = request.respond(response).unwrap();
            })],
        );

        let plugin_name = "TestGlobalRequest".to_string();
        let server_addr = "127.0.0.1:1805".to_string();
        let config = load_test_config("global_request_middleware.toml");

        let request_hits_clone = request_hits.clone();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let mut container = PluginContainer::new_empty();
                    container.add_plugin(
                        plugin_name_clone.clone(),
                        PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                            TestGlobalRequestMiddleware {
                                hits: request_hits_clone.clone(),
                            },
                        ))),
                    );
                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut response = ureq::get(&format!("http://{}/posts/post", server_addr))
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

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_addr = "127.0.0.1:2906".to_string();
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = create_server_with(
            backend_addr.clone(),
            handle.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("base-response");
                let _ = request.respond(response).unwrap();
            })],
        );

        let plugin_name = "TestGlobalResponse".to_string();
        let server_addr = "127.0.0.1:1806".to_string();
        let config = load_test_config("global_response_middleware.toml");

        let response_hits = Arc::new(AtomicUsize::new(0));
        let response_hits_clone = response_hits.clone();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let mut container = PluginContainer::new_empty();
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
                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut response = ureq::get(&format!("http://{}/posts/post", server_addr))
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
        let backend_addr = "127.0.0.1:2908".to_string();
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = create_server_with(
            backend_addr.clone(),
            handle.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("should-not-see");
                let _ = request.respond(response).unwrap();
            })],
        );

        let plugin_name = "ShortCircuitInbound".to_string();
        let server_addr = "127.0.0.1:1808".to_string();
        let config = load_test_config("destination_short_circuit.toml");

        let middleware_hits = Arc::new(AtomicUsize::new(0));
        let middleware_hits_clone = middleware_hits.clone();
        let plugin_name_clone = plugin_name.clone();
        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let mut container = PluginContainer::new_empty();
                    container.add_plugin(
                        plugin_name_clone.clone(),
                        PluginHandler::Builtin(PluginBuiltInType::Inbound(Arc::new(
                            TestRequestShortCircuitMiddleware {
                                hits: middleware_hits_clone.clone(),
                            },
                        ))),
                    );
                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        tokio::time::sleep(Duration::from_millis(500)).await;

        let err = ureq::get(&format!("http://{}/posts/post", server_addr))
            .call()
            .expect_err("expected short-circuit response");

        match err {
            UreqError::StatusCode(code) => {
                assert_eq!(code, 418);
            }
            _ => panic!("unexpected error variant"),
        };

        assert_eq!(middleware_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wasm_response_plugin_injects_headers() {
        let handle = runtime_handle().await;

        let backend_hits = Arc::new(AtomicUsize::new(0));
        let backend_addr = "127.0.0.1:2907".to_string();
        let backend_hits_clone = backend_hits.clone();
        let _backend_server = create_server_with(
            backend_addr.clone(),
            handle.clone(),
            vec![Route::new(Method::Get, "/post", move |request| {
                backend_hits_clone.fetch_add(1, Ordering::SeqCst);
                let response = Response::from_string("wasm-backend");
                let _ = request.respond(response).unwrap();
            })],
        );

        let wasm_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/wasm-plugins/allow/plugin.wasm");
        let wasm_plugin = Arc::new(
            WasmPlugin::from_path(&wasm_path)
                .unwrap_or_else(|e| panic!("failed to load wasm plugin {:?}: {}", wasm_path, e)),
        );

        let plugin_name = "TestWasmResponse".to_string();
        let server_addr = "127.0.0.1:1807".to_string();
        let config = load_test_config("wasm_response_plugin.toml");

        let response_hits = Arc::new(AtomicUsize::new(0));
        let response_hits_clone = response_hits.clone();
        let plugin_name_clone = plugin_name.clone();
        let wasm_plugin_clone = wasm_plugin.clone();
        let cardinal = Cardinal::builder(config)
            .register_provider_with_factory::<PluginContainer, _>(
                ProviderScope::Singleton,
                move |_ctx| {
                    let mut container = PluginContainer::new_empty();
                    container.add_plugin(
                        plugin_name_clone.clone(),
                        PluginHandler::Builtin(PluginBuiltInType::Outbound(Arc::new(
                            TestWasmResponseMiddleware::new(
                                wasm_plugin_clone.clone(),
                                response_hits_clone.clone(),
                            ),
                        ))),
                    );
                    Ok(container)
                },
            )
            .build();

        let _cardinal_thread = spawn_cardinal(cardinal);
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut response = ureq::get(&format!(
            "http://{}/posts/post?tenant=cardinal",
            server_addr
        ))
        .header("Authorization", "Bearer wasm")
        .call()
        .unwrap();

        assert_eq!(response.status(), 200);
        let headers = response.headers();
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

        assert_eq!(response_hits.load(Ordering::SeqCst), 1);
        assert_eq!(backend_hits.load(Ordering::SeqCst), 1);
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

    struct TestWasmResponseMiddleware {
        plugin: Arc<WasmPlugin>,
        hits: Arc<AtomicUsize>,
    }

    impl TestWasmResponseMiddleware {
        fn new(plugin: Arc<WasmPlugin>, hits: Arc<AtomicUsize>) -> Self {
            Self { plugin, hits }
        }
    }

    #[async_trait]
    impl ResponseMiddleware for TestWasmResponseMiddleware {
        async fn on_response(
            &self,
            session: &mut Session,
            _backend: Arc<DestinationWrapper>,
            response: &mut pingora::http::ResponseHeader,
            _cardinal: Arc<CardinalContext>,
        ) {
            self.hits.fetch_add(1, Ordering::SeqCst);

            let mut req_headers: HashMap<String, String> = HashMap::new();
            for (name, value) in session.req_header().headers.iter() {
                if let Ok(val) = value.to_str() {
                    req_headers.insert(name.to_string().to_ascii_lowercase(), val.to_string());
                }
            }

            let query_raw = session.req_header().uri.query().unwrap_or("");
            let query = parse_query_string_multi(query_raw)
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect::<HashMap<_, _>>();

            let exec_ctx = ExecutionContext {
                memory: None,
                req_headers,
                query,
                resp_headers: HashMap::new(),
                status: response.status.as_u16() as i32,
                body: None,
            };

            if let Ok(result) = WasmRunner::new(&self.plugin).run(exec_ctx) {
                let status = result.execution_context.status as u16;
                if status != response.status.as_u16() {
                    let _ = response.set_status(status);
                }

                let headers: Vec<(String, String)> = result
                    .execution_context
                    .resp_headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                for (name, value) in headers {
                    let _ = response.insert_header(name, value);
                }
            }
        }
    }
}
