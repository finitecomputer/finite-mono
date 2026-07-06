use std::path::{Component, Path, PathBuf};

pub(crate) fn resolved_archive_link_path(
    entry_path: &Path,
    link_name: &Path,
) -> Result<PathBuf, &'static str> {
    if link_name.as_os_str().is_empty() {
        return Err("link target is empty");
    }

    let mut components = Vec::new();
    if let Some(parent) = entry_path.parent() {
        push_archive_components(&mut components, parent)?;
    }

    for component in link_name.components() {
        match component {
            Component::Normal(part) => components.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err("link target escapes bundle root");
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("link target must be relative");
            }
        }
    }

    let mut resolved = PathBuf::new();
    for component in components {
        resolved.push(component);
    }
    Ok(resolved)
}

pub(crate) fn validate_archive_link_target(
    entry_path: &Path,
    link_name: &Path,
) -> Result<(), &'static str> {
    resolved_archive_link_path(entry_path, link_name).map(|_| ())
}

fn push_archive_components(
    components: &mut Vec<std::ffi::OsString>,
    path: &Path,
) -> Result<(), &'static str> {
    for component in path.components() {
        match component {
            Component::Normal(part) => components.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("archive entry path is unsafe");
            }
        }
    }
    Ok(())
}
