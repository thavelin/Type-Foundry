//! Refuse to replace an existing file unless the caller passes force, and keep a copy when it does.
//!
//! A `.ufo` is a folder. The copy is a sibling folder named `Name.ufo.bak`, so the next save can
//! still rebuild the UFO from what was loaded without being the only copy of `features.fea`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::FoundryError;

/// When `path` already exists, copy it aside if `force` is set. Otherwise refuse.
pub(crate) fn prepare_write(path: &Path, force: bool) -> Result<(), FoundryError> {
    if !path.exists() {
        return Ok(());
    }
    if !force {
        return Err(FoundryError::Exists(path.display().to_string()));
    }
    copy_path(path, &backup_path(path))
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "font".to_string());
    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let mut n = 0u32;
    loop {
        let file_name = if n == 0 {
            format!("{name}.bak")
        } else {
            format!("{name}.bak{n}")
        };
        let candidate = match parent {
            Some(parent) => parent.join(&file_name),
            None => PathBuf::from(&file_name),
        };
        if !candidate.exists() || n > 1000 {
            return candidate;
        }
        n += 1;
    }
}

fn copy_path(from: &Path, to: &Path) -> Result<(), FoundryError> {
    if from.is_dir() {
        return copy_dir(from, to);
    }
    if let Some(parent) = to.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| FoundryError::Io(err.to_string()))?;
    }
    fs::copy(from, to).map_err(|err| FoundryError::Io(err.to_string()))?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), FoundryError> {
    fs::create_dir_all(to).map_err(|err| FoundryError::Io(err.to_string()))?;
    for entry in fs::read_dir(from).map_err(|err| FoundryError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| FoundryError::Io(err.to_string()))?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).map_err(|err| FoundryError::Io(err.to_string()))?;
        }
    }
    Ok(())
}
