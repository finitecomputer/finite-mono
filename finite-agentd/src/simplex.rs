use crate::AgentdError;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(crate) fn script_path() -> PathBuf {
    std::env::var_os("FINITE_AGENTD_SIMPLEX_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/simplex_runtime.py"))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SimplexStatus {
    pub enabled: bool,
    pub ready: bool,
    pub address: Option<String>,
    pub qr: Vec<String>,
    pub approved: Vec<crate::connections::PairedContact>,
}

pub(crate) fn control(
    agent_home: &std::path::Path,
    hermes_home: &std::path::Path,
    operation: &str,
    input: Option<&str>,
) -> Result<Value, AgentdError> {
    let mut child = Command::new("python3")
        .arg(script_path())
        .arg(operation)
        .env("HERMES_HOME", hermes_home)
        .env("FINITECHAT_HOME", agent_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| AgentdError::Config("SimpleX control input is unavailable".to_owned()))?
            .write_all(input.as_bytes())?;
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let message = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "SimpleX is unavailable; try again after it starts".to_owned());
        return Err(AgentdError::Config(message));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub(crate) fn qr_rows(address: &str) -> Result<Vec<String>, AgentdError> {
    let code = qrcode::QrCode::new(address.as_bytes())
        .map_err(|_| AgentdError::Config("SimpleX address is too large to display".to_owned()))?;
    Ok((0..code.width())
        .map(|y| {
            (0..code.width())
                .map(|x| {
                    if code[(x, y)] == qrcode::Color::Dark {
                        '1'
                    } else {
                        '0'
                    }
                })
                .collect()
        })
        .collect())
}
