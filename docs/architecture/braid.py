"""The braid — every shared thing in finite-mono, tracked so it can be cut.

A SHARED THING is anything where changing it forces coordinated changes in
more than one place: a wire type, a protocol implementation, a file format,
a concept implemented twice, a duplicated algorithm, an implicit coupling,
a shared schema, or a build unit. The platform rule this registry enforces:
**every shared thing must either become a self-service interface with ONE
owner, or die.** No third state.

Sources: 11 read-only audit workers + mechanical scans (scripts/discover-braid),
coordinator-validated 2026-08-31 against origin/main @ 68236fb8 (spot-checks
passed on the six most load-bearing claims). Evidence is file:line at that
rev. Rows are the argument list — each one is either getting an owner or
getting deleted.

Dispositions:
  interface — should become (or already nearly is) a self-service interface
              with one named owner
  die       — should be deleted; nothing earns the complexity
  contested — a real decision owed: not obviously either, or awaiting a
              campaign decision

Fields: owner is the lane that SHOULD own the interface (usually "none"
today — that is the finding).
"""

MAIN_REV = "31bf31d6"
UPDATED = "2026-09-02"

LANES = {
    "finitechat": "chat server, devices, clients, hermes plugin",
    "finite-brain": "brain server + fbrain CLI",
    "finite-sites": "finitesitesd + fsite CLI",
    "finite-identity": "identity library + NIP-05 directory service",
    "finite-nostr": "reusable nostr primitives",
    "finite-mail": "mail transport library",
    "finite-agentd": "agent-owned platform boundary in the runtime",
    "core": "finite-saas-core + limiter + saas-local",
    "runner": "runner + specialization worker + runtime images",
    "dashboard": "Next.js dashboard",
    "devfinity": "local full-stack harness",
    "skills": "managed skills baseline + ab-testing",
    "infra": "nixos, deploy, runbooks, scripts/",
    "scripts": "operator scripts (finite-status, rollout, checks)",
}

