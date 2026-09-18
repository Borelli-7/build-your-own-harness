//! Filesystem tools sandboxed to a working-directory root.

use std::path::{Component, Path, PathBuf};

use anyhow::bail;

/// Joins `path` onto `root`, rejecting absolute paths and `..` components to prevent escaping the sandbox.
fn resolve(root: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let rel = Path::new(path);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("path '{path}' is not allowed (absolute paths and '..' are rejected)");
    }
    Ok(root.join(rel))
}

pub async fn read_file(root: &Path, path: &str) -> anyhow::Result<String> {
    let resolved = resolve(root, path)?;
    Ok(tokio::fs::read_to_string(resolved).await?)
}

pub async fn write_file(root: &Path, path: &str, contents: &str) -> anyhow::Result<()> {
    let resolved = resolve(root, path)?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(resolved, contents).await?;
    Ok(())
}

pub async fn list_dir(root: &Path, path: &str) -> anyhow::Result<Vec<String>> {
    let resolved = resolve(root, path)?;
    let mut entries = tokio::fs::read_dir(resolved).await?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "notes/todo.txt", "buy milk").await.unwrap();
        let content = read_file(dir.path(), "notes/todo.txt").await.unwrap();
        assert_eq!(content, "buy milk");
    }

    #[tokio::test]
    async fn list_dir_returns_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "a.txt", "").await.unwrap();
        write_file(dir.path(), "b.txt", "").await.unwrap();
        let mut entries = list_dir(dir.path(), ".").await.unwrap();
        entries.sort();
        assert_eq!(entries, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[tokio::test]
    async fn parent_dir_traversal_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_file(dir.path(), "../escape.txt").await.unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn absolute_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_file(dir.path(), "/etc/passwd").await.unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }
}

