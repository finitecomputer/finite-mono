# FIN-89 Brain overview prototype

Selected design: categorized membership cards (A), approved by Austin on September 15.

Run from the repository root in the pinned development environment:

```sh
just dashboard brain-prototype
```

Open http://127.0.0.1:13089/dashboard/machines/runtime_web_design/brain-prototype.
The existing design fixture supplies the dashboard shell and its example agent, Moss.
The Brain page uses in-memory synthetic memberships and makes no Brain requests.

The selected A cards show each Brain's folder count and up to three folder names, followed by `+N` for the remainder. “Folders shown” refers to the metadata visible to the selected identity; it excludes linked folders and does not measure local sync. Sample agent and human views have different folder visibility. These are example names and counts, not live inventory.

Use the preview controls to change identity and inspect empty, unavailable, stale, access-lost, and unavailable-folder-detail states. Missing folder details preserve the Brain's name and role without displaying a false zero count. Changing identity clears the displayed result before loading the new example. Refresh shows loading and updates the successful-fetch time; the failure scenarios remain failures until another scenario is selected. Expand “Prototype state” for the full example state.

Only the selected card layout remains on this branch. The original three-layout study is preserved in commit `89c9806c`; old `?variant=` links now display the selected cards.

The route requires development mode, the existing local-account fixture switch, and the fixture machine ID. Production requests return not found, and production navigation stays disabled. No new authentication bypass or Brain integration is introduced.

This is a throwaway review artifact on `prototype/brain-overview`, tracked in [FIN-89](https://linear.app/finitecomputer/issue/FIN-89/add-a-basic-brain-overview-with-categorized-membership-cards-and). Austin selected A (categorized cards) on September 15. The shipping identity scope and authorized agent inventory path remain unresolved; sample memberships do not validate that contract. File counts and storage size are deferred.
