# Source structure checks

Run `just source-structure-check`. The required CI Source structure job runs
the same checker and its tests on every PR, including guide-only edits.

The initial scope is all Rust source and tests in `finite-saas-core`, the
checker itself, and the root/v2 agent guides. Other components are not yet
claimed to conform. Add their source/test roots to
`.config/source-structure.json` after cleaning them up.

- Rust files: at most 1,000 physical lines, including tests and comments.
  Prefer 300–700 lines with room for ordinary maintenance.
- Modules: external files only, including `#[cfg(test)]` modules. Group by
  responsibility, not numbered chunks. `include!` cannot bypass the limit;
  `include_str!` for migrations and fixtures is fine.
- Structs: at most 20 declared fields, named or tuple. Group internal state
  by responsibility. Existing public wire/row contracts may have an exact,
  reasoned exception keyed by file and struct in the configuration. Growth,
  shrinkage, and stale paths fail until the exception is reviewed or removed.
  Do not reshape serialized or persisted data merely to satisfy this limit.
- Agent guides: root at most 60 lines and v2 at most 35. Keep essential
  invariants and precise triggers there; use component `AGENTS.md` files and
  link existing contracts or runbooks for conditional detail.

The checker uses `syn` rather than text matching. It visits all written syntax,
including cfg-disabled items and local structs; comments, strings, and enum
variants are not structs. Macro expansion is outside its scope. Keep reusable
source in modules rather than generating handwritten application structure
through macros. Missing roots, unreadable files, and parse errors fail the
check. No schema, migration, or API behavior is changed by these checks.
