//! Shared HTTP/WebSocket ingress over Substrate's actor-scoped CONNECT API.
//! The two listeners expose complete services: native Hermes publicly, agentd
//! only on the private Core/Runner network. There is no per-agent route table.
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Method, Request, Response, StatusCode, header},
    response::IntoResponse,
};
use hyper_util::rt::TokioIo;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct Target {
    router: Arc<str>,
    atespace: Arc<str>,
    port: u16,
}

pub async fn serve(
    router: String,
    atespace: String,
    public: SocketAddr,
    private: SocketAddr,
    allowed_origins: Vec<String>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        public != private,
        "public and control listeners must be distinct"
    );
    // Validate the value once; no incoming header can choose another atespace.
    anyhow::ensure!(
        !atespace.is_empty()
            && atespace.len() <= 63
            && atespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "invalid atespace"
    );
    let origins = crate::hosted_hermes_caddy::validate_browser_origins(&allowed_origins)?
        .into_iter()
        .map(|origin| origin.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()?;
    let target = Target {
        router: router.into(),
        atespace: atespace.into(),
        port: 8642,
    };
    let control = Target {
        port: 8080,
        ..target.clone()
    };
    let public = TcpListener::bind(public).await?;
    let private = TcpListener::bind(private).await?;
    tokio::try_join!(
        axum::serve(public, public_router(target, origins)),
        axum::serve(private, Router::new().fallback(proxy).with_state(control)),
    )?;
    Ok(())
}

