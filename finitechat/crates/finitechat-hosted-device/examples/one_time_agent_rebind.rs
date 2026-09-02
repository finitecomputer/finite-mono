//! Unmerged local-only utility for replacing one exact hosted Agent binding.

use std::env;
use std::fs;
use std::path::PathBuf;

use finitechat_hosted_device::{
    HostedDeviceConfig, OneTimeHostedAgentRebindIntent, one_time_rebind_hosted_agent_on_scratch,
};

fn main() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(
            "usage: one_time_agent_rebind REBIND_INTENT.json HOSTED_DATA_ROOT LOOPBACK_SERVER_URL"
                .to_owned(),
        );
    }
    let intent_path = PathBuf::from(&args[0]);
    let data_root = PathBuf::from(&args[1])
        .canonicalize()
        .map_err(|error| format!("failed to resolve hosted data root: {error}"))?;
    let intent: OneTimeHostedAgentRebindIntent = serde_json::from_slice(
        &fs::read(&intent_path)
            .map_err(|error| format!("failed to read {}: {error}", intent_path.display()))?,
    )
    .map_err(|error| format!("failed to parse {}: {error}", intent_path.display()))?;
    let server_url = args[2].clone();
    let report = one_time_rebind_hosted_agent_on_scratch(
        HostedDeviceConfig {
            data_root,
            server_url: server_url.clone(),
            public_url: server_url,
            api_token: "local-unmerged-migration-only".to_owned(),
        },
        intent,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}
