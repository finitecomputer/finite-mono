# Mail delivery is an adapter

Finite Identity owns the Email Challenge flow, including token creation, hashing, expiry, redemption, and rate limits. Actual delivery is handled by a Mailer Adapter selected by deployment configuration, so local development can use an outbox and production can use providers such as Resend or Postmark without changing identity semantics.
