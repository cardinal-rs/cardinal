//! Shared benchmarking helpers for Cardinal.
//!
//! Provides lightweight copies of integration test utilities so Criterion benches can
//! re-use the same configuration and routing scenarios without depending on the test
//! module (which is private to the `cardinal-rs` crate).

pub mod support {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use cardinal_base::provider::ProviderScope;
    use cardinal_config::{
        load_config, CardinalConfig, Destination, DestinationMatch, ServerConfig,
    };
    use cardinal_plugins::container::PluginContainer;
    use cardinal_rs::Cardinal;

    pub use http_support::{create_server_with, Route, TestHttpServer};

    const STARTUP_WAIT_MS: u64 = 500;

    /// Simple blocking wait used after spinning up Cardinal and backend fixtures.
    pub fn wait_for_startup() {
        std::thread::sleep(Duration::from_millis(STARTUP_WAIT_MS));
    }

    /// Convenience helper to format an HTTP url from host and path.
    pub fn http_url(address: &str, path: &str) -> String {
        format!("http://{}{}", address, path)
    }

    /// Returns the configured destination url or panics if missing.
    pub fn destination_url(config: &CardinalConfig, name: &str) -> String {
        config
            .destinations
            .get(name)
            .unwrap_or_else(|| panic!("missing destination {name}"))
            .url
            .clone()
    }

    /// Spawns a backend server with the provided routes.
    pub fn spawn_backend(address: impl Into<String>, routes: Vec<Route>) -> TestHttpServer {
        create_server_with(address.into(), routes)
    }

    /// Creates a Cardinal configuration with the supplied destinations.
    pub fn config_with_destinations(
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

    /// Simple destination builder helper mirroring the integration tests.
    pub fn destination_with_match(
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

    /// Convenience function to wrap a single matcher in a vector.
    pub fn single_match(matcher: DestinationMatch) -> Option<Vec<DestinationMatch>> {
        Some(vec![matcher])
    }

    /// Loads a test configuration from the `cardinal` crate's fixtures.
    pub fn load_test_config(name: &str) -> CardinalConfig {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../cardinal/src/tests/configs")
            .join(name);
        let path_str = path.to_string_lossy().to_string();
        load_config(&[path_str]).expect("failed to load test config")
    }

    /// Spawns Cardinal on a background thread. The caller is expected to keep the returned
    /// handle alive for the lifetime of the benchmark.
    pub fn spawn_cardinal(cardinal: Cardinal) -> JoinHandle<()> {
        std::thread::spawn(move || {
            cardinal.run().expect("cardinal run failed");
        })
    }

    /// Helper that mirrors the integration test factory for plugin containers.
    pub fn cardinal_with_plugin_factory<F>(config: CardinalConfig, init: F) -> Cardinal
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

    /// Lightweight re-export of the HTTP server used in the integration tests.
    pub mod http_support {
        use std::collections::HashMap;
        use std::sync::Arc;
        use std::thread::{self, JoinHandle};

        use tiny_http::{Method, Response, Server, StatusCode};

        type RouteKey = (Method, String);
        type RouteHandler = Arc<dyn Fn(tiny_http::Request) + Send + Sync + 'static>;

        /// Lightweight HTTP server used for benchmarks.
        pub struct TestHttpServer {
            address: String,
            server: Arc<Server>,
            worker: Option<JoinHandle<()>>,
        }

        impl TestHttpServer {
            pub fn spawn_with_routes(
                server: String,
                routes: impl IntoIterator<Item = Route>,
            ) -> Self {
                let route_map = Arc::new(build_route_map(routes));
                let server = Arc::new(Server::http(server).expect("failed to start test server"));
                let address = server.server_addr().to_string();
                let worker = spawn_worker(server.clone(), route_map);

                Self {
                    address,
                    server,
                    worker: Some(worker),
                }
            }

            pub fn address(&self) -> &str {
                &self.address
            }
        }

        /// Starts a server with custom routes.
        pub fn create_server_with(
            server: String,
            routes: impl IntoIterator<Item = Route>,
        ) -> TestHttpServer {
            TestHttpServer::spawn_with_routes(server, routes)
        }

        impl Drop for TestHttpServer {
            fn drop(&mut self) {
                self.server.unblock();

                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
            }
        }

        /// Route registration helper used to populate the server.
        pub struct Route {
            method: Method,
            path: String,
            handler: RouteHandler,
        }

        impl Route {
            /// Registers a new route using the provided closure.
            pub fn new<F>(method: Method, path: impl Into<String>, handler: F) -> Self
            where
                F: Fn(tiny_http::Request) + Send + Sync + 'static,
            {
                let raw_path = path.into();
                Self {
                    method,
                    path: clean_path(&raw_path),
                    handler: Arc::new(handler),
                }
            }
        }

        fn spawn_worker(
            server: Arc<Server>,
            routes: Arc<HashMap<RouteKey, RouteHandler>>,
        ) -> JoinHandle<()> {
            thread::spawn(move || {
                for request in server.incoming_requests() {
                    let method = request.method().clone();
                    let url = request.url().to_string();
                    let key = (method, clean_path(&url));

                    if let Some(handler) = routes.get(&key).cloned() {
                        handler(request);
                        continue;
                    }

                    let _ = request.respond(Response::empty(StatusCode(404)));
                }
            })
        }

        fn build_route_map(
            routes: impl IntoIterator<Item = Route>,
        ) -> HashMap<RouteKey, RouteHandler> {
            let mut map = HashMap::new();
            for route in routes {
                map.insert((route.method, route.path), route.handler);
            }
            map
        }

        fn clean_path(path: &str) -> String {
            path.split('?').next().unwrap_or(path).to_string()
        }
    }

    /// Utility to resolve the project path for wasm fixtures.
    pub fn wasm_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/wasm-plugins")
            .join(name)
            .join("plugin.wasm")
    }
}

/// Placeholder to ensure the crate compiles until real helpers land.
pub fn ensure_initialized() {}
