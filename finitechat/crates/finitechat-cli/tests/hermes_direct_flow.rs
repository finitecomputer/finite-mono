//! Direct transport end to end: an agent with real MLS history folds it into
//! a direct room log, then serves the person's signed web requests, Hermes
//! and agentd over the sidecar's loopback API. Its own test binary, because
//! the transport is selected by process environment.

mod direct_support;

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Duration;

use finitechat_core::{
    AppAction, AppProfileSummary, AppView, FiniteChatRuntime, OpenOptions, room_digest,
};
use finitechat_proto::npub_encode;
use serde_json::{Value, json};

use direct_support::*;

#[test]
fn direct_transport_folds_mls_history_and_serves_web_hermes_and_agentd() {
    let dir = tempfile::tempdir().unwrap();
    let finite_home = dir.path().join("finite-home");
    // SAFETY: this test binary has one test; nothing else reads the env.
    unsafe {
        std::env::set_var("FINITE_HOME", &finite_home);
        std::env::set_var("FINITE_AGENT_ID", AGENT_ID);
    }
    let server_url = spawn_chat_server(&dir.path().join("server.sqlite3"));
    let agent_home = dir.path().join("agent").display().to_string();
    let now_arg = now().to_string();

    // Today's path: the agent and the person share an MLS room with topics.
    let init = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "init",
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--skip-agent-profile",
        "--json",
    ]);
    let agent_account = init["account_id"].as_str().unwrap().to_owned();
    let user = FiniteChatRuntime::open(OpenOptions {
        data_dir: dir.path().join("user").display().to_string(),
        server_url: server_url.clone(),
        device_id: "hosted-web".to_owned(),
        account_secret_hex: Some(hex_lower(&USER_SECRET)),
        now_unix_seconds: None,
    })
    .unwrap();
    let person = Person::new(USER_SECRET);
    assert_eq!(user.state().unwrap().identity.account_id, person.account());
    // As agent binding does today: the hosted device creates the room and
    // admits the agent, whose KeyPackages its resident start published.
    let agent_start = [
        "app",
        "--data-dir",
        &agent_home,
        "--server",
        &server_url,
        "--device-id",
        "agent",
        "--now",
        &now_arg,
        "start",
    ];
    cli_json(&agent_start);
    let room_id = user
        .dispatch_and_wait(AppAction::CreateRoom {
            display_name: "Direct Flow".to_owned(),
        })
        .unwrap()
        .selected_room_id
        .unwrap();
    user.dispatch_and_wait(AppAction::AddRoomMembers {
        room_id: room_id.clone(),
        profiles: vec![AppProfileSummary {
            account_id: agent_account.clone(),
            npub: npub_encode(&agent_account).unwrap(),
            display_name: "Agent".to_owned(),
            about: None,
            picture: None,
            stale: true,
            is_agent: true,
        }],
    })
    .unwrap();
    cli_json(&agent_start);
    let topic = user
        .dispatch_and_wait(AppAction::CreateTopic {
            room_id: room_id.clone(),
            title: "Trip planning".to_owned(),
        })
        .unwrap();
    let topic_id = topic.selected_topic_id.clone().unwrap();
    let chat_id = topic.selected_chat_id.clone().unwrap();
    user.dispatch_and_wait(AppAction::SendChatMessage {
        room_id: room_id.clone(),
        topic_id: topic_id.clone(),
        chat_id: chat_id.clone(),
        text: "history before direct".to_owned(),
        metadata_json: None,
    })
    .unwrap();
    user.dispatch_and_wait(AppAction::RenameChat {
        room_id: room_id.clone(),
        topic_id: topic_id.clone(),
        chat_id: chat_id.clone(),
        title: "Flights".to_owned(),
    })
    .unwrap();

    // The MLS sidecar already handed this history to Hermes; its inbox
    // cursor carries over, so the fold must not hand it over again.
    let request = json!({"timeout_millis": 2000}).to_string();
    let polled = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "--request-json",
        &request,
        "poll",
    ]);
    let delivered = polled["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["text"] == "history before direct")
        .unwrap_or_else(|| panic!("MLS poll did not deliver history: {polled}"))
        .clone();
    let ack = json!({
        "room_id": room_id, "seq": delivered["seq"], "message_id": delivered["message_id"],
    })
    .to_string();
    cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "--request-json",
        &ack,
        "ack",
    ]);

    // The MLS sidecar's delivery state, which a rolled-back image resumes.
    let mls_delivery_state = || {
        [
            "hermes-inbox.json",
            "agentd-inbox.json",
            "hermes-running.json",
        ]
        .map(|name| std::fs::read_to_string(PathBuf::from(&agent_home).join(name)).ok())
    };
    let before_direct = mls_delivery_state();
    assert!(
        before_direct[0].is_some(),
        "the MLS poll recorded its cursor"
    );

    // Switch the agent to direct transport: the first start folds.
    // SAFETY: as above.
    unsafe { std::env::set_var("FINITECHAT_CHAT_TRANSPORT", "direct") };
    let bridge = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-1.json")),
        http: reqwest::blocking::Client::new(),
    };

    let info: Value = bridge
        .http
        .get(format!("{}/v1/app/info", bridge.url))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(info["transport"], "direct");
    assert_eq!(info["migration"]["state"], "migrated");
    assert_eq!(info["canonical_room_id"], room_id);
    assert_eq!(info["agent_account_id"], agent_account);
    let readyz: Value = reqwest::blocking::get(format!("{}/readyz", bridge.url))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(readyz["transport"], "direct");

    // The hosted device and the folded log show the person the same room.
    let topics = user.state().unwrap().topics;
    let hosted_digest = room_digest(&room_id, &topics, |topic, chat| {
        user.state_for_view(AppView {
            room_id: Some(room_id.clone()),
            topic_id: Some(topic.to_owned()),
            chat_id: Some(chat.to_owned()),
            limit: Some(u32::MAX),
            oldest_message_id: None,
        })
        .unwrap()
        .messages
    });
    let folded_digest =
        bridge.json(bridge.get(&person, &format!("/v1/app/digest?room_id={room_id}")));
    assert_eq!(folded_digest, serde_json::to_value(&hosted_digest).unwrap());
    let fold_status: Value = serde_json::from_slice(
        &std::fs::read(PathBuf::from(&agent_home).join("direct/fold-status.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(fold_status["origin"], "fold");
    assert_eq!(fold_status["owner_account_id"], person.account());
    assert_eq!(
        fold_status["digests"][0], folded_digest,
        "operators can read the fold digest"
    );

    // The person sees the folded topic, renamed chat and message.
    let path = format!("/v1/app/state?room_id={room_id}&topic_id={topic_id}&chat_id={chat_id}");
    let state = bridge.json(bridge.get(&person, &path));
    assert_eq!(state["identity"]["account_id"], person.account());
    let trip = state["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|topic| topic["topic_id"] == topic_id)
        .unwrap();
    assert_eq!(trip["title"], "Trip planning");
    assert_eq!(trip["chats"][0]["title"], "Flights");
    assert_eq!(chat_messages(&state), vec!["history before direct"]);
    assert!(state["messages"][0]["is_mine"].as_bool().unwrap());

    // Authorization: missing, foreign, tampered and replayed requests fail.
    let anonymous = bridge
        .http
        .get(format!("{}{path}", bridge.url))
        .send()
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let stranger = Person::new(STRANGER_SECRET);
    assert_eq!(bridge.get(&stranger, &path).status(), 403);
    let signed = person.sign("GET", "/v1/app/state", None);
    let elsewhere = bridge
        .http
        .get(format!("{}{path}", bridge.url))
        .header("x-finite-chat-authorization", &signed)
        .send()
        .unwrap();
    assert_eq!(elsewhere.status(), 401, "a signature binds its exact path");
    let once = person.sign("GET", &path, None);
    for expected in [200, 401] {
        let response = bridge
            .http
            .get(format!("{}{path}", bridge.url))
            .header("x-finite-chat-authorization", &once)
            .send()
            .unwrap();
        assert_eq!(response.status(), expected, "a signature is single-use");
    }
    let body = serde_json::to_vec(&json!({"MarkRoomRead": {"room_id": room_id}})).unwrap();
    let tampered = bridge
        .http
        .post(format!("{}/v1/app/actions", bridge.url))
        .header(
            "x-finite-chat-authorization",
            person.sign("POST", "/v1/app/actions", Some(b"{}")),
        )
        .body(body)
        .send()
        .unwrap();
    assert_eq!(tampered.status(), 401, "a signature binds its body");

    // A web send reaches Hermes with the person's identity and route.
    let sent = bridge.json(bridge.post(
        &person,
        "/v1/app/actions",
        &json!({"SendChatMessage": {
            "room_id": room_id, "topic_id": topic_id, "chat_id": chat_id,
            "text": "hello direct", "metadata_json": null,
        }}),
    ));
    assert_eq!(chat_messages(&sent).last().unwrap(), "hello direct");
    let polled = bridge.loopback("/v1/hermes/poll", &json!({"timeout_millis": 5000}));
    let event = polled["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["text"] == "hello direct")
        .unwrap_or_else(|| panic!("Hermes did not receive the web message: {polled}"))
        .clone();
    assert_eq!(event["source"]["user_id"], person.account());
    assert_eq!(event["source"]["thread_id"], chat_id);
    assert!(
        polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["text"] != "history before direct"),
        "folded history is not replayed to Hermes"
    );
    bridge.loopback(
        "/v1/hermes/ack",
        &json!({"room_id": room_id, "seq": event["seq"], "message_id": event["message_id"]}),
    );

    // Hermes' reply, sent by thread id, lands in the person's chat.
    let reply = bridge.loopback(
        "/v1/hermes/send",
        &json!({
            "room_id": room_id, "conversation_id": null, "thread_id": chat_id,
            "text": "hello from Hermes", "kind": "message", "status": "complete",
            "attachments": [], "reply_to_message_id": null, "metadata": {},
        }),
    );
    assert!(reply["message_id"].as_str().unwrap().starts_with("direct-"));
    let after = bridge.json(bridge.get(&person, &path));
    let last = after["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(last["text"], "hello from Hermes");
    assert!(!last["is_mine"].as_bool().unwrap());

    // Organization actions behave as on the hosted device.
    let created = bridge.json(bridge.post(
        &person,
        "/v1/app/new-chat",
        &json!({"room_id": room_id, "topic_id": topic_id, "intent_key": "intent-1"}),
    ));
    let new_chat = created["selected_chat_id"].as_str().unwrap().to_owned();
    let again = bridge.json(bridge.post(
        &person,
        "/v1/app/new-chat",
        &json!({"room_id": room_id, "topic_id": topic_id, "intent_key": "intent-1"}),
    ));
    assert_eq!(again["selected_chat_id"], new_chat.as_str());
    let archived = bridge.json(bridge.post(
        &person,
        "/v1/app/actions",
        &json!({"SetChatArchived": {
            "room_id": room_id, "topic_id": topic_id, "chat_id": new_chat, "archived": true,
        }}),
    ));
    let archived_chat = archived["topics"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|topic| topic["chats"].as_array().unwrap())
        .find(|chat| chat["chat_id"] == new_chat.as_str())
        .unwrap();
    assert_eq!(archived_chat["archived"], true);

    // Attachments: upload, then download the same bytes.
    let boundary = "direct-flow-boundary";
    let mut multipart = Vec::new();
    for (name, value) in [
        ("room_id", room_id.as_str()),
        ("topic_id", &topic_id),
        ("chat_id", &chat_id),
        ("caption", "a photo"),
    ] {
        multipart.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    let photo = b"\x89PNG direct flow bytes".to_vec();
    multipart.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
    );
    multipart.extend_from_slice(&photo);
    multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let uploaded = bridge.json(
        bridge
            .http
            .post(format!("{}/v1/app/attachments", bridge.url))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(
                "x-finite-chat-authorization",
                person.sign("POST", "/v1/app/attachments", Some(&multipart)),
            )
            .body(multipart.clone())
            .send()
            .unwrap(),
    );
    let media_message = uploaded["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(media_message["text"], "a photo");
    let attachment_id = media_message["media"][0]["attachment_id"].as_str().unwrap();
    let download_path = format!(
        "/v1/app/attachments/{room_id}/{}/{}",
        media_message["message_id"].as_str().unwrap(),
        attachment_id
    );
    let download = bridge.get(&person, &download_path);
    assert_eq!(download.status(), 200);
    assert_eq!(download.headers()["x-content-type-options"], "nosniff");
    assert_eq!(download.bytes().unwrap().to_vec(), photo);
    let media_event = bridge.loopback("/v1/hermes/poll", &json!({"timeout_millis": 5000}));
    let media_event = media_event["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["text"] == "a photo")
        .unwrap()
        .clone();
    let local_path = media_event["attachments"][0]["path"].as_str().unwrap();
    assert_eq!(
        std::fs::read(local_path).unwrap(),
        photo,
        "Hermes reads the upload locally"
    );

    // A Connections command travels web → agentd → web over the direct log.
    let command_bridge = Bridge {
        url: bridge.url.clone(),
        http: reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap(),
    };
    let command_person = Person::new(USER_SECRET);
    let command_room = room_id.clone();
    let command_agent = agent_account.clone();
    let command = std::thread::spawn(move || {
        command_bridge.json(command_bridge.post(
            &command_person,
            "/v1/app/runtime-commands",
            &json!({
                "room_id": command_room, "target_account_id": command_agent,
                "command": "agent.connections.status", "schema": "finite.agent.empty.request.v1",
                "body": {}, "wait_millis": 20000,
            }),
        ))
    });
    let inbound = bridge
        .http
        .get(format!(
            "{}/v1/agentd/inbound?timeout_millis=10000",
            bridge.url
        ))
        .timeout(Duration::from_secs(20))
        .send()
        .unwrap();
    let mut delivery = Value::Null;
    for line in BufReader::new(inbound).lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        delivery = serde_json::from_str(&line).unwrap();
        break;
    }
    assert_eq!(delivery["sender"]["account_id"], person.account());
    let request = &delivery["payload"]["value"];
    assert_eq!(request["command"], "agent.connections.status");
    bridge.loopback(
        "/v1/agentd/result",
        &json!({
            "room_id": room_id, "conversation_id": null,
            "result": {
                "type": "runtime.command.result", "request_id": request["request_id"], "status": "succeeded",
                "body": {"schema": "finite.agent.connections.status.v1", "json_payload": serde_json::to_vec(&json!({"ok": true})).unwrap()},
                "error": null,
            },
        }),
    );
    bridge.loopback(
        "/v1/agentd/ack",
        &json!({"room_id": room_id, "seq": delivery["seq"], "message_id": delivery["message_id"]}),
    );
    let result = command.join().unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["body"], json!({"ok": true}));

    // Where the person was is durable: a view naming no room resumes it.
    bridge.json(bridge.post(
        &person,
        "/v1/app/actions",
        &json!({"OpenChat": {"room_id": room_id, "topic_id": topic_id, "chat_id": chat_id}}),
    ));

    // Digest is hashes only, and a second process sees the same durable log.
    let digest = bridge.json(bridge.get(&person, &format!("/v1/app/digest?room_id={room_id}")));
    assert!(!digest.to_string().contains("hello direct"));
    let restarted = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-2.json")),
        http: reqwest::blocking::Client::new(),
    };
    let restarted_digest =
        restarted.json(restarted.get(&person, &format!("/v1/app/digest?room_id={room_id}")));
    assert_eq!(restarted_digest, digest);
    let landing = restarted.json(restarted.get(&person, "/v1/app/state"));
    assert_eq!(landing["selected_room_id"], room_id.as_str());
    assert_eq!(landing["selected_topic_id"], topic_id.as_str());
    assert_eq!(landing["selected_chat_id"], chat_id.as_str());
    assert!(
        chat_messages(&landing)
            .iter()
            .any(|text| text == "hello from Hermes")
    );
    let mut sink = Vec::new();
    finitechat_cli::run(
        ["hermes", "--agent-home", &agent_home, "direct-digest"]
            .iter()
            .map(|arg| arg.to_string()),
        &mut sink,
    )
    .unwrap();
    let operator: Value = serde_json::from_slice(&sink).unwrap();
    assert_eq!(operator["rooms"][0], digest);

    // Rollback, then forward again: an older image runs MLS chat, so the
    // room moves on without the folded log. The next direct start refuses
    // the stale log and keeps serving MLS chat, saying why.
    user.dispatch_and_wait(AppAction::SendChatMessage {
        room_id: room_id.clone(),
        topic_id: topic_id.clone(),
        chat_id: chat_id.clone(),
        text: "sent while rolled back".to_owned(),
        metadata_json: None,
    })
    .unwrap();
    // Direct seqs run past the MLS cursor, so direct chat must not have
    // moved the MLS sidecar's cursors: the rolled-back sidecar would skip
    // every new MLS message.
    assert_eq!(
        mls_delivery_state(),
        before_direct,
        "direct chat leaves the MLS delivery state as it was"
    );
    // SAFETY: the running direct sidecars read the transport only at start.
    unsafe { std::env::set_var("FINITECHAT_CHAT_TRANSPORT", "mls") };
    let request = json!({"timeout_millis": 5000}).to_string();
    let polled = cli_json(&[
        "hermes",
        "--agent-home",
        &agent_home,
        "--request-json",
        &request,
        "poll",
    ]);
    assert!(
        polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["text"] == "sent while rolled back"),
        "the rolled-back sidecar hands new MLS messages to Hermes: {polled}"
    );
    unsafe { std::env::set_var("FINITECHAT_CHAT_TRANSPORT", "direct") };
    let refused = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-3.json")),
        http: reqwest::blocking::Client::new(),
    };
    let info = refused
        .http
        .get(format!("{}/v1/app/info", refused.url))
        .send()
        .unwrap();
    assert_eq!(info.status(), 404, "a stale direct log is not served");
    let readyz: Value = reqwest::blocking::get(format!("{}/readyz", refused.url))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(readyz["transport"], "mls");
    assert!(
        readyz["direct_fold"]["error"]
            .as_str()
            .unwrap()
            .contains("MLS chat advanced after the fold")
    );
    // Moving the log aside folds again, now with the rolled-back message.
    std::fs::rename(
        PathBuf::from(&agent_home).join("direct"),
        PathBuf::from(&agent_home).join("direct.rolled-back"),
    )
    .unwrap();
    let refolded = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-4.json")),
        http: reqwest::blocking::Client::new(),
    };
    let state = refolded.json(refolded.get(&person, &path));
    assert!(chat_messages(&state).contains(&"sent while rolled back".to_owned()));
}
