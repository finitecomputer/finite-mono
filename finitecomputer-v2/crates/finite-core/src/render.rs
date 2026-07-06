use crate::cluster::ClusterConfig;
use crate::models::{EndpointAuth, PublishedEndpointRecord, WorkloadRecord};
use crate::util::{bounded_kube_name, normalize_email, normalize_emails, sanitize_name};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use urlencoding::encode;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct RuntimeImageRevisionRecord {
    pub base_image: Option<String>,
    pub image: Option<String>,
    pub image_name: Option<String>,
    pub image_tag: Option<String>,
    pub store_path: Option<String>,
}

pub type RuntimeImageRevisionRegistry = BTreeMap<String, RuntimeImageRevisionRecord>;

pub fn read_runtime_image_revisions(
    control_plane_root: &Path,
) -> Result<RuntimeImageRevisionRegistry> {
    let Some(host_root) = control_plane_root.parent() else {
        return Ok(RuntimeImageRevisionRegistry::default());
    };
    let path = host_root.join("agent-cluster/runtime-image-revisions.json");
    if !path.exists() {
        return Ok(RuntimeImageRevisionRegistry::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read runtime image revisions from {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("parse runtime image revisions from {}", path.display()))
}

fn runtime_image_revision_value(
    runtime_image_revisions: Option<&RuntimeImageRevisionRegistry>,
    runtime_profile_id: &str,
) -> Option<String> {
    runtime_image_revisions
        .and_then(|registry| registry.get(runtime_profile_id))
        .and_then(|record| {
            record
                .store_path
                .clone()
                .or_else(|| record.image.clone())
                .or_else(
                    || match (record.image_name.as_deref(), record.image_tag.as_deref()) {
                        (Some(image_name), Some(image_tag)) => {
                            Some(format!("{image_name}:{image_tag}"))
                        }
                        _ => None,
                    },
                )
        })
}

fn runtime_image_reference(
    runtime_image_revisions: Option<&RuntimeImageRevisionRegistry>,
    runtime_profile_id: &str,
    fallback_image: &str,
) -> String {
    runtime_image_revisions
        .and_then(|registry| registry.get(runtime_profile_id))
        .and_then(|record| record.image.clone())
        .filter(|image| !image.starts_with('/'))
        .unwrap_or_else(|| fallback_image.to_string())
}

pub fn runtime_agents_text(cluster: &ClusterConfig) -> String {
    let home_dir = cluster.runtime_home();
    let workspace_dir = cluster.runtime_workspace();
    let default_project = cluster.default_project_dir();
    format!(
        "# Agent Computer Runtime Notes\n\n\
- Read `/platform/FINITE.md` for the current finitecomputer platform contract.\n\
- Privacy boundary: this legacy runtime is not a private vault. Finite operators run the box, and model inference may be visible to the configured inference provider, so chat contents and tool outputs may be visible to those services.\n\
- Help protect the human: remind them when a task may involve personal, confidential, regulated, risky, or irreversible information, and suggest redaction, fake data, local-only handling, or pausing before they paste secrets or sensitive details.\n\
- Treat this runtime as a sandbox for learning, building, non-risky vibe coding, and agentic experiments, not as the place for secrets-heavy, high-stakes, or sensitive personal work.\n\
- This is a persistent per-user runtime pod, not a full mutable Linux machine.\n\
- Durable user state lives under `{home_dir}`.\n\
- Durable project work belongs under `{workspace_dir}` and `{home_dir}/dev`.\n\
- OpenCode starts in `{default_project}` by default so it is immediately useful for Hermes troubleshooting.\n\
- The root filesystem is intentionally read-only; do not expect `apt` or writes under `/usr` to persist.\n\
- npm, bun, pip, pipx, and uv installs should land under `{home_dir}` and survive pod restarts.\n\
- This runtime applies a seven-day dependency cooldown for npm, Bun, and uv where the pinned package manager supports it; prefer pinned versions and existing lockfiles, and do not run broad upgrades during active registry incidents without explicit approval.\n\
- Do not try to install new system binaries yourself; ask Paul or Austin to change the runtime baseline instead.\n\
- Hermes state lives under `{home_dir}/.hermes`.\n\
- OpenCode is expected to run continuously inside the pod.\n\
- Codex is intentionally interactive and bootstrap-managed, not a daemon.\n\
- Use `finitec publish` to ask the host control plane for hostnames and published routes.\n\
- Do not invent ad hoc host port mappings or edit Traefik from inside the pod.\n\
- Use `finitec` for platform-specific shell commands. It is the supported wrapper already on `PATH`.\n\
- Before using a generic built-in or user-local skill, check whether a relevant Finite-managed `-finite` skill exists and prefer it.\n\
- Finite-managed skills sync from `finitecomputer/finite-skills` into `~/.finite/managed-skills/finite/current`; shared/team managed skills, when configured, sync into `~/.finite/managed-skills/user/current`. Treat managed trees as read-only baselines and copy a skill into `{home_dir}/.hermes/skills/...` if you want to customize it locally.\n\
- For sharing a skill with the team or installing a shared/team skill, prefer `shared-skills-finite`.\n\
- In particular, prefer `shared-skills-finite` for publishing or pulling team-shared skills, `google-workspace-finite` for Gmail/Drive/Calendar/Apps Script work, `monday-com-finite` for Monday boards/items/updates when the machine integration is connected, `perplexity-research-finite` for grounded research, `impeccable-finite` for design direction and polish on web UI, `website-building-finite` for end-to-end websites and dashboards, `publish-web-apps-finite` for host-mediated publishing, and `x-api-finite` when the human pastes a specific X/Twitter post URL or wants exact tweet/profile data.\n\
- Reserve a hostname first with `finitec publish reserve --label NAME`, then expose the app with `finitec publish expose --hostname HOSTNAME --port PORT --mode self` unless your human asked for something else.\n\
- Use `finitec publish list` and `finitec publish remove --hostname HOSTNAME` to inspect or remove published endpoints.\n\
- For final static public sites that should survive this runtime stopping, use `finitec publish static-here-now --dir DIST --confirm-public \"MAKE PUBLIC\"`; publish only the final artifact directory, never a project root.\n\
- For marketing sites, dashboards, and other web UI work, use `impeccable-finite` when the problem is taste, hierarchy, or polish, then use `website-building-finite` for implementation, Playwright review, and publish flow.\n\
- Public exposure is dangerous; never make anything public unless the human sends one standalone message containing `MAKE PUBLIC`.\n\
- Do not infer public consent from earlier discussion or paraphrases.\n\
- For grounded web research, prefer the `perplexity-research-finite` skill: raw source search first, exact-source fetch second, cited brief third.\n\
- SSH exists only as an admin escape hatch. Normal setup should be driven by browser/dashboard flows.\n\
- If you need a new project, create a git repo first:\n\
  `mkdir -p {home_dir}/dev/NEW_PROJECT && cd {home_dir}/dev/NEW_PROJECT && git init && git branch -m main`\n"
    )
}

pub fn runtime_hermes_agents_text(cluster: &ClusterConfig) -> String {
    let home_dir = cluster.runtime_home();
    format!(
        "# Finite Runtime Context\n\n\
- Read `/platform/FINITE.md` for the current finitecomputer platform contract before making platform assumptions.\n\
- This is a persistent per-user runtime pod with durable state under `{home_dir}`.\n\
- Use `finitec` for platform-specific operations such as publishing, repo access, connection setup, and managed skill sync.\n\
- Do not edit Traefik, k3s, host networking, or host files from inside the pod.\n\
- Before using a generic built-in or local skill, check whether a relevant Finite-managed `-finite` skill exists and prefer it when applicable.\n\
- For sharing a skill with the team or installing a shared/team skill, prefer `shared-skills-finite`.\n\
- Public publishing is dangerous and requires one standalone human message containing `MAKE PUBLIC`.\n\
- Static public site publishing should use `finitec publish static-here-now --dir DIST --confirm-public \"MAKE PUBLIC\"` against a final build directory, never a project root.\n\
- Treat this runtime as a sandbox for learning, building, non-risky vibe coding, and agentic experiments, not as a private vault.\n"
    )
}

pub fn runtime_opencode_agents_text(cluster: &ClusterConfig) -> String {
    let home_dir = cluster.runtime_home();
    let workspace_dir = cluster.runtime_workspace();
    let base_domain = cluster.base_domain();
    format!(
        "# finitecomputer runtime global rules\n\n\
- Read `/platform/FINITE.md` for the current finitecomputer platform contract.\n\
- Privacy boundary: this legacy runtime is not a private vault. Finite operators run the box, and model inference may be visible to the configured inference provider, so chat contents and tool outputs may be visible to those services.\n\
- Help protect the human: remind them when a task may involve personal, confidential, regulated, risky, or irreversible information, and suggest redaction, fake data, local-only handling, or pausing before they paste secrets or sensitive details.\n\
- Treat this runtime as a sandbox for learning, building, non-risky vibe coding, and agentic experiments, not as the place for secrets-heavy, high-stakes, or sensitive personal work.\n\
- This pod uses a persistent home at `{home_dir}`.\n\
- Install user-space tools with npm, bun, pip, pipx, or uv so they land under the durable home.\n\
- This runtime applies a seven-day dependency cooldown for npm, Bun, and uv where the pinned package manager supports it; prefer pinned versions and existing lockfiles, and do not run broad upgrades during active registry incidents without explicit approval.\n\
- Do not assume root writes persist.\n\
- Do not try to install new system binaries yourself; ask Paul or Austin to change the runtime baseline instead.\n\
- Hermes runtime state lives in `{home_dir}/.hermes`.\n\
- OpenCode is the always-on web fallback and support surface.\n\
- Codex is allowed to remain npm-managed because device auth is manual and user-specific.\n\
- OpenCode in this runtime explicitly allows normal `edit`, `bash`, and `webfetch` work, plus external-directory access, so work in `/tmp`, `/home/node/dev`, and uploaded files does not constantly interrupt normal iteration.\n\
- Finite-managed skills sync from `finitecomputer/finite-skills` into `{home_dir}/.finite/managed-skills/finite/current`; shared/team managed skills, when configured, sync into `{home_dir}/.finite/managed-skills/user/current`. OpenCode does not read Hermes `external_dirs`, so inspect those trees directly when you need to confirm which managed skills exist.\n\
- Treat synced skill trees as read-only; if you want to customize a managed skill, copy it into `{home_dir}/.hermes/skills/...` and edit the local copy there.\n\
- For sharing a skill with the team or installing a shared/team skill, prefer `shared-skills-finite`.\n\
- The default published endpoint is the OpenCode hostname under `*.{base_domain}`.\n\
- Use `finitec publish` for any additional host-mediated published endpoints.\n\
- Before using a generic built-in or user-local skill, check whether a relevant Finite-managed `-finite` skill exists and prefer it.\n\
- In particular, prefer `shared-skills-finite` for publishing or pulling team-shared skills, `google-workspace-finite` for Gmail/Drive/Calendar/Apps Script work, `monday-com-finite` for Monday boards/items/updates when the machine integration is connected, `perplexity-research-finite` for grounded research, `impeccable-finite` for design direction and polish on web UI, `website-building-finite` for end-to-end websites and dashboards, `publish-web-apps-finite` for host-mediated publishing, and `x-api-finite` when the human pastes a specific X/Twitter post URL or wants exact tweet/profile data.\n\
- For grounded web research, prefer the `perplexity-research-finite` skill and inspect cited URLs before relying on a brief.\n\
- For websites and dashboards, prefer `impeccable-finite` for design direction and `website-building-finite` for the actual build-and-publish workflow.\n\
- Do not edit Traefik config yourself.\n\
- Use `finitec` for platform-specific shell commands. It is the supported wrapper already on `PATH`.\n\
- If you start a new project, create a git repo immediately, get the scaffold running, then make an initial commit before the first big feature pass.\n\
- Commit again at each meaningful checkpoint instead of waiting until the end.\n\
- Reserve a hostname first with `finitec publish reserve --label NAME`, then expose the app with `finitec publish expose --hostname HOSTNAME --port PORT --mode self` unless your human asked for something else.\n\
- For final static public sites that should survive this runtime stopping, use `finitec publish static-here-now --dir DIST --confirm-public \"MAKE PUBLIC\"`; publish only the final artifact directory, never a project root.\n\
- Public exposure is dangerous and requires one standalone human message containing `MAKE PUBLIC`.\n\
- Keep work under `{workspace_dir}` or `{home_dir}/dev`.\n"
    )
}

fn runtime_opencode_config_text() -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "$schema": "https://opencode.ai/config.json",
        "permission": {
            "edit": "allow",
            "bash": "allow",
            "webfetch": "allow",
            "external_directory": {
                "*": "allow"
            }
        }
    }))?)
}

