import type {
  HostedChatMediaAttachment,
  HostedChatMessage,
  HostedChatReferenceSearchResult,
} from "@/lib/hosted-web-device";

export type ChatReferenceKind = "file" | "skill" | "site";

export type ChatReference = {
  kind: ChatReferenceKind;
  id: string;
  label: string;
  detail: string;
  path?: string;
  url?: string;
  fingerprint?: string;
  attachment?: {
    room_id: string;
    message_id: string;
    attachment_id: string;
    mime_type: string;
  };
};

export type ActiveAtQuery = {
  start: number;
  end: number;
  query: string;
};

const REFERENCE_LINE_PREFIX = "FINITE_CHAT_REFERENCES_V1 ";
const MAX_REFERENCE_COUNT = 12;

export function activeAtQuery(
  text: string,
  cursor: number,
  references: ChatReference[] = [],
): ActiveAtQuery | null {
  const end = Math.max(0, Math.min(cursor, text.length));
  const before = text.slice(0, end);
  if (insideCode(before)) return null;
  const start = before.lastIndexOf("@");
  if (start < 0) return null;
  if (
    references.some((reference) => {
      const token = inlineReferenceToken(reference);
      return end >= start + token.length && hasInlineReferenceAt(text, start, token);
    })
  ) return null;
  const previous = start > 0 ? before[start - 1] : "";
  if (previous && /[\p{L}\p{N}_]/u.test(previous)) return null;
  const query = before.slice(start + 1);
  if (query.includes("\n") || query.length > 128) return null;
  return { start, end, query };
}

function insideCode(value: string) {
  const fences = value.match(/```/gu)?.length ?? 0;
  if (fences % 2 === 1) return true;
  const withoutFencedBlocks = value.replace(/```[\s\S]*?```/gu, "");
  const ticks = withoutFencedBlocks.match(/(?<!`)`(?!`)/gu)?.length ?? 0;
  return ticks % 2 === 1;
}

export function insertAtReference(
  text: string,
  query: ActiveAtQuery,
  reference: ChatReference,
): { text: string; cursor: number } {
  const token = inlineReferenceToken(reference);
  const next = `${text.slice(0, query.start)}${token}${text.slice(query.end)}`;
  return { text: next, cursor: query.start + token.length };
}

export function retainInlineReferences(
  text: string,
  references: ChatReference[],
) {
  return references.filter((reference) => {
    const token = inlineReferenceToken(reference);
    let start = text.indexOf(token);
    while (start >= 0) {
      if (hasInlineReferenceAt(text, start, token)) return true;
      start = text.indexOf(token, start + token.length);
    }
    return false;
  });
}

export function inlineReferenceToken(reference: ChatReference) {
  return `@${reference.label}`;
}

