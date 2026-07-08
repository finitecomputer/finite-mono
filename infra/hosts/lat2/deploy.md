# Deploying finite-sites to finite-lat-2

## Today's flow — DEPRECATED (build-on-box)

Documented in full in `finite-sites/docs/deploy-finite-lat-2.md` (§3 box
setup, §5a routine rollout); this is a summary, not a replacement. The flow
still works and is what produced the running v0.2.16 binaries:

1. From a dev machine: `rsync -az --delete` the finite-sites source to
   `finite-lat-2:~/finite-sites/` (excluding `.git`, `target`, env files).
2. On the box: `cargo build --release`.
3. On the box: `sudo install -m 0755 target/release/{finitesitesd,fsite}
   /usr/local/bin/` — previous binaries are kept alongside as `*.prev-<stamp>`
   (e.g. `finitesitesd.prev-20260619T155747Z`), which is the rollback path.
4. `sudo systemctl daemon-reload && sudo systemctl restart finite-saas-sites`,
   then curl the public health/serving endpoints.
5. Unit/Caddy changes: `sudo install` the files from this tree
   (`infra/hosts/lat2/systemd/`, `infra/hosts/lat2/caddy/Caddyfile`), then
   daemon-reload / `systemctl reload caddy`.

Why deprecated (infra/README.md deploy principles 1–2): the on-box checkout
is rsync'd, **not a git repo** — the running binaries have no commit
provenance; the build toolchain lives on a prod box; and "what was on the
box" is unreproducible from a tag. Since `.git` is excluded by the rsync,
provenance is structurally absent, not just unrecorded.

## Target flow — binaries from release tags

1. **CI builds** `finitesitesd` + `fsite` at a component release tag
   (`fsite/v*`, per the mono tag scheme) as release artifacts, on a runner
   with the right labels (see `runners.md` — cutover to finite-mono
   required first).
2. **Deploy script** (to live in `infra/runbooks/`, idempotent, takes an
   explicit tag): downloads the tagged artifacts and rsyncs the **binaries
   only** — never source — to the box; `sudo install -m 0755` to
   `/usr/local/bin/`, keeping the `*.prev-<stamp>` copies exactly as today;
   restarts `finite-saas-sites`.
3. **Verify what was deployed**: the script must confirm the running version
   matches the tag before declaring success — today that is
   `finitesitesd --version` (0.2.16 at capture) plus
   `curl https://api.finite.chat/api/v1/healthz`; the finitechat-style
   contract gate (a health endpoint reporting `source_commit`) is the bar
   once finitesitesd exposes it.
4. Rollback = `sudo install` the `.prev` binary back and restart, same as
   today.

Config (units, Caddyfile, polkit, sudoers) deploys from this tree, not from
the sites source checkout; the on-box `~/finite-sites` checkout stops being
a deploy input entirely and can be deleted once the target flow lands.
