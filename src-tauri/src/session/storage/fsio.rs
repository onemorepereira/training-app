//! Durable file primitives for ride data.
//!
//! `write_durable` = write `<path>.tmp` → fsync file → rename → fsync parent dir,
//! so after it returns the new content survives a power loss and a crash mid-way
//! never leaves a truncated file under the final name.

use std::io;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::error::AppError;

pub(crate) fn io_error(context: &str, path: &Path, e: io::Error) -> AppError {
    AppError::Session(format!("{} {}: {}", context, path.display(), e))
}

pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// fsync a directory so a rename/unlink inside it is durable. No-op off unix
/// (Windows cannot open directories as files; NTFS journals metadata).
pub(crate) async fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::File::open(dir).await?.sync_all().await?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

pub(crate) async fn write_durable(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Session(format!("No parent dir for {}", path.display())))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| io_error("Failed to create", parent, e))?;
    let tmp = tmp_path(path);
    {
        let mut f = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| io_error("Failed to create", &tmp, e))?;
        f.write_all(bytes)
            .await
            .map_err(|e| io_error("Failed to write", &tmp, e))?;
        f.sync_all()
            .await
            .map_err(|e| io_error("Failed to fsync", &tmp, e))?;
    }
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| io_error("Failed to rename into", path, e))?;
    sync_dir(parent)
        .await
        .map_err(|e| io_error("Failed to fsync dir", parent, e))
}

/// Remove a file durably. Returns Ok(false) if it did not exist.
pub(crate) async fn remove_durable(path: &Path) -> Result<bool, AppError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_dir(parent)
                    .await
                    .map_err(|e| io_error("Failed to fsync dir", parent, e))?;
            }
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error("Failed to remove", path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_durable_replaces_content_and_leaves_no_tmp() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("sessions").join("a.bin");
        write_durable(&p, b"first").await.unwrap();
        write_durable(&p, b"second").await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"second");
        assert!(!tmp_path(&p).exists());
    }

    #[tokio::test]
    async fn remove_durable_reports_missing_file_as_false() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("x.bin");
        assert!(!remove_durable(&p).await.unwrap());
        std::fs::write(&p, b"x").unwrap();
        assert!(remove_durable(&p).await.unwrap());
        assert!(!p.exists());
    }

    #[test]
    fn tmp_path_appends_suffix_to_full_name() {
        assert_eq!(
            tmp_path(Path::new("/d/.autosave_x.bin")),
            PathBuf::from("/d/.autosave_x.bin.tmp")
        );
    }
}