function hasInlineReferenceAt(text: string, start: number, token: string) {
  if (text.slice(start, start + token.length) !== token) return false;
  const previous = start > 0 ? text[start - 1] : "";
  const next = text[start + token.length] ?? "";
  return (!previous || /[\s([{"']/u.test(previous))
    && (!next || /[\s.,!?;:)\]}"']/u.test(next));
}

export function serializeChatReferences(text: string, references: ChatReference[]) {
  if (references.length === 0) return text;
  const safe = references.slice(0, MAX_REFERENCE_COUNT).map(referenceWireValue);
  const instructions = safe.map(referenceInstruction).join("\n");
  return [
    text.trim(),
    instructions,
    `${REFERENCE_LINE_PREFIX}${JSON.stringify(safe)}`,
  ].filter(Boolean).join("\n\n");
}

export function parseChatReferences(text: string): {
  text: string;
  references: ChatReference[];
} {
  const lines = text.split("\n");
  const encodedIndex = lines.findLastIndex((line) => line.startsWith(REFERENCE_LINE_PREFIX));
  if (encodedIndex < 0) return { text, references: [] };
  let parsed: unknown;
  try {
    parsed = JSON.parse(lines[encodedIndex]!.slice(REFERENCE_LINE_PREFIX.length));
  } catch {
    return { text, references: [] };
  }
  if (!Array.isArray(parsed) || parsed.length > MAX_REFERENCE_COUNT) {
    return { text, references: [] };
  }
  const references = parsed.map(parseReference).filter((value): value is ChatReference => Boolean(value));
  if (references.length !== parsed.length) return { text, references: [] };
  const instructionStart = Math.max(0, encodedIndex - references.length * 2);
  return {
    text: lines.slice(0, instructionStart).join("\n").trimEnd(),
    references,
  };
}

function referenceWireValue(reference: ChatReference): ChatReference {
  return {
    kind: reference.kind,
    id: reference.id.slice(0, 512),
    label: reference.label.slice(0, 256),
    detail: reference.detail.slice(0, 1024),
    ...(reference.path ? { path: reference.path.slice(0, 1024) } : {}),
    ...(reference.url ? { url: reference.url.slice(0, 2048) } : {}),
    ...(reference.fingerprint ? { fingerprint: reference.fingerprint.slice(0, 256) } : {}),
    ...(reference.attachment ? { attachment: reference.attachment } : {}),
  };
}

function referenceInstruction(reference: ChatReference) {
  if (reference.kind === "skill") {
    return `Skill reference: ${JSON.stringify(reference.label)}. Load and follow this skill for this turn. If it is unavailable or inappropriate, explain why and offer the closest alternative.`;
  }
  if (reference.kind === "site") {
    return `Site reference: ${JSON.stringify(reference.label)} at ${reference.url}. Use this exact Finite Site for the user's request; edit it only if existing authorization permits.`;
  }
  if (reference.path) {
    const version = reference.fingerprint
      ? ` It was selected with source fingerprint ${JSON.stringify(reference.fingerprint)}; verify the current file before use, and if it changed, tell the user and ask whether to use the update.`
      : "";
    return `File reference: ${JSON.stringify(reference.path)}. Open and use this Agent workspace Markdown file for this turn.${version}`;
  }
  return `File attachment reference: ${JSON.stringify(reference.label)}. Use the attached file for this turn.`;
}

function parseReference(value: unknown): ChatReference | null {
  if (!value || typeof value !== "object") return null;
  const entry = value as Record<string, unknown>;
  if (
    (entry.kind !== "file" && entry.kind !== "skill" && entry.kind !== "site")
    || typeof entry.id !== "string"
    || typeof entry.label !== "string"
    || typeof entry.detail !== "string"
  ) return null;
  return {
    kind: entry.kind,
    id: entry.id,
    label: entry.label,
    detail: entry.detail,
    ...(typeof entry.path === "string" ? { path: entry.path } : {}),
    ...(typeof entry.url === "string" ? { url: entry.url } : {}),
    ...(typeof entry.fingerprint === "string" ? { fingerprint: entry.fingerprint } : {}),
  };
}

export function runtimeReference(
  result: HostedChatReferenceSearchResult
): ChatReference {
  return {
    kind: result.kind,
    id: result.id,
    label: result.label,
    detail: result.description || result.detail,
    ...(result.path ? { path: result.path } : {}),
    ...(result.url ? { url: result.url } : {}),
    ...(result.fingerprint ? { fingerprint: result.fingerprint } : {}),
  };
}

export function uploadedFileReferences(
  messages: HostedChatMessage[]
): ChatReference[] {
  const seen = new Set<string>();
  const results: ChatReference[] = [];
  for (const message of [...messages].reverse()) {
    for (const media of message.media ?? []) {
      const key = `${message.room_id}:${message.message_id}:${media.attachment_id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      results.push(uploadedFileReference(message, media));
    }
  }
  return results;
}

function uploadedFileReference(
  message: HostedChatMessage,
  media: HostedChatMediaAttachment
): ChatReference {
  return {
    kind: "file",
    id: `attachment:${message.room_id}:${message.message_id}:${media.attachment_id}`,
    label: media.filename,
    detail: "Uploaded in this chat",
    attachment: {
      room_id: message.room_id,
      message_id: message.message_id,
      attachment_id: media.attachment_id,
      mime_type: media.mime_type,
    },
  };
}

export function rankLocalReferences(references: ChatReference[], query: string) {
  const normalized = query.trim().toLowerCase();
  return references
    .map((reference, index) => ({
      reference,
      index,
      score: localReferenceScore(reference, normalized),
    }))
    .filter((entry) => entry.score > 0)
    .sort((left, right) => right.score - left.score || left.index - right.index)
    .map((entry) => entry.reference);
}

function localReferenceScore(reference: ChatReference, query: string) {
  if (!query) return 1;
  const label = reference.label.toLowerCase();
  const detail = reference.detail.toLowerCase();
  if (label === query) return 100;
  if (label.startsWith(query)) return 90;
  if (label.includes(query)) return 80;
  if (detail.includes(query)) return 50;
  return 0;
}
