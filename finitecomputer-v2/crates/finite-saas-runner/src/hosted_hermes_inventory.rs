//! Credential-free container inventory for hosted Hermes routing and allocation.
//!
//! `nerdctl port <full ID>` reads saved bindings even for stopped containers;
//! Docker-compatible inspect's HostConfig.PortBindings disappears when stopped.
//! Callers must collect every namespace under the host lifecycle fence. This
//! snapshot does not itself reserve a port or authorize provider mutations.
use crate::hosted_hermes_caddy::HostedHermesRoute;
use finite_saas_core::store::hosted_hermes::HostedRouteTarget;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

// Below both nerdctl's 49153–60999 random range and Linux's usual ephemeral
// range. Other saved TCP bindings still take precedence, regardless of owner.
pub const HOSTED_PORT_FIRST: u16 = 30000;
pub const HOSTED_PORT_LAST: u16 = 31023;
pub const HERMES_CONTAINER_PORT: u16 = 8642;
const MAX_INSPECT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONTAINERS: usize = 4096;

#[derive(Debug, thiserror::Error)]
#[error("invalid hosted Hermes inventory: {0}")]
pub struct InvalidInventory(&'static str);

impl InvalidInventory {
    pub fn port_read_failed() -> Self {
        Self("saved port mappings could not be read")
    }
}

#[derive(Default)]
pub struct HostedHermesInventory {
    containers: Vec<(String, Container)>,
    reservations: BTreeMap<u16, (String, String)>,
}

impl HostedHermesInventory {
    /// Add the full inspect output for one namespace, including stopped and
    /// foreign containers. Never pass a label-filtered list here. On any error
    /// discard the inventory; a partial snapshot cannot support allocation.
    pub fn add_namespace(
        &mut self,
        namespace: &str,
        json: &[u8],
        mut read_ports: impl FnMut(&str) -> Result<String, InvalidInventory>,
    ) -> Result<(), InvalidInventory> {
        if !provider_identifier(namespace) || json.len() > MAX_INSPECT_BYTES {
            return Err(InvalidInventory("namespace or inspect size is invalid"));
        }
        let mut containers: Vec<Container> = serde_json::from_slice(json)
            .map_err(|_| InvalidInventory("incomplete or malformed inspect output"))?;
        if self.containers.len() + containers.len() > MAX_CONTAINERS {
            return Err(InvalidInventory("container count exceeds bound"));
        }
        let mut identities = self
            .containers
            .iter()
            .map(|(ns, container)| (ns.clone(), container.id.clone()))
            .collect::<BTreeSet<_>>();
        let mut names = self
            .containers
            .iter()
            .map(|(ns, container)| {
                (
                    ns.clone(),
                    container.name.trim_start_matches('/').to_owned(),
                )
            })
            .collect::<BTreeSet<_>>();
        let mut reservations = self.reservations.clone();
        for container in &mut containers {
            if !segment(&container.id)
                || !provider_identifier(container.name.trim_start_matches('/'))
                || !identities.insert((namespace.to_owned(), container.id.clone()))
                || container.name.starts_with("//")
                || !names.insert((
                    namespace.to_owned(),
                    container.name.trim_start_matches('/').to_owned(),
                ))
            {
                return Err(InvalidInventory("ambiguous container identity"));
            }
            container.ports = parse_ports(&read_ports(&container.id)?)?;
            for binding in &container.ports {
                if binding.protocol != "tcp" {
                    continue;
                }
                let port = binding.host_port;
                let identity = (namespace.to_owned(), container.id.clone());
                if let Some(previous) = reservations.insert(port, identity.clone()) {
                    // Repeated IPv4/IPv6 bindings on one container reserve
                    // one port. Two containers claiming a managed port are
                    // ambiguous even if only one is currently running.
                    if previous != identity && managed_port(port) {
                        return Err(InvalidInventory(
                            "managed port has multiple container owners",
                        ));
                    }
                }
            }
        }
        self.reservations = reservations;
        self.containers.extend(
            containers
                .into_iter()
                .map(|container| (namespace.to_owned(), container)),
        );
        Ok(())
    }

    /// Omit every route belonging to a record about to be removed, while
    /// retaining its reservation until the provider confirms removal.
    pub fn exclude_container(&mut self, namespace: &str, name: &str) {
        self.containers.retain(|(ns, container)| {
            ns != namespace
                || (container.id != name && container.name.trim_start_matches('/') != name)
        });
    }

    /// The caller also checks real host listeners and creates the container
    /// while holding the same fence; choosing a free number is not a lease.
    pub fn available_ports(&self) -> impl Iterator<Item = u16> + '_ {
        (HOSTED_PORT_FIRST..=HOSTED_PORT_LAST).filter(|port| !self.reservations.contains_key(port))
    }

    /// Join Core's current assignments to exact local canonical containers.
    /// Unready/unenrolled containers have no route; contradictory ownership or
    /// binding facts fail the whole projection instead of choosing a winner.
    pub fn routes(
        &self,
        namespace: &str,
        source_host: &str,
        work_root: &Path,
        targets: &[HostedRouteTarget],
    ) -> Result<Vec<HostedHermesRoute>, InvalidInventory> {
        let mut ids = BTreeSet::new();
        let mut machines = BTreeSet::new();
        let mut routes = Vec::new();
        if !work_root.is_absolute()
            || targets.len() > finite_saas_core::hosted_hermes::MAX_HOSTED_HERMES_ROUTES
        {
            return Err(InvalidInventory("invalid route projection bounds"));
        }
        for target in targets {
            if !segment(&target.runtime_id)
                || !segment(&target.source_machine_id)
                || target.generation <= 0
                || !ids.insert(&target.runtime_id)
                || !machines.insert(&target.source_machine_id)
            {
                return Err(InvalidInventory("ambiguous Core assignment"));
            }
            let Some((_, container)) = self.containers.iter().find(|(ns, container)| {
                ns == namespace
                    && container.name.trim_start_matches('/') == target.source_machine_id
            }) else {
                continue;
            };
            let expected = [
                ("computer.finite.v2.runtime", "true"),
                ("computer.finite.v2.source_host_id", source_host),
                (
                    "computer.finite.v2.source_machine_id",
                    target.source_machine_id.as_str(),
                ),
                ("computer.finite.v2.project_id", target.project_id.as_str()),
            ];
            let data_root = work_root.join("kata").join(&target.runtime_id);
            let data_mounts = container
                .mounts
                .iter()
                .filter(|mount| mount.destination == Path::new("/data"))
                .collect::<Vec<_>>();
            if expected.iter().any(|(key, value)| {
                container.config.labels.get(*key).map(String::as_str) != Some(value)
            }) || data_mounts.len() != 1
                || data_mounts[0].source != data_root
                || !data_mounts[0].read_write
            {
                return Err(InvalidInventory(
                    "canonical container ownership does not match Core",
                ));
            }
            if container.state.status != "running" {
                continue;
            }
            let bindings = container
                .ports
                .iter()
                .filter(|binding| {
                    binding.container_port == HERMES_CONTAINER_PORT && binding.protocol == "tcp"
                })
                .collect::<Vec<_>>();
            if bindings.is_empty() {
                continue;
            }
            if bindings.len() != 1 || bindings[0].host_ip != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                return Err(InvalidInventory(
                    "Hermes requires exactly one loopback binding",
                ));
            }
            let port = bindings[0].host_port;
            if !managed_port(port) {
                return Err(InvalidInventory(
                    "Hermes binding is outside the reserved range",
                ));
            }
            routes.push(HostedHermesRoute {
                runtime_id: target.runtime_id.clone(),
                host_port: port,
            });
        }
        routes.sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
        Ok(routes)
    }
}

