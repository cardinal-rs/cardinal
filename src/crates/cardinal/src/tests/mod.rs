pub mod http;

#[cfg(test)]
mod tests {
    use crate::tests::http::http::{create_server_with, Route, TestHttpServer};
    use crate::Cardinal;
    use cardinal_config::{CardinalConfigBuilder, Destination, ServerConfigBuilder};
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::sync::{Arc, Mutex, OnceLock};
    use tiny_http::{Method, Response};
    use tokio::runtime::{Handle, Runtime};
    use tokio::sync::OnceCell;

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

    fn create_cardinal_ins() -> Mutex<std::thread::JoinHandle<()>> {
        let cardinal = Cardinal::new(
            CardinalConfigBuilder::default()
                .server(
                    ServerConfigBuilder::default()
                        .address("127.0.0.1:1704".to_string())
                        .force_path_parameter(true)
                        .log_upstream_response(true)
                        .global_request_middleware(vec![])
                        .global_response_middleware(vec![])
                        .build()
                        .unwrap(),
                )
                .destinations(BTreeMap::from_iter(vec![
                    (
                        "posts".to_string(),
                        Destination {
                            name: "posts".to_string(),
                            url: "127.0.0.1:9995".to_string(),
                            health_check: None,
                            routes: vec![],
                            middleware: vec![],
                        },
                    ),
                    (
                        "auth".to_string(),
                        Destination {
                            name: "auth".to_string(),
                            url: "127.0.0.1:9992".to_string(),
                            health_check: None,
                            routes: vec![],
                            middleware: vec![],
                        },
                    ),
                ]))
                .plugins(vec![])
                .build()
                .unwrap(),
        );

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
                Route::new(Method::Post, "/post", move |mut request| {
                    let response = Response::from_string("Hello World");
                    let _ = request.respond(response).unwrap();
                }),
                Route::new(Method::Get, "/post", move |mut request| {
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
            vec![Route::new(Method::Post, "/current", move |mut request| {
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
    async fn servers_only() {
        let _servers = get_servers().await;
        tokio::time::sleep(std::time::Duration::from_millis(60000)).await;
    }

    #[tokio::test]
    async fn custom_route_allows_dynamic_handlers() {
        let _run_cardinal = run_cardinal();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _servers = get_servers().await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let response = ureq::post("http://127.0.0.1:1704/posts/post")
            .config()
            .timeout_send_request(Some(std::time::Duration::from_secs(1)))
            .build()
            .send_empty()
            .unwrap();

        println!("Hello");

        let status = response.status();
        let mut buffer = vec![];
        let _ = response.into_body().as_reader().read_to_end(&mut buffer);
        assert_eq!(status, 200);
        assert_eq!(String::from_utf8(buffer).unwrap(), "Hello World");
    }
}
