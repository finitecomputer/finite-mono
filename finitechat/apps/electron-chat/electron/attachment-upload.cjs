const maxAttachmentFiles = 8;
const maxAttachmentBytes = 32 * 1024 * 1024;
const maxAttachmentTotalBytes = 64 * 1024 * 1024;
const maxAttachmentTextBytes = 16 * 1024;
const maxAttachmentFilenameBytes = 255;
const maxAttachmentMimeTypeBytes = 128;

function attachmentActionUsesBinaryTransport(action) {
  return Boolean(
    action &&
      typeof action === "object" &&
      !Array.isArray(action) &&
      ("SendAttachment" in action ||
        "SendAttachments" in action ||
        "SendChatAttachment" in action ||
        "SendChatAttachments" in action)
  );
}

function normalizedAttachmentText(value, fieldName, { required = false } = {}) {
  if (value === null || value === undefined) {
    if (required) {
      throw new Error(`Finite Chat attachment ${fieldName} is required`);
    }
    return null;
  }
  if (typeof value !== "string" || Buffer.byteLength(value) > maxAttachmentTextBytes) {
    throw new Error(`Finite Chat attachment ${fieldName} is invalid`);
  }
  const normalized = value.trim();
  if (required && !normalized) {
    throw new Error(`Finite Chat attachment ${fieldName} is invalid`);
  }
  return normalized || null;
}

function normalizedAttachmentFilename(value) {
  if (typeof value !== "string") {
    throw new Error("Finite Chat attachment filename is invalid");
  }
  const filename = value.split(/[\\/]/).at(-1)?.trim() ?? "";
  if (
    !filename ||
    Buffer.byteLength(filename) > maxAttachmentFilenameBytes ||
    [...filename].some((character) => /\p{Cc}/u.test(character))
  ) {
    throw new Error("Finite Chat attachment filename is invalid");
  }
  return filename;
}

function normalizedAttachmentMimeType(value) {
  if (typeof value !== "string") {
    return "application/octet-stream";
  }
  const normalized = value.trim().toLowerCase();
  const tokens = normalized.split("/");
  const validToken = (token) => token.length > 0 && /^[A-Za-z0-9!#$&^_.+-]+$/.test(token);
  return (
    Buffer.byteLength(normalized) <= maxAttachmentMimeTypeBytes &&
    tokens.length === 2 &&
    validToken(tokens[0]) &&
    validToken(tokens[1])
  )
    ? normalized
    : "application/octet-stream";
}

function attachmentBytes(value) {
  if (value instanceof ArrayBuffer) {
    return Buffer.from(value);
  }
  if (ArrayBuffer.isView(value)) {
    return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new Error("Finite Chat attachment bytes are invalid");
}

function validateAttachmentByteLengths(lengths) {
  if (!Array.isArray(lengths) || lengths.length === 0 || lengths.length > maxAttachmentFiles) {
    throw new Error(`Finite Chat attachments must include between 1 and ${maxAttachmentFiles} files`);
  }
  let totalBytes = 0;
  for (const length of lengths) {
    if (!Number.isSafeInteger(length) || length <= 0 || length > maxAttachmentBytes) {
      throw new Error(`Each Finite Chat attachment must be between 1 and ${maxAttachmentBytes} bytes`);
    }
    totalBytes += length;
    if (totalBytes > maxAttachmentTotalBytes) {
      throw new Error(`Finite Chat attachments must total at most ${maxAttachmentTotalBytes} bytes`);
    }
  }
  return totalBytes;
}

function attachmentUploadForm(upload) {
  if (!upload || typeof upload !== "object" || Array.isArray(upload)) {
    throw new Error("Finite Chat attachment upload is invalid");
  }
  if (!Array.isArray(upload.files)) {
    throw new Error("Finite Chat attachment files are invalid");
  }
  const roomId = normalizedAttachmentText(upload.room_id, "room_id", { required: true });
  const topicId = normalizedAttachmentText(upload.topic_id, "topic_id");
  const chatId = normalizedAttachmentText(upload.chat_id, "chat_id");
  if (Boolean(topicId) !== Boolean(chatId)) {
    throw new Error("Finite Chat attachment topic_id and chat_id must be provided together");
  }
  const caption = normalizedAttachmentText(upload.caption, "caption") ?? "";
  const replyToMessageId = normalizedAttachmentText(upload.reply_to_message_id, "reply_to_message_id");
  const files = upload.files.map((file) => {
    if (!file || typeof file !== "object" || Array.isArray(file)) {
      throw new Error("Finite Chat attachment file is invalid");
    }
    return {
      filename: normalizedAttachmentFilename(file.filename),
      mimeType: normalizedAttachmentMimeType(file.mime_type),
      bytes: attachmentBytes(file.bytes),
    };
  });
  validateAttachmentByteLengths(files.map((file) => file.bytes.byteLength));

  const form = new FormData();
  form.append("room_id", roomId);
  if (topicId && chatId) {
    form.append("topic_id", topicId);
    form.append("chat_id", chatId);
  }
  form.append("caption", caption);
  if (replyToMessageId) {
    form.append("reply_to_message_id", replyToMessageId);
  }
  for (const file of files) {
    form.append("files", new Blob([file.bytes], { type: file.mimeType }), file.filename);
  }
  return form;
}

async function forwardAttachmentUpload(upload, sendForm) {
  if (typeof sendForm !== "function") {
    throw new Error("Finite Chat attachment transport is unavailable");
  }
  return sendForm(attachmentUploadForm(upload));
}

module.exports = {
  attachmentActionUsesBinaryTransport,
  attachmentUploadForm,
  forwardAttachmentUpload,
  validateAttachmentByteLengths,
};
