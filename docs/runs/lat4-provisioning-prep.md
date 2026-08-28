# lat4 Provisioning Prep (finite-lat-4)

Status: **prep complete; activation scaffold on branch
`infra/lat4-nixos-runner-twin` (PR #736).** No production mutation has
happened or is authorized by this record; the box is only ever touched
read-only until the gated install runbook executes with fresh approval.

Scope decision: finite-lat-4 is a **dedicated Agent Runner host** cloned from
the finite-lat-3 personality. It holds sandboxes only — no app services, no
edge proxy, no database, no monitoring receiver. Per ADR 0007 (PR #715),
lat4 is the **third** runner host: lat2 rejoins as the second runner twin,
and lat4 follows the same model — admitted drained, one-creator rule intact,
Gate-E-style undrain as a separate owner decision.

Related: `docs/runs/finite-lat-capacity-and-redundancy.md`,
`infra/nixos/hosts/finite-lat-3/`, `infra/README.md`.

## 1. Hardware evidence (read-only SSH, 2026-08-28, interim Ubuntu 24.04.4)

| Item | Observed value |
|---|---|
| Board | Supermicro H13SRE-F, chassis AS -3015MR-H10TNR |
| CPU | AMD EPYC 4564P 16C/32T (Raphael-based, `kvm-amd`) |
| RAM | 187 GiB |
| Root disks | 2x Micron_7450_MTFDKBA480TFR (937,703,088 sectors each) |
| Data disks | 2x SAMSUNG MZQL21T9HCJR-00A07 (3,750,748,848 sectors each) |
| WAN NIC | `eno1` `90:5a:08:2e:65:df` — 152.236.34.15/31, gw 152.236.34.14; v6 2605:6440:2004:1ac::2/64, gw 2605:6440:2004:1ac::1 |
| Unused NIC | `eno2` `90:5a:08:2e:65:de` |
| Extra link | `enxbe3af2b6059f` — USB device (`parentbus usb`, DOWN); likely BMC virtual NIC; do not configure |
| Interim OS | Ubuntu 24.04.4, throwaway; interim RAID1 md0 (`/`) + md1 (`/data`) already assembled, passwordless sudo for `ubuntu` |

Geometry fit against the lat3 contract (all comparisons are lat4 last-usable
sector vs lat3 partition end sector):

- Root member: 937,703,054 >= 935,331,839 — **fits, ~11.7 GiB spare**
- Data member: 3,750,748,814 >= 3,747,612,671 — **fits, ~1.5 GiB spare**

Because both member partitions can span the identical lat3 sector ranges, the
whole disk contract (ESP 2048s-2099199s, root 2099200s-935331839s, data
2048s-3747612671s, MD sizes 464519168K/1871708160K, data-offset 1024K, 64M
internal bitmap) is cloned verbatim; only identities differ. The exact
`component_size` values must still be re-observed on the physical host at
install time and confirmed against the health gate, as on lat3.

## 2. Fresh identity set (generated 2026-08-28; never reuse lat3 values)

Drafted in `infra/nixos/hosts/finite-lat-4/storage-ids.nix`:

- Disk mapping: rootA = nvme0n1 (Micron …59f3f7), rootB = nvme1n1 (Micron
  …5a0002), dataA = nvme2n1 (Samsung …0161), dataB = nvme3n1 (Samsung …0160),
  all pinned by `nvme-eui.*` by-id paths.
- 6 fresh PARTUUIDs, 2 fresh MD UUIDs, 2 fresh ext4 UUIDs, 2 fresh vfat
  volume IDs — all machine-generated, uniqueness asserted by
  `invariants.nix`.

The interim arrays' UUIDs (5555f02b…, 97d709fd…) are throwaway and are
replaced at disko install time.

## 3. Alignment with PR #715 / ADR 0007 (2026-08-28, updated for the pivot)

PR #715 was amended before merging: **lat1 went down (thermal, full product
outage) and lat2 became the emergency replacement app-plane host** — Core,
Postgres, chat, Identity, edge, backups — plus the `wg-finite` hub at
`10.254.3.1`. The runner lane moved to a future host: **finite-lat-4 is now
the SECOND storage-qualified Runner host**, and its role is doubly load-
bearing because lat1's existing-Agent fleet (22 Runtimes, ~31G durable
state) needs a home.

| Topic | 715 (as amended) decision | lat4 adoption |
|---|---|---|
| WireGuard hub | lat2 = `10.254.3.1/29`, Core + Identity socket proxies; lat3's peer re-pointed to lat2 (`iuzuWHBSrPPbanAdiS86jABhwieo+wyig8I1f+FuPBk=`, endpoint `64.34.80.19`) | lat4's peer mirrors lat3's flip exactly; drafted in default.nix. The lat2-side lat4 peer entry lands after 715 merges AND lat2 is live (Gate D/E wait on that) |
| Runner lane | lat2 ships with NO runner role; the lane moves to lat4 | lat4 = second runner host (was drafted as "third" before the pivot; docs updated) |
| Ceiling | lat3's owner-authorized **42** (swap is not capacity) | `maxSandboxes = 42`, `FC_RUNNER_DRAIN=true`; drained still serves existing-Agent lifecycle work, only creation is withheld |
| Existing-Agent fleet | lat1's 22 Runtimes / ~31G state are homeless; cold-relocation contract exists (`infra/runbooks/runtime-cold-relocation.md`) | **Ownership open** — see §5 item 11 |
| Storage identity | `captured` gate; build script refuses uncaptured | lat4 ships `captured = true` with live-captured values; guard kept |
| Install | CI artifact carries toplevel + disko + same-pin kexec; Gates A–E | cloned for lat4 |
| Contracts | runner-host stays lat1+lat3 on 715 | lat4 activation adds itself (3 hosts) |

