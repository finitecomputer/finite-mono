# The lat-2 mover module (modules/finite-mover.nix) on a VM beside a Runner
# VM that carries finite-mover-host.nix. The mover is the real binary; Core's
# CLI is a stand-in that says who ran it, with what. Postgres holds Core's
# real schema and one live agent, whose tree is on the Runner.
#
# finite.chat is a fake on the mover VM's loopback, and neither VM can
# reach past QEMU (restrictNetwork), sandboxed or not: a test never touches
# production. The module has no default origin, so a config that forgets
# to name the fake fails to evaluate.
{
  nixpkgs,
  finitePackages,
}:
let
  pkgs = nixpkgs.legacyPackages.x86_64-linux;
  inherit (pkgs) lib;
  sshKeys = import (nixpkgs + "/nixos/tests/ssh-keys.nix") pkgs;

  migrations = ../../../finitecomputer-v2/crates/finite-saas-core/migrations;
  coreSchema = pkgs.writeText "core-schema.sql" (
    lib.concatMapStringsSep "\n" (name: builtins.readFile (migrations + "/${name}")) (
      lib.sort lib.lessThan (
        builtins.filter (name: builtins.match "[0-9]{4}_.*[.]sql" name != null) (
          builtins.attrNames (builtins.readDir migrations)
        )
      )
    )
  );
  # NIP-19's example npub: the key the live agent signs with, which places
  # its brains' owners (FIN-499).
  agentNpub = "npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg";

  # One live agent on the Runner, as Core records it.
  liveAgent = pkgs.writeText "live-agent.sql" ''
    INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
      VALUES ('u_one', 'one@example.test', 'linked', 'user_w1', now(), now());
    INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at)
      VALUES ('org_one', 'u_one', 'One', 'standard', now(), now());
    INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
      VALUES ('proj_live', 'org_one', 'u_one', 'Agent', now(), now());
    INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id, source_import_key, placement_runner_class, runtime_resource_class, host_facts, created_at, updated_at)
      VALUES ('rt_live', 'proj_live', 'runner', 'machine_live', 'key_live', 'kata', 'vcpu2_memory4_gib', '{}', now(), now());
    UPDATE agent_runtimes SET health_reporting_npub = '${agentNpub}' WHERE id = 'rt_live';
    INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
      VALUES ('link_live', 'proj_live', 'rt_live', true, now());
    INSERT INTO agent_creation_requests (id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name, runner_class, status, agent_runtime_id, runtime_spec, created_at, updated_at)
      VALUES ('request_live', 'org_one', 'u_one', 'proj_live', 'idem_live', 'Agent', 'kata', 'running', 'rt_live',
              '{"schema":"runtime_spec.v1","spec":{"projectId":"-","agentRuntimeId":"rt_live","durableStateId":"tree_live"}}'::jsonb, now(), now());
  '';

  # Core's CLI as the mover's wrapper runs it.
  stubCore = pkgs.writeShellScriptBin "finite-saas-core" ''
    echo "user=$(${pkgs.coreutils}/bin/id -un)"
    echo "database=''${FC_CORE_DATABASE_URL:+set}"
    echo "runtime_env=$FC_CORE_RUNTIME_ENV_JSON"
    printf 'arg=%s\n' "$@"
    case "$*" in *--exit-3*) exit 3 ;; esac
  '';
  testPackages = finitePackages // {
    finite-saas-core = stubCore;
  };

  # finite.chat's mover routes, as far as the timers go: an empty mailbox,
  # and a roster taken. Each request is logged as `METHOD PATH nostr` (or
  # `unsigned`), and the last roster kept.
  fakeOrigin = "http://127.0.0.1:8090";
  fakeState = "/var/lib/fake-finite-chat";
  fakeFiniteChat = pkgs.writeText "fake-finite-chat.py" ''
    import json
    from http.server import BaseHTTPRequestHandler, HTTPServer

    ANSWERS = {
        ("GET", "/api/migrations/mover/mailbox"): {"commands": []},
        ("PUT", "/api/migrations/mover/roster"): {"bound": 0, "withdrawn": 0, "replay": False},
    }


    class Handler(BaseHTTPRequestHandler):
        def answer(self):
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
            signed = (self.headers.get("Authorization") or "").startswith("Nostr ")
            with open("${fakeState}/requests", "a") as log:
                log.write(f"{self.command} {self.path} {'nostr' if signed else 'unsigned'}\n")
            if (self.command, self.path) == ("PUT", "/api/migrations/mover/roster"):
                with open("${fakeState}/roster.json", "wb") as roster:
                    roster.write(body)
            answer = ANSWERS.get((self.command, self.path))
            data = json.dumps(answer).encode() if answer is not None else b""
            self.send_response(200 if answer is not None else 404)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        do_GET = do_PUT = do_POST = answer


    HTTPServer(("127.0.0.1", 8090), Handler).serve_forever()
  '';