fn managed_port(port: u16) -> bool {
    (HOSTED_PORT_FIRST..=HOSTED_PORT_LAST).contains(&port)
}
fn provider_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}
fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

// Deliberately omit Config.Env: raw inspect can contain bootstrap credentials.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Container {
    id: String,
    name: String,
    config: Config,
    state: State,
    mounts: Vec<Mount>,
    #[serde(skip)]
    ports: Vec<Binding>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Config {
    #[serde(default)]
    labels: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    status: String,
}
struct Binding {
    host_ip: IpAddr,
    host_port: u16,
    container_port: u16,
    protocol: String,
}
fn parse_ports(text: &str) -> Result<Vec<Binding>, InvalidInventory> {
    if text.len() > 64 * 1024 {
        return Err(InvalidInventory("saved port listing exceeds bound"));
    }
    let port = |text: &str| {
        text.parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or(InvalidInventory("saved port is invalid"))
    };
    text.lines()
        .map(|line| {
            let (container, host) = line
                .split_once(" -> ")
                .ok_or(InvalidInventory("malformed saved port listing"))?;
            let (container_port, protocol) = container
                .split_once('/')
                .ok_or(InvalidInventory("malformed saved protocol"))?;
            if !matches!(protocol, "tcp" | "udp" | "sctp") {
                return Err(InvalidInventory("invalid saved protocol"));
            }
            // nerdctl prints IPv6 without brackets; split at the final colon.
            let (host_ip, host_port) = host
                .rsplit_once(':')
                .ok_or(InvalidInventory("malformed saved host port"))?;
            Ok(Binding {
                host_ip: host_ip
                    .parse()
                    .map_err(|_| InvalidInventory("saved host IP is invalid"))?,
                host_port: port(host_port)?,
                container_port: port(container_port)?,
                protocol: protocol.into(),
            })
        })
        .collect()
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Mount {
    source: PathBuf,
    destination: PathBuf,
    #[serde(rename = "RW")]
    read_write: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn container(name: &str, port: u16) -> Value {
        json!({
            "Id":format!("id-{name}"), "Name":name,
            "Config":{"Labels":{
                "computer.finite.v2.runtime":"true",
                "computer.finite.v2.source_host_id":"host-a",
                "computer.finite.v2.source_machine_id":name,
                "computer.finite.v2.project_id":"project-a",
            }},
            "State":{"Status":"running"},
            "SavedPorts":format!("8642/tcp -> 127.0.0.1:{port}\n"),
            "Mounts":[{"Source":"/state/kata/runtime-a","Destination":"/data","RW":true}]
        })
    }
    fn target() -> HostedRouteTarget {
        HostedRouteTarget {
            runtime_id: "runtime-a".into(),
            project_id: "project-a".into(),
            source_machine_id: "agent-a".into(),
            generation: 2,
        }
    }
    fn inventory(containers: &[Value]) -> HostedHermesInventory {
        let mut result = HostedHermesInventory::default();
        add(&mut result, "finite", containers).unwrap();
        result
    }
    fn add(
        inventory: &mut HostedHermesInventory,
        namespace: &str,
        containers: &[Value],
    ) -> Result<(), InvalidInventory> {
        inventory.add_namespace(namespace, &serde_json::to_vec(containers).unwrap(), |id| {
            Ok(
                containers.iter().find(|record| record["Id"] == id).unwrap()["SavedPorts"]
                    .as_str()
                    .unwrap()
                    .into(),
            )
        })
    }
    fn routes(
        inventory: &HostedHermesInventory,
        targets: &[HostedRouteTarget],
    ) -> Result<Vec<HostedHermesRoute>, InvalidInventory> {
        inventory.routes("finite", "host-a", Path::new("/state"), targets)
    }

    #[test]
    fn stopped_and_foreign_containers_reserve_ports_across_namespaces() {
        let mut stopped = container("agent-a", HOSTED_PORT_FIRST);
        stopped["State"]["Status"] = json!("exited");
        stopped["HostConfig"] = json!({"PortBindings":{}}); // real nerdctl stopped shape
        let mut foreign = container("foreign", HOSTED_PORT_FIRST + 1);
        foreign["Config"]["Labels"] = json!({});
        let mut inventory = inventory(&[stopped]);
        add(&mut inventory, "k8s.io", &[foreign]).unwrap();
        assert_eq!(
            inventory.available_ports().next(),
            Some(HOSTED_PORT_FIRST + 2)
        );
        assert!(routes(&inventory, &[target()]).unwrap().is_empty());
    }

    #[test]
    fn projection_requires_exact_assignment_bind_and_running_canonical() {
        let inventory = inventory(&[
            container("agent-a", HOSTED_PORT_FIRST),
            container("candidate", HOSTED_PORT_FIRST + 1),
        ]);
        let result = routes(&inventory, &[target()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].runtime_id, "runtime-a");
        assert_eq!(result[0].host_port, HOSTED_PORT_FIRST);
        assert!(routes(&inventory, &[]).unwrap().is_empty());
        let mut missing = target();
        missing.source_machine_id = "absent".into();
        assert!(routes(&inventory, &[missing]).unwrap().is_empty());
        assert!(
            inventory
                .routes("other", "host-a", Path::new("/state"), &[target()])
                .unwrap()
                .is_empty()
        );
        assert!(
            inventory
                .routes("finite", "wrong-host", Path::new("/state"), &[target()])
                .is_err()
        );
        assert!(routes(&inventory, &[target(), target()]).is_err());
        let mut traversal = target();
        traversal.runtime_id = "../runtime-a".into();
        assert!(routes(&inventory, &[traversal]).is_err());
    }

    #[test]
    fn removal_projection_excludes_target_but_keeps_its_reservation() {
        let mut inventory = inventory(&[container("agent-a", HOSTED_PORT_FIRST)]);
        inventory.exclude_container("finite", "agent-a");
        assert!(routes(&inventory, &[target()]).unwrap().is_empty());
        assert_eq!(
            inventory.available_ports().next(),
            Some(HOSTED_PORT_FIRST + 1)
        );
    }

    #[test]
    fn contradictory_ownership_mounts_or_bindings_never_publish() {
        for path_and_value in [
            (
                "/Config/Labels/computer.finite.v2.project_id",
                json!("other-project"),
            ),
            ("/Mounts/0/Source", json!("/state/kata/another-runtime")),
            ("/Mounts/0/RW", json!(false)),
            ("/SavedPorts", json!("8642/tcp -> 0.0.0.0:30000\n")),
            ("/SavedPorts", json!("8642/tcp -> 127.0.0.1:49153\n")),
        ] {
            let mut record = container("agent-a", HOSTED_PORT_FIRST);
            *record.pointer_mut(path_and_value.0).unwrap() = path_and_value.1;
            assert!(
                routes(&inventory(&[record]), &[target()]).is_err(),
                "{}",
                path_and_value.0
            );
        }
        let mut unenrolled = container("agent-a", HOSTED_PORT_FIRST);
        unenrolled["SavedPorts"] = json!("");
        assert!(
            routes(&inventory(&[unenrolled]), &[target()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn partial_or_ambiguous_inventory_is_rejected_atomically() {
        let first = container("agent-a", HOSTED_PORT_FIRST);
        let mut inventory = inventory(std::slice::from_ref(&first));
        assert!(add(&mut inventory, "finite", &[first]).is_err());
        assert!(
            add(
                &mut inventory,
                "other",
                &[container("other", HOSTED_PORT_FIRST)]
            )
            .is_err()
        );
        assert_eq!(
            inventory.available_ports().next(),
            Some(HOSTED_PORT_FIRST + 1)
        );
        let mut malformed = container("bad", HOSTED_PORT_FIRST + 1);
        malformed["SavedPorts"] = json!("8642/tcp -> 127.0.0.1:\n");
        assert!(add(&mut inventory, "other", &[malformed]).is_err());
        assert!(
            inventory
                .add_namespace("other", b"[{}", |_| unreachable!())
                .is_err()
        );
        assert!(
            inventory
                .add_namespace("other", b"[{}]", |_| unreachable!())
                .is_err()
        );
        assert!(
            inventory
                .add_namespace(
                    "other",
                    &vec![b' '; MAX_INSPECT_BYTES + 1],
                    |_| unreachable!()
                )
                .is_err()
        );
        assert!(
            inventory
                .add_namespace(
                    "other",
                    &serde_json::to_vec(&[container("unreadable", HOSTED_PORT_FIRST + 1)]).unwrap(),
                    |_| Err(InvalidInventory::port_read_failed())
                )
                .is_err()
        );
    }

    #[test]
    fn saved_port_parser_handles_ipv6_and_rejects_partial_output() {
        let ports = parse_ports("8080/tcp -> :::30003\n53/udp -> 0.0.0.0:30004\n").unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host_ip, "::".parse::<IpAddr>().unwrap());
        assert_eq!(ports[0].host_port, 30003);
        for invalid in [
            "warning: partial",
            "8080/tcp -> localhost:30000",
            "0/tcp -> 127.0.0.1:30000",
            "8642/tcp -> 127.0.0.1:0",
        ] {
            assert!(parse_ports(invalid).is_err());
        }
    }
}
