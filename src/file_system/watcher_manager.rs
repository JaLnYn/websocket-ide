use tokio::sync::{broadcast, mpsc, RwLock};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use notify::{Watcher, RecursiveMode, Event};

use crate::file_system::event_batcher::EventBatcher;
use crate::file_system::file_event::FileEvent;
use super::directory_manager::DirectoryManager;
use super::event_batcher::spawn_timeout_checker;

pub struct WatcherManager {
    event_sender: broadcast::Sender<FileEvent>,
    event_batcher: Arc<RwLock<EventBatcher>>,
    directory_manager: Arc<DirectoryManager>,
}

impl WatcherManager {
    pub fn new(
        directory_manager: Arc<DirectoryManager>,
        batch_size: usize,
        batch_timeout: Duration,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let (batch_tx, mut batch_rx) = mpsc::channel(32);

        // Spawn event processing task
        let event_sender = event_tx.clone();
        tokio::spawn(async move {
            while let Some(batch) = batch_rx.recv().await {
                for event in batch {
                    let _ = event_sender.send(event);
                }
            }
        });

        let event_batcher = Arc::new(RwLock::new(EventBatcher::new(
            batch_size,
            batch_timeout,
            batch_tx,
        )));

        // Spawn the timeout checker
        spawn_timeout_checker(Arc::clone(&event_batcher));

        Self {
            event_sender: event_tx,
            event_batcher,
            directory_manager,
        }
    }

    pub async fn start_watching(&self) -> Result<()> {
        let workspace_path = self.directory_manager.get_workspace_path().clone();
        let (tx, mut rx) = mpsc::channel(100);
        
        // Clone what we need from self
        let directory_manager = Arc::clone(&self.directory_manager);
        let event_batcher = Arc::clone(&self.event_batcher);
        
        std::thread::spawn(move || {
            let tx = tx.clone();
            let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    println!("Watcher sending event to channel: {:?}", event);
                    let _ = tx.blocking_send(event);
                }
            }).unwrap();

            watcher.watch(&workspace_path, RecursiveMode::Recursive).unwrap();
            std::thread::park();
        });
        
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                println!("Received event in processor: {:?}", event);
                if let Some(file_event) = FileEvent::from_notify_event(event).await {
                    // Get the parent directory path for cache invalidation
                    let parent = match &file_event {
                        FileEvent::Created { path, .. } |
                        FileEvent::Modified { path, .. } |
                        FileEvent::Deleted { path, .. } => {
                            path.parent().map(|p| p.to_path_buf())
                        }
                    };

                    if let Some(parent) = parent {
                        println!("Invalidating cache for parent: {:?}", parent);
                        directory_manager.invalidate_cache(&parent).await;
                    }
                    
                    println!("Sending event to batcher: {:?}", file_event);
                    event_batcher.write().await.add_event(file_event).await;
                }
            }
        });

        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FileEvent> {
        self.event_sender.subscribe()
    }
}