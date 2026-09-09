# Code map

CodeCharta displays committed first-party source metrics for `finite-mono`.
Canonical producer source lives here. The Finite Sites Project Repository
contains a copy of these scripts and built bytes; only its `site/` directory
is served. This deploy does not change chat, runtime state, or fleet topology.

## Build and preview

From the monorepo root:

```sh
nix develop .#code-map -c python3 infra/code-map/test_metrics.py
nix develop .#code-map -c python3 infra/code-map/build.py --output /tmp/finite-code-map-site
nix develop .#code-map -c python3 -m http.server 8742 --bind 127.0.0.1 --directory /tmp/finite-code-map-site
```

The output directory must not exist. `--ref COMMIT` selects another snapshot.
Uncommitted/untracked source edits are not analyzed. The scanner is pinned by
the root Nix flake; the CodeCharta 2.2.0 npm archive is SHA-512 pinned in build.py.
The browser loads `/finite-mono.cc.json`. `/build.json` records the source SHA,
snapshot date, history window and tool versions. No source file contents,
credentials or local identity files are included in the served output.

## Publish and refresh

Initialize the Project through `fsite project init --config infra/code-map/finite.toml`
after its dry run passes and the owner mailbox is verified. Use
`fsite auth git finite-code-map --store --output json` for a scoped credential.
Keep output access private unless the owner explicitly authorizes public access.
Inspect the exact returned remote and URL with `fsite project status`.

```sh
nix develop .#code-map -c python3 infra/code-map/publish.py \
  --site /tmp/finite-code-map-site --remote "$PROJECT_GIT_REMOTE"
fsite project status finite-code-map --output json
```

The Code map GitHub workflow builds after merges to main and on manual runs.
It retains each build as a 30-day Actions artifact and pushes successful main
builds to the Sites Project. Pull requests touching this tooling test/build
without receiving publishing credentials. Automatic refresh starts only once
the workflow is merged and the following repository settings are configured:

- Variable `FINITE_CODE_MAP_GIT_REMOTE`: the remote returned by fsite.
- Secrets `FINITE_CODE_MAP_GIT_USERNAME` and `FINITE_CODE_MAP_GIT_PASSWORD`:
  a Project-scoped Git credential. Never use the owner's identity key here.

The publisher uses ordinary Git after fsite establishes Project authority.
It refuses a changed remote configuration and never force-pushes. Sites
Versions and deployment Git history retain prior published artifacts. If an
update fails, the previous successful Version remains available. Do not claim
that restoring old metrics restores any application or user state.

## Reading the map

- Area `rloc`: source lines, excluding blanks/comments; includes tests.
- Height `complexity_estimate`: scc lexical branch/loop estimate, not an AST
  analysis, architecture score, or quality verdict. Compare within a language.
- Color `touches_30d`: first-parent commit touches in the 30 days before the
  snapshot's commit date. Merged changes count once; root commits are excluded.
- `churn_30d` sums additions and deletions, including comments and blanks.
  It is activity, not net growth. Renames are not followed; deleted files are
  absent. Pre-monorepo component history was not imported.
- `main_file_loc` excludes dedicated test/fixture paths but INCLUDES inline
  tests. `contains_inline_rust_tests` flags literal `#[cfg(test)]` occurrences.
- Markdown, manifests, lockfiles and non-source data are excluded. Generated
  and vendor filtering uses path/header heuristics. See metrics.py's language
  allowlist and summary.json's exclusions for the exact scope.
- Explorer percentages represent file counts, not lines of code.

Use large or active files to choose code to read. Red is activity, not a defect.
Splitting a file does not by itself simplify responsibilities or recovery paths.
Historical comparison UI and dependency edges are not part of this first version.