SHARED_THINGS = [
    # ---- the nostr/identity substrate --------------------------------------
    dict(id="npub-codec-x4", name="npub/nsec bech32 codec, four copies",
         kind="dup-logic", touched=["finite-identity", "finite-sites", "finitechat", "finite-nostr"],
         owner="finite-nostr", disposition="interface", confidence="high",
         evidence=["finite-identity/src/npub.rs:7-24", "finite-sites/crates/finitesites-proto/src/npub.rs:8-38",
                   "finitechat/crates/finitechat-proto/src/nostr.rs:26-98", "finite-nostr/src/identity.rs:46-53",
                   "finite-identity/Cargo.toml:18-19 (bech32 version-pinned by comment only)"],
         story="Four implementations of NIP-19 encoding held together by prose promises and "
               "version pins — a fifth copy is one paste away and none of them is authoritative."),
    dict(id="nip98-x5", name="NIP-98 signed requests, five implementations",
         kind="protocol-impl", touched=["finite-sites", "finite-nostr", "finite-brain", "finite-identity", "dashboard"],
         owner="finite-nostr", disposition="interface", confidence="high",
         evidence=["finitechat/crates/finitechat-server/Cargo.toml:15 (chat CONSOLIDATED onto finite-nostr — merged)",
                   "finite-sites/crates/finitesites-proto/src/nip98.rs:13,42 (raw schnorr, 60s skew, exact-string URL — branch cleanup/single-nip98 @ 2a473720 ready, unmerged)",
                   "finite-brain/crates/finite-brain-core/src/lib.rs:1967 (verifies exact tag/kind/content rules)",
                   "finite-identity/src/nip98.rs:13",
                   "dashboard/src/lib/brain-hosted-client.ts:84-108 (hand-builds kind-27235 event in TypeScript)"],
         story="Chat landed on finite-nostr; the sites branch is reviewed and waiting. Remaining after merge: "
               "brain-core's verifier, identity's copy, and the TS signer in the dashboard."),
    dict(id="identity-client-dead-routes", name="identity client builders for deleted routes",
         kind="wire-type", touched=["finite-identity"],
         owner="finite-identity", disposition="die", confidence="high",
         evidence=["finite-identity/src/client.rs:81-111 (email-only-principals/redeem, mailbox-proofs/redeem)",
                   "finite-identity/src/client.rs:115-129 (satisfies-grant)",
                   "finite-identity/src/authority.rs:517-527 (public router mounts none of them)"],
         story="The auth kernel shrank the authority but not the client: three request builders and error "
               "mappings now target 404s, referenced by nothing outside the crate's own tests."),
    dict(id="identity-crate-lib-plus-service", name="finite-identity bundles key-file lib with directory service",
         kind="build-unit", touched=["finite-identity", "finitechat", "finite-sites", "finite-brain"],
         owner="finite-identity", disposition="interface", confidence="high",
         evidence=["finite-identity/Cargo.toml:14-36 (axum, rusqlite, tokio, finite-mail non-optional)",
                   "finite-identity/src/lib.rs:20-25 (authority/client/nip98 modules in the key-loader lib)",
                   "finitechat/crates/finitechat-core/Cargo.toml:13 (wants only the loader)",
                   "finite-sites/crates/fsite-cli/Cargo.toml:15", "finite-brain/crates/finite-brain-cli/Cargo.toml:20"],
         story="A Directory service change (route, schema, mail transport) forces rebuilds of chat/sites/brain "
               "binaries that linked the crate only for the 200-line identity.json loader."),
    dict(id="identity-base-url-x5", name="identity base URL + env name copied in five components",
         kind="implicit-coupling", touched=["finite-identity", "finite-brain", "finite-sites", "runner", "devfinity"],
         owner="finite-identity", disposition="interface", confidence="high",
         evidence=["finite-brain/crates/finite-brain-cli/src/environment.rs:14,69",
                   "finite-sites/crates/fsite-cli/src/api.rs:24-25", "finite-saas-runner/src/main.rs:594-604",
                   "devfinity/src/lib.rs:2071,3575", "finite-identity/scripts/identity-edge-contract-gate.py:31"],
         story="'https://identity.finite.vip' and FINITE_IDENTITY_AUTHORITY are string literals in five "
               "places while finite-identity exports neither — moving the service is grep-and-hope."),
    dict(id="nip05-localpart-policy-x3", name="valid NIP-05 local-part defined three times",
         kind="concept", touched=["finite-identity", "finite-nostr", "finite-brain"],
         owner="finite-nostr", disposition="interface", confidence="high",
         evidence=["finite-identity/src/authority.rs:988-993 (charset, no length cap)",
                   "finite-nostr/src/nip05.rs:245-263 (same charset + 64-char cap)",
                   "finite-brain/crates/finite-brain-cli/src/identity_authority.rs:158-160 (hard-codes @finite.vip)"],
         story="The directory can bind a name that nostr-based consumers reject (or stop rejecting what "
               "brain's CLI pre-rejects) because the eligibility rule has three slightly different definitions."),
    dict(id="nostrjson-producer-validator-split", name="nostr.json producer/validator split",
         kind="schema", touched=["finite-identity", "finite-nostr", "finite-brain"],
         owner="finite-nostr", disposition="interface", confidence="high",
         evidence=["finite-identity/src/authority.rs:559-578 (hand-builds json!({names:...}) + CORS)",
                   "finite-nostr/src/nip05.rs:123-218 (owns parse rules: 64KiB/1024-name limits, hex keys, relays)",
                   "finite-brain/crates/finite-brain-server/src/lib.rs:615-680 (fetches and enforces)"],
         story="The directory emits the document as ad-hoc JSON while finite-nostr separately defines validity "
               "and brain enforces it — the producer never sees the validator's type."),
    dict(id="directory-response-shapes-hand-copied", name="directory response structs redeclared per consumer",
         kind="schema", touched=["finite-identity", "finite-brain", "finite-sites"],
         owner="finite-identity", disposition="interface", confidence="high",
         evidence=["finite-identity/src/authority.rs:1056-1074 (owns VipEmailRedeemResponse, Nip05ResolutionResponse)",
                   "finite-brain/crates/finite-brain-cli/src/identity_authority.rs:108-113 (redeclares)",
                   "finite-sites/crates/fsite-cli/src/api.rs:53-58 (redeclares)"],
         story="The client module signs requests but exports no response types, so every consumer hand-copies "
               "the wire structs and serde silently defaults on drift."),
    dict(id="operator-binding-protocol", name="agent-email-binding operator protocol untyped ×3",
         kind="protocol-impl", touched=["finite-identity", "runner", "devfinity"],
         owner="finite-identity", disposition="interface", confidence="high",
         evidence=["finite-identity/src/authority.rs:772-812 (route + {email, agent_npub} + x-finite-operator-token)",
                   "finite-saas-runner/src/lib.rs:766-793 (rebuilds URL/header/json! untyped, validates by echo)",
                   "devfinity/src/lib.rs:683 (writes the shared secret)"],
         story="The one surviving identity hop is implemented from string literals on both sides with no shared "
               "type or secret-name owner — the claim-token campaign (directory-claim-token) is its real fix."),

    # ---- cross-product wire the runtime depends on --------------------------
    dict(id="requester-context-lease", name="Hermes requester-context lease file",
         kind="file-format", touched=["finitechat", "finite-sites", "finite-brain"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat/integrations/hermes/finitechat/adapter.py:63-67,353-372 (writer: v1/v2 dirs, session_key, TTL)",
                   "finite-sites/crates/fsite-cli/src/requester_context.rs:16-80 (reader, v1+v2)",
                   "finite-brain/crates/finite-brain-cli/src/requester_context.rs:21-67 (reader, v1 only)"],
         story="One Python writer and two independent Rust readers of a lease JSON plus HERMES_SESSION_* env "
               "vars — every schema or TTL change is a three-lane coordinated edit with no owner."),
    dict(id="hosted-requester-assertion", name="hosted requester assertion wire",
         kind="wire-type", touched=["dashboard", "finite-sites"],
         owner="finite-sites", disposition="interface", confidence="high",
         evidence=["dashboard/src/lib/hosted-web-chat.ts:287-315 (mints {email, requester_npub, agent_npub})",
                   "finite-sites/crates/finitesitesd/src/api.rs:74-75,351-364", "finitesites-engine/src/lib.rs:1396-1476",
                   "finitesites-proto/src/dto.rs:182,189"],
         story="The dashboard mints the assertion and sites' engine must later satisfy the npub/TTL match — "
               "shape and semantics change together or project init breaks."),
    dict(id="runtime-env-path-port-contract", name="runtime env/path/port contract written three times",
         kind="concept", touched=["runner", "finite-agentd", "finitechat", "infra"],
         owner="runner", disposition="interface", confidence="high",
         evidence=["finite-saas-runner/src/lib.rs:2839-2895 (writes FINITECHAT_HOME/HERMES_HOME/FINITE_AGENT_HTTP_PORT...)",
                   "runtime.Dockerfile:154-176 (ENV + EXPOSE 8080 defaults again)",
                   "finitechat/containers/agent/run_hermes_gateway.sh:21,69,142 + finitechat/containers/agent/health_server.py:17 (default a third time)",
                   "finite-agentd/src/daemon.rs:104-160 (fourth copy + FINITE_AGENTD_REQUIRED=1 with no consumer; "
                   "FINITE_AGENTD_AUTHORIZED_ACCOUNT_IDS provisioned nowhere)"],
         story="The variable names, /data/agent layout, and port 8080 are defined independently by the runner, "
               "the image, the container scripts, and agentd — one rename splits the gateway's identity."),
    dict(id="mail-transport-wiring-x4", name="mail transport adapter rebuilt per service + dead dedup surface",
         kind="dup-logic", touched=["finite-mail", "finite-identity", "finite-brain", "finite-sites"],
         owner="finite-mail", disposition="die", confidence="medium",
         evidence=["finite-mail/src/lib.rs:45-57 (idempotency-key API: zero callers outside own tests)",
                   "finite-identity/src/authority.rs:41-93 (Mailer trait + Dev/Http pair)",
                   "finitesitesd/src/mailer.rs:220-398 (re-delegates six message kinds)",
                   "finite-brain-server/src/lib.rs:961-974 (closure mailer)"],
         story="Every service hand-wires the same dev/prod transport selection around finite-mail, and the "
               "transport's dedup contract has no implementer; identity's mailer now exists for one email "
               "(VIP name-claim challenges)."),
    dict(id="token-ceremony-brain-vs-sites", name="capability-token ceremony designed twice",
         kind="concept", touched=["finite-brain", "finite-sites"],
         owner="finite-brain", disposition="contested", confidence="medium",
         evidence=["finitesites-engine/src/lib.rs:1286-1328 (opaque email token: hex64, sha256-at-rest, single redeem)",
                   "finite-brain-server/src/routes/invite_tokens.rs:5,42-43 (fbit- prefix, same ceremony)"],
         story="Two independent instances of the same capability concept with different surface formats — "
               "hardening applied to one never reaches the other; capability-tokens-v1 (05c) resolves this."),

    # ---- brain internals that escape the crate ------------------------------
    dict(id="brain-grant-wire-strict-vs-loose", name="folder-key grant wire: strict core opener, looser CLI copies",
         kind="protocol-impl", touched=["finite-brain", "finitechat"],
         owner="finite-brain", disposition="interface", confidence="high",
         evidence=["finite-brain-core/src/lib.rs:2528-2599 (strict tag scheme; used by the chat hosted path)",
                   "finite-brain-cli/src/admin.rs:376-433 (re-implements payload+tags+wrap)",
                   "finite-brain-cli/src/sync_engine.rs:1430-1466 (opener without tag/canonical-json checks)"],
         story="The NIP-59 grant format is built and opened twice with different strictness — CLI-issued grants "
               "can stop opening server-side after any format change."),
    dict(id="brain-cli-mirrors-server-wire", name="fbrain CLI mirrors brain-core/server wire untyped",
         kind="schema", touched=["finite-brain"],
         owner="finite-brain", disposition="interface", confidence="high",
         evidence=["finite-brain-cli/src/sync_engine.rs:3449-3557 (CliEncryptedBrainExport/CliFolderKeyGrant/CliSyncRecord)",
                   "finite-brain-server/src/contracts.rs:258-413 (the real shapes)",
                   "finite-brain-cli/src/sync_engine.rs:679-684 (rotation request built as raw serde_json)",
                   "finite-brain-cli/src/sync_engine.rs:2035-2072 + finite-brain-cli/src/admin.rs:456-533 (tag builders mirrored; parity held by one test)",
                   "finite-brain-cli/Cargo.toml (finite-brain-server is dev-dependency only — wire types unavailable)"],
         story="Every server response shape and tag scheme is re-typed or built blind in the CLI, so wire changes "
               "are silent two-crate edits until runtime parse failures."),
    dict(id="working-tree-ignore-drift", name="working-tree scaffold set defined twice, already drifted",
         kind="implicit-coupling", touched=["finite-brain"],
         owner="finite-brain", disposition="interface", confidence="high",
         evidence=["finite-brain-cli/src/sync_engine.rs:2911-2928 (is_generated_folder_file: AGENTS.md, _index.md, _wiki/, .keep)",
                   "finite-brain-core/src/portability/working_tree.rs:4-5,158-200 (generator's convention set)"],
         story="What core generates and what the CLI refuses to upload are two lists that must move together "
               "and have already diverged (compiled/.keep, raw/assets/.keep)."),
    dict(id="email-canonicalization", name="email canonicalization is identity-bearing but private",
         kind="concept", touched=["finite-brain"],
         owner="finite-brain", disposition="interface", confidence="medium",
         evidence=["finite-brain-server/src/lib.rs:865-884 (private canonical_email)",
                   "finite-brain-store/src/schema.rs:1043-1052 (unique index on the canonical string)",
                   "finite-brain-cli/src/identity_authority.rs:158-160 (second inline normalization)"],
         story="The canonical form defines stored identity in a unique index yet lives as a private helper — "
               "any second writer must copy it exactly."),
    dict(id="llms-txt-cli-syntax", name="CLI grammar pinned in server text + skill prose + check markers",
         kind="implicit-coupling", touched=["finite-brain", "finite-sites", "skills"],
         owner="skills", disposition="contested", confidence="medium",
         evidence=["finite-brain-server/src/lib.rs:976-1007 (llms.txt embeds 'fbrain invite brain accept ...' syntax)",
                   "finite-skills/skills/software-development/website-building-finite/SKILL.md:94 + finite-skills/skills/software-development/website-building-finite/references/shared/19-backend.md:57-58 (fsite 0.4.0 grammar)",
                   "finite-skills/skills/software-development/finitebrain/references/fbrain-cli.md:53,316 (hand-typed mirror of usage string)",
                   "finite-skills/scripts/check-static.sh:94-108,149-193 (freezes the prose as literal markers)"],
         story="Renaming any fbrain/fsite flag requires synchronized edits to skill prose, reference docs, "
               "static-check markers, and the server's own llms.txt — nobody owns the CLI contract text."),

    # ---- agentd / runtime lane ------------------------------------------------
    dict(id="agentd-bridge-contracts", name="agentd↔sidecar bridge: paths, NDJSON, 409-replay, hermes verbs",
         kind="protocol-impl", touched=["finite-agentd", "finitechat"],
         owner="finite-agentd", disposition="interface", confidence="high",
         evidence=["finite-agentd/src/transport.rs:25,59-60,113-183 (loopback paths, NDJSON long-poll, 409=byte-identical replay)",
                   "finitechat-cli/src/hermes.rs:551-552,872-916 (/v1/hermes/{action} string dispatch)",
                   "finitechat/integrations/hermes/finitechat/adapter.py:757-758,1846-1852 (2400-line Python twin, hard-coded metadata keys)"],
         story="The bridge message format exists as Rust constants plus a full Python reimplementation with no "
               "shared contract — renaming an action verb is a three-codebase edit."),
    dict(id="agent-command-vocabulary", name="agentd command/schema strings hand-typed ×4",
         kind="wire-type", touched=["finite-agentd", "dashboard", "finitechat"],
         owner="finite-agentd", disposition="interface", confidence="high",
         evidence=["finite-agentd/src/daemon.rs:31-42,427-537 ('agent.owner.claim', 'finite.agent.inference.apply.v1')",
                   "dashboard/src/lib/hosted-agent-controls.ts:16-17,91,270-292", "dashboard/src/lib/hosted-web-chat.ts:35-36",
                   "finitechat-hosted-device/src/lib.rs:84"],
         story="Command names and body-schema strings are private constants redeclared in four places across "
               "Rust and TypeScript — no registry catches drift."),
    dict(id="owner-claim-semantics-x3", name="owner-claim 'first principal wins' encoded three times",
         kind="concept", touched=["finite-agentd", "dashboard", "finitechat"],
         owner="finite-agentd", disposition="contested", confidence="high",
         evidence=["finite-agentd/src/daemon.rs:37,377-388 (ledger owns the rule)",
                   "dashboard/src/lib/hosted-agent-controls.ts:145-208,217-221 (reuse_succeeded_owner_claim + duplicated bootstrap)",
                   "finitechat-hosted-device/src/lib.rs:1947,2053-2082 (scans 5,000 bridge events to re-derive it)"],
         story="The claim rule lives in agentd's ledger but is re-assumed by two dashboard flows and a hosted-"
               "device event scan — any claim/reset behavior change must be re-derived in all three."),
    dict(id="admission-seed-choreography", name="chat admission seed choreography (agentd shells into the CLI)",
         kind="implicit-coupling", touched=["finite-agentd", "finitechat", "core"],
         owner="finitechat", disposition="contested", confidence="high",
         evidence=["finite-agentd/src/daemon.rs:195-200,738-805 (subprocess 'finitechat hermes admission seed', soft-fail)",
                   "finitechat-cli/src/hermes.rs:3892-3950 (the enforcing seed; env precedence, birth-time-only)",
                   "finite-saas-core/src/lib.rs:3035-3048 (injects FINITECHAT_OWNER_NPUBS at lease time)"],
         story="Three components share one env var, a CLI argv, and an 'agentd is soft, sidecar is hard' dual-run "
               "convention — the authz design is sound but the choreography has no single owner."),
    dict(id="aeon-dead-bundle", name="AEON specialization bundle",
         kind="build-unit", touched=["runner", "finite-agentd", "finite-brain", "infra"],
         owner="none", disposition="die", confidence="high", closed="2026-09-01 (#782: worker crate/image, agentd writer, runner shim all removed; flake.lock single-sourcing pattern from #789 to reuse)",
         evidence=["finite-specialization-worker/src/lib.rs:23-48 (URLs, models, prompt versions)",
                   "finite-specialization/config/working/vision-input.spark-nemotron3-nano.hermes-fragment.yaml:41-49 (same constants copied)",
                   "infra/hosts/clawland/finite-specialization-worker.yaml:81-86,149-152 (third copy + hostPath secrets)",
                   "finite-agentd/src/daemon.rs:456 + finite-agentd/src/config.rs:20-21 (agent.specialization.aeon.reconcile still served)",
                   "finite-saas-runner/src/lib.rs:1960-1975 (retired AEON key reservation)"],
         story="One dead deployment braided across worker crate, fragment config, clawland manifest, agentd "
               "commands, and runner reserved-keys — audit 03's decommission retires it all at once."),

    # ---- control plane --------------------------------------------------------
    dict(id="runner-credential-keyring", name="runner credential keyring JSON ×4",
         kind="file-format", touched=["core", "runner", "devfinity", "infra"],
         owner="core", disposition="interface", confidence="high",
         evidence=["finite-saas-core/src/auth.rs:18,87-110,1387 (FC_CORE_RUNNER_CREDENTIALS_JSON + tokenEnv naming rule)",
                   "devfinity/src/lib.rs:269-272,347-357 (hand-builds the JSON + env constant)",
                   "infra/nixos/README.md:207 (secret-file layout contract)"],
         story="Adding or rotating a runner credential is a lockstep edit across core's parser, devfinity's "
               "string-built JSON, and infra's env-naming convention."),
    dict(id="reserved-env-lists-drifted", name="reserved/secret env-key lists maintained twice — already drifted",
         kind="dup-logic", touched=["core", "runner"],
         owner="core", disposition="interface", confidence="high",
         evidence=["finite-saas-core/src/lib.rs:2926-2997 (validate + reserved list + KEY/TOKEN/SECRET heuristic)",
                   "finite-saas-runner/src/lib.rs:1851-1935,1970-2015 (second list; reserves FINITE_PRIVATE_CONTEXT_LENGTH "
                   "and FINITECHAT_HERMES_CONTEXT_LENGTH which core's list lacks — verified 6 vs 0)"],
         story="Core can persist an env value into a RuntimeSpec that runner rejects at launch; runner already "
               "links the core crate, so the constants have one obvious home."),
    dict(id="runtime-env-json-endpoints-x4", name="runtime-env JSON endpoint literals ×4+",
         kind="implicit-coupling", touched=["core", "runner", "devfinity", "infra"],
         owner="core", disposition="interface", confidence="high",
         evidence=["finite-saas-core/src/main.rs:1598-1606 + finite-saas-core/src/store.rs:1556-1562 (persisted into every lease spec)",
                   "devfinity/src/lib.rs:1528-1537,2086-2094 (built twice in the same file)",
                   "infra/nixos/modules/finite-saas-core.nix:54-58", "kata-runner-host.nix:42-44", "finite-lat-3/runner.env.example:27"],
         story="FINITE_SITES_API / FINITE_BRAIN_SERVER_URL / FINITE_BRAIN_PUBLIC_BASE_URL are copy-pasted into "
               "four configs — a brain/sites URL change (the VPS split!) must be replayed everywhere."),
    dict(id="limiter-wire-mirror", name="Finite Private reserve/settle wire mirrored by the limiter",
         kind="wire-type", touched=["core"],
         owner="core", disposition="interface", confidence="high",
         evidence=["finite-private-limiter/src/lib.rs:1479-1521 (hand-written request/decision structs)",
                   ":17 USAGE_FORMULA_VERSION", "finite-saas-core/src/store.rs:7397-7398 (duplicate '2026-05-26.v1' literal)"],
         story="Any metering field or formula bump must be edited in both crates or settlements silently record "
               "stale formula versions."),
    dict(id="dashboard-mirrors-core-schema", name="dashboard hand-mirrors core's whole API schema in TS",
         kind="schema", touched=["core", "dashboard"],
         owner="core", disposition="interface", confidence="high",
         evidence=["dashboard/src/lib/core-client.ts:88,147,360-364,605-666 (CoreAgentCreationRequest etc. + N-1 retry fallback)",
                   "finite-saas-core/src/lib.rs:1109-1121 (the real shapes)"],
         story="Every core schema change is a coordinated hand-edit of the TS mirror, with drift currently "
               "absorbed by ad-hoc N-1 retry heuristics."),
    dict(id="owner-chat-account-id-grammar", name="owner_chat_account_id grammar defined four ways",
         kind="concept", touched=["core", "runner", "finitechat", "dashboard"],
         owner="core", disposition="contested", confidence="high",
         evidence=["finite-saas-core/src/lib.rs:3035-3048 (exactly 64 lowercase hex)",
                   "finitechat-cli/src/hermes.rs:120-137 (accepts 64-hex OR npub1..., one-shot birth seed)",
                   "dashboard/src/app/dashboard/agent-creation-requests/route.ts:181 (sources from hosted identity)",
                   "finite-saas-runner/src/lib.rs:2786-2870 (carries spec env into the container)"],
         story="The meaning and format of the owner key is jointly defined by four validators of differing "
               "strictness — nobody owns the value's grammar."),
    dict(id="runner-pins-skill-prose", name="runner tests pin exact prose of skills docs and Dockerfile lines",
         kind="implicit-coupling", touched=["runner", "skills", "infra"],
         owner="runner", disposition="contested", confidence="high",
         evidence=["finite-saas-runner/src/lib.rs:5268-5294 (asserts literal sentences in finitebrain SKILL.md + brain-creation.md)",
                   ":5249-5264 (asserts ENV/ENTRYPOINT lines in runtime.Dockerfile)"],
         story="Editing a sentence in a skills doc or an ENV line in the image breaks runner's test build with "
               "zero coordination signal between the owners."),
    dict(id="saas-local-limiter-url", name="finite-saas-local hard-codes the production limiter chain",
         kind="implicit-coupling", touched=["core", "infra"],
         owner="core", disposition="interface", confidence="medium",
         evidence=["finite-saas-local/src/lib.rs:25-34,159-163 (tinfoil URL + env names owned by core auth/runner)",
                   "kata-runner-host.nix:62 (same URL literal)", "infra/runbooks/runner-finite-private-route.md:8"],
         story="The dev harness pins the production limiter URL and other components' env names, so rotation "
               "silently leaves local rungs targeting a stale upstream."),

    # ---- runner fleet mechanics ------------------------------------------------
    dict(id="kata-sizing-envelope-x3", name="kata sandbox sizing envelope declared three times",
         kind="implicit-coupling", touched=["runner", "infra"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["finite-saas-runner/src/main.rs:337-339 (cpus=4, memory=8G defaults)",
                   "kata-runner-host.nix:52-53 (FC_RUNNER_KATA_CPUS/MEMORY)",
                   "kata-host-runtime.nix:30-43 (hypervisor default_vcpus/memory — the only one that sizes the VM)"],
         story="OCI-level limits never reach hypervisor sizing, so a resize must coordinate runner defaults, "
               "shared env, and the host's kata config patch."),
    dict(id="sites-kata-invocation-contract", name="sites tier-2 kata invocation vs sudoers/cni host contract",
         kind="implicit-coupling", touched=["finite-sites", "infra"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["finitesitesd/src/apps.rs:208-267 (sudo -n nerdctl --runtime kata-clh invocation + env contract)",
                   "infra/nixos/modules/finitesitesd.nix:14-58,101-104 (sudoers rule, cni path, ExecStart flags)"],
         story="The daemon's command shape is only valid against the exact sudoers rule and kata config the "
               "NixOS module installs — mirrored by hand on both sides."),
    dict(id="kata-label-vocabulary", name="computer.finite.v2.* container-label vocabulary",
         kind="wire-type", touched=["runner", "scripts"],
         owner="runner", disposition="interface", confidence="high",
         evidence=["finite-saas-runner/src/kata.rs:2978-3021 (writer)", "finite-saas-runner/src/apple_container.rs:731-764 (second writer)",
                   "finite-saas-runner/src/lifecycle_probe.rs:460-466 (reader)", "scripts/rollout-lat1-runtime-artifact:391,410 (jq templates)"],
         story="Label names are bare literals in two launchers, the probe, and the operator rollout script — "
               "renaming one silently breaks upgrade eligibility checks."),
    dict(id="lifecycle-probe-schema", name="finite.lifecycle-probe.v1 schema + finding ids in operator scripts",
         kind="schema", touched=["runner", "scripts"],
         owner="runner", disposition="interface", confidence="high",
         evidence=["finite-saas-runner/src/lifecycle_probe.rs:31,157-175 (schema const + finding vocabulary)",
                   "scripts/finite_status.py:187 (schema literal re-hardcoded)",
                   "scripts/rollout-lat1-runtime-artifact:963,1029-1042 (jq asserts schema + verdict mapping)"],
         story="The probe report schema and check identifiers are copied as literals into two operator scripts "
               "with no shared owner."),
    dict(id="startup-report-schema", name="startup-report.json + /contact schema (recovery probe)",
         kind="schema", touched=["finitechat", "runner"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat/containers/agent/recover_chat_boot.py:514-583 (writer)",
                   "finitechat/containers/agent/health_server.py:198-321 (re-validator + embeds into /contact)",
                   "finite-saas-runner/src/kata.rs:3120-3191 (field-by-field parser + npub match)",
                   "finite-saas-runner/src/health_reports.rs:1-46 (standing poll)"],
         story="The recovery report shape is written by one chat script, re-validated by another, and re-parsed "
               "by the runner's probe — three definitions, no owning type."),
    dict(id="artifact-pin-handoff", name="runtime artifact pin reaches runners by hand-copied env",
         kind="implicit-coupling", touched=["core", "runner", "infra"],
         owner="core", disposition="contested", confidence="medium",
         evidence=["finite-saas-runner/src/main.rs:171-172 (resolves FC_RUNNER_RUNTIME_ARTIFACT_ID against core)",
                   "kata-runner-host.nix:32-37 (pin deliberately operator-only)",
                   "infra/runbooks/runtime-image.md:75-98 (promotion = register in core, then hand-edit env per host)"],
         story="Core owns the promoted artifact record but each runner's pin is a manually copied env value — "
               "promotion state and launched reality can drift until the runner fails closed."),
    dict(id="capacity-ceilings-x2", name="runner capacity ceilings re-declared in the guard script",
         kind="dup-logic", touched=["infra", "scripts"],
         owner="infra", disposition="contested", confidence="medium",
         evidence=["hosts/finite-lat-1/default.nix:47 (12)", "finite-lat-3:46 (42)", "finite-lat-4:55 (42)",
                   "scripts/check_runner_host_contract.py:24-30 (re-hardcodes 12/42/42)"],
         story="Every capacity change is made twice — host Nix and the guard's literal table — and the guard "
               "cannot see the operator drain flag at all."),

    # ---- chat surface ----------------------------------------------------------
    dict(id="app-state-redaction-x2", name="AppState redaction + SSE framing duplicated per daemon",
         kind="dup-logic", touched=["finitechat", "dashboard"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat-daemon/src/lib.rs:766-828 (redact_app_state, state/error events)",
                   "finitechat-hosted-device/src/lib.rs:4157-4179 (same algorithm again)"],
         story="Both daemons hand-maintain the same secret-redaction list — a new secret field in AppState must "
               "be patched in two places or one daemon leaks it."),
    dict(id="hosted-device-contract-ts-mirror", name="hosted-device HTTP contract hand-mirrored in TypeScript",
         kind="schema", touched=["finitechat", "dashboard"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat-hosted-device/src/lib.rs:453-511 (route table)",
                   "dashboard/src/lib/hosted-web-device.ts:394,494-763 (routes + HostedChatState redeclared)",
                   "dashboard/src/lib/hosted-web-device.ts:340-354 + dashboard/src/lib/electron-chat-runtime.ts:174-180 (reconcile schema validated twice)",
                   "electron-chat/electron/preload.cjs:42-53 vs electron-chat-runtime.ts:7-18 (bridge versions/capabilities declared twice)"],
         story="Every route string, state shape, reconcile status, and desktop-bridge capability is redeclared "
               "by hand in TS with no codegen — the field changes compile nowhere and break at runtime."),
    dict(id="dashboard-json-body-reader-x3", name="bounded JSON body reader triplicated in the dashboard",
         kind="dup-logic", touched=["dashboard"],
         owner="dashboard", disposition="interface", confidence="high",
         evidence=["dashboard/src/lib/hosted-web-chat.ts:186-236", "dashboard/src/lib/device-link.ts:79-129", "dashboard/src/lib/site-preview.ts:31-84"],
         story="Three copies of the content-type/length-check/stream-cap/decode/parse algorithm must change "
               "together for any body-parsing policy change."),
    dict(id="chat-error-envelope-per-route", name="per-route chat error envelope hand-rolled ×7",
         kind="implicit-coupling", touched=["dashboard"],
         owner="dashboard", disposition="interface", confidence="high",
         evidence=["dashboard/src/app/api/chat/machines/[machineId]/hosted-device/state/route.ts:23-36 (and six sibling routes)",
                   "dashboard/src/components/hosted-chat-provider.tsx:762-793 (client re-parses)"],
         story="Seven route handlers each re-implement the same error→502+{error,code} mapping the browser "
               "client then re-parses — new codes touch every route plus the client."),
    dict(id="device-link-driver-x2", name="device-link target driver implemented twice",
         kind="dup-logic", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat-core/src/native_device_link.rs:7-26 (pairing requests + 128KiB/10s/400ms constants)",
                   "finitechat-daemon/src/device_link.rs:4-21,136-196 (same request set + constants again)"],
         story="The NIP-AB target-side polling flow exists as two independent copies that must change together "
               "whenever pairing routes or timing change."),
    dict(id="electron-matches-error-prose", name="Electron classifies daemon failures by error Display prose",
         kind="implicit-coupling", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["electron-chat/electron/daemon-process.cjs:30-56 (map keyed by exact thiserror strings) + :72-76 (regex)",
                   "finitechat-daemon/src/device_link.rs:107-117 (the #[error(\"...\")] strings)"],
         story="Rewording any DeviceLinkBootstrapError variant silently degrades the desktop UI to a generic "
               "failure — the contract is the prose."),
    dict(id="blob-url-shape-rederived", name="attachment blob URL shape re-derived by the hermes sidecar",
         kind="implicit-coupling", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat-server/src/validate.rs:48-62 + finitechat-server/src/routes.rs:107 ({origin}/blobs/{sha256})",
                   "finitechat-cli/src/hermes.rs:2789-2827 (re-derives expected_path to rewrite loopback origins)"],
         story="The URL baked into encrypted attachment references is generated by the server but its shape is "
               "independently re-implemented by the sidecar — changing the route breaks old attachments."),
    dict(id="nip98-url-reconstruction", name="NIP-98 validity depends on silent URL equality",
         kind="implicit-coupling", touched=["finitechat"],
         owner="finitechat", disposition="contested", confidence="high",
         evidence=["finitechat-client/src/lib.rs:5130-5151 (signs the dial URL)",
                   "finitechat-server/src/auth.rs:61-79,135-154 (validates server-reconstructed URL; mismatch "
                   "silently downgrades to unsigned)"],
         story="Signature validity hinges on the client's base_url silently equaling the server's public_url or "
               "forwarded-header reconstruction — divergence fails quietly, not loudly."),
    dict(id="chat-route-literals-x3", name="chat route path strings retyped in three crates",
         kind="wire-type", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat-server/src/routes.rs:67-127 (canonical)",
                   "finitechat-client/src/lib.rs:5173-5697 (literals)", "finitechat-cli/src/lib.rs:335-482 (literals)"],
         story="Types live in finitechat-http but route strings are re-typed in client and CLI — renaming a "
               "route compiles cleanly and breaks at runtime."),

    # ---- dev harness -------------------------------------------------------------
    dict(id="workos-fixture-mirror", name="WorkOS fixture hand-mirrors the real wire + file layout ×4",
         kind="protocol-impl", touched=["devfinity", "core", "skills", "dashboard"],
         owner="devfinity", disposition="interface", confidence="high",
         evidence=["devfinity/src/workos_fixture.rs:154-232 (routes, Claims, JWK shapes)",
                   "finite-saas-core/src/auth.rs:272-558 (the real client contract)",
                   "devfinity/src/workos_fixture.rs:53-63 vs devfinity/tests/stack_smoke.rs:314-322, scripts/devfinity-smoke:96,128, "
                   "finite-skills/ab-testing/scripts/run-devfinity-agent-turn.mjs:127, dashboard/scripts/stripe-billing-test-clock-e2e.ts:101"],
         story="Local auth silently drifts from production when core's WorkOS client changes, and the fixture "
               "credential file names are a string convention re-derived by four consumers."),
    dict(id="smoke-health-assertions", name="smoke harnesses assert exact health bodies of five services",
         kind="wire-type", touched=["devfinity", "core", "finitechat", "finite-sites", "finite-brain", "dashboard"],
         owner="devfinity", disposition="contested", confidence="high",
         evidence=["scripts/devfinity-smoke:561-566 (exact-substring checks)", "devfinity/tests/stack_smoke.rs:20-36 (again in Rust)",
                   "devfinity/src/lib.rs:1570-1786 (probe paths duplicated a third time)"],
         story="Changing any health endpoint's path or body forces coordinated edits to two smoke harnesses "
               "and the probe generator — services export no health contract."),
    dict(id="build-report-schema", name="runtime-image build-report schema + image contents assumed by devfinity",
         kind="file-format", touched=["devfinity", "runner"],
         owner="runner", disposition="interface", confidence="medium",
         evidence=["devfinity/src/lib.rs:1991-1999 (jq regex on .image_metadata.digest) + :1898-1932 (assumes bash+curl in image)",
                   "finitecomputer-v2/scripts/build_runtime_image.py:130-137,184,200 (owns the report)"],
         story="Reshaping the build report or slimming the image silently breaks local artifact registration "
               "and the readiness gate."),

    # ---- skills / ab-testing --------------------------------------------------------
    dict(id="ab-harness-mirrors-clients", name="A/B harness re-implements product clients",
         kind="protocol-impl", touched=["skills", "finitechat", "core", "runner", "finite-agentd"],
         owner="skills", disposition="interface", confidence="high",
         evidence=["ab-testing/scripts/run-devfinity-agent-turn.mjs:354-481,555 (untyped hosted-device client: routes, action tags, field names)",
                   ":121-207,516 (core route URLs + shapes + container label)",
                   ":259 (agentd status --json nesting)", ":231-252 (skills staging swap without atomic exchange)"],
         story="The harness hand-rolls a JS client for every product surface it touches — each field rename in "
               "finitechat/core/agentd must be mirrored here by hand."),
    dict(id="skills-sync-algorithm", name="managed-skills atomic-swap algorithm reimplemented",
         kind="dup-logic", touched=["skills", "finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["finitechat/containers/agent/finite.py:242-270 (real exchange with fsync/atomicity)",
                   "ab-testing/scripts/run-devfinity-agent-turn.mjs:231-252 (simplified current/staging/previous swap)",
                   "finite-skills/docs/runtime-delivery-contract.md:34-38"],
         story="The delivery layout and exchange protocol have a second, weaker implementation — layout changes "
               "get fixed twice."),

    # ---- infra ------------------------------------------------------------------
    dict(id="wg-hub-literals-x4", name="WireGuard hub role cloned across four host files",
         kind="dup-logic", touched=["infra"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["hosts/finite-lat-1/default.nix:116-195", "finite-lat-2:209-314", "finite-lat-3:20-195", "finite-lat-4:29-202",
                   "runbooks/lat4-nixos-runner-install.md:371"],
         story="Hub address, ports, peer pubkeys, and firewall rules are hand-copied per host (lat1 /30 vs lat2 "
               "/29 mask drift already shows it) — the mesh has no shared module."),
    dict(id="port-map-literals", name="loopback port map duplicated across modules",
         kind="implicit-coupling", touched=["infra", "core", "finitechat", "finite-brain", "finite-sites", "finite-identity", "dashboard"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["caddy.nix:17,29,33,44,55,61", "dashboard.nix:31-35", "finitechat-hosted-device.nix:26",
                   "finite-identity.nix:24-46", "finitechat-server.nix:26", "finite-saas-core.nix:43", "finite-brain.nix:22", "finitesitesd.nix:92"],
         story="Every service's bind port is a literal repeated in caddy, dashboard env, and cross-service "
               "modules — the VPS split multiplies this unless ports become per-host declarations."),
    dict(id="db-path-roster", name="chat/brain db-path roster duplicated across replication and restore",
         kind="schema", touched=["infra", "finitechat", "finite-brain"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["finitechat-server.nix:26,31", "finite-brain.nix:23,48",
                   "finite-lat-2/default.nix:124-138 (litestream)", "backups.nix:131-384 (three copies)",
                   "infra/scripts/restore-hosted-web-chat-snapshot:49-63", "test-hosted-web-chat-restore:73-103"],
         story="Live db paths plus the recovery-set roster are hand-copied into litestream, backups, restore "
               "tooling, and runbooks — moving a StateDirectory breaks DR silently."),
    dict(id="digest-pins-hand-copied", name="GHCR digest pins hand-copied from CI outputs",
         kind="implicit-coupling", touched=["infra", "dashboard"],
         owner="infra", disposition="interface", confidence="medium",
         evidence=[".github/workflows/service-images.yml:53-67,117 (emits the digest, propagates nothing)",
                   "dashboard.nix:10 (hand-pinned; the clawland manifest half closed with AEON #782)"],
         story="Every release requires a human to copy a sha256 into consuming configs with no checker for "
               "stale or mistyped pins."),
    dict(id="unit-roster-x3", name="product unit roster maintained three times",
         kind="concept", touched=["infra", "scripts"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["monitoring.nix:41-48 (probedServiceUnits)", "scripts/finite_status.py:28-46 (CONTRACT)",
                   "finite-lat-2/default.nix:39-66 (importModeUnits)", "scripts/check_finite_status_contract.py:35-44"],
         story="Adding a service or renaming a unit can leave import-mode holding down a phantom or finite-"
               "status probing a dead port — the roster has no single source."),
    # ---- sync 2026-09-02: chat/runner source-of-truth work --------------------
    dict(id="chat-error-classification", name="chat error taxonomy re-derived per bridge",
         kind="concept", touched=["finitechat", "dashboard"],
         owner="finitechat", disposition="interface", confidence="high",
         closed="2026-09-02 (#825: core owns classification()/http_status(), exhaustive match, bridges derive — CLI carries Core errors whole, daemon IntoResponse reuses it)",
         evidence=["finitechat/crates/finitechat-core/src/lib.rs:284 (classification())"],
         story="The CLI flattened errors to strings, the daemon re-matched for statuses, the hermes bridge "
               "guessed retryability — closed by making core the single classifier."),
    dict(id="chat-inbox-cursor-heuristics", name="hermes inbox cursor had two resume truths",
         kind="implicit-coupling", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         closed="2026-09-02 (#819: event-scan seeding heuristic deleted; hermes-inbox.json is the single cursor; core cursors pass through the bridge untouched)",
         evidence=["finitechat/crates/finitechat-cli/src/hermes.rs:74 (HERMES_INBOX_FILE)"],
         story="The bridge persisted a delivery high-water mark AND derived a resume point from the event "
               "scan; the heuristic is gone and the file is the only truth."),
    dict(id="chat-client-outbox", name="durable client outbox — a second send path",
         kind="concept", touched=["finitechat"],
         owner="finitechat", disposition="die", confidence="high",
         closed="2026-09-02 (#820: outbox deleted — sends succeed synchronously or fail loudly; snapshot-exclusion assertion gated to tests)",
         evidence=["finitechat/crates/finitechat-client/src/ (outbox.rs gone)"],
         story="The outbox was a parallel durability story next to the sync path; deleting it left one way "
               "a send can be true."),
    dict(id="durable-root-liveness-truth", name="durable-root liveness: writer lease vs container records",
         kind="concept", touched=["runner", "core", "finitechat"],
         owner="runner", disposition="interface", confidence="high",
         evidence=["finite-saas-runner/src/kata.rs:3182 (durable_tree_is_quiescent: flock the agent/client.sqlite3.writer-lease + two-instant change manifest)",
                   "finite-saas-runner/src/kata.rs:398,3123 (typed DurableStateRootLive on migration and relocation)",
                   "005a643a: the lease check cannot see locks a Kata VM does not forward — 90s quiescence window",
                   "252457ea: machine-named state-root fallback deleted, legacy roots migrate once",
                   "2b137994: core synthesizes pre-RuntimeSpec specs with the runtime id as durable state id"],
         story="Provider records are not liveness truth — a Kata VM writes through bind mounts after its "
               "container records vanish. The quiescence protocol makes the writer-lease file the authority "
               "and the runtime id the one durable-state key; watch that core's spec synthesis and the "
               "runner's migration never re-diverge on that key."),
    dict(id="chat-device-currency-gate", name="device-state currency: server witness as the gate",
         kind="concept", touched=["finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["5564881c: device blob snapshot v10 — own_send_high_water_seq + behind_server evidence; sync apply refuses rewound stores (no server change needed)",
                   "580b9794: the silent same-device-id re-mint deleted (audit 16's hole)"],
         story="A rewound MLS sender reused consumed generations and every receiver quarantined; the sync "
               "stream itself is now the witness, and the re-mint path is gone — recovery epochs become "
               "checkable instead of hoped-for."),

    # ---- meta-audit pass (cross-cutting gaps the lanes missed) -----------------
    dict(id="hermes-version-pin-mirror", name="Hermes agent version pinned in five places",
         kind="build-unit", touched=["finitechat", "infra", "scripts"],
         owner="infra", disposition="interface", confidence="high", closed="2026-09-01 (#789 item 5 v2: flake.lock is the single declaration; CI literals deleted, version stamped at build time)",
         evidence=["flake.nix:19 (hermes-agent.url v2026.8.3)",
                   ".github/workflows/ci.yml:232-377 (FINITE_HERMES_AGENT_VERSION=\"0.20.0\" + in-job asserts)",
                   "finitechat/tests/container/test_hermes_durable_home_smoke.py:107 (asserts ci.yml's own YAML text)",
                   "finitechat/scripts/hermes-chat-interruption-docker-smoke.py:26 (EXPECTED_HERMES_VERSION)"],
         story="Bumping Hermes is a lockstep edit across the flake input, CI env and asserts, chat smoke "
               "scripts, and audit fixtures — one of which pins the CI workflow's own YAML prose."),
    dict(id="ledger-lint-unwired", name="this architecture ledger's lint is not wired into CI",
         kind="concept", touched=["infra", "scripts"],
         owner="infra", disposition="interface", confidence="high",
         evidence=["scripts/render-architecture:1-26 (the --check flag exists)",
                   "docs/architecture/contracts.py:1-22 (declares itself canonical)",
                   "no workflow, justfile, or scripts/ci invokes render-architecture --check (grep-verified)"],
         story="The declared couplings and generated snapshots can silently drift from the code they "
               "describe until a CI job (or just recipe) runs the check — the registry policing the "
               "braid is itself an unwired contract."),
    dict(id="source-fingerprint-provenance", name="chat build-provenance is a four-party wire",
         kind="build-unit", touched=["finitechat", "infra", "scripts"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["infra/nixos/packages.nix:154-163 (sourceFingerprint + build env names + passthru attr)",
                   "finitechat-server/src/routes.rs:236 (option_env! compiled into /health)",
                   ".github/workflows/ci.yml:721-753 (nix eval + boots binary + asserts health JSON fields)"],
         story="Renaming the passthru attr, env var, or /health field breaks the landing gate while "
               "everything else still builds."),
    dict(id="nix-sourcepaths-mirror-cargo", name="nix sourcePaths hand-mirror the cargo dependency graph",
         kind="implicit-coupling", touched=["infra", "finitechat", "finite-brain", "finite-sites", "core"],
         owner="infra", disposition="contested", confidence="high",
         evidence=["infra/nixos/packages.nix:6-8,211-318 (per-package directory rosters; forgotten entry = "
                   "build fails 'no targets specified')",
                   "scripts/ci/affected-rust-packages:85-88 (derives the same graph from cargo metadata)"],
         story="Every new inter-crate dependency must be re-declared by hand as a directory list, "
               "restating knowledge Cargo already owns and the CI scoper already derives."),
    dict(id="chat-ui-cross-tree-dep", name="dashboard vendors chat's UI package via relative file:",
         kind="implicit-coupling", touched=["dashboard", "finitechat"],
         owner="finitechat", disposition="interface", confidence="high",
         evidence=["dashboard/package.json:29 (\"@finite/chat-ui\": \"file:../../../finitechat/packages/finitechat-chat-ui\")",
                   "dashboard/next.config.ts:9 (transpilePackages)",
                   "pnpm-workspace.yaml:7-11 (chat-ui absent from the workspace)",
                   "scripts/ci/select-harnesses:396-399 (CI path-map remembers the link)"],
         story="Every chat-ui change is silently a dashboard change through a dependency outside the "
               "workspace, with only the CI path-map remembering the link exists."),
]
