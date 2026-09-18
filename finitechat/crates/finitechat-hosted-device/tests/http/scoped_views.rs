use super::*;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

async fn view_response(device: axum::Router, user: &str, path: &str) -> axum::response::Response {
    device
        .oneshot(
            Request::get(path)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header(WORKOS_USER_HEADER, user)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn first_state(response: axum::response::Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame = String::from_utf8(first.to_vec()).unwrap();
    serde_json::from_str(
        frame
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scoped_views_read_and_reconnect_without_changing_the_shared_or_saved_selection() {
    let root = TempDir::new().unwrap();
    let (server_url, _, server_task) =
        spawn_chat_server(&root.path().join("server.sqlite3"), None).await;
    let config = HostedDeviceConfig {
        data_root: root.path().join("devices"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    };
    let device = app(config.clone());
    let first = action_for(
        device.clone(),
        "viewer",
        json!({"CreateRoom": {"display_name": "Two tabs"}}),
    )
    .await;
    let room = first["selected_room_id"].as_str().unwrap();
    let topic = first["selected_topic_id"].as_str().unwrap();
    let chat_a = first["selected_chat_id"].as_str().unwrap();
    let send = |chat: &str, text: &str| {
        json!({"SendChatMessage": {
            "room_id": room, "topic_id": topic, "chat_id": chat, "text": text, "metadata_json": null,
        }})
    };
    action_for(device.clone(), "viewer", send(chat_a, "History in A")).await;
    let second = action_for(
        device.clone(),
        "viewer",
        json!({"StartTopicChat": {
            "room_id": room, "topic_id": topic, "reason": null,
        }}),
    )
    .await;
    let chat_b = second["selected_chat_id"].as_str().unwrap();
    action_for(device.clone(), "viewer", send(chat_b, "History in B")).await;
    let before = state_for(device.clone(), "viewer").await;
    let query = format!("?room_id={room}&topic_id={topic}&chat_id={chat_a}&limit=50");
    let response = view_response(device.clone(), "viewer", &format!("/v1/app/state{query}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let view: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(view["selected_chat_id"], chat_a);
    assert_eq!(view["messages"].as_array().unwrap().len(), 1);
    assert_eq!(view["messages"][0]["text"], "History in A");
    assert_eq!(view["identity"]["account_secret_hex"], "");
    assert_eq!(
        state_for(device.clone(), "viewer").await,
        before,
        "reading A must not navigate the Device away from B or bump its revision"
    );

    for _ in 0..2 {
        let streamed = first_state(
            view_response(device.clone(), "viewer", &format!("/v1/app/updates{query}")).await,
        )
        .await;
        assert_eq!(streamed["selected_chat_id"], chat_a);
        assert_eq!(
            streamed["messages"], view["messages"],
            "each reconnect projects the subscriber's own transcript"
        );
    }
    // The same read is never authority to access another user's history.
    let other = view_response(
        device.clone(),
        "another-user",
        &format!("/v1/app/state{query}"),
    )
    .await;
    assert!(!other.status().is_success());
    let invalid = view_response(
        device.clone(),
        "viewer",
        &format!("/v1/app/state?room_id={room}&topic_id={topic}&chat_id=missing"),
    )
    .await;
    assert!(!invalid.status().is_success());
    assert_eq!(state_for(device.clone(), "viewer").await, before);

    // Paging one tab's transcript is also a read. It must not depend on B's
    // loaded window, and a reconnect must include messages added to A.
    for index in 0..51 {
        action_for(
            device.clone(),
            "viewer",
            send(chat_a, &format!("A update {index}")),
        )
        .await;
    }
    action_for(
        device.clone(),
        "viewer",
        json!({"OpenChat": {
            "room_id": room, "topic_id": topic, "chat_id": chat_b,
        }}),
    )
    .await;
    let baseline = state_for(device.clone(), "viewer").await;
    let window = first_state(
        view_response(device.clone(), "viewer", &format!("/v1/app/updates{query}")).await,
    )
    .await;
    assert_eq!(window["messages"].as_array().unwrap().len(), 50);
    assert_eq!(
        window["messages"].as_array().unwrap().last().unwrap()["text"],
        "A update 50"
    );
    assert_eq!(window["rooms"][0]["can_load_older"], true);
    let expanded = first_state(
        view_response(
            device.clone(),
            "viewer",
            &format!("/v1/app/updates{}", query.replace("limit=50", "limit=100")),
        )
        .await,
    )
    .await;
    assert_eq!(expanded["messages"].as_array().unwrap().len(), 52);
    assert_eq!(expanded["messages"][0]["text"], "History in A");
    assert_eq!(expanded["rooms"][0]["can_load_older"], false);
    assert_eq!(state_for(device.clone(), "viewer").await, baseline);

    // New reads must not change the durable cursor that unmodified clients
    // and a restarted Hosted Web Device consume.
    drop(device);
    let restarted = app(config);
    let saved = state_for(restarted.clone(), "viewer").await;
    assert_eq!(saved["selected_chat_id"], chat_b);
    let legacy = first_state(view_response(restarted, "viewer", "/v1/app/updates").await).await;
    assert_eq!(legacy["selected_chat_id"], chat_b);
    assert_eq!(legacy["messages"][0]["text"], "History in B");
    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unscoped_reads_stay_available_while_scoped_reads_wait_for_a_busy_runtime() {
    let root = TempDir::new().unwrap();
    let server = HttpServerState::from_sqlite_path(root.path().join("server.sqlite3")).unwrap();
    let pause_next_send = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(1));
    // Dropping this permit also releases the request if an assertion fails.
    let hold = Arc::clone(&release).acquire_owned().await.unwrap();
    let router = http_router(server).layer(axum::middleware::from_fn({
        let pause_next_send = Arc::clone(&pause_next_send);
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        move |request: Request<Body>, next: axum::middleware::Next| {
            let pause_next_send = Arc::clone(&pause_next_send);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            async move {
                if request.uri().path() == "/events"
                    && pause_next_send.swap(false, Ordering::SeqCst)
                {
                    entered.notify_one();
                    let _permit = release.acquire().await.unwrap();
                }
                next.run(request).await
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let server_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let device = app(HostedDeviceConfig {
        data_root: root.path().join("devices"),
        server_url,
        public_url: PUBLIC_SERVER_URL.to_owned(),
        api_token: TOKEN.to_owned(),
    });
    let initial = action_for(
        device.clone(),
        "viewer",
        json!({"CreateRoom": {"display_name": "Busy Device"}}),
    )
    .await;
    let room = initial["selected_room_id"].as_str().unwrap();
    let topic = initial["selected_topic_id"].as_str().unwrap();
    let chat_a = initial["selected_chat_id"].as_str().unwrap();
    let send = |chat: &str, text: &str| {
        json!({"SendChatMessage": {
            "room_id": room, "topic_id": topic, "chat_id": chat,
            "text": text, "metadata_json": null,
        }})
    };
    action_for(device.clone(), "viewer", send(chat_a, "Retained A")).await;
    let second = action_for(
        device.clone(),
        "viewer",
        json!({"StartTopicChat": {"room_id": room, "topic_id": topic, "reason": null}}),
    )
    .await;
    let chat_b = second["selected_chat_id"].as_str().unwrap();
    action_for(device.clone(), "viewer", send(chat_b, "Retained B")).await;
    let before = state_for(device.clone(), "viewer").await;

    // Exercise the real actor and encrypted send path; only the upstream
    // network response is held. No production test hook or fake actor.
    pause_next_send.store(true, Ordering::SeqCst);
    let sending = tokio::spawn(action_for(
        device.clone(),
        "viewer",
        send(chat_b, "B accepted after delay"),
    ));
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("the send must reach the held server request");
    let query_a = format!("?room_id={room}&topic_id={topic}&chat_id={chat_a}&limit=50");
    let query_b = format!("?room_id={room}&topic_id={topic}&chat_id={chat_b}&limit=50");
    let path_a = format!("/v1/app/state{query_a}");
    let path_b = format!("/v1/app/state{query_b}");
    let stream_path = format!("/v1/app/updates{query_a}");
    let mut read_a = Box::pin(view_response(device.clone(), "viewer", &path_a));
    let mut read_b = Box::pin(view_response(device.clone(), "viewer", &path_b));
    let mut stream_a = Box::pin(view_response(device.clone(), "viewer", &stream_path));
    let (a, b, stream) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(100), &mut read_a),
        tokio::time::timeout(Duration::from_millis(100), &mut read_b),
        tokio::time::timeout(Duration::from_millis(100), &mut stream_a),
    );
    assert!(
        a.is_err() && b.is_err() && stream.is_err(),
        "scoped reads must wait for a coherent projection"
    );
    assert!(!sending.is_finished());

    // Old clients can still read the last published snapshot while that
    // actor is busy and scoped reads are queued behind it.
    let legacy = tokio::time::timeout(Duration::from_secs(1), state_for(device.clone(), "viewer"))
        .await
        .expect("unscoped state must not wait behind the send or scoped reads");
    assert_eq!(legacy, before);
    let legacy_stream = tokio::time::timeout(Duration::from_secs(1), async {
        first_state(view_response(device.clone(), "viewer", "/v1/app/updates").await).await
    })
    .await
    .expect("the unscoped SSE baseline must not wait behind the send or scoped reads");
    assert_eq!(legacy_stream, before);

    drop(hold);
    let after_send = tokio::time::timeout(Duration::from_secs(3), sending)
        .await
        .unwrap()
        .unwrap();
    let (a, b, stream) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(read_a, read_b, stream_a)
    })
    .await
    .expect("queued views must complete after the send is released");
    async fn json_state(response: axum::response::Response) -> Value {
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    let a = json_state(a).await;
    let b = json_state(b).await;
    assert_eq!(a["selected_chat_id"], chat_a);
    assert_eq!(a["messages"].as_array().unwrap().len(), 1);
    assert_eq!(a["messages"][0]["text"], "Retained A");
    assert_eq!(b["selected_chat_id"], chat_b);
    assert_eq!(b["messages"].as_array().unwrap().len(), 2);
    assert_eq!(b["messages"][1]["text"], "B accepted after delay");
    let stream = first_state(stream).await;
    assert_eq!(stream["selected_chat_id"], chat_a);
    assert_eq!(stream["messages"], a["messages"]);
    assert_eq!(
        state_for(device.clone(), "viewer").await,
        after_send,
        "queued reads must not rewrite selection, messages, or revision"
    );

    // Draining the read queue must leave subsequent writes usable.
    let later = tokio::time::timeout(
        Duration::from_secs(3),
        action_for(
            device.clone(),
            "viewer",
            send(chat_a, "A after queued reads"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(later["selected_chat_id"], chat_b);
    let updated_a = json_state(view_response(device, "viewer", &path_a).await).await;
    assert_eq!(updated_a["messages"].as_array().unwrap().len(), 2);
    assert_eq!(updated_a["messages"][1]["text"], "A after queued reads");
    server_task.abort();
}