fn shared_runtime_seed_path(workspace_root: &Path, relative_path: &str) -> PathBuf {
    if let Some(repo_root) = workspace_root.parent().and_then(Path::parent) {
        let shared = repo_root
            .join("nix/agent-runtime/runtime-seed")
            .join(relative_path);
        if shared.exists() {
            return shared;
        }
    }
    workspace_root
        .join("agent-cluster/runtime-seed")
        .join(relative_path)
}

pub fn seed_text(
    workspace_root: &Path,
    cluster: &ClusterConfig,
) -> Result<serde_json::Map<String, Value>> {
    let runtime_seed_root = workspace_root.join("agent-cluster/runtime-seed");
    let seed_root = runtime_seed_root.join("hermes");
    Ok(serde_json::Map::from_iter([
        (
            "AGENTS.md".to_string(),
            Value::String(runtime_agents_text(cluster)),
        ),
        (
            "hermes-AGENTS.md".to_string(),
            Value::String(runtime_hermes_agents_text(cluster)),
        ),
        (
            "opencode-AGENTS.md".to_string(),
            Value::String(runtime_opencode_agents_text(cluster)),
        ),
        (
            "codex-AGENTS.md".to_string(),
            Value::String(runtime_opencode_agents_text(cluster)),
        ),
        (
            "opencode-config.json".to_string(),
            Value::String(runtime_opencode_config_text()?),
        ),
        (
            "admin_authorized_keys".to_string(),
            Value::String(cluster.admin_authorized_keys_text()),
        ),
        (
            "FINITE.md".to_string(),
            Value::String(fs::read_to_string(shared_runtime_seed_path(
                workspace_root,
                "FINITE.md",
            ))?),
        ),
        (
            "hermes-config.yaml".to_string(),
            Value::String(fs::read_to_string(seed_root.join("config.yaml"))?),
        ),
        (
            "hermes-SOUL.md".to_string(),
            Value::String(fs::read_to_string(shared_runtime_seed_path(
                workspace_root,
                "hermes/SOUL.md",
            ))?),
        ),
        (
            "hermes-MEMORY.md".to_string(),
            Value::String(fs::read_to_string(seed_root.join("memories/MEMORY.md"))?),
        ),
        (
            "hermes-USER.md".to_string(),
            Value::String(fs::read_to_string(seed_root.join("memories/USER.md"))?),
        ),
    ]))
}

