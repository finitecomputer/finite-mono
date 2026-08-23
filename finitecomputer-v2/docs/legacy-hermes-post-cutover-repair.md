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
| `/home/node/.brain/finite-mono/...` | Fresh `fbrain open` Working Tree; never copy the old Agent State or identity |
| `~/dev/llm-wiki/...` and other source-only paths | Review individually; map only after the target location exists |
| `/home/node/.hermes/*_cache/...` session media | Archive-only in the frozen source Recovery Set during the canary |

## Repair order

1. Keep the 12 legacy cron definitions and every legacy skill inactive in the
   migration review-only directory.
2. Establish the new Agent Principal's Finite Brain Email Access Delegation and
   Folder Key Grants through the normal product flow. Do not copy an nsec,
   `.finite`, `.brain`, or fbrain Agent State from box1.
3. Run `fbrain doctor`, list the authoritative Brains, open the approved Brain,
   sync it, and require zero unresolved conflicts.
4. Search imported `MEMORY.md`, `USER.md`, review-only jobs, and review-only
   skills for `/home/node`, `~/dev`, and `.brain` references. Use the map above
   only where the destination is proven.
5. Compare each review-only skill name with the Runtime's Managed Skills
   Baseline. Retire stale Finite-managed copies. Promote a user-owned skill only
   after its tools and paths pass on the target.
6. Review each scheduled job's paths, Brain dependency, delivery route, and
   credential need. Recreate it paused, then activate and verify one job at a
   time under separate approval.

## Complete when

- the new Agent Principal can sync the approved Brain with no copied legacy
  identity state;
- no active memory, skill, or job depends on an unresolved source-only path;
- every legacy skill and job has a recorded retire, replace, or promote
  disposition; and
- archive-only media and cron-output history remain named in the retained
  Recovery Set rather than being mistaken for migrated live state.
