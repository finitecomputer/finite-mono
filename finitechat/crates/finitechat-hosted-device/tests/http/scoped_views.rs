use super::*;
use serde_json::json;

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
