/** A product-facing chat failure: its status and copy reach the browser unchanged. */
export class HostedWebChatError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: "binding_authorization_required"
  ) {
    super(message);
  }
}
