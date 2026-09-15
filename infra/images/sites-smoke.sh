#!/usr/bin/env bash
# Exercises the exact image with disposable state, never a production volume.
set -euo pipefail
image="${1:?usage: bash infra/images/sites-smoke.sh IMAGE}"
name="sites-smoke-$$-$RANDOM"
source_name="$name"
volume="$name-data"
source_volume="$volume"
backup_volume="$name-backup"
restore_volume="$name-restore"
backup_mode="${SITES_BACKUP_SMOKE:-0}"
completed=0
cleanup() {
    status=$?
    if [ "$status" -eq 0 ] && [ "$completed" != 1 ]; then status=1; fi
    if [ "$status" -ne 0 ]; then docker logs "$name" >&2 || true; fi
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker rm -f "$source_name" >/dev/null 2>&1 || true
    docker volume rm "$volume" >/dev/null 2>&1 || true
    docker volume rm "$source_volume" "$backup_volume" "$restore_volume" >/dev/null 2>&1 || true
    exit "$status"
}
trap cleanup EXIT

docker volume create "$volume" >/dev/null
if [ "$backup_mode" = 1 ]; then
    docker volume create "$backup_volume" >/dev/null
    docker run --rm --entrypoint /bin/sh \
        --mount "type=volume,src=$backup_volume,dst=/backup-test" "$image" -eu -c '
        umask 077
        mkdir /backup-test/credentials
        python3 -c "import secrets; print(secrets.token_hex(32))" > /backup-test/credentials/borg-passphrase
        printf synthetic-local-test > /backup-test/credentials/id_ed25519
        printf synthetic-local-test > /backup-test/credentials/known_hosts
        export BORG_REPO=/backup-test/repository BORG_PASSCOMMAND="cat /backup-test/credentials/borg-passphrase"
        borg init --encryption=repokey-blake2
        borg key export :: /backup-test/exported-key
    '
fi
if docker run --rm "$image" serve --data /var/lib/finite-sites --mailer dev; then
    echo 'Sites unexpectedly started without a data volume' >&2
    exit 1
fi
start() {
    extra=(-e FINITE_SITES_BACKUP_ENABLED=0)
    if [ "$backup_mode" = 1 ]; then
        extra=(--mount "type=volume,src=$backup_volume,dst=/backup-test"
            -e FINITE_SITES_BACKUP_ENABLED=1
            -e FINITE_SITES_BACKUP_REPOSITORY=/backup-test/repository
            -e FINITE_SITES_BACKUP_CREDENTIALS_DIR=/backup-test/credentials)
    fi
    docker run -d --name "$name" -p 127.0.0.1::8787 \
        --mount "type=volume,src=$volume,dst=/var/lib/finite-sites" \
        "${extra[@]}" \
        "$image" serve --data /var/lib/finite-sites --listen 0.0.0.0:8787 \
        --api-url http://127.0.0.1:8787 --git-url http://127.0.0.1:8787 \
        --base-domain sites.localhost --mailer dev >/dev/null
    endpoint="http://$(docker port "$name" 8787/tcp)"
    for _ in {1..30}; do
        if curl -fsS "$endpoint/api/v2/healthz" >/dev/null 2>&1; then break; fi
        sleep 1
    done
    curl -fsS "$endpoint/api/v2/healthz"
    if [ "$backup_mode" = 1 ]; then
        docker exec "$name" sh -c 'pid=$(supervisorctl -c /run/sites-supervisor.conf pid sites); test "$(awk "/^Uid:/ {print \$2}" /proc/$pid/status)" = 65532'
        if docker exec --user 65532:65532 "$name" supervisorctl -c /run/sites-supervisor.conf status; then
            echo 'Serving user unexpectedly accessed the supervisor' >&2
            exit 1
        fi
    else
        docker exec "$name" sh -c 'test "$(awk "/^Uid:/ {print \$2}" /proc/1/status)" = 65532'
    fi
}
client() {
    docker exec -i --user 65532:65532 \
        -e HOME=/var/lib/finite-sites/smoke-client \
        -e FINITE_HOME=/var/lib/finite-sites/smoke-client/.finite \
        -e FINITE_SITES_API=http://127.0.0.1:8787 \
        -e GIT_TERMINAL_PROMPT=0 "$name" sh -eu
}
private_site() {
    test "$(curl -sS -o /dev/null -w '%{http_code}' \
        -H 'Host: smoke.sites.localhost' "$endpoint/")" = 401
}
stop() {
    docker stop --time 30 "$name" >/dev/null
    test "$(docker inspect --format '{{.State.ExitCode}}' "$name")" = 0
}

