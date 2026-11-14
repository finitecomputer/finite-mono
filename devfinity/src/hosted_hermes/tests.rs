use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn ingress() -> IngressConfig {
    IngressConfig {
        core_url: "http://127.0.0.1:14200".into(),
        runner_token: "unused".into(),
        source_host_id: "devfinity-docker".into(),
        public_host: "agents.devfinity.test".into(),
        port: 18643,
        allowed_origins: vec!["http://127.0.0.1:13002".into()],
        admin_socket: "/tmp/devfinity-pc-0/hermes-caddy.sock".into(),
        state_dir: "/tmp/devfinity-hosted-hermes".into(),
        docker_bin: "docker".into(),
        renderer_bin: "finite-saas-runner".into(),
        caddy_bin: "caddy".into(),
    }
}

#[test]
fn manifest_is_the_production_renderer_contract_for_one_loopback_host() {
    let routes = [Route {
        runtime_id: "runtime_1".into(),
        host_port: 18642,
    }];
    assert_eq!(
        manifest(&ingress(), &routes),
        json!({
            "public_origin": "https://agents.devfinity.test:18643",
            "listen": "127.0.0.1:18643",
            "admin_socket": "/tmp/devfinity-pc-0/hermes-caddy.sock",
            "allowed_origins": ["http://127.0.0.1:13002"],
            "routes": [{ "runtime_id": "runtime_1", "host_port": 18642 }],
        })
    );
    assert_eq!(manifest(&ingress(), &[])["routes"], json!([]));
}

#[test]
fn local_pki_only_adds_an_internal_issuer_for_the_synthetic_host() {
    let rendered = json!({
        "admin": { "listen": "unix//tmp/admin.sock" },
        "apps": {
            "http": { "servers": { "hosted_hermes": { "listen": ["127.0.0.1:18643"] } } },
            "pki": { "certificate_authorities": { "local": { "install_trust": false } } },
        },
    });
    let configured = with_local_pki(rendered.clone(), "agents.devfinity.test").unwrap();
    assert_eq!(configured["apps"]["http"], rendered["apps"]["http"]);
    assert_eq!(configured["apps"]["pki"], rendered["apps"]["pki"]);
    assert_eq!(configured["admin"], rendered["admin"]);
    assert_eq!(
        configured["apps"]["tls"],
        json!({ "automation": { "policies": [{
            "subjects": ["agents.devfinity.test"],
            "issuers": [{ "module": "internal" }],
        }] } })
    );
    assert!(with_local_pki(configured, "agents.devfinity.test").is_err());
    assert!(with_local_pki(json!({}), "agents.devfinity.test").is_err());
}

#[test]
fn native_port_must_be_one_loopback_binding() {
    assert_eq!(loopback_port("127.0.0.1:18642\n"), Some(18642));
    for output in [
        "",
        "0.0.0.0:18642\n",
        "[::]:18642\n",
        "127.0.0.1:0\n",
        "127.0.0.1:18642\n127.0.0.1:18643\n",
        "127.0.0.1:not-a-port\n",
    ] {
        assert_eq!(loopback_port(output), None, "accepted {output:?}");
    }
}

#[test]
fn routes_require_this_hosts_exact_runtime_ownership_labels() {
    let target = RouteTarget {
        runtime_id: "runtime_1".into(),
        project_id: "project_1".into(),
        source_machine_id: "finite-agent_1".into(),
    };
    let labels = BTreeMap::from([
        (RUNTIME_LABEL.to_string(), "true".to_string()),
        (
            SOURCE_HOST_LABEL.to_string(),
            "devfinity-docker-300".to_string(),
        ),
        (
            SOURCE_MACHINE_LABEL.to_string(),
            "finite-agent_1".to_string(),
        ),
        (PROJECT_LABEL.to_string(), "project_1".to_string()),
    ]);
    assert!(owned_runtime(&labels, "devfinity-docker-300", &target));
    assert!(!owned_runtime(&labels, "devfinity-docker", &target));
    for (name, value) in [
        (RUNTIME_LABEL, "false"),
        (SOURCE_MACHINE_LABEL, "finite-agent_2"),
        (PROJECT_LABEL, "project_2"),
    ] {
        let mut changed = labels.clone();
        changed.insert(name.to_string(), value.to_string());
        assert!(!owned_runtime(&changed, "devfinity-docker-300", &target));
    }
}

