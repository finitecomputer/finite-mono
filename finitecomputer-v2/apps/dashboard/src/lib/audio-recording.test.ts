import assert from "node:assert/strict";
import test from "node:test";

import {
  AUDIO_RECORDING_BITS_PER_SECOND,
  audioRecordingErrorMessage,
  audioRecordingDurationLabel,
  audioRecordingFilename,
  MAX_AUDIO_RECORDING_BYTES,
  MAX_AUDIO_RECORDING_SECONDS,
  supportedAudioRecordingFormat,
} from "./audio-recording";

test("audio recording prefers Opus WebM when Chrome supports it", () => {
  const format = supportedAudioRecordingFormat((mimeType) =>
    new Set(["audio/webm;codecs=opus", "audio/webm"]).has(mimeType)
  );
  assert.deepEqual(format, {
    recorderMimeType: "audio/webm;codecs=opus",
    fileMimeType: "audio/webm",
    extension: "webm",
  });
});

test("audio recording falls back to Safari-compatible MP4", () => {
  const format = supportedAudioRecordingFormat((mimeType) => mimeType === "audio/mp4");
  assert.deepEqual(format, {
    recorderMimeType: "audio/mp4",
    fileMimeType: "audio/mp4",
    extension: "m4a",
  });
  assert.equal(
    audioRecordingFilename(new Date("2026-07-23T14:05:06.789Z"), format!),
    "voice-2026-07-23T14-05-06-789Z.m4a"
  );
});

test("audio recording fails closed when no upload-safe format is supported", () => {
  assert.equal(supportedAudioRecordingFormat(() => false), null);
});

test("ten minutes of requested voice audio stays well below the encoded safety stop", () => {
  const requestedBytes =
    (AUDIO_RECORDING_BITS_PER_SECOND * MAX_AUDIO_RECORDING_SECONDS) / 8;
  assert(requestedBytes < MAX_AUDIO_RECORDING_BYTES / 2);
  assert.equal(audioRecordingDurationLabel(0), "0:00");
  assert.equal(audioRecordingDurationLabel(65.9), "1:05");
  assert.equal(audioRecordingDurationLabel(9_999), "10:00");
});

test("audio recording permission errors have actionable copy", () => {
  assert.match(
    audioRecordingErrorMessage({ name: "NotAllowedError" }),
    /browser or system settings/u
  );
  assert.equal(
    audioRecordingErrorMessage({ name: "NotFoundError" }),
    "No microphone is available."
  );
});
