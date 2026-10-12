# Offline CI evidence contract

This FIN-174 tool validates synthetic JSON and an inert fake executor. CI runs
only the tool's own tests; no Ronin workflow or input producer calls it, and it
cannot qualify a real mechanism.
`FIXTURE_PASS` means only that the supplied offline fixture met this contract
and the fake process returned zero within its budgets. All reports include
`real_mechanism_qualified: false`.

Run scoped Ruff format and lint checks and the suite with the repository's
Nix-managed tools. CI selects this recipe for code or fixture changes here and
whenever it selects every harness. README-only changes are docs-only.

```sh
just ronin-ci-evidence-check
```

`run.py --manifest PATH --scenario PATH` invokes only `fake_tool.py` using the
same Python interpreter. The scenario is JSON with exactly `exit_code` (integer
0..255), `stdout` and `stderr` (strings), and `delay_ms` (integer 0..5000).
It never executes text from those fields. The `run_fixture` API also permits an
injected executor returning `Execution`; tests use that seam for invalid UTF-8,
signed signal status, and executor failures. No arbitrary-command CLI exists.

The manifest is trusted input, distinct from evidence. Both are strict UTF-8
JSON objects with integer schema version 1 and kind `OFFLINE_FIXTURE`. Unknown
fields, duplicate keys, unknown versions, nonstandard numbers and trailing text
are errors. Missing input is an error. Required values have no inferred defaults.

The manifest sets an explicit run nonce, declared source revision (40 lowercase
hex characters), declared binary SHA-256 (64 lowercase hex characters), topology,
required capability and an ordered, nonempty list of uniquely named cases.
Evidence must match the exact nonce and declared identities, and contain every
case once in manifest order. These identity comparisons do not measure source
or executable bytes. The fixture's repeated `a` digest is intentionally
synthetic. The fixed fixture nonce is a test value; freshness requires a future
workflow to create and independently supply a fresh nonce.

Version 1 recognizes only `single-process-fixture-v1` and `synthetic-json-v1`.
Unknown manifest topology/capability requirements fail closed before execution;
an unsupported declared evidence topology or a false capability cannot pass.
Each case's required progress uses the recognized fixture markers in the
manifest's exact order. A shorter prefix is inconclusive; extra, unknown,
duplicate or reordered markers are errors. These markers describe synthetic
fixture declarations, never observed guest progress.

Success/completion, case success/completion, capability and cleanup resolution
must be JSON Booleans; numeric 0/1 and strings are rejected. Acceptance requires
outcome `COMPLETE`, phase `COMPLETE`, every success/completion true, exact progress,
cleanup resolved true and pending cleanup empty. Valid evidence takes the first
status that applies: `ERROR` for outcome `ERROR`; `UNSUPPORTED` for an
unsupported topology, a false capability or outcome `UNSUPPORTED`;
`INCONCLUSIVE` for outcome `INCONCLUSIVE`, partial phase/completion or
unresolved cleanup, even when success is false; then `ERROR` for failed overall
or case success. Explicit `UNSUPPORTED`, `INCONCLUSIVE` and `ERROR` outcomes
require success false and remain nonpassing. The whole document is checked
before any acceptance.

The fake process has a 2-second wall-time budget and a combined stdout/stderr
budget of 65,536 bytes. CLI test options may tighten these ceilings, never widen
them. JSON read from a file or passed to `validate_evidence` is capped at
262,144 bytes. Through the wrapper the fake's 65,536-byte output budget is
smaller, so oversized fake output exits 125 whatever the evidence says. Both
pipes are drained incrementally; on timeout or overflow the owned fake is
killed and reaped. These are offline test-tool bounds, with no runtime memory,
admission, capacity or retry policy.

The report retains stdout/stderr as UTF-8 decoded display strings, using
replacement for invalid or truncated sequences. These strings are not exact
raw bytes. The combined captured-byte ceiling applies before decoding, so
re-encoding the display strings can produce a different byte count. The actual
signed inner returncode is retained.
For an ordinary nonzero process exit, the outer exit is the same value,
including 42, 126 and 137, even with otherwise valid evidence. A negative inner
signal status is retained and maps to 128+signal for the outer exit. Wrapper
guards exit with timeout 124, output limit 125, executor failure/missing status
127 and invalid manifest/input 64. With inner zero, evidence error exits 65,
unsupported exits 77 and inconclusive exits 78. Only a complete fixture result
exits zero. Outer exits are not unique: an inner 77, 78 or 124 passes through
as `ERROR` with the same code as unsupported or inconclusive evidence or the
timeout guard. The report's `status` is authoritative and every nonzero exit is
nonpassing; never map an exit code to a softer outcome. Printed `PASS` text has
no authority. The tests assert each ordinary mutation fails; they do not turn a
recorded failure into a pass.

Set `RONIN_CI_TEST_ARTIFACTS` to a scratch directory to retain each actual CLI
command, scenario, raw outer output, report and observed outer exit during the
scoped suite. Invalid UTF-8 and signal mapping are explicitly injected cases;
the positive control and the 42/126/137 controls use actual OS subprocesses.

Before any future integration can claim real qualification, reviewers must
approve the real workflow and input producer, independently verify source and
binary identity and nonce freshness, observe authenticated guest progress and
all required cases/phases, verify the actual topology and capabilities, and
prove execution status and cleanup through the real outer workflow. None of
those gates is met here. The investigation is tracked in
[FIN-174](https://linear.app/finitecomputer/issue/FIN-174).