fn runtime(id: &str, started_at: &str) -> RuntimeContainer {
    RuntimeContainer {
        id: id.into(),
        name: format!("finite-agent_{id}"),
        image: "sha256:image".into(),
        started_at: started_at.into(),
    }
}

fn bridge(id: &str, target: &str, running: bool) -> BridgeContainer {
    BridgeContainer {
        id: id.into(),
        target: target.into(),
        running,
    }
}

#[test]
fn bridges_follow_each_running_runtime_namespace() {
    let current = runtime("a", "t2");
    let new = runtime("b", "t1");
    let plan = bridge_plan(
        &[current.clone(), new.clone()],
        &[
            bridge("keep", "a@t2", true),
            bridge("duplicate", "a@t2", true),
            bridge("restarted", "a@t1", true),
            bridge("exited", "b@t1", false),
            bridge("vanished", "c@t1", true),
        ],
    );
    assert_eq!(
        plan,
        BridgePlan {
            remove: vec![
                "duplicate".into(),
                "restarted".into(),
                "exited".into(),
                "vanished".into()
            ],
            create: vec![new],
        }
    );
    assert_eq!(
        bridge_plan(&[], &[bridge("stale", "a@t2", true)]).remove,
        ["stale"]
    );
    assert_eq!(
        bridge_plan(std::slice::from_ref(&current), &[]).create,
        [current]
    );
}

#[test]
fn bridge_shares_only_the_runtime_network_namespace() {
    let config = CoreBridgeConfig {
        source_host_id: "devfinity-docker-300".into(),
        port: 14501,
        upstream_host: "host.docker.internal".into(),
        docker_bin: "docker".into(),
    };
    let args = bridge_run_args(&config, &runtime("abc", "t1"));
    let pairs = args.windows(2).collect::<Vec<_>>();
    for expected in [
        ["--name", "finite-agent_abc-core-bridge"],
        ["--network", "container:abc"],
        [
            "--label",
            "computer.finite.devfinity.core-bridge.source-host-id=devfinity-docker-300",
        ],
        [
            "--label",
            "computer.finite.devfinity.core-bridge.target=abc@t1",
        ],
        ["--entrypoint", "python3"],
    ] {
        assert!(
            pairs.contains(&&expected.map(String::from)[..]),
            "{expected:?}"
        );
    }
    assert!(args.contains(&"--no-healthcheck".to_string()));
    assert!(!args.iter().any(|arg| arg == "--publish" || arg == "-p"));
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("computer.finite.v2.")),
        "a bridge must never look like a Runner-owned runtime"
    );
    assert_eq!(
        args[args.len() - 5..],
        [
            "-c",
            CORE_BRIDGE_FORWARDER,
            "14501",
            "host.docker.internal",
            "14501"
        ]
    );
}

#[test]
fn core_bridge_forwarder_relays_a_loopback_connection() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let echo = std::thread::spawn(move || {
        let (mut stream, _) = upstream.accept().unwrap();
        let mut request = [0_u8; 4];
        stream.read_exact(&mut request).unwrap();
        stream.write_all(b"pong").unwrap();
        request
    });
    let listen_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut forwarder = Command::new("python3")
        .args(["-c", CORE_BRIDGE_FORWARDER])
        .args([
            listen_port.to_string(),
            "127.0.0.1".to_string(),
            upstream_port.to_string(),
        ])
        .spawn()
        .expect("python3 is part of the pinned development shell");
    let started = Instant::now();
    let mut client = loop {
        match TcpStream::connect(("127.0.0.1", listen_port)) {
            Ok(stream) => break stream,
            Err(_) if started.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => panic!("forwarder did not listen: {error}"),
        }
    };
    client.write_all(b"ping").unwrap();
    let mut response = [0_u8; 4];
    client.read_exact(&mut response).unwrap();
    let _ = forwarder.kill();
    let _ = forwarder.wait();
    assert_eq!(&echo.join().unwrap(), b"ping");
    assert_eq!(&response, b"pong");
}
