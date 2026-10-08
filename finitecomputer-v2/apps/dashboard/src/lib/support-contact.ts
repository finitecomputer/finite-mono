/** Deployment-owned contact; an unset self-hosted install never routes to Finite. */
export function supportContact(value: string | undefined): string | null {
  if (!value || value.length > 254 || /[^\x21-\x7e]/u.test(value)) return null;
  const parts = value.split("@");
  if (parts.length !== 2) return null;
  const [local, domain] = parts as [string, string];
  if (!local || local.length > 64 || local.startsWith(".") || local.endsWith(".") || local.includes("..")) return null;
  if (!/^[a-zA-Z0-9.!#$%&'*+\-/=?^_`{|}~]+$/u.test(local)) return null;
  if (!domain.includes(".") || !domain.split(".").every((label) => /^[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?$/u.test(label))) return null;
  return value;
}

/** Exact command only: ordinary text and /supporting remain normal chat. */
export function supportCommand(text: string): string | null {
  const match = /^\/support(?:\s+([\s\S]*))?$/u.exec(text.trim());
  return match ? (match[1] ?? "").trim() : null;
}

export function supportEmailHref(email: string): string {
  return `mailto:${encodeURIComponent(email)}`;
}