fn site_owner_email(auth: &EndpointAuth, fallback_owner_email: Option<&str>) -> Option<String> {
    auth.owner_email
        .as_deref()
        .and_then(|value| normalize_email(Some(value)))
        .or_else(|| normalize_email(fallback_owner_email))
}

fn site_allowed_emails(auth: &EndpointAuth) -> Vec<String> {
    normalize_emails(&auth.emails)
}

fn site_org_domain(cluster: &ClusterConfig, auth: &EndpointAuth) -> String {
    auth.org_domain
        .clone()
        .or_else(|| cluster.org_domain.clone())
        .unwrap_or_else(|| cluster.base_domain().to_string())
}

fn site_auth_enabled(cluster: &ClusterConfig, auth: &EndpointAuth) -> bool {
    cluster.oauth_enabled() && auth.mode != "public"
}

fn site_auth_query_params(
    cluster: &ClusterConfig,
    auth: &EndpointAuth,
    fallback_owner_email: Option<&str>,
) -> String {
    match auth.mode.as_str() {
        "self" => site_owner_email(auth, fallback_owner_email)
            .map(|owner_email| format!("?allowed_emails={}", encode(&owner_email)))
            .unwrap_or_default(),
        "emails" => {
            let emails = site_allowed_emails(auth);
            if emails.is_empty() {
                String::new()
            } else {
                format!("?allowed_emails={}", encode(&emails.join(",")))
            }
        }
        "org" => format!(
            "?allowed_email_domains={}",
            encode(&site_org_domain(cluster, auth))
        ),
        _ => String::new(),
    }
}

