#[cfg(test)]
pub mod http {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use tiny_http::{Header, Method, Response, Server, StatusCode};

    type RouteKey = (Method, String);
    type RouteHandler = Arc<dyn Fn(tiny_http::Request) + Send + Sync + 'static>;

    /// Lightweight HTTP server used for integration tests.
    pub struct TestHttpServer {
        address: String,
        server: Arc<Server>,
        worker: Option<JoinHandle<()>>,
    }

    impl TestHttpServer {
        /// Starts the testing server on a random local port with the provided routes.
        pub fn spawn_with_routes(server: String, routes: impl IntoIterator<Item = Route>) -> Self {
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

        /// Convenience helper for JSON responses.
        pub fn json(method: Method, path: impl Into<String>, body: impl Into<String>) -> Self {
            let body = Arc::new(body.into());
            Self::new(method, path, move |request| {
                let response =
                    Response::from_data(body.as_bytes().to_vec()).with_header(json_header());
                let _ = request.respond(response);
            })
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

    fn build_route_map(routes: impl IntoIterator<Item = Route>) -> HashMap<RouteKey, RouteHandler> {
        let mut map = HashMap::new();
        for route in routes {
            map.insert((route.method, route.path), route.handler);
        }
        map
    }

    fn default_routes() -> Vec<Route> {
        vec![
            Route::json(Method::Get, "/api", r#"{"endpoint":"api"}"#),
            Route::json(Method::Get, "/user", r#"{"endpoint":"user"}"#),
            Route::json(Method::Get, "/post", r#"{"endpoint":"post"}"#),
            Route::json(Method::Post, "/post", r#"{"endpoint":"post"}"#),
        ]
    }

    fn clean_path(path: &str) -> String {
        path.split('?').next().unwrap_or(path).to_string()
    }

    fn json_header() -> Header {
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
            .expect("failed to build header")
    }
}
