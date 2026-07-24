use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::{ActivityEntry, CliError, write_private_file_atomic};

pub(crate) fn write_json_file<T>(path: &Path, value: &T) -> Result<(), CliError>
where
    T: Serialize,
{
    write_private_file_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub(crate) fn write_json<W, T>(output: &mut W, value: &T) -> Result<(), CliError>
where
    W: Write,
    T: Serialize,
{
    serde_json::to_writer_pretty(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}

pub(crate) fn write_activity_rows<W: Write>(
    output: &mut W,
    rows: &[ActivityEntry],
) -> Result<(), CliError> {
    if rows.is_empty() {
        writeln!(output, "no activity")?;
        return Ok(());
    }
    for row in rows {
        writeln!(output, "{} {} {}", row.at, row.kind, row.message)?;
    }
    Ok(())
}

pub(crate) fn write_command_response<W: Write>(
    output: &mut W,
    json: bool,
    value: &serde_json::Value,
) -> Result<(), CliError> {
    if json {
        write_json(output, value)
    } else if let Some(id) = value
        .get("id")
        .or_else(|| value.get("brainId"))
        .or_else(|| value.get("folderId"))
        .and_then(serde_json::Value::as_str)
    {
        writeln!(output, "ok {id}")?;
        Ok(())
    } else {
        writeln!(output, "ok")?;
        Ok(())
    }
}
