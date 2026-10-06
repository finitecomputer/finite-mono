# Skill Freshness

Use this branch when the installed CLI, a server response or a user-owned
skill disagrees with this skill.

## Managed baseline

In a hosted Agent Runtime this skill is a plain copy, without git metadata, in
the managed baseline `/data/agent/managed-skills/finite/current`. The baseline
was copied from the Runtime image bundle `/runtime/finite-skills` when the
agent was created and changes only when `finite skills sync` runs. A Runtime
restart or image upgrade leaves it as it was.

Verify command syntax with the installed CLI's help and errors, and server
behavior with current server responses. When they differ from the skill,
report the discrepancy and compare the baseline with the image:

```sh
diff -rq -x __pycache__ /runtime/finite-skills /data/agent/managed-skills/finite/current
```

No output means the baseline matches the running image. Any difference means
the baseline is behind or ahead of this image: tell the user, and run
`finite skills sync` when they agree. It replaces only the managed baseline.

A runtime that lists `finitebrain-agent` has a stale skill name; the current
name is `finitebrain`.

## User-owned skills

User-owned skills in `$HERMES_HOME/skills` load alongside this one and are the
user's data. Verify their technical claims against the installed CLI and
current server evidence. Preserve user instructions, authorization limits and
intentional customizations, including restrictions on bearer invitations.

Report an unresolved conflict before acting on the conflicting instruction:
name the skill, its path and the conflicting sentence. Edit or remove that
skill only when the user asks. Saved notes about a defect or workaround are
dated observations: recheck the CLI and current server behavior before
repeating them as advice.
