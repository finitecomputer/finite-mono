const attachmentMediaScheme = "finitechat-media";
const maxOpaqueIdBytes = 1024;

function validOpaqueId(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    Buffer.byteLength(value) <= maxOpaqueIdBytes &&
    value !== "." &&
    value !== ".." &&
    !/[\\/]/.test(value) &&
    ![...value].some((character) => /\p{Cc}/u.test(character))
  );
}

function attachmentMediaUrl({ room_id, message_id, attachment_id }) {
  if (![room_id, message_id, attachment_id].every(validOpaqueId)) {
    throw new Error("Finite Chat attachment media identifier is invalid");
  }
  return `${attachmentMediaScheme}://attachment/${encodeURIComponent(room_id)}/${encodeURIComponent(message_id)}/${encodeURIComponent(attachment_id)}`;
}

function parseAttachmentMediaUrl(rawUrl) {
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new Error("Finite Chat attachment media address is invalid");
  }
  if (
    url.protocol !== `${attachmentMediaScheme}:` ||
    url.hostname !== "attachment" ||
    url.username ||
    url.password ||
    url.port ||
    url.search ||
    url.hash
  ) {
    throw new Error("Finite Chat attachment media address is invalid");
  }
  const pathParts = url.pathname.split("/");
  if (pathParts.length !== 4 || pathParts[0] !== "" || pathParts.slice(1).some((part) => !part)) {
    throw new Error("Finite Chat attachment media address is invalid");
  }
  const encodedParts = pathParts.slice(1);
  let parts;
  try {
    parts = encodedParts.map(decodeURIComponent);
  } catch {
    throw new Error("Finite Chat attachment media address is invalid");
  }
  if (!parts.every(validOpaqueId)) {
    throw new Error("Finite Chat attachment media address is invalid");
  }
  return {
    room_id: parts[0],
    message_id: parts[1],
    attachment_id: parts[2],
  };
}

module.exports = {
  attachmentMediaScheme,
  attachmentMediaUrl,
  parseAttachmentMediaUrl,
};
