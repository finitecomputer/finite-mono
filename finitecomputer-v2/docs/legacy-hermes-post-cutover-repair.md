# Legacy Hermes post-cutover repair brief

Use this after the target passes its first Finite Chat round trip and before
activating any legacy skill, scheduled job, or external credential. It repairs
references; it does not change migration identity or import more state.

## Path map

| Legacy reference | Target disposition |
| --- | --- |
| `/home/node/workspace/...` | `/data/workspace/legacy-box1/workspace/...` |
| `/home/node/dev/...` | `/data/workspace/legacy-box1/dev/...` |
| `/home/node/uploads/...` | `/data/workspace/legacy-box1/uploads/...` |
| `/home/node/.brain/finite-mono/...` | Fresh `fbrain open` Working Tree; old Agent State and identity remain inert inside `source-home.tar` |
| `~/dev/llm-wiki/...` and other source-only paths | Preserved automatically in `source-home.tar`; map into active state only when a compatible target exists |
| `/home/node/.hermes/*_cache/...` session media | Preserved in `source-home.tar`, not active during the canary |
| any path named by the source-volume inventory | Follow its automatic `activate`, `converted`, `preserve`, `quarantine`, or `rebuild` disposition |

## Repair order

1. Keep every frozen legacy cron definition and legacy skill inactive in the
   migration review-only directory.
2. Establish the new Agent Principal's Finite Brain Email Access Delegation and
   Folder Key Grants through the normal product flow. Do not copy an nsec,
   `.finite`, `.brain`, or fbrain Agent State from the inert source snapshot
   into active target paths.
3. Run `fbrain doctor`, list the authoritative Brains, open the approved Brain,
   sync it, and require zero unresolved conflicts.
4. Search imported `MEMORY.md`, `USER.md`, review-only jobs, and review-only
   skills for `/home/node`, `~/dev`, and `.brain` references. Use the map above
   only where the destination is proven.
5. Compare those references with the sealed source-volume inventory. A
   preserved, quarantined, or rebuilt path remains available in
   `source-home.tar`, but it must not become an active dependency without a
   compatible target mapping.
6. Compare each review-only skill name with the Runtime's Managed Skills
   Baseline. Retire stale Finite-managed copies. Promote a user-owned skill only
   after its tools and paths pass on the target.
7. Review each scheduled job's paths, Brain dependency, delivery route, and
   credential need. Recreate it paused, then activate and verify one job at a
   time under separate approval.

## Complete when

- the new Agent Principal can sync the approved Brain with no copied legacy
  identity state;
- no active memory, skill, or job depends on a preserve-only source path;
- every legacy skill and job has a recorded retire, replace, or promote
  disposition; and
- preserved media and cron-output history remain sealed in `source-home.tar`
  rather than being mistaken for active state.
