/** A composer error, tagged when it is the result of the person's own send. */
export type HostedChatComposerError = { message: string; fromSend: boolean };

export type HostedChatAlert = {
  kind: "session" | "send" | "transport" | "claim" | "composer";
  message: string;
};

/**
 * Picks the one alert shown above the composer. Session expiry comes first.
 * The result of the person's own send (a refused or failed send) comes next,
 * so a connection or claim error cannot hide it. Other composer errors, such
 * as file limits, audio or rename failures, stay below connection and claim.
 */
export function hostedChatAlert(errors: {
  sessionError: string | null;
  transportError: string | null;
  claimError: string | null;
  composerError: HostedChatComposerError | null;
}): HostedChatAlert | null {
  const { sessionError, transportError, claimError, composerError } = errors;
  if (sessionError) return { kind: "session", message: sessionError };
  if (composerError?.fromSend) return { kind: "send", message: composerError.message };
  if (transportError) return { kind: "transport", message: transportError };
  if (claimError) return { kind: "claim", message: claimError };
  if (composerError) return { kind: "composer", message: composerError.message };
  return null;
}
