use std::path::PathBuf;
use anyhow::Result;
use crate::file_system::VersionedDocument; 

pub fn get_full_path(workspace_root: &PathBuf, relative_path: &str) -> Result<PathBuf> {
    // If empty path, return workspace root
    if relative_path.is_empty() {
        return Ok(workspace_root.clone());
    }

    // Handle absolute vs relative paths
    let path = PathBuf::from(relative_path);
    let path = if path.is_absolute() {
        path
    } else {
        workspace_root.join(relative_path)
    };
    
    // Canonicalize and validate
    let canonical = path.canonicalize()?;
    validate_workspace_path(workspace_root, &canonical)?;
    
    Ok(canonical)
}

pub fn canonicalize_document_path(workspace_root: &PathBuf, doc: &VersionedDocument) -> Result<PathBuf> {
    // Handle absolute paths
    if doc.uri.is_absolute() {
        let canonical = doc.uri.canonicalize()?;
        if canonical.starts_with(workspace_root) {
            return Ok(canonical);
        }
    }

    // Handle relative or empty paths
    let path = if doc.uri.to_string_lossy().is_empty() {
        workspace_root.clone()
    } else {
        workspace_root.join(&doc.uri)
    };
    
    let canonical = path.canonicalize()?;
    validate_workspace_path(workspace_root, &canonical)?;
    
    Ok(canonical)
}

fn validate_workspace_path(workspace_root: &PathBuf, path: &PathBuf) -> Result<()> {
    if !path.starts_with(workspace_root) {
        anyhow::bail!("Path is outside of workspace: {:?}", path);
    }
    Ok(())
}

pub fn to_relative_path(workspace_root: &PathBuf, path: &PathBuf) -> Option<PathBuf> {
    path.strip_prefix(workspace_root).ok().map(|p| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_workspace() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("test.txt"), "test").unwrap();
        dir
    }

    #[test]
    fn test_path_utils() -> Result<()> {
        let workspace = setup_test_workspace();
        let workspace_root = workspace.path().to_path_buf();

        // Test empty path
        assert_eq!(
            get_full_path(&workspace_root, "")?,
            workspace_root.canonicalize()?
        );

        // Test relative path
        assert_eq!(
            get_full_path(&workspace_root, "test.txt")?,
            workspace_root.join("test.txt").canonicalize()?
        );

        // Test path outside workspace
        assert!(get_full_path(&workspace_root, "/tmp").is_err());

        Ok(())
    }
}