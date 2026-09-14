export type ChatResponseActionResult = "cancelled" | "copied" | "failed" | "shared";

type ClipboardTarget = {
  clipboard?: {
    writeText: (text: string) => Promise<void>;
  };
};

type ShareTarget = ClipboardTarget & {
  share?: (data: { text: string; title: string }) => Promise<void>;
};

export async function copyChatResponse(
  target: ClipboardTarget,
  text: string
): Promise<ChatResponseActionResult> {
  try {
    if (!target.clipboard) return "failed";
    await target.clipboard.writeText(text);
    return "copied";
  } catch {
    return "failed";
  }
}

export async function shareChatResponse(
  target: ShareTarget,
  data: { text: string; title: string }
): Promise<ChatResponseActionResult> {
  if (target.share) {
    try {
      await target.share(data);
      return "shared";
    } catch (error) {
      if (isShareCancellation(error)) return "cancelled";
    }
  }

  return copyChatResponse(target, data.text);
}

function isShareCancellation(error: unknown) {
  return error instanceof Error && error.name === "AbortError";
}
