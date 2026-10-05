//! The optional Brain identity listener can never take a mandatory
//! listener's address or stop Core.

use super::*;
use axum::routing::get;

fn probe(body: &'static str) -> axum::Router {
    axum::Router::new().route("/probe", get(move || async move { body }))
}

async fn fetch(addr: SocketAddr) -> String {
    reqwest::get(format!("http://{addr}/probe"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

#[tokio::test]
async fn optional_listener_on_a_mandatory_address_is_disabled_and_core_keeps_serving() {
    let main = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let main_addr = main.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(main, probe("main")).await.unwrap() });
    let runtime = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let runtime_addr = runtime.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(runtime, probe("runtime")).await.unwrap() });
    let reserved = [main_addr, runtime_addr];

    for taken in [main_addr, runtime_addr] {
        let started =
            spawn_optional_listener("test", &taken.to_string(), &reserved, probe("optional")).await;
        assert_eq!(started, None, "{taken}");
    }
    assert_eq!(fetch(main_addr).await, "main");
    assert_eq!(fetch(runtime_addr).await, "runtime");

    // A port held by something else: bind fails, the feature is disabled.
    let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_addr = other.local_addr().unwrap();
    assert_eq!(
        spawn_optional_listener(
            "test",
            &other_addr.to_string(),
            &reserved,
            probe("optional")
        )
        .await,
        None
    );
    assert_eq!(
        spawn_optional_listener("test", "not-an-address", &reserved, probe("optional")).await,
        None
    );

    // A free private address starts and serves only its own router.
    let started = spawn_optional_listener("test", "127.0.0.1:0", &reserved, probe("optional"))
        .await
        .unwrap();
    assert_eq!(fetch(started).await, "optional");
    assert_eq!(fetch(main_addr).await, "main");
}