fn endpoint_auth(auth: &EndpointAuth) -> EndpointAuth {
    EndpointAuth {
        mode: if auth.mode.is_empty() {
            "self".to_string()
        } else {
            auth.mode.clone()
        },
        owner_email: auth.owner_email.clone(),
        emails: auth.emails.clone(),
        org_domain: auth.org_domain.clone(),
    }
}

pub fn render_workload_manifest(
    workspace_root: &Path,
    cluster: &ClusterConfig,
    workload: &WorkloadRecord,
    published_endpoints: &[PublishedEndpointRecord],
    runtime_image_revisions: Option<&RuntimeImageRevisionRegistry>,
) -> Result<Value> {
    let machine_id = &workload.id;
    let (runtime_profile_id, runtime_profile) =
        cluster.resolve_runtime_profile(Some(&workload.runtime_profile))?;
    let runtime_image_fallback = format!(
        "{}:{}",
        runtime_profile.image_name, runtime_profile.image_tag
    );
    let runtime_image = runtime_image_reference(
        runtime_image_revisions,
        &runtime_profile_id,
        &runtime_image_fallback,
    );
    let runtime_image_revision =
        runtime_image_revision_value(runtime_image_revisions, &runtime_profile_id);
    let namespace = if workload.namespace.is_empty() {
        machine_id.clone()
    } else {
        workload.namespace.clone()
    };
    let labels = json!({
        "app.kubernetes.io/name": "fc-agent-computer",
        "app.kubernetes.io/instance": machine_id,
        "fc.finitecomputer.dev/workload": machine_id,
    });
    let sanitized = sanitize_name(machine_id);
    let opencode = &workload.opencode;
    let hostname = &opencode.hostname;
    let project_dir = if opencode.project_dir.is_empty() {
        cluster.default_project_dir()
    } else {
        opencode.project_dir.clone()
    };
    let headless_service_name = format!("{sanitized}-headless");
    let opencode_service_name = format!("{sanitized}-opencode");
    let ssh_service_name = format!("{sanitized}-ssh");
    let seed_config_map_name = format!("{sanitized}-seed");
    let oauth_proxy_alias_service_name = format!("{sanitized}-oauth2-proxy");
    let oauth_proxy_auth_address = format!(
        "http://{}.{}.svc.cluster.local:4180",
        oauth_proxy_alias_service_name, namespace
    );
    let oauth_middleware_name = format!("{sanitized}-oauth2-auth");
    let oauth_errors_middleware_name = format!("{sanitized}-oauth2-errors");
    let profile_assets_host_path = format!(
        "/var/lib/finitecomputer/agent-cluster/profile-assets/{}",
        runtime_profile.feature_set
    );
    let profile_assets_mount_path = format!("/profile-assets/{}", runtime_profile.feature_set);
    let secrets_host_path = format!("/var/lib/finitecomputer/agent-cluster/secrets/{machine_id}");
    let shared_secrets_host_path = "/var/lib/finitecomputer/agent-cluster/secrets/shared";

    let tls = cluster
        .letsencrypt_enabled()
        .then(|| json!({ "certResolver": "letsencrypt" }));
    let site_auth = endpoint_auth(&opencode.auth);

    let mut items = vec![
        json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": namespace },
        }),
        json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": seed_config_map_name, "namespace": namespace },
            "data": Value::Object(seed_text(workspace_root, cluster)?),
        }),
        json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": headless_service_name, "namespace": namespace },
            "spec": {
                "clusterIP": "None",
                "selector": labels,
            },
        }),
        json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": opencode_service_name, "namespace": namespace },
            "spec": {
                "type": "ClusterIP",
                "selector": labels,
                "ports": [{
                    "name": "http",
                    "port": opencode.port,
                    "targetPort": opencode.port,
                }],
            },
        }),
    ];

    if workload.ssh.enable {
        items.push(json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": ssh_service_name, "namespace": namespace },
            "spec": {
                "type": "NodePort",
                "selector": labels,
                "ports": [{
                    "name": "ssh",
                    "port": 2222,
                    "targetPort": 2222,
                    "nodePort": workload.ssh.node_port,
                }],
            },
        }));
    }

    if site_auth_enabled(cluster, &site_auth) {
        items.extend([
            json!({
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": { "name": oauth_proxy_alias_service_name, "namespace": namespace },
                "spec": {
                    "type": "ExternalName",
                    "externalName": "oauth2-proxy.fc-auth.svc.cluster.local",
                    "ports": [{
                        "name": "http",
                        "port": 4180,
                        "targetPort": 4180,
                    }],
                },
            }),
            json!({
                "apiVersion": "traefik.io/v1alpha1",
                "kind": "Middleware",
                "metadata": { "name": oauth_errors_middleware_name, "namespace": namespace },
                "spec": {
                    "errors": {
                        "status": ["401"],
                        "statusRewrites": { "401": 302 },
                        "service": { "name": oauth_proxy_alias_service_name, "port": 4180 },
                        "query": "/oauth2/sign_in?rd={url}",
                    },
                },
            }),
            json!({
                "apiVersion": "traefik.io/v1alpha1",
                "kind": "Middleware",
                "metadata": { "name": oauth_middleware_name, "namespace": namespace },
                "spec": {
                    "forwardAuth": {
                        "address": format!(
                            "{}/oauth2/auth{}",
                            oauth_proxy_auth_address,
                            site_auth_query_params(cluster, &site_auth, workload.owner_email.as_deref())
                        ),
                        "trustForwardHeader": true,
                        "authResponseHeaders": [
                            "X-Auth-Request-User",
                            "X-Auth-Request-Email",
                            "X-Auth-Request-Preferred-Username",
                            "Authorization",
                        ],
                    },
                },
            }),
        ]);
    }

    let mut containers = vec![json!({
        "name": "runtime",
        "image": runtime_image,
        "imagePullPolicy": "Never",
        "workingDir": project_dir,
        "env": [
            { "name": "OPENCODE_WEB_PORT", "value": opencode.port.to_string() },
            { "name": "FC_WORKLOAD_ID", "value": machine_id },
            { "name": "FC_RUNTIME_PROFILE", "value": workload.runtime_profile },
            {
                "name": "FINITE_USER_EMAIL",
                "value": workload.owner_email.clone().unwrap_or_default(),
            },
            {
                "name": "FC_PUBLISHED_OPENCODE_URL",
                "value": format!("{}://{}", cluster.published_url_scheme(), hostname),
            },
            { "name": "FC_CONTROL_PLANE_URL", "value": cluster.dashboard_internal_url() },
            {
                "name": "FC_RELAY_URL",
                "value": format!(
                    "http://dashboard-relay.{}.svc.cluster.local:4100",
                    cluster.dashboard_namespace()
                ),
            },
            { "name": "OPENCODE_PROJECT_DIR", "value": project_dir },
        ],
        "ports": [{ "name": "opencode", "containerPort": opencode.port }],
        "readinessProbe": {
            "tcpSocket": { "port": opencode.port },
            "initialDelaySeconds": 10,
            "periodSeconds": 5,
        },
        "livenessProbe": {
            "tcpSocket": { "port": opencode.port },
            "initialDelaySeconds": 30,
            "periodSeconds": 10,
        },
        "securityContext": {
            "runAsUser": 0,
            "runAsGroup": 0,
            "readOnlyRootFilesystem": true,
            "allowPrivilegeEscalation": false,
        },
        "volumeMounts": [
            { "name": "home", "mountPath": cluster.runtime_home() },
            { "name": "profile-assets", "mountPath": profile_assets_mount_path, "readOnly": true },
            { "name": "seed", "mountPath": "/seed", "readOnly": true },
            { "name": "seed", "mountPath": "/platform", "readOnly": true },
            { "name": "tmp", "mountPath": "/tmp" },
            { "name": "run", "mountPath": "/run" },
            { "name": "secrets", "mountPath": "/secrets", "readOnly": true },
            { "name": "shared-secrets", "mountPath": "/shared-secrets", "readOnly": true },
        ],
    })];

    if workload.ssh.enable {
        containers.push(json!({
            "name": "sshd",
            "image": runtime_image,
            "imagePullPolicy": "Never",
            "command": ["/bin/fc-agent-sshd"],
            "ports": [{ "name": "ssh", "containerPort": 2222 }],
            "readinessProbe": {
                "tcpSocket": { "port": 2222 },
                "initialDelaySeconds": 5,
                "periodSeconds": 5,
            },
            "securityContext": {
                "runAsUser": 0,
                "runAsGroup": 0,
                "readOnlyRootFilesystem": true,
                "allowPrivilegeEscalation": false,
            },
            "volumeMounts": [
                { "name": "home", "mountPath": cluster.runtime_home() },
                { "name": "profile-assets", "mountPath": profile_assets_mount_path, "readOnly": true },
                { "name": "seed", "mountPath": "/seed", "readOnly": true },
                { "name": "seed", "mountPath": "/platform", "readOnly": true },
                { "name": "tmp", "mountPath": "/tmp" },
                { "name": "run", "mountPath": "/run" },
            ],
        }));
    }

    let mut pod_template_metadata =
        serde_json::Map::from_iter([("labels".to_string(), labels.clone())]);
    if let Some(runtime_image_revision) = runtime_image_revision {
        pod_template_metadata.insert(
            "annotations".to_string(),
            json!({
                "fc.finitecomputer.dev/runtime-image-revision": runtime_image_revision,
            }),
        );
    }

    items.push(json!({
        "apiVersion": "apps/v1",
        "kind": "StatefulSet",
        "metadata": { "name": sanitized, "namespace": namespace },
        "spec": {
            "serviceName": headless_service_name,
            "replicas": 1,
            "selector": { "matchLabels": labels },
            "template": {
                "metadata": Value::Object(pod_template_metadata),
                "spec": {
                    "automountServiceAccountToken": false,
                    "terminationGracePeriodSeconds": 30,
                    "securityContext": { "fsGroup": cluster.agent_gid() },
                    "volumes": [
                        { "name": "profile-assets", "hostPath": { "path": profile_assets_host_path, "type": "Directory" }},
                        { "name": "seed", "configMap": { "name": seed_config_map_name }},
                        { "name": "tmp", "emptyDir": {} },
                        { "name": "run", "emptyDir": {} },
                        { "name": "secrets", "hostPath": { "path": secrets_host_path, "type": "DirectoryOrCreate" }},
                        { "name": "shared-secrets", "hostPath": { "path": shared_secrets_host_path, "type": "DirectoryOrCreate" }},
                    ],
                    "containers": containers,
                },
            },
            "volumeClaimTemplates": [{
                "metadata": { "name": "home" },
                "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "resources": { "requests": { "storage": workload.home_volume_size }},
                },
            }],
        },
    }));

    let mut ingress_routes = Vec::new();
    if site_auth_enabled(cluster, &site_auth) {
        ingress_routes.push(json!({
            "match": format!("Host(`{hostname}`) && PathPrefix(`/oauth2/`)"),
            "kind": "Rule",
            "services": [{ "name": oauth_proxy_alias_service_name, "port": 4180 }],
        }));
    }

    let mut main_route = json!({
        "match": format!("Host(`{hostname}`)"),
        "kind": "Rule",
        "services": [{ "name": opencode_service_name, "port": opencode.port }],
    });
    if site_auth_enabled(cluster, &site_auth) {
        main_route["middlewares"] = json!([
            { "name": oauth_errors_middleware_name },
            { "name": oauth_middleware_name },
        ]);
    }
    ingress_routes.push(main_route);

    let mut ingress_route = json!({
        "apiVersion": "traefik.io/v1alpha1",
        "kind": "IngressRoute",
        "metadata": { "name": format!("{sanitized}-opencode"), "namespace": namespace },
        "spec": {
            "entryPoints": ["websecure"],
            "routes": ingress_routes,
        },
    });
    if let Some(tls) = &tls {
        ingress_route["spec"]["tls"] = tls.clone();
    }
    items.push(ingress_route);

    for endpoint in published_endpoints {
        let Some(target_port) = endpoint.target_port else {
            continue;
        };

        let endpoint_hostname = endpoint.hostname.clone();
        let endpoint_ingress_route_name = bounded_kube_name(&endpoint_hostname, "");
        let endpoint_service_name = bounded_kube_name(&endpoint_hostname, "svc");
        let endpoint_oauth_proxy_alias_service_name =
            bounded_kube_name(&endpoint_hostname, "oauth2-proxy");
        let endpoint_oauth_proxy_auth_address = format!(
            "http://{}.{}.svc.cluster.local:4180",
            endpoint_oauth_proxy_alias_service_name, namespace
        );
        let endpoint_oauth_middleware_name = bounded_kube_name(&endpoint_hostname, "oauth2-auth");
        let endpoint_oauth_errors_middleware_name =
            bounded_kube_name(&endpoint_hostname, "oauth2-errors");
        let endpoint_auth = endpoint_auth(&endpoint.auth);

        items.push(json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": endpoint_service_name, "namespace": namespace },
            "spec": {
                "type": "ClusterIP",
                "selector": labels,
                "ports": [{
                    "name": "http",
                    "port": target_port,
                    "targetPort": target_port,
                }],
            },
        }));

        if site_auth_enabled(cluster, &endpoint_auth) {
            items.extend([
                json!({
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": { "name": endpoint_oauth_proxy_alias_service_name, "namespace": namespace },
                    "spec": {
                        "type": "ExternalName",
                        "externalName": "oauth2-proxy.fc-auth.svc.cluster.local",
                        "ports": [{ "name": "http", "port": 4180, "targetPort": 4180 }],
                    },
                }),
                json!({
                    "apiVersion": "traefik.io/v1alpha1",
                    "kind": "Middleware",
                    "metadata": { "name": endpoint_oauth_errors_middleware_name, "namespace": namespace },
                    "spec": {
                        "errors": {
                            "status": ["401"],
                            "statusRewrites": { "401": 302 },
                            "service": { "name": endpoint_oauth_proxy_alias_service_name, "port": 4180 },
                            "query": "/oauth2/sign_in?rd={url}",
                        },
                    },
                }),
                json!({
                    "apiVersion": "traefik.io/v1alpha1",
                    "kind": "Middleware",
                    "metadata": { "name": endpoint_oauth_middleware_name, "namespace": namespace },
                    "spec": {
                        "forwardAuth": {
                            "address": format!(
                                "{}/oauth2/auth{}",
                                endpoint_oauth_proxy_auth_address,
                                site_auth_query_params(cluster, &endpoint_auth, workload.owner_email.as_deref())
                            ),
                            "trustForwardHeader": true,
                            "authResponseHeaders": [
                                "X-Auth-Request-User",
                                "X-Auth-Request-Email",
                                "X-Auth-Request-Preferred-Username",
                                "Authorization",
                            ],
                        },
                    },
                }),
            ]);
        }

        let mut endpoint_routes = Vec::new();
        if site_auth_enabled(cluster, &endpoint_auth) {
            endpoint_routes.push(json!({
                "match": format!("Host(`{endpoint_hostname}`) && PathPrefix(`/oauth2/`)"),
                "kind": "Rule",
                "services": [{ "name": endpoint_oauth_proxy_alias_service_name, "port": 4180 }],
            }));
        }

        let mut endpoint_main_route = json!({
            "match": format!("Host(`{endpoint_hostname}`)"),
            "kind": "Rule",
            "services": [{ "name": endpoint_service_name, "port": target_port }],
        });
        if site_auth_enabled(cluster, &endpoint_auth) {
            endpoint_main_route["middlewares"] = json!([
                { "name": endpoint_oauth_errors_middleware_name },
                { "name": endpoint_oauth_middleware_name },
            ]);
        }
        endpoint_routes.push(endpoint_main_route);

        let mut endpoint_ingress_route = json!({
            "apiVersion": "traefik.io/v1alpha1",
            "kind": "IngressRoute",
            "metadata": { "name": endpoint_ingress_route_name, "namespace": namespace },
            "spec": {
                "entryPoints": ["websecure"],
                "routes": endpoint_routes,
            },
        });
        if let Some(tls) = &tls {
            endpoint_ingress_route["spec"]["tls"] = tls.clone();
        }
        items.push(endpoint_ingress_route);
    }

    Ok(json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": items,
    }))
}