Paul's review note on #715 ("too much copy and pasting where instead we
could share stuff") applies to lat4's verbatim clones too. Before or during
the lat4 activation PR, extract the twin files
(disko/invariants/esp-guard/storage-health) into a shared, storage-ids +
host-label-parameterized module set. Sequence this with the outcome of that
#715 review discussion.

## 4. Host config and activation scaffold (PR #736)

- `infra/nixos/hosts/finite-lat-4/storage-ids.nix`
- `infra/nixos/hosts/finite-lat-4/disko.nix` — verbatim lat3 layout
- `infra/nixos/hosts/finite-lat-4/invariants.nix` — verbatim, relabeled
- `infra/nixos/hosts/finite-lat-4/esp-guard.nix` — verbatim
- `infra/nixos/hosts/finite-lat-4/storage-health.nix` — verbatim, relabeled
- `infra/nixos/hosts/finite-lat-4/default.nix` — runner-only personality:
  `finite-kata-runner-4`, `kataHostAddress 10.254.3.4`, `coreUrl`
  and `identityAuthority` unchanged (lat1), `maxSandboxes 42`, WAN block from
  §1, WireGuard `10.254.3.4/29` (per ADR 0007), same firewall/swap/zswap/boot
  contracts, `kvm-amd` + `amdgpu` blacklist retained for the Raphael-based
  EPYC iGPU.
- `infra/nixos/hosts/finite-lat-4/runner.env.example` — **starts
  `FC_RUNNER_DRAIN=true`**; lat4 stays drained until its admission review
  passes, per the availability doctrine.

Deliberately NOT done in prep: flake.nix registration
(`lat4Modules`/`lat4`/`lat4Kexec`/`nixosConfigurations.finite-lat-4`), closure
build/deploy script fork, CI workflow clone, `check_runner_host_contract.py`
HOSTS update. Those land together in the activation PR once §4 is resolved.

- `infra/nixos/hosts/finite-lat-4/storage-ids.nix` — `captured = true` with
  live-captured disk identities and fresh unique identifiers
- `infra/nixos/hosts/finite-lat-4/default.nix` — runner-only personality:
  `finite-kata-runner-4`, `kataHostAddress 10.254.3.4`, `coreUrl`
  and `identityAuthority` unchanged (lat1), `maxSandboxes 42`, WAN block from
  §1, WireGuard `10.254.3.4/29` (per ADR 0007), same firewall/swap/zswap/boot
  contracts, `kvm-amd` + `amdgpu` blacklist retained for the Raphael-based
  EPYC iGPU.
- `infra/nixos/hosts/finite-lat-4/runner.env.example` — **starts
  `FC_RUNNER_DRAIN=true`**; lat4 stays drained until its separate admission
  decision, per the one-creator rule.

Deliberately NOT done in prep: flake.nix registration
(`lat4Modules`/`lat4`/`lat4Kexec`/`nixosConfigurations.finite-lat-4`), closure
build/deploy script fork, CI workflow clone, `check_runner_host_contract.py`
HOSTS update. Those land together in the activation PR once §5 is resolved,
based on the post-#715 tree.

Update 2026-08-28 (same day): the activation scaffold landed on the
`infra/lat4-nixos-runner-twin` branch / PR #736 — flake registration,
`build-lat4-nixos-closure-artifact` (captured guard + disko + same-pin kexec,
schema `finite.lat4.nixos-closure.v2`), `deploy-lat4-closure-cache`,
`Lat4 NixOS Closure` workflow, `capture-lat4-host-evidence`,
`lat4-nixos-runner-install.md` (Gates A–E), just recipes,
`check_runner_host_contract.py` (3 hosts, lat4 at 42),
`check_monitoring_nixos_contract.py` lat4 Alloy/log-unit/role entries,
select-harnesses paths, and README/deployment-queue/capacity-doc updates.
CI (evals + contracts on x86_64-linux) is the remaining local-verification
gap: this Mac has no Nix toolchain.

## 5. Open questions (must be answered before activation PR)

Answered by PR #715 / ADR 0007 since the first draft (see §3):

1. ~~WireGuard topology~~ → single overlay, /29 widening via #715, lat4 =
   10.254.3.4; **hub is lat2** after the emergency pivot (was lat1). The
   lat2-side lat4 peer entry lands post-#715 and post-lat2-go-live.
