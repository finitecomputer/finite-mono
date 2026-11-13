//! `fbrain brain personal-agent-consent`: the Agent's half of Personal Brain
//! setup. The Agent signs a short-lived consent naming one owner; the owner
//! then signs the creation request that carries it.

use std::io::Write;

use finite_brain_core::{
    PERSONAL_AGENT_CONSENT_VERSION, PersonalAgentConsentPayload,
    personal_agent_consent_event_template, personal_agent_consent_nonce,
    personal_brain_id_for_owner, sign_brain_event_template,
};
use finite_nostr::NostrPublicKey;

use crate::{
    CliEnvironment, CliError, load_existing_signer, option_value, signed_brain_origin,
    unix_timestamp, write_json,
};

/// How long the owner has to submit the consent; the server allows at most 15 minutes.
const CONSENT_LIFETIME_SECONDS: u64 = 10 * 60;

pub(crate) fn personal_agent_consent<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
) -> Result<(), CliError> {
    let owner = option_value(args, "--owner").ok_or(CliError::MissingArgument("--owner"))?;
    let owner_npub = NostrPublicKey::parse(owner.trim())
        .and_then(|public_key| public_key.to_npub())
        .map_err(|error| {
            CliError::InvalidInput(format!(
                "--owner must be an npub or 64-character hex public key: {error}"
            ))
        })?;
    // Consent comes only from this Agent's existing key; never mint one.
    let signer = load_existing_signer(env)?;
    let invalid =
        |error: finite_brain_core::CryptoRecordError| CliError::InvalidInput(error.to_string());
    let created_at = unix_timestamp();
    let payload = PersonalAgentConsentPayload {
        version: PERSONAL_AGENT_CONSENT_VERSION.to_owned(),
        agent_npub: signer.npub.clone(),
        brain_id: personal_brain_id_for_owner(&owner_npub)
            .map_err(invalid)?
            .as_str()
            .to_owned(),
        owner_npub,
        brain_server: signed_brain_origin(env, args)?,
        nonce: personal_agent_consent_nonce(),
        expires_at: created_at + CONSENT_LIFETIME_SECONDS,
    };
    let template = personal_agent_consent_event_template(&payload, created_at).map_err(invalid)?;
    let event = sign_brain_event_template(&signer.keys, &template).map_err(invalid)?;
    let consent: serde_json::Value = serde_json::from_str(&event.as_json())?;
    if json {
        return write_json(
            output,
            &serde_json::json!({
                "version": PERSONAL_AGENT_CONSENT_VERSION,
                "agentNpub": payload.agent_npub,
                "ownerNpub": payload.owner_npub,
                "brainId": payload.brain_id,
                "brainServer": payload.brain_server,
                "expiresAt": payload.expires_at,
                "consent": consent,
            }),
        );
    }
    writeln!(
        output,
        "Personal Agent consent for {} to set up {} on {}; expires at {}",
        payload.owner_npub, payload.brain_id, payload.brain_server, payload.expires_at
    )?;
    writeln!(output, "{}", event.as_json())?;
    Ok(())
}
