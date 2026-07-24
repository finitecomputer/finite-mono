use std::str::FromStr;

use nostr::{Kind, Tag};

use crate::{APP_SPECIFIC_KIND, CliError};

pub(crate) fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|arg| arg != flag);
    before != args.len()
}

pub(crate) fn take_option_value(
    args: &mut Vec<String>,
    flag: &'static str,
) -> Result<Option<String>, CliError> {
    let mut found = None;
    let mut index = 0;
    let prefix = format!("{flag}=");
    while index < args.len() {
        if args[index] == flag {
            if index + 1 >= args.len() {
                return Err(CliError::MissingArgument(flag));
            }
            found = Some(args.remove(index + 1));
            args.remove(index);
            continue;
        }
        if let Some(value) = args[index].strip_prefix(&prefix) {
            found = Some(value.to_owned());
            args.remove(index);
            continue;
        }
        index += 1;
    }
    Ok(found)
}

pub(crate) fn option_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == flag).then(|| window[1].clone()))
}

pub(crate) fn unique_option_value(
    args: &[String],
    flag: &'static str,
) -> Result<Option<String>, CliError> {
    let prefix = format!("{flag}=");
    let mut values = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if arg == flag {
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or(CliError::MissingArgument(flag))?;
            values.push(value.clone());
        } else if let Some(value) = arg.strip_prefix(&prefix) {
            if value.is_empty() {
                return Err(CliError::MissingArgument(flag));
            }
            values.push(value.to_owned());
        }
    }
    if values.len() > 1 {
        return Err(CliError::InvalidInput(format!(
            "{flag} may only be supplied once"
        )));
    }
    Ok(values.into_iter().next())
}

pub(crate) fn required_option_or_positional(
    args: &[String],
    option: &str,
    positional_index: usize,
    name: &'static str,
) -> Result<String, CliError> {
    option_value(args, option)
        .or_else(|| positional_values(args).get(positional_index).cloned())
        .ok_or(CliError::MissingArgument(name))
}

pub(crate) fn normalize_brain_kind(kind: &str) -> Result<&'static str, CliError> {
    match kind {
        "personal" => Ok("personal"),
        "organization" | "org" => Ok("organization"),
        other => Err(CliError::InvalidInput(format!(
            "unknown brain kind {other}"
        ))),
    }
}

pub(crate) fn normalize_folder_role(role: &str) -> Result<&'static str, CliError> {
    match role {
        "personal_home" | "personal-home" => Ok("personal_home"),
        "brain_ops" | "brain-ops" => Ok("brain_ops"),
        "general" => Ok("general"),
        "folder" => Ok("folder"),
        other => Err(CliError::InvalidInput(format!(
            "unknown folder role {other}"
        ))),
    }
}

pub(crate) fn normalize_folder_access(access: &str) -> Result<&'static str, CliError> {
    match access {
        "owner" => Ok("owner"),
        "admin_only" | "admin-only" | "admin" => Ok("admin_only"),
        "all_members" | "all-members" | "members" => Ok("all_members"),
        "restricted" => Ok("restricted"),
        other => Err(CliError::InvalidInput(format!(
            "unknown folder access mode {other}"
        ))),
    }
}

pub(crate) fn parse_kind(value: &str) -> Result<Kind, CliError> {
    match value {
        "text" | "text-note" => Ok(Kind::TextNote),
        "http-auth" => Ok(Kind::HttpAuth),
        "app" | "application-specific" => Ok(Kind::Custom(APP_SPECIFIC_KIND)),
        other => u16::from_str(other)
            .map(Kind::from_u16)
            .map_err(|_| CliError::InvalidInput(format!("event kind {other} did not parse"))),
    }
}

pub(crate) fn parse_cli_tag(value: String) -> Result<Tag, CliError> {
    let parts = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(CliError::InvalidInput("tag cannot be empty".to_owned()));
    }
    Tag::parse(parts).map_err(|error| CliError::InvalidInput(error.to_string()))
}

pub(crate) fn tag_vec<const N: usize>(parts: [&str; N]) -> Result<Tag, CliError> {
    Tag::parse(parts.into_iter().map(ToOwned::to_owned).collect::<Vec<_>>())
        .map_err(|error| CliError::InvalidInput(error.to_string()))
}

pub(crate) fn option_values(args: &[String], flag: &str) -> Vec<String> {
    args.windows(2)
        .filter(|window| window[0] == flag)
        .map(|window| window[1].clone())
        .collect()
}

pub(crate) fn positional_values(args: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    let mut skip_next = false;
    for (index, arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg.starts_with("--") {
            if args
                .get(index + 1)
                .is_some_and(|next| !next.starts_with("--"))
            {
                skip_next = true;
            }
            continue;
        }
        values.push(arg.clone());
    }
    values
}