start
client <<'SH'
mkdir -p "$HOME/site"
cd "$HOME"
printf '[project]\nslug = "smoke"\n[site]\nname = "smoke"\nbranch = "main"\npath = "site"\n' > finite.toml
printf 'sites-image-first-version\n' > site/index.html
fsite auth register --output json
fsite auth sites-key request smoke@example.com
# Only this disposable test's dev-mail outbox is read; never print the token.
token="$(awk '$1 == "fsite" && $2 == "auth" && $3 == "redeem" {print $5}' /var/lib/finite-sites/outbox/*)"
test -n "$token"
fsite auth sites-key add smoke@example.com "$token" --output json
unset token
fsite project init --config finite.toml --owner-email smoke@example.com --output json
fsite auth git smoke --store --output json
git init -b main
git config user.name 'Sites image smoke'
git config user.email 'smoke@example.invalid'
git remote add finite http://127.0.0.1:8787/smoke.git
git add finite.toml site
git commit -m 'Initial smoke version'
git push finite main
fsite project share smoke --public --yes-public --output json
SH
test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-first-version
client <<'SH'
fsite project share smoke --private --output json
SH
private_site
secret_hash="$(docker exec "$name" sha256sum /var/lib/finite-sites/cookie-secret)"
stop
docker start "$name" >/dev/null
endpoint="http://$(docker port "$name" 8787/tcp)"
# Replacement below also verifies state is not tied to the container's filesystem.
for _ in {1..30}; do
    if curl -fsS "$endpoint/api/v2/healthz" >/dev/null 2>&1; then break; fi
    sleep 1
done
private_site
stop
docker rm "$name" >/dev/null
start
test "$secret_hash" = "$(docker exec "$name" sha256sum /var/lib/finite-sites/cookie-secret)"
private_site
client <<'SH'
fsite project status smoke --output json
fsite project share smoke --public --yes-public --output json
SH
test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-first-version
client <<'SH'
cd "$HOME"
printf 'sites-image-second-version\n' > site/index.html
git add site/index.html
git commit -m 'Update after container replacement'
git push finite main
SH
test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-second-version
if [ "$backup_mode" = 1 ]; then
    docker exec "$name" sites-backup run --config /run/sites-backup.json
    docker exec "$name" finite-status --sites-backup-state /var/backups/finite-sites/status.json --json
    docker exec "$name" cp /var/backups/finite-sites/status.json /backup-test/status.json
    test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-second-version
    source_endpoint="$endpoint"
    docker volume create "$restore_volume" >/dev/null
    docker run --rm --entrypoint /bin/sh \
        --mount "type=volume,src=$backup_volume,dst=/backup-test" \
        --mount "type=volume,src=$restore_volume,dst=/var/lib/finite-sites" "$image" -eu -c '
        umask 077
        export BORG_REPO=/backup-test/repository BORG_PASSCOMMAND="cat /backup-test/credentials/borg-passphrase"
        archive=$(python3 -c "import json; print(json.load(open(\"/backup-test/status.json\"))[\"archive\"])")
        mkdir /tmp/extracted
        cd /tmp/extracted
        borg check --verify-data "::$archive"
        borg extract "::$archive"
        sites-backup restore --snapshot /tmp/extracted/snapshot --target /tmp/verified-sites
        test -z "$(ls -A /var/lib/finite-sites)"
        rsync -a /tmp/verified-sites/ /var/lib/finite-sites/
    '
    name="$source_name-restored"
    volume="$restore_volume"
    backup_mode=0
    start
    test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-second-version
    client <<'SH'
cd "$HOME"
git clone http://127.0.0.1:8787/smoke.git restored-clone
cd restored-clone
git config user.name 'Sites restore smoke'
git config user.email 'restore@example.invalid'
printf 'sites-image-restored-third-version\n' > site/index.html
git add site/index.html
git commit -m 'Publish using original credential after Borg restore'
git push origin main
fsite project share smoke --private --output json
SH
    private_site
    client <<'SH'
fsite project share smoke --public --yes-public --output json
SH
    test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$endpoint/")" = sites-image-restored-third-version
    test "$(curl -fsS -H 'Host: smoke.sites.localhost' "$source_endpoint/")" = sites-image-second-version
    echo 'Sites backup smoke passed: supervised capture/restart, encrypted Borg, fresh-client restore, original-credential clone/push and preserved sharing.'
fi
stop
echo 'Sites image smoke passed: publishing, visibility, non-root serving, restart, replacement, cookie-secret persistence and SIGINT shutdown.'
completed=1
