mod directory_manager;
mod document_manager;
mod event_batcher;
mod file_event;
mod watcher_manager;

use anyhow::Result;
use document_manager::DocumentState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub use directory_manager::{DirectoryManager, FileNode};
pub use document_manager::{DiffChange, DocumentManager, DocumentMetadata, VersionedDocument};
pub use file_event::FileEvent;
use watcher_manager::WatcherManager;

pub struct FileSystem {
    directory_manager: Arc<DirectoryManager>,
    watcher_manager: WatcherManager,
    document_manager: Arc<DocumentManager>,
}

impl FileSystem {
    pub fn new(workspace_path: PathBuf) -> Result<Self> {
        let directory_manager = Arc::new(DirectoryManager::new(workspace_path.clone())?);
        let document_manager = Arc::new(DocumentManager::new(workspace_path.clone())?);

        let watcher_manager = WatcherManager::new(
            Arc::clone(&directory_manager),
            100,                        // batch size
            Duration::from_millis(100), // batch timeout
        );

        Ok(Self {
            directory_manager,
            watcher_manager,
            document_manager,
        })
    }

    pub async fn init(&self) -> Result<()> {
        self.directory_manager.init().await
    }

    pub async fn start_watching(&self) -> Result<()> {
        self.watcher_manager.start_watching().await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FileEvent> {
        self.watcher_manager.subscribe()
    }

    pub fn get_workspace_path(&self) -> &PathBuf {
        self.directory_manager.get_workspace_path()
    }

    pub async fn load_directory(&self, path: &PathBuf) -> Result<Vec<FileNode>> {
        self.directory_manager.load_directory(path).await
    }

    pub async fn refresh_directory(&self, path: &PathBuf) -> Result<Vec<FileNode>> {
        self.directory_manager.refresh_directory(path).await
    }

    pub async fn open_file(&self, path: &PathBuf) -> Result<(String, DocumentMetadata, i32)> {
        Ok(self.document_manager.open_file(path).await?)
    }

    pub async fn close_file(&self, path: &PathBuf) -> Result<()> {
        self.document_manager.close_file(path).await;
        Ok(())
    }

    pub async fn change_document(
        &self,
        document: VersionedDocument,
        changes: Vec<DiffChange>,
    ) -> Result<VersionedDocument> {
        Ok(self
            .document_manager
            .change_document(&document, changes)
            .await?)
    }

    pub async fn save_document(&self, document: VersionedDocument) -> Result<VersionedDocument> {
        Ok(self.document_manager.save_document(&document).await?)
    }

    pub async fn get_document_content(&self, path: &PathBuf) -> Result<String> {
        Ok(self.document_manager.get_document_content(path).await?)
    }

    pub async fn get_document_state(&self, path: &PathBuf) -> Result<DocumentState> {
        self.document_manager.get_document_state(path).await
    }

    pub async fn invalidate_document_cache(&self, path: &PathBuf) -> Result<()> {
        self.document_manager.invalidate_cache_for_file(path).await;
        Ok(())
    }

    pub async fn create_file(&self, path: &PathBuf, is_directory: bool) -> Result<()> {
        println!("Creating file {:?}", path);
        self.document_manager.create_file(path, is_directory).await
    }

    pub async fn delete_file(&self, path: &PathBuf) -> Result<()> {
        println!("Deleting file: {:?}", path);
        self.document_manager.delete_file(path).await
    }

    pub async fn rename_file(&self, old_path: &PathBuf, new_path: &PathBuf) -> Result<()> {
        println!("Renaming file: {:?} -> {:?}", old_path, new_path);
        self.document_manager.rename_file(old_path, new_path).await
    }
}