in
pkgs.testers.runNixOSTest {
  name = "finite-mover";
  node.specialArgs.finitePackages = testPackages;
  defaults.virtualisation.restrictNetwork = true;

  nodes.app = {
    imports = [ ../modules/finite-mover.nix ];
    virtualisation.memorySize = 2048;
    environment.systemPackages = [ pkgs.jq ];

    services.postgresql = {
      enable = true;
      ensureDatabases = [ "finite_core" ];
      ensureUsers = [ { name = "finite"; } ];
    };
    # Core's unit, as the wrapper reads it; it never starts here.
    systemd.services.finite-saas-core = {
      environment.FC_CORE_RUNTIME_ENV_JSON = builtins.toJSON {
        FINITE_SITES_API = "https://sites.example";
      };
      serviceConfig = {
        ExecStart = "${stubCore}/bin/finite-saas-core";
        EnvironmentFile = "/etc/finite/core.env";
      };
    };
    environment.etc."finite/core.env" = {
      text = "FC_CORE_DATABASE_URL=postgresql://test-only@127.0.0.1/finite_core\n";
      mode = "0600";
    };

    systemd.services.fake-finite-chat = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python3 ${fakeFiniteChat}";
        StateDirectory = "fake-finite-chat";
      };
    };

    finite.mover = {
      enable = true;
      finiteChat = fakeOrigin;
      core = {
        adminEmail = "mover@example.test";
        adminWorkosUserId = "user_mover";
      };
      hosts.runner = {
        address = "runner";
        hostKey = sshKeys.snakeOilPublicKey;
      };
    };
    specialisation.timers.configuration.finite.mover.timers.enable = true;

    # The Runner knows this ssh key, so it's placed before the mover's keys
    # are made, which must keep it.
    systemd.services.place-test-ssh-key = {
      requiredBy = [ "finite-mover-keys.service" ];
      before = [ "finite-mover-keys.service" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      serviceConfig.Type = "oneshot";
      script = ''
        [ -e /var/lib/finite-mover/ssh/id_ed25519 ] && exit 0
        install -o finite-mover -g finite-mover -m 0600 ${sshKeys.snakeOilEd25519PrivateKey} /var/lib/finite-mover/ssh/id_ed25519
      '';
    };
  };

  nodes.runner = {
    imports = [ ../modules/finite-mover-host.nix ];
    finite.moverHost = {
      authorizedKeys = [ sshKeys.snakeOilEd25519PublicKey ];
      workRoot = "/srv/work";
    };
    services.openssh = {
      enable = true;
      hostKeys = [ ];
      extraConfig = "HostKey /etc/ssh/test_host_key";
    };
    environment.etc."ssh/test_host_key" = {
      source = sshKeys.snakeOilPrivateKey;
      mode = "0600";
    };
    systemd.tmpfiles.rules = [
      "d /srv/work/kata/tree_live/work 0700 root root -"
      "f /srv/work/kata/tree_live/work/notes.md 0600 root root - hello"
      # Two brains, opened where fbrain puts them: both come, whoever owns them.
      "d /srv/work/kata/tree_live/workspace/finitebrain/brain-live/.finitebrain 0700 root root -"
      "f /srv/work/kata/tree_live/workspace/finitebrain/brain-live/page.md 0600 root root - brainpage"
      "d /srv/work/kata/tree_live/workspace/finitebrain/brain-other/.finitebrain 0700 root root -"
      "f /srv/work/kata/tree_live/workspace/finitebrain/brain-other/page.md 0600 root root - not-theirs"
    ];
  };

  testScript = ''
    import json
    import re
    import shlex

    state = "/var/lib/finite-mover"
    config = "/etc/finite-mover/config.json"
    npub_cmd = f"runuser -u finite-mover -- finite-mover --config {config} npub"

    def mover_key():
        return app.succeed(f"cat {state}/mover.key")

    start_all()
    runner.wait_for_unit("sshd.service")
    app.wait_for_unit("multi-user.target")

    with subtest("the mover key is made on the host, private, and never logged"):
        app.succeed("systemctl is-active finite-mover-keys.service")
        key = mover_key()
        assert re.fullmatch("[0-9a-f]{64}", key), "the key is 64 hex"
        assert app.succeed(f"stat -c '%a %U %G' {state}/mover.key").strip() == "600 finite-mover finite-mover"
        app.fail(f"journalctl -b | grep -F {key}")
        assert app.succeed(f"(grep -rlsF {key} /etc /nix/store/*finite-mover* || true) | wc -l").strip() == "0"

    with subtest("an ssh key already there is kept, and its public half printed"):
        app.succeed("cmp ${sshKeys.snakeOilEd25519PrivateKey} /var/lib/finite-mover/ssh/id_ed25519")
        app.succeed("journalctl -u finite-mover-keys -o cat | grep -F 'AAAAC3NzaC1lZDI1NTE5AAAAIDPQXmEVMVLmeFRyafKMVWgPDkv8/uRBTwmcEDatZzMD'")

    with subtest("npub prints, the same across a restart and a reboot"):
        npub = app.succeed(npub_cmd).strip()
        assert re.fullmatch("npub1[02-9ac-hj-np-z]{58}", npub), npub
        app.succeed("systemctl restart finite-mover-keys.service")
        assert mover_key() == key
        app.shutdown()
        app.start()
        app.wait_for_unit("multi-user.target")
        assert mover_key() == key
        assert app.succeed(npub_cmd).strip() == npub
        app.fail("journalctl -b -u finite-mover-keys -o cat | grep 'made the'")

    with subtest("the config is root's, readable by the mover's group alone"):
        assert app.succeed(f"stat -L -c '%a %U %G' {config}").strip() == "640 root finite-mover"
        assert json.loads(app.succeed(f"cat {config}"))["overrides"] == f"{state}/overrides.json"
        app.fail(f"runuser -u nobody -- cat {config}")

    with subtest("the mover's finite.chat is the fake on loopback, never the real one"):
        assert json.loads(app.succeed(f"cat {config}"))["finite_chat"] == "${fakeOrigin}"
        status, _ = app.execute("grep -RqF finite.chat /etc/finite-mover")
        assert status == 1, status

    with subtest("the timers are off by default"):
        app.fail("systemctl cat finite-mover-mailbox.timer")
        assert "finite-mover" not in app.succeed("systemctl list-timers --all --no-legend")

    with subtest("without Core's tables, the grant fails visibly"):
        app.succeed("systemctl is-failed finite-mover-postgres.service")

    with subtest("with them, the role reads its columns by peer authentication, and nothing else"):
        psql = "runuser -u postgres -- psql -v ON_ERROR_STOP=1 -d finite_core"
        app.succeed("runuser -u postgres -- psql -v ON_ERROR_STOP=1 -c 'ALTER DATABASE finite_core OWNER TO finite'")
        app.succeed(f"{psql} -c 'SET ROLE finite' -f ${coreSchema} -f ${liveAgent}")
        app.succeed("systemctl restart finite-mover-postgres.service")
        conninfo = "'host=/run/postgresql user=finite_mover dbname=finite_core'"
        as_mover = f"runuser -u finite-mover -- psql {conninfo} -v ON_ERROR_STOP=1 -tAc"
        assert app.succeed(f"{as_mover} 'SELECT workos_user_id FROM users'").strip() == "user_w1"
        # The keys that place each brain's owner (FIN-499).
        app.succeed(f"{as_mover} 'SELECT user_id, public_key_hex, status FROM account_brain_principals'")
        app.succeed(f"{as_mover} 'SELECT owner_chat_account_id FROM agent_creation_requests'")
        app.fail(f"{as_mover} 'SELECT * FROM users'")
        app.fail(f"{as_mover} \"UPDATE projects SET display_name = 'x'\"")
        app.fail(f"runuser -u nobody -- psql {conninfo} -c 'SELECT 1'")

    with subtest("sizes reads Core's database and the Runner's tree over ssh"):
        app.succeed("systemctl start finite-mover-sizes.service")
        out = app.succeed("journalctl -u finite-mover-sizes -o cat")
        assert '"measured":1' in out and '"failed":0' in out, out
        app.succeed(f"jq -e 'length > 0' {state}/sizes.json")
        # Every brain counts in its move: "brainpage" and "not-theirs".
        moved = app.succeed(f"jq -e '.agents.proj_live.move' {state}/sizes.json").strip()
        assert moved == "19", moved

    with subtest("root's overrides file tweaks the next run, with no release"):
        app.succeed(
            f"echo '{{\"*\": {{\"leave\": [\"workspace/finitebrain/brain-other\"]}}}}' > {state}/overrides.json"
        )
        app.succeed("systemctl start finite-mover-sizes.service")
        moved = app.succeed(f"jq -e '.agents.proj_live.move' {state}/sizes.json").strip()
        assert moved == "9", moved
        app.succeed(f"rm {state}/overrides.json")

    program = json.loads(app.succeed(f"cat {config}"))["core"]["program"]
    core = " ".join(shlex.quote(word) for word in program)

    with subtest("Core's four subcommands run as Core's user, in Core's environment"):
        for subcommand in ["runtime-stop-exact", "runtime-retire-exact", "runtime-mark-moved-exact", "runtime-restart-exact"]:
            out = app.succeed(f"runuser -u finite-mover -- {core} {subcommand} --project-id p")
            assert "user=finite-saas-core" in out, out
            assert "database=set" in out, out
            assert 'runtime_env={"FINITE_SITES_API":"https://sites.example"}' in out, out
            assert f"arg={subcommand}" in out, out
        out = app.succeed(f"runuser -u finite-mover -- {core} runtime-stop-exact --admin-email '$FC_CORE_DATABASE_URL' '%u'")
        assert "arg=$FC_CORE_DATABASE_URL" in out and "arg=%u" in out, out
        status, _ = app.execute(f"runuser -u finite-mover -- {core} runtime-stop-exact --exit-3")
        assert status == 3, status
        # As the mover's own services run it.
        app.succeed(
            "systemd-run --wait --pipe --quiet -p User=finite-mover -p ProtectSystem=strict"
            f" -p ReadWritePaths={state} -p ProtectHome=yes -p PrivateTmp=yes"
            f" -- {core} runtime-stop-exact --project-id p | grep -x user=finite-saas-core"
        )

    with subtest("sudo refuses a fifth subcommand, and anything else"):
        # sudo itself refuses these, before the wrapper would.
        for other in ["runtime-cold-relocate-exact", "runtime-destroy-exact", "--version"]:
            status, out = app.execute(f"runuser -u finite-mover -- {core} {other} --project-id p 2>&1")
            assert status == 1 and "a password is required" in out, (other, status, out)
        status, out = app.execute(f"runuser -u finite-mover -- {core} 'runtime-stop-exact --project-id' p 2>&1")
        assert status == 64 and "only runtime-stop-exact" in out, (status, out)
        app.fail("runuser -u finite-mover -- sudo -n id")
        app.fail(f"runuser -u finite-mover -- sudo -n {program[2]}")

    with subtest("a missing ssh key is made once, and its public half printed"):
        app.succeed(f"rm {state}/ssh/id_ed25519 {state}/ssh/id_ed25519.pub")
        app.succeed("systemctl restart finite-mover-keys.service")
        public = app.succeed(f"cat {state}/ssh/id_ed25519.pub").strip()
        assert public.startswith("ssh-ed25519 "), public
        assert app.succeed(f"stat -c '%a %U' {state}/ssh/id_ed25519").strip() == "600 finite-mover"
        app.succeed(f"journalctl -u finite-mover-keys -o cat | grep -F {shlex.quote(public)}")
        app.succeed("systemctl restart finite-mover-keys.service")
        assert app.succeed(f"cat {state}/ssh/id_ed25519.pub").strip() == public
        assert mover_key() == key

    with subtest("timers.enable turns the three timers on, and the mailbox is read, signed"):
        switch = "/run/current-system/specialisation/timers/bin/switch-to-configuration"
        # It stops and restarts nothing. A dry run doesn't name the new
        # timers: they start with timers.target, which is already active.
        dry = app.succeed(f"{switch} dry-activate 2>&1")
        assert not [line for line in dry.splitlines() if line.startswith(("would stop", "would restart"))], dry
        app.succeed(f"{switch} test")
        timers = app.succeed("systemctl list-timers --all --no-legend")
        for name in ["finite-mover-mailbox", "finite-mover-roster", "finite-mover-sizes"]:
            assert f"{name}.timer" in timers, timers
        app.wait_until_succeeds(
            "grep -qx 'GET /api/migrations/mover/mailbox nostr' ${fakeState}/requests", timeout=60
        )
        # A run the timer started succeeded; the next is due in 10 s.
        app.wait_until_succeeds(
            "journalctl -u finite-mover-mailbox -o cat | grep -q 'Finished finite-mover mailbox'", timeout=60
        )
        app.succeed("systemctl start finite-mover-mailbox.service")

    with subtest("the roster pushes every live agent from Core's database"):
        app.succeed("systemctl start finite-mover-roster.service")
        taken = set(app.succeed("journalctl -u finite-mover-roster -o cat | grep -F bound").splitlines())
        assert taken == {'{"bound":0,"withdrawn":0,"replay":false}'}, taken
        app.succeed("grep -qx 'PUT /api/migrations/mover/roster nostr' ${fakeState}/requests")
        roster = json.loads(app.succeed("cat ${fakeState}/roster.json"))
        agents = [agent["old"] for person in roster["people"] for agent in person["agents"]]
        assert agents == ["proj_live"], roster
        # Nothing else was asked of finite.chat, and nothing unsigned.
        asked = set(app.succeed("cat ${fakeState}/requests").splitlines())
        assert asked == {
            "GET /api/migrations/mover/mailbox nostr",
            "PUT /api/migrations/mover/roster nostr",
        }, asked
  '';
}
