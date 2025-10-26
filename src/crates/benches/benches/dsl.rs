use std::io::Read;

use benches::support::http_support::Route;
use benches::support::http_url;
use benches::support::{
    config_with_destinations, destination_with_match, single_match, spawn_backend, spawn_cardinal,
    wait_for_startup,
};
use cardinal_config::{DestinationMatch, DestinationMatchValue};
use cardinal_rs::Cardinal;
use criterion::{criterion_group, criterion_main, Criterion};
use tiny_http::{Method, Response};

fn bench_routes_path_exact_before_prefix(c: &mut Criterion) {
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
        vec![exact_destination, prefix_destination],
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
    wait_for_startup();

    let url_exact = http_url(server_addr, "/status");
    let url_prefix = http_url(server_addr, "/status/health");
    let host = "status.example.com";

    let agent = ureq::Agent::new_with_defaults();

    let mut group = c.benchmark_group("dsl_simple");
    group.bench_function("routes_path_exact_before_prefix", |b| {
        b.iter(|| {
            let mut body = String::new();

            {
                let mut response = agent
                    .get(&url_exact)
                    .header("Host", host)
                    .call()
                    .expect("exact route response");
                body.clear();
                let body = response.body_mut().read_to_string().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(body, "exact");
            }

            {
                let mut response = agent
                    .get(&url_prefix)
                    .header("Host", host)
                    .call()
                    .expect("prefix route response");
                body.clear();
                let body = response.body_mut().read_to_string().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(body, "prefix");
            }
        });
    });
    group.finish();
}

fn bench_routes_regex_hosts_and_fallback(c: &mut Criterion) {
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
    wait_for_startup();

    let url_v1 = http_url(server_addr, "/v1/items");
    let url_v2 = http_url(server_addr, "/v2/items");
    let url_v3 = http_url(server_addr, "/v3/unknown");
    let host = "api.eu.example.com";

    let agent = ureq::Agent::new_with_defaults();

    let mut group = c.benchmark_group("dsl_heavy");
    group.bench_function("routes_regex_hosts_and_fallback", |b| {
        b.iter(|| {
            let mut body = String::new();

            {
                let mut response = agent
                    .get(&url_v1)
                    .header("Host", host)
                    .call()
                    .expect("v1 response");
                body.clear();
                let body = response.body_mut().read_to_string().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(body, "v1");
            }

            {
                let mut response = agent
                    .get(&url_v2)
                    .header("Host", host)
                    .call()
                    .expect("v2 response");
                body.clear();
                let body = response.body_mut().read_to_string().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(body, "v2");
            }

            {
                let mut response = agent
                    .get(&url_v3)
                    .header("Host", host)
                    .call()
                    .expect("fallback response");
                body.clear();
                let body = response.body_mut().read_to_string().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(body, "fallback");
            }
        });
    });
    group.finish();
}

criterion_group!(
    dsl,
    bench_routes_path_exact_before_prefix,
    bench_routes_regex_hosts_and_fallback
);
criterion_main!(dsl);
