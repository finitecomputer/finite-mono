import type {
  HostedChatMediaAttachment,
  HostedChatMessage,
  HostedChatReference,
  HostedChatReferenceSearchResult,
} from "@/lib/hosted-web-device";

export type ChatReferenceKind = "file" | "skill" | "site";

export type ChatReference = {
  kind: ChatReferenceKind;
  id: string;
  label: string;
  detail: string;
  token?: string;
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
  references: ChatReference[] = [],
): { text: string; cursor: number; reference: ChatReference } {
  const baseToken = `@${reference.label}`;
  const usedTokens = new Set(references.map(inlineReferenceToken));
  let token = baseToken;
  for (let occurrence = 2; usedTokens.has(token); occurrence += 1) {
    token = `${baseToken}#${occurrence}`;
  }
  const next = `${text.slice(0, query.start)}${token}${text.slice(query.end)}`;
  return {
    text: next,
    cursor: query.start + token.length,
    reference: { ...reference, token },
  };
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
  return reference.token ?? `@${reference.label}`;
}

export function hasInlineReferenceAt(text: string, start: number, token: string) {
  if (text.slice(start, start + token.length) !== token) return false;
  const previous = start > 0 ? text[start - 1] : "";
  const next = text[start + token.length] ?? "";
  return (!previous || /[\s([{"']/u.test(previous))
    && (!next || /[\s.,!?;:)\]}"']/u.test(next));
}

export function chatReferencePayloads(
  references: ChatReference[],
): HostedChatReference[] {
  return references.slice(0, MAX_REFERENCE_COUNT).map(referenceWireValue);
}

function referenceWireValue(reference: ChatReference): HostedChatReference {
  return {
    kind: reference.kind,
    id: reference.id.slice(0, 512),
    label: reference.label.slice(0, 256),
    detail: reference.detail.slice(0, 1024),
    token: inlineReferenceToken(reference).slice(0, 512),
    ...(reference.path ? { path: reference.path.slice(0, 1024) } : {}),
    ...(reference.url ? { url: reference.url.slice(0, 2048) } : {}),
    ...(reference.fingerprint ? { fingerprint: reference.fingerprint.slice(0, 256) } : {}),
  };
}

export function messageChatReferences(message: HostedChatMessage): ChatReference[] {
  return (message.references ?? []).map((reference) => ({
    kind: reference.kind,
    id: reference.id,
    label: reference.label,
    detail: reference.detail,
    token: reference.token,
    ...(reference.path ? { path: reference.path } : {}),
    ...(reference.url ? { url: reference.url } : {}),
    ...(reference.fingerprint ? { fingerprint: reference.fingerprint } : {}),
  }));
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