fn manifest_item_id(item: &Value) -> Option<(String, String, String, String)> {
    let metadata = item.get("metadata")?;
    Some((
        item.get("apiVersion")?.as_str()?.to_string(),
        item.get("kind")?.as_str()?.to_string(),
        metadata
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        metadata.get("name")?.as_str()?.to_string(),
    ))
}

pub fn deletion_manifest_for_removed_items(previous: &Value, current: &Value) -> Option<Value> {
    let previous_items = previous.get("items")?.as_array()?;
    let current_ids = current
        .get("items")?
        .as_array()?
        .iter()
        .filter_map(manifest_item_id)
        .collect::<std::collections::BTreeSet<_>>();

    let removed_items = previous_items
        .iter()
        .filter(|item| {
            manifest_item_id(item)
                .map(|id| !current_ids.contains(&id))
                .unwrap_or(false)
        })
        .map(|item| {
            let metadata = item.get("metadata").cloned().unwrap_or_else(|| json!({}));
            json!({
                "apiVersion": item.get("apiVersion").cloned().unwrap_or(Value::Null),
                "kind": item.get("kind").cloned().unwrap_or(Value::Null),
                "metadata": {
                    "name": metadata.get("name").cloned().unwrap_or(Value::Null),
                    "namespace": metadata.get("namespace").cloned().unwrap_or(Value::Null),
                },
            })
        })
        .collect::<Vec<_>>();

    if removed_items.is_empty() {
        None
    } else {
        Some(json!({
            "apiVersion": "v1",
            "kind": "List",
            "items": removed_items,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeImageRevisionRecord, deletion_manifest_for_removed_items, render_workload_manifest,
        runtime_agents_text, runtime_opencode_agents_text,
    };
    use crate::cluster::{ClusterConfig, RuntimeProfileConfig};
    use crate::models::{EndpointAuth, WorkloadOpencode, WorkloadRecord, WorkloadSsh};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn runtime_agents_text_mentions_finitec() {
        let cluster = ClusterConfig::default();
        let text = runtime_agents_text(&cluster);
        assert!(text.contains("finitec publish"));
    }

    #[test]
    fn runtime_agent_texts_include_privacy_boundary() {
        let cluster = ClusterConfig::default();
        let agent_text = runtime_agents_text(&cluster);
        let opencode_text = runtime_opencode_agents_text(&cluster);

        for text in [agent_text, opencode_text] {
            assert!(text.contains("not a private vault"));
            assert!(text.contains("configured inference provider"));
            assert!(text.contains("non-risky vibe coding"));
            assert!(text.contains("seven-day dependency cooldown"));
            assert!(text.contains("existing lockfiles"));
        }
    }

    #[test]
    fn deletion_manifest_only_contains_removed_items() {
        let previous = json!({
            "items": [
                {
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": { "name": "a", "namespace": "ns" }
                },
                {
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": { "name": "b", "namespace": "ns" }
                }
            ]
        });
        let current = json!({
            "items": [
                {
                    "apiVersion": "v1",
                    "kind": "Service",
                    "metadata": { "name": "a", "namespace": "ns" }
                }
            ]
        });

        let deletion = deletion_manifest_for_removed_items(&previous, &current).unwrap();
        assert_eq!(deletion["items"].as_array().unwrap().len(), 1);
        assert_eq!(deletion["items"][0]["metadata"]["name"], "b");
    }

    #[test]
    fn render_workload_manifest_tracks_runtime_image_revision_in_pod_template() {
        let temp = tempdir().unwrap();
        let seed_root = temp
            .path()
            .join("agent-cluster/runtime-seed/hermes/memories");
        fs::create_dir_all(&seed_root).unwrap();
        fs::write(
            temp.path().join("agent-cluster/runtime-seed/FINITE.md"),
            "finite",
        )
        .unwrap();
        fs::write(
            temp.path()
                .join("agent-cluster/runtime-seed/hermes/config.yaml"),
            "display:\n  tool_progress: all\n",
        )
        .unwrap();
        fs::write(
            temp.path()
                .join("agent-cluster/runtime-seed/hermes/SOUL.md"),
            "soul",
        )
        .unwrap();
        fs::write(seed_root.join("MEMORY.md"), "memory").unwrap();
        fs::write(seed_root.join("USER.md"), "user").unwrap();

        let mut runtime_profiles = BTreeMap::new();
        runtime_profiles.insert(
            "main".to_string(),
            RuntimeProfileConfig {
                label: Some("Hermes Runtime".to_string()),
                image_name: Some("fc-agent-runtime".to_string()),
                image_tag: Some("2026-04-09-finited".to_string()),
                feature_set: Some("hermes-local".to_string()),
            },
        );
        let cluster = ClusterConfig {
            base_domain: Some("smoke.finite.computer".to_string()),
            default_runtime_profile: Some("main".to_string()),
            runtime_profiles: Some(runtime_profiles),
            agent_user: Some("node".to_string()),
            agent_uid: Some(1000),
            agent_gid: Some(1000),
            ..ClusterConfig::default()
        };
        let workload = WorkloadRecord {
            id: "smoke-fresh".to_string(),
            owner: "smoke-fresh".to_string(),
            owner_email: Some("test@finite.vip".to_string()),
            namespace: "smoke-fresh".to_string(),
            runtime_profile: "main".to_string(),
            home_volume_size: "20Gi".to_string(),
            opencode: WorkloadOpencode {
                port: 4101,
                hostname: "smoke-fresh-opencode.smoke.finite.computer".to_string(),
                project_dir: "/home/node/.hermes".to_string(),
                auth: EndpointAuth {
                    mode: "self".to_string(),
                    owner_email: Some("test@finite.vip".to_string()),
                    emails: Vec::new(),
                    org_domain: None,
                },
            },
            ssh: WorkloadSsh {
                enable: false,
                node_port: None,
            },
        };
        let revisions = BTreeMap::from([(
            "main".to_string(),
            RuntimeImageRevisionRecord {
                base_image: Some("fc-agent-runtime:2026-04-10-browser1".to_string()),
                image: Some("fc-agent-runtime:2026-04-09-finited".to_string()),
                image_name: Some("fc-agent-runtime".to_string()),
                image_tag: Some("2026-04-09-finited".to_string()),
                store_path: Some("/nix/store/runtime-main.tar.gz".to_string()),
            },
        )]);

        let manifest =
            render_workload_manifest(temp.path(), &cluster, &workload, &[], Some(&revisions))
                .unwrap();
        let items = manifest["items"].as_array().unwrap();
        let statefulset = items
            .iter()
            .find(|item| item["kind"] == "StatefulSet")
            .unwrap();

        assert_eq!(
            statefulset["spec"]["template"]["metadata"]["annotations"]["fc.finitecomputer.dev/runtime-image-revision"],
            "/nix/store/runtime-main.tar.gz"
        );
        assert_eq!(
            statefulset["spec"]["template"]["spec"]["containers"][0]["image"],
            "fc-agent-runtime:2026-04-09-finited"
        );
        assert_eq!(
            statefulset["spec"]["template"]["spec"]["automountServiceAccountToken"],
            false
        );
        assert_eq!(
            statefulset["spec"]["template"]["spec"]["containers"][0]["volumeMounts"][1]["mountPath"],
            "/profile-assets/hermes-local"
        );
        assert_eq!(
            statefulset["spec"]["template"]["spec"]["volumes"][0]["hostPath"]["path"],
            "/var/lib/finitecomputer/agent-cluster/profile-assets/hermes-local"
        );
    }
}
