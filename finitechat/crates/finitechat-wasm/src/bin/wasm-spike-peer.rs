#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use axum::{Json, routing::get};
    use finitechat_client::{
        AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery,
        ReqwestHttpRuntimeTransport, RuntimeSyncOptions, SqliteClientStore,
        SqliteClientStoreOptions, generate_account_secret, run_runtime_sync_tick,
    };
    use finitechat_proto::DurableAppEventKind;
    use finitechat_server::{HttpServerState, http_router};
    use nostr::ToBech32;
    use std::{
        net::SocketAddr,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tower_http::cors::CorsLayer;

    let state_dir = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!("usage: wasm-spike-peer EMPTY_STATE_DIR [PORT] [DASHBOARD_ORIGIN]")
    })?;
    let port: u16 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "28789".into())
        .parse()?;
    let dashboard_origin = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "http://127.0.0.1:13010".into());
    // Fresh identities + fresh durable agent store per run; never open user state.
    std::fs::create_dir(&state_dir)?;
    let (user_account_id, user_nsec) = {
        let user_secret = generate_account_secret()?;
        (
            hex::encode(user_secret.public_key().as_bytes()),
            nostr::SecretKey::from_slice(user_secret.as_bytes())?.to_bech32()?,
        )
    };
    let agent_secret = generate_account_secret()?;
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let config = FiniteChatDeviceConfig {
        account_secret_key: agent_secret.clone(),
        device_id: "native-spike-agent".into(),
        now_unix_seconds: now(),
        credential_not_before_unix_seconds: now() - 60,
        credential_not_after_unix_seconds: now() + 86400,
    };
    let mut device = FiniteChatDevice::new(config)?;
    let bootstrap = serde_json::json!({
        "nsec": user_nsec,
        "agentAccountId": device.device_ref().account_id,
        "serverUrl": format!("http://127.0.0.1:{port}"),
    });
    // Only the simulated Core bootstrap retains the User Key. The agent
    // worker receives its own key and the authorized user's public identity.
    let server_url = format!("http://127.0.0.1:{port}");
    let state = HttpServerState::from_sqlite_path(format!("{state_dir}/relay.sqlite3"))?
        .with_public_url(&server_url)?
        .with_require_signed_requests(true);
    let app = http_router(state)
        .route(
            "/spike/bootstrap",
            get(move || {
                let bootstrap = bootstrap.clone();
                async move { ([("Cache-Control", "no-store")], Json(bootstrap)) }
            }),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(dashboard_origin.parse::<axum::http::HeaderValue>()?)
                .allow_methods([axum::http::Method::POST])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ]),
        );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let worker_state_dir = state_dir.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<()> {
            let mut store = SqliteClientStore::open(
                format!("{worker_state_dir}/agent.sqlite3"),
                SqliteClientStoreOptions::from_nostr_secret(&agent_secret, "native-spike-agent")?,
            )?;
            store.save_device_state(&device)?;
            let mut delivery = HttpRuntimeDelivery::new(
                ReqwestHttpRuntimeTransport::new(server_url).with_signer(*agent_secret.as_bytes()),
            );
            let options = RuntimeSyncOptions {
                key_package_target_available: 8,
                max_sync_pages_per_room: 64,
            };
            let mut reply_index = 0;
            loop {
                device.set_now_unix_seconds(now());
                let report =
                    run_runtime_sync_tick(&mut store, &mut device, &mut delivery, &options)?;
                for applied in report.applied_entries {
                    if let AppliedLogEntry::Application { plaintext, sender } = applied.entry {
                        if sender.account_id != user_account_id {
                            continue;
                        }
                        if let Some(text) = finitechat_wasm::event_text(&plaintext) {
                            reply_index += 1;
                            let reply = format!("Native agent received over MLS: {text}");
                            let event = finitechat_wasm::text_event(&applied.room_id, &reply)
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let request = device.create_application_request(
                                &applied.room_id,
                                &event,
                                format!("reply_{reply_index}"),
                            )?;
                            let accepted = delivery.append_event(
                                &request,
                                DurableAppEventKind::ChatMessage.delivery_policy(),
                            )?;
                            device.record_own_send_accepted(
                                &applied.room_id,
                                accepted.seq,
                                &accepted.message_id,
                            )?;
                            store.save_device_state(&device)?;
                            println!(
                                "native peer: decrypted browser message and published encrypted reply {reply_index}"
                            );
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })();
        if let Err(error) = result {
            eprintln!("native peer failed: {error}");
            std::process::exit(1);
        }
    });
    println!(
        "WASM spike relay and native peer listening on 127.0.0.1:{port}; signed requests required"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
