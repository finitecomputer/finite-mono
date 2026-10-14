//! Direct transport for an agent created after the cutover: no MLS history,
//! so the first start creates the room and its Home topic. The owner is the
//! one Core names; with none named, nobody can claim the chat. Its own test
//! binary, because the transport is selected by process environment.

mod direct_support;

use serde_json::{Value, json};

use direct_support::*;

fn init_agent(agent_home: &str, server_url: &str) -> String {
    cli_json(&[
        "hermes",
        "--agent-home",
        agent_home,
        "init",
        "--server",
        server_url,
        "--device-id",
        "agent",
        "--skip-agent-profile",
        "--json",
    ])["account_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn info(bridge: &Bridge) -> Value {
    bridge
        .http
        .get(format!("{}/v1/app/info", bridge.url))
        .send()
        .unwrap()
        .json()
        .unwrap()
}

#[test]
fn a_new_agent_starts_direct_with_one_home_topic_for_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let person = Person::new(USER_SECRET);
    let stranger = Person::new(STRANGER_SECRET);
    // SAFETY: this test binary has one test and sets the environment before
    // starting any sidecar thread.
    unsafe {
        std::env::set_var("FINITE_HOME", dir.path().join("finite-home"));
        std::env::set_var("FINITE_AGENT_ID", AGENT_ID);
        std::env::set_var("FINITECHAT_CHAT_TRANSPORT", "direct");
        std::env::set_var("FINITECHAT_OWNER_NPUBS", person.account());
        std::env::set_var("FINITECHAT_HERMES_ROOM_NAME", "Fresh Agent");
    }
    let server_url = spawn_chat_server(&dir.path().join("server.sqlite3"));

    // Core named the owner at creation.
    let agent_home = dir.path().join("agent").display().to_string();
    let agent_account = init_agent(&agent_home, &server_url);
    let bridge = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-1.json")),
        http: reqwest::blocking::Client::new(),
    };
    let fresh = info(&bridge);
    assert_eq!(fresh["transport"], "direct");
    assert_eq!(fresh["migration"]["state"], "fresh");
    assert_eq!(fresh["agent_account_id"], agent_account);
    let room_id = fresh["canonical_room_id"].as_str().unwrap().to_owned();

    assert_eq!(bridge.get(&stranger, "/v1/app/state").status(), 403);
    let state = bridge.json(bridge.get(&person, &format!("/v1/app/state?room_id={room_id}")));
    assert_eq!(state["rooms"][0]["display_name"], "Fresh Agent");
    let topics = state["topics"].as_array().unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0]["topic_id"], "home");
    assert_eq!(topics[0]["title"], "Home");
    assert_eq!(state["selected_topic_id"], "home");

    let sent = bridge.json(bridge.post(
        &person,
        "/v1/app/actions",
        &json!({"SendMessage": {"room_id": room_id, "text": "first words", "metadata_json": null}}),
    ));
    assert_eq!(sent["selected_topic_id"], "home");
    let polled = bridge.loopback("/v1/hermes/poll", &json!({"timeout_millis": 5000}));
    let event = &polled["events"][0];
    assert_eq!(event["text"], "first words");
    assert_eq!(event["source"]["user_id"], person.account());

    // A restart keeps the same room and owner.
    let restarted = Bridge {
        url: spawn_sidecar(&agent_home, dir.path().join("ready-2.json")),
        http: reqwest::blocking::Client::new(),
    };
    assert_eq!(
        restarted.json(restarted.get(&person, &format!("/v1/app/state?room_id={room_id}")))["topics"]
            [0]["title"],
        "Home"
    );

    // Core named no owner: no signer can claim the chat, because code inside
    // the agent can reach the bridge and sign first with a key of its own.
    // SAFETY: as above; the first sidecar is not used again.
    unsafe { std::env::remove_var("FINITECHAT_OWNER_NPUBS") };
    let unclaimed_home = dir.path().join("unclaimed").display().to_string();
    init_agent(&unclaimed_home, &server_url);
    let unclaimed = Bridge {
        url: spawn_sidecar(&unclaimed_home, dir.path().join("ready-3.json")),
        http: reqwest::blocking::Client::new(),
    };
    for signer in [&stranger, &person] {
        let refused = unclaimed.get(signer, "/v1/app/state");
        assert_eq!(refused.status(), 403);
        assert!(refused.text().unwrap().contains("has no owner"));
    }
    assert_eq!(info(&unclaimed)["transport"], "direct");

    // Naming several possible owners lets exactly one of them claim it.
    // SAFETY: as above; earlier sidecars are not used again.
    unsafe {
        std::env::set_var(
            "FINITECHAT_OWNER_NPUBS",
            format!("{},{}", person.account(), "c".repeat(64)),
        )
    };
    let named_home = dir.path().join("named").display().to_string();
    init_agent(&named_home, &server_url);
    let named = Bridge {
        url: spawn_sidecar(&named_home, dir.path().join("ready-4.json")),
        http: reqwest::blocking::Client::new(),
    };
    assert_eq!(named.get(&stranger, "/v1/app/state").status(), 403);
    let claimed = named.json(named.get(&person, "/v1/app/state"));
    assert_eq!(claimed["identity"]["account_id"], person.account());
    assert_eq!(named.get(&person, "/v1/app/state").status(), 200);
}
