#!/usr/bin/env bash
# Deploys the finitechat server to its FUTURE home on lat1 (finite-lat-1,
# 64.34.82.77). The LIVE server today runs on clawland (15.204.108.57 —
# chat.finite.computer resolves there). The 2026-07-07 lat1 deploy attempt
# was rolled back after ~2 minutes; leftovers remain in /var/lib/finite-chat
# on lat1 (see ../README.md, captured-state appendix).
#
# Known host mismatches (from the 2026-07-08 capture) — this script was
# written for a NixOS/Traefik host, and lat1 is neither:
#   - the remote build uses `nix shell`; lat1 has no /nix (the Jul 7 run
#     built with cargo directly, so the script actually piped that day
#     differed from this copy)
#   - it applies Traefik IngressRoute CRDs; lat1 k3s runs --disable=traefik
#     and has no IngressRoute CRD (edge is host Caddy)
#   - the finite-chat user shell /run/current-system/sw/bin/nologin is a
#     dangling NixOS path on Ubuntu
# Reconcile these before the real migration.
#
# Formerly finitecomputer-v2/scripts/deploy_finitechat_server_lat1.sh
# (paths below adapted for the new location under infra/hosts/lat1/scripts/).
set -euo pipefail

workspace_rel="${1:-finitecomputer-v2/deploy/finite-chat/lat1}"
git_ref="${2:-}"

# finite-mono root (this script lives at infra/hosts/lat1/scripts/).
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
workspace_root="${repo_root}/${workspace_rel}"
if [[ -n "${FINITECHAT_REPO_PATH:-}" ]]; then
  chat_repo="${FINITECHAT_REPO_PATH}"
elif [[ -d "${repo_root}/finite-chat-darkmatter/.git" ]]; then
  chat_repo="${repo_root}/finite-chat-darkmatter"
else
  chat_repo="${repo_root}/finitechat"
fi
git_url="${FINITECHAT_GIT_URL:-https://github.com/finitecomputer/finitechat.git}"

if [[ ! -d "${workspace_root}" ]]; then
  echo "workspace not found: ${workspace_root}" >&2
  exit 1
fi

if [[ -z "${git_ref}" ]]; then
  git_ref="$(git -C "${chat_repo}" rev-parse HEAD)"
fi

if [[ ! "${git_ref}" =~ ^[0-9a-f]{40}$ ]]; then
  git_ref="$(git -C "${chat_repo}" rev-parse "${git_ref}")"
fi

set -a
if [[ -f "${workspace_root}/secrets/workspace.env" ]]; then
  # shellcheck disable=SC1091
  source "${workspace_root}/secrets/workspace.env"
elif [[ -f "${workspace_root}/workspace.env" ]]; then
  # shellcheck disable=SC1091
  source "${workspace_root}/workspace.env"
fi
set +a

if [[ -z "${HOST_SSH_HOST:-}" ]]; then
  echo "Set HOST_SSH_HOST in ${workspace_root}/secrets/workspace.env." >&2
  echo "Use ${workspace_root}/workspace.env.example as the template." >&2
  exit 1
fi

remote="${HOST_SSH_USER:-root}@${HOST_SSH_HOST}"
port="${HOST_SSH_PORT:-22}"
ssh_args=(-p "${port}" -o StrictHostKeyChecking=accept-new)

echo "Deploying finitechat-server ${git_ref} to ${remote} (${workspace_rel}) ..."

ssh "${ssh_args[@]}" "${remote}" "bash -s" -- "${git_url}" "${git_ref}" <<'REMOTE'
set -euo pipefail

git_url="${1:?git url required}"
git_ref="${2:?git ref required}"

state_root="/var/lib/finite-chat"
src_dir="${state_root}/src"
release_dir="${state_root}/releases"
bin_dir="${state_root}/bin"
data_dir="${state_root}/data"
bind_addr="${FINITECHAT_BIND_ADDR:-10.42.0.1:8787}"
sqlite_path="${data_dir}/server.sqlite3"
systemd_unit_dir="${FINITECHAT_SYSTEMD_UNIT_DIR:-/run/systemd/system}"
tmp_manifest="$(mktemp)"

cleanup() {
  rm -f "${tmp_manifest}"
}
trap cleanup EXIT

install -d -m 0755 "${state_root}" "${src_dir}" "${release_dir}" "${bin_dir}" "${data_dir}"

if ! getent group finite-chat >/dev/null 2>&1; then
  groupadd --system finite-chat
fi

if ! id finite-chat >/dev/null 2>&1; then
  useradd --system --gid finite-chat --home-dir "${state_root}" --shell /run/current-system/sw/bin/nologin finite-chat
elif [[ "$(id -gn finite-chat)" != "finite-chat" ]]; then
  usermod --gid finite-chat finite-chat
fi

chown root:root "${state_root}" "${src_dir}" "${release_dir}" "${bin_dir}"
if [[ -d "${src_dir}/.git" ]]; then
  chown -R root:root "${src_dir}"
