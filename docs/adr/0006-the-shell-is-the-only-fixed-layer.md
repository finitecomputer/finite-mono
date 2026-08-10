# The shell is the only fixed layer; everything else is a pulled generation

Status: proposed. Supersedes the transport half of ADR 0002.

An Agent Runtime is split into a minimal `finite-shell` (boot, verify, fetch,
flip, rollback, supervise agentd, report health) baked into the rarely-changing
OCI image, and a runtime payload (agentd, finitechat, Hermes, CLIs) that lives
on `/data` as signed, versioned generations selected by an atomic symlink.
Agents converge themselves by pulling their release channel's head from the
signed service directory; nobody stops or replaces compute to update software.
Managed skills move on their own faster content channel through the same
fetch/verify/atomic-swap discipline, keeping ADR 0002's guarantees (failed
sync leaves the previous baseline usable; user-owned skills untouched) while
reversing its no-transport stance: observed skill/service drift proved that
update-at-your-own-pace does not converge. The shell is deterministic code
with no LLM involvement; Hermes may only choose the timing of a flip, never
its mechanics. The previous generation always remains on disk; an agent more
than one generation behind a channel head is a repair signal, not a supported
configuration.
