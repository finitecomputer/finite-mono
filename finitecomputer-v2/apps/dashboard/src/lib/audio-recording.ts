export type AudioRecordingFormat = {
  recorderMimeType: string;
  fileMimeType: "audio/webm" | "audio/mp4";
  extension: "webm" | "m4a";
};

export const MAX_AUDIO_RECORDING_SECONDS = 10 * 60;
export const AUDIO_RECORDING_BITS_PER_SECOND = 96_000;
export const MAX_AUDIO_RECORDING_BYTES = 24 * 1024 * 1024;

const AUDIO_RECORDING_FORMATS: readonly AudioRecordingFormat[] = [
  {
    recorderMimeType: "audio/webm;codecs=opus",
    fileMimeType: "audio/webm",
    extension: "webm",
  },
  {
    recorderMimeType: "audio/webm",
    fileMimeType: "audio/webm",
    extension: "webm",
  },
  {
    recorderMimeType: "audio/mp4;codecs=mp4a.40.2",
    fileMimeType: "audio/mp4",
    extension: "m4a",
  },
  {
    recorderMimeType: "audio/mp4",
    fileMimeType: "audio/mp4",
    extension: "m4a",
  },
];

export function supportedAudioRecordingFormat(
  isTypeSupported: (mimeType: string) => boolean
): AudioRecordingFormat | null {
  return AUDIO_RECORDING_FORMATS.find(
    ({ recorderMimeType }) => isTypeSupported(recorderMimeType)
  ) ?? null;
}

export function audioRecordingFilename(
  recordedAt: Date,
  format: AudioRecordingFormat
): string {
  const timestamp = recordedAt.toISOString().replace(/[:.]/gu, "-");
  return `voice-${timestamp}.${format.extension}`;
}

export function audioRecordingDurationLabel(totalSeconds: number): string {
  const boundedSeconds = Math.max(
    0,
    Math.min(MAX_AUDIO_RECORDING_SECONDS, Math.floor(totalSeconds))
  );
  const minutes = Math.floor(boundedSeconds / 60);
  const seconds = boundedSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function audioRecordingErrorMessage(reason: unknown): string {
  const name = reason instanceof DOMException
    ? reason.name
    : typeof reason === "object" && reason !== null && "name" in reason
      ? String(reason.name)
      : "";
  switch (name) {
    case "NotAllowedError":
    case "PermissionDeniedError":
      return "Microphone access was denied. Allow it in your browser or system settings and try again.";
    case "NotFoundError":
    case "DevicesNotFoundError":
      return "No microphone is available.";
    case "NotReadableError":
    case "TrackStartError":
      return "The microphone is already in use or unavailable.";
    case "SecurityError":
      return "Microphone recording requires a secure connection.";
    default:
      return "Could not start microphone recording. Try again.";
  }
}
