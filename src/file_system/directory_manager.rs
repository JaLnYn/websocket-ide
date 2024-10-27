// src/file_system/directory_manager.rs

use std::path::PathBuf;
use std::collections::HashMap;
use tokio::sync::RwLock;
use anyhow::Result;
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_directory: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
    pub is_loaded: bool,
}

#[derive(Debug)]
pub struct DirectoryManager {
    workspace_path: PathBuf,
    cache: RwLock<HashMap<PathBuf, Vec<FileNode>>>,
    root: RwLock<Option<FileNode>>,
}

impl DirectoryManager {
    pub fn new(workspace_path: PathBuf) -> Result<Self> {
        let workspace_path = workspace_path.canonicalize()?;
        println!("Initialized directory manager at: {:?}", workspace_path);

        Ok(Self {
            workspace_path,
            cache: RwLock::new(HashMap::new()),
            root: RwLock::new(None),
        })
    }

    pub fn get_workspace_path(&self) -> &PathBuf {
        &self.workspace_path
    }

    // pub fn get_full_path(&self, relative_path: &str) -> Result<PathBuf> {
    //     let path = if relative_path.is_empty() {
    //         self.workspace_path.clone()
    //     } else {
    //         self.workspace_path.join(relative_path)
    //     };
    //     
    //     let canonical = path.canonicalize()?;
    //     if !canonical.starts_with(&self.workspace_path) {
    //         anyhow::bail!("Path is outside of workspace: {:?}", canonical);
    //     }
    //     
    //     Ok(canonical)
    // }

    async fn read_directory(&self, path: &PathBuf) -> Result<Vec<FileNode>> {
        println!("Reading directory contents: {:?}", path);
        
        let mut entries = tokio::fs::read_dir(path).await?;
        let mut nodes = Vec::new();
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;
            
            nodes.push(FileNode {
                name: path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: path.canonicalize()?,
                is_directory: metadata.is_dir(),
                size: metadata.len(),
                children: None,
                is_loaded: false,
            });
        }
        
        Ok(nodes)
    }

    pub async fn load_directory(&self, path: &PathBuf) -> Result<Vec<FileNode>> {
        if let Some(cached) = self.cache.read().await.get(path) {
            return Ok(cached.clone());
        }

        let nodes = self.read_directory(path).await?;
        self.cache.write().await.insert(path.clone(), nodes.clone());
        
        Ok(nodes)
    }

    pub async fn init(&self) -> Result<()> {
        let root_contents = self.load_directory(&self.workspace_path).await?;
        *self.root.write().await = Some(FileNode {
            name: self.workspace_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: self.workspace_path.clone(),
            is_directory: true,
            size: 0,
            children: Some(root_contents),
            is_loaded: true,
        });
        Ok(())
    }

    pub async fn refresh_directory(&self, path: &PathBuf) -> Result<Vec<FileNode>> {
        let nodes = self.read_directory(path).await?;
        self.cache.write().await.insert(path.clone(), nodes.clone());
        Ok(nodes)
    }

    pub async fn invalidate_cache(&self, path: &PathBuf) {
        self.cache.write().await.remove(path);
    }
}