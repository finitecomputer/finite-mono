# lat4 Provisioning Prep (finite-lat-4)

Status: **prep/research — no production mutation, no flake registration yet.**
This record captures the read-only evidence gathered for the finite-lat-4
runner-only host and the decisions taken before the implementation PR.

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

## 3. Alignment with PR #715 / ADR 0007 (2026-08-28)

PR #715 (`infra/lat2-nixos-runner-twin`, Paul) scaffolds lat2 as the second
NixOS runner twin of lat3 and is the planning authority for the runner-twin
pattern. lat4's prep was re-aligned to it:

| Topic | PR #715 / ADR 0007 decision | lat4 adoption |
|---|---|---|
| WireGuard | `wg-finite` widens /30 → /29: lat1=.1, lat3=.2, **lat2=.3** | lat4 = **10.254.3.4/29**; draft updated from the earlier 10.254.4.0/30 proposal; allowedIPs minimal `10.254.3.1/32` like lat2 |
| Ceiling | mirrors lat3's owner-authorized **42** (same chassis class, 4 vCPU/8 GiB, swap is not capacity) | `maxSandboxes = 42` in default.nix and runner.env.example (was 32) |
| Admission | drained-first; undrain is the separate Gate E owner decision; one-creator rule stands | `FC_RUNNER_DRAIN=true`; undrain deferred to a lat4 admission gate |
| Storage identity | `storage-ids.nix` carries `captured = true/false`; build script refuses uncaptured closures | lat4 ships `captured = true` with live-captured values + evidence; the lat4 build-script fork must keep the same guard |
| Install | CI artifact carries toplevel + **disko script + same-pin kexec tarball**; rescue-mode artifact-driven install; runbook with Gates A–E | lat4 clones the lat2 runbook/artifact pattern instead of inventing a path |
| Evidence | `infra/nixos/scripts/capture-lat2-host-evidence` (read-only, Gate B) | adapt as `capture-lat4-host-evidence`; lat4 §1-2 evidence was gathered the same way manually |
| Contracts | runner-host contract goes to 3 hosts in #715 | lat4 activation PR goes to 4 hosts, based on the post-#715 tree |

Paul's review note on #715 ("very nix-shaped and probably does too much copy
and pasting where instead we could share stuff") applies to lat4's verbatim
clones too. Before or during the lat4 activation PR, extract the twin files
(disko/invariants/esp-guard/storage-health) into a shared, storage-ids +
host-label-parameterized module set rather than a third copy. Sequence this
with the outcome of that #715 review discussion.

Sequence: the /29 widening and lat2=.3 land first via #715; lat4=.4 is only
valid after that merge. If #715 stalls, lat4 falls back to a separate /30 —
but the default plan is ADR 0007's single overlay.

## 4. Drafted artifacts (uncommitted; flake untouched)

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

## 5. Open questions (must be answered before activation PR)

Answered by PR #715 / ADR 0007 since the first draft (see §3):

1. ~~WireGuard topology~~ → single overlay, /29 widening first, lat4 =
   10.254.3.4. **Sequencing dependency: #715 must merge before lat4=.4 is
   valid.** lat1's lat4 peer entry + firewall are the only lat1-side changes.
2. ~~Sandbox cap~~ → mirror lat3's owner-authorized 42; undraining lat4 needs
   its own owner authorization at the admission gate.

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
   gap) and `FC_CORE_RUNNER_CREDENTIALS_JSON` registration for
   `finite-kata-runner-4` in Core.
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