fi
chown -R finite-chat:finite-chat "${data_dir}"

if [[ ! -d "${src_dir}/.git" ]]; then
  rm -rf "${src_dir}"
  git clone "${git_url}" "${src_dir}"
fi

git -C "${src_dir}" remote set-url origin "${git_url}"
git -C "${src_dir}" fetch --prune origin
git -C "${src_dir}" checkout --detach "${git_ref}"
git -C "${src_dir}" reset --hard "${git_ref}"

echo "Building finitechat-server ${git_ref} on $(hostname) ..."
(
  cd "${src_dir}"
  nix --extra-experimental-features 'nix-command flakes' shell \
    nixpkgs#cargo \
    nixpkgs#rustc \
    nixpkgs#gcc \
    nixpkgs#pkg-config \
    -c cargo build --release -p finitechat-server
)

install -m 0755 "${src_dir}/target/release/finitechat-server" "${release_dir}/finitechat-server-${git_ref}"
ln -sfn "${release_dir}/finitechat-server-${git_ref}" "${bin_dir}/finitechat-server"
chown -R root:root "${src_dir}" "${release_dir}" "${bin_dir}"
chown -R finite-chat:finite-chat "${data_dir}"

install -d -m 0755 "${systemd_unit_dir}"
cat >"${systemd_unit_dir}/finitechat-server.service" <<UNIT
[Unit]
Description=Finite Chat room server
After=network-online.target k3s.service
Wants=network-online.target

[Service]
User=finite-chat
Group=finite-chat
WorkingDirectory=${state_root}
ExecStart=${bin_dir}/finitechat-server serve ${bind_addr} --sqlite ${sqlite_path}
Restart=always
RestartSec=2
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=${state_root}

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --runtime finitechat-server.service >/dev/null 2>&1 || true
systemctl restart finitechat-server.service

for _ in $(seq 1 30); do
  if curl -fsS "http://${bind_addr}/health" >/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS "http://${bind_addr}/health" | jq .

cat >"${tmp_manifest}" <<'YAML'
apiVersion: v1
kind: Namespace
metadata:
  name: fc-chat
---
apiVersion: v1
kind: Service
metadata:
  name: finitechat-server
  namespace: fc-chat
spec:
  ports:
    - name: http
      port: 8787
      targetPort: 8787
---
apiVersion: v1
kind: Endpoints
metadata:
  name: finitechat-server
  namespace: fc-chat
subsets:
  - addresses:
      - ip: 10.42.0.1
    ports:
      - name: http
        port: 8787
---
apiVersion: traefik.io/v1alpha1
kind: IngressRoute
metadata:
  name: finitechat-server-canonical
  namespace: fc-chat
spec:
  entryPoints:
    - websecure
  routes:
    - kind: Rule
      match: Host(`chat.finite.computer`)
      services:
        - name: finitechat-server
          port: 8787
  tls:
    certResolver: letsencrypt
---
apiVersion: traefik.io/v1alpha1
kind: IngressRoute
metadata:
  name: finitechat-server-temporary-vip
  namespace: fc-chat
spec:
  entryPoints:
    - websecure
  routes:
    - kind: Rule
      match: Host(`chat.finite.vip`)
      services:
        - name: finitechat-server
          port: 8787
  tls:
    certResolver: letsencrypt
YAML

k3s kubectl apply -f "${tmp_manifest}"
k3s kubectl -n fc-chat get service finitechat-server
k3s kubectl -n fc-chat get endpoints finitechat-server
k3s kubectl -n fc-chat get ingressroute

echo "finitechat-server deploy metadata:"
jq -n \
  --arg git_url "${git_url}" \
  --arg git_ref "${git_ref}" \
  --arg bind_addr "${bind_addr}" \
  --arg sqlite_path "${sqlite_path}" \
  --arg deployed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{git_url: $git_url, git_ref: $git_ref, bind_addr: $bind_addr, sqlite_path: $sqlite_path, deployed_at: $deployed_at}' \
  | tee "${state_root}/last-deploy.json"
REMOTE

echo "Checking temporary public alias ..."
vip_ok=0
for _ in $(seq 1 30); do
  if curl -fsS "https://chat.finite.vip/health" | jq .; then
    vip_ok=1
    break
  fi
  sleep 2
done
if [[ "${vip_ok}" != "1" ]]; then
  echo "warning: https://chat.finite.vip/health is not passing with a trusted certificate yet." >&2
  echo "Host-local health passed; check Traefik ACME logs before treating the public alias as usable." >&2
fi

if dig +short chat.finite.computer A | grep -q .; then
  echo "Checking canonical public URL ..."
  curl -fsS "https://chat.finite.computer/health" | jq .
else
  echo "chat.finite.computer does not resolve yet. Add an A record to ${HOST_SSH_HOST}; temporary URL is https://chat.finite.vip."
fi