2. ~~Sandbox cap~~ → mirror lat3's owner-authorized 42; undraining lat4 needs
   its own owner authorization at the admission gate. Drained lat4 still
   serves existing-Agent lifecycle work (lat1's drained model), so fleet
   adoption does not require undraining.

Still open:

3. ~~Public address confirmation.~~ **Answered 2026-08-28 by the operator:**
   152.236.34.15 is the confirmed address, matching the live capture (§1).
   The IPv6 /64 and gateways were captured from the same live state; treat
   them as the working values and re-verify at Gate C first boot.
4. **BMC USB NIC** (`enxbe3af2b6059f`): identify (likely Supermicro virtual
   USB NIC) and confirm it stays unconfigured.
5. **SMART pre-flight.** `smartctl` is absent on the interim OS; run the
   4-disk SMART check before wiping (install smartmontools or boot the NixOS
   kexec image, which carries it).
6. **Secret placement** (names only, off-host custody): `/etc/finite/`
   runner.env, runtime-secrets.env, identity-operator.env,
   metrics-remote-write.env, logs-write.env, wireguard-private-key — plus a
   values-free lat4 secret contract (lat3 never got one; do not repeat that
   gap). The contract JSON is **deliberately deferred to Gate D
   preparation**: its content depends on the final secret-custody decisions,
   and `scripts/check_nixos_secrets_contract.py` grows a finite-lat-4 entry
   together with that contract file. Also `FC_CORE_RUNNER_CREDENTIALS_JSON`
   registration for `finite-kata-runner-4` in Core (runbook Gate D).
7. **Monitoring receiver credentials** for lat4 on the monitoring host must
   match the new `metrics-remote-write.env`/`logs-write.env` pairs.
8. **Install runbook.** Clone the `lat2-nixos-runner-install.md` Gates A–E
   pattern as `lat4-nixos-runner-install.md`: Gate A pre-wipe evidence +
   provider console steps, Gate B fill storage-ids (already drafted) + CI
   artifact at merged rev, Gate C rescue-mode artifact-driven install,
   Gate D secrets drained + WG handshake + drained first-lease proof +
   storage drills, Gate E admission decision. Record observed
   `component_size` + rollback boundary.
9. **Recovery Authority.** ADR 0007 keeps lat2 a non-authority "exactly as
   lat3 is not"; decide deliberately whether lat4 follows or breaks that
   posture instead of cloning it silently. Default while undecided: follow
   the fleet posture (same as lat3/lat2), flagged here as deliberate.
10. **Shared-module refactor.** Paul's #715 review note asks for less
    copy-paste across runner twins. Decide whether the shared
    disko/invariants/esp-guard/storage-health module extraction lands with
    #715 feedback, with the lat4 activation PR, or after both.
11. **Existing-Agent fleet migration (unowned).** lat1's 22 Runtimes (~31G
    durable state) need a home on lat4: state import + Core
    `source_host_id` re-point. The one-Runtime vehicle exists
    (`infra/runbooks/runtime-cold-relocation.md`, `runtime_relocation.v1`);
    a bulk fleet path needs its own gated run doc. Ownership was raised by
    Paul on 2026-08-28 and is pending the owner's call (see PR #736
    discussion). This must not be speculative: per-agent provenance (Runtime
    ID, durable state ID, Principal, artifact, schema) verified from the
    lat1 recovery set, chat-preserving verification, and the source kept
    stopped-and-intact per the cold-relocation contract.

Items 4-8 are execution-time steps inside the Gates A-E runbook, not prep
blockers. Item 9 carries the documented default above. The only true prep
decisions (address plan, ceiling, admission posture, public IP) are closed.

## 6. Acceptance gates for the lat4 skeleton (before any admission)

- `nix build .#finite-lat-4-system --option builders ''` on x86_64-linux,
  with the lat4 closure artifact carrying the disko script + same-pin kexec
  tarball like #715
- `nix build .#finite-lat-4-disko` proof on target hardware
- Post-install: `finite-storage-health` green (arrays idle, exact component
  sizes, quotas, swapfile, zswap, SMART), both ESPs pass the esp-guard
  contract, `just runner-host-contract` green with lat4 as the fourth host,
  monitoring contract green with lat4 present
- `scripts/finite-status` before/after; Runner stays drained until the
  separate admission decision

## 7. Checkpoints

| Date | Event | Result |
|---|---|---|
| 2026-08-28 | Read-only hardware/network evidence captured from lat4 pre-wipe; fresh storage identity set generated | §1-2 |
| 2026-08-28 | Draft host directory + prep record written (uncommitted); codex review pass, all files PASS | §4 |
| 2026-08-28 | Re-aligned draft to PR #715 / ADR 0007 (WG /29 lat4=.4, ceiling 42, drained admission, captured pattern, artifact-driven install) | §3 |
| 2026-08-28 | Operator confirmed public IP 152.236.34.15 (matches live capture); prep decisions closed | §5 |
| 2026-08-28 | Activation scaffold on branch `infra/lat4-nixos-runner-twin` (PR #736): flake, build/deploy/CI/capture/runbook, contracts, docs; `test_lat4_closure_artifact` 9/9 green locally | PR #736 |