// The edge owns CORS, while Hermes still authenticates every actual request.
// Its localhost-only middleware runs behind auth and rejects browser preflights.
fn public_router(target: Target, origins: Vec<HeaderValue>) -> Router {
    Router::new()
        .fallback(proxy)
        .with_state(target)
        .layer(axum::middleware::map_response(
            |mut response: Response<Body>| async move {
                for name in [
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    header::ACCESS_CONTROL_EXPOSE_HEADERS,
                    header::ACCESS_CONTROL_MAX_AGE,
                ] {
                    response.headers_mut().remove(name);
                }
                response
            },
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .max_age(Duration::from_secs(300)),
        )
}

async fn proxy(State(target): State<Target>, mut request: Request<Body>) -> impl IntoResponse {
    let Some((runtime, path)) = request
        .uri()
        .path_and_query()
        .and_then(|p| p.as_str().strip_prefix("/runtimes/"))
        .and_then(|p| p.split_once('/'))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(actor) = super::launcher::SubstrateLauncher::actor_name(runtime) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let prefix = format!("/runtimes/{runtime}");
    let Ok(uri) = format!("/{path}").parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    request.headers_mut().remove("ate-target-actor");
    request.headers_mut().remove("proxy-authorization");
    request.headers_mut().remove("proxy-connection");
    request.headers_mut().insert(
        "x-forwarded-prefix",
        prefix.parse().expect("validated runtime id"),
    );
    // Each request gets its own actor-bound tunnel. A pooled connection must
    // never be reused for another actor, even when the router address matches.
    match tokio::time::timeout(Duration::from_secs(30), forward(target, actor, request)).await {
        Ok(Ok(response)) => response,
        _ => StatusCode::BAD_GATEWAY.into_response(),
    }
}

async fn forward(
    target: Target,
    actor: String,
    mut request: Request<Body>,
) -> anyhow::Result<Response<Body>> {
    let stream = TcpStream::connect(target.router.as_ref()).await?;
    let (mut tunnel, connection) =
        hyper::client::conn::http1::handshake::<_, Body>(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    let authority = format!("actor:{}", target.port);
    let response = tunnel
        .send_request(
            Request::builder()
                .method("CONNECT")
                .uri(&authority)
                .header("host", &authority)
                .header("ate-target-actor", format!("{}/{}", target.atespace, actor))
                .body(Body::empty())?,
        )
        .await?;
    anyhow::ensure!(response.status().is_success(), "actor tunnel unavailable");
    let stream = hyper::upgrade::on(response).await?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(stream).await?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    let downstream = hyper::upgrade::on(&mut request);
    let mut response = sender.send_request(request).await?;
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            if let (Ok(downstream), Ok(upstream)) = tokio::join!(downstream, upstream) {
                let _ = tokio::io::copy_bidirectional(
                    &mut TokioIo::new(downstream),
                    &mut TokioIo::new(upstream),
                )
                .await;
            }
        });
    }
    Ok(response.map(Body::new))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn head(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(stream.read_u8().await.unwrap());
            assert!(bytes.len() < 8192);
        }
        String::from_utf8(bytes).unwrap()
    }

    #[tokio::test]
    async fn browser_preflight_is_local_but_native_auth_and_origin_are_preserved() {
        use tower::ServiceExt;
        tokio::time::timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target = Target { router: listener.local_addr().unwrap().to_string().into(), atespace: "test-space".into(), port: 8642 };
            let app = public_router(target, vec![HeaderValue::from_static("https://dashboard.test")]);
            let backend = tokio::spawn(async move {
                for origin in ["https://dashboard.test", "https://untrusted.test"] {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    assert!(head(&mut stream).await.starts_with("CONNECT "));
                    stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
                    let request = head(&mut stream).await.to_ascii_lowercase();
                    assert!(request.starts_with("get /api/sessions http/1.1"));
                    assert!(request.contains(&format!("origin: {origin}\r\n")));
                    assert!(request.contains("authorization: bearer synthetic\r\n"));
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Credentials: true\r\n\r\n").await.unwrap();
                }
            });
            for origin in ["https://dashboard.test", "https://untrusted.test"] {
                for method in [Method::OPTIONS, Method::GET] {
                    let response = app.clone().oneshot(Request::builder().method(method.clone())
                        .uri("/runtimes/runtime_0123456789abcdefabcd/api/sessions")
                        .header(header::ORIGIN, origin).header(header::AUTHORIZATION, "Bearer synthetic")
                        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                        .body(Body::empty()).unwrap()).await.unwrap();
                    assert_eq!(response.status(), if method == Method::OPTIONS { StatusCode::OK } else { StatusCode::UNAUTHORIZED });
                    assert_eq!(response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).and_then(|h| h.to_str().ok()), (origin == "https://dashboard.test").then_some(origin));
                    assert!(!response.headers().contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
                }
            }
            backend.await.unwrap();
        }).await.unwrap();
    }

    #[tokio::test]
    async fn actor_bound_connect_preserves_websocket_upgrade_and_both_directions() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let router = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target = Target { router: router.local_addr().unwrap().to_string().into(), atespace: "test-space".into(), port: 8642 };
            let backend = tokio::spawn(async move {
                let (mut stream, _) = router.accept().await.unwrap();
                let connect = head(&mut stream).await.to_ascii_lowercase();
                assert!(connect.starts_with("connect actor:8642 http/1.1\r\n"));
                assert!(connect.contains("ate-target-actor: test-space/runtime-0123456789abcdefabcd\r\n"));
                stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
                let request = head(&mut stream).await.to_ascii_lowercase();
                assert!(request.starts_with("get /api/ws?ticket=synthetic http/1.1\r\n"));
                assert!(request.contains("origin: https://dashboard.test\r\n"));
                assert!(request.contains("x-forwarded-prefix: /runtimes/runtime_0123456789abcdefabcd\r\n"));
                assert!(!request.contains("another-agent"));
                stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n\x81\x02ok").await.unwrap();
                let mut frame = [0; 8];
                stream.read_exact(&mut frame).await.unwrap();
                assert_eq!(&frame, b"\x81\x82\0\0\0\0hi");
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
            let proxy = tokio::spawn(async move { axum::serve(listener, Router::new().fallback(proxy).with_state(target)).await.unwrap(); });
            client.write_all(b"GET /runtimes/runtime_0123456789abcdefabcd/api/ws?ticket=synthetic HTTP/1.1\r\nHost: agents.test\r\nOrigin: https://dashboard.test\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nate-target-actor: test-space/another-agent\r\n\r\n").await.unwrap();
            assert!(head(&mut client).await.starts_with("HTTP/1.1 101"));
            let mut frame = [0; 4];
            client.read_exact(&mut frame).await.unwrap();
            assert_eq!(&frame, b"\x81\x02ok");
            client.write_all(b"\x81\x82\0\0\0\0hi").await.unwrap();
            backend.await.unwrap();
            proxy.abort();
        }).await.unwrap();
    }
}
