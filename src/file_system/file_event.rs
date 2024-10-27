use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size: u64,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub created_at: Option<u64>,
    pub modified_at: Option<u64>,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModificationType {
    Content,    // File content was modified
    Metadata,   // File metadata changed (permissions, timestamps)
    Name,       // File was renamed
    Other,      // Other modifications
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileEvent {
    Created {
        path: PathBuf,
        timestamp_ms: u128,
        metadata: FileMetadata,
    },
    Modified {
        path: PathBuf,
        timestamp_ms: u128,
        modification_type: ModificationType,
        new_metadata: FileMetadata,
    },
    Deleted {
        path: PathBuf,
        timestamp_ms: u128,
    },
}

impl FileEvent {
    pub async fn from_notify_event(event: notify::Event) -> Option<Self> {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        println!("Processing notify event: {:?}", event);

        async fn get_metadata(path: &PathBuf) -> Option<FileMetadata> {
            match tokio::fs::metadata(path).await {
                Ok(metadata) => {
                    println!("Got metadata for: {:?}", path);
                    Some(FileMetadata {
                        size: metadata.len(),
                        is_directory: metadata.is_dir(),
                        is_symlink: metadata.file_type().is_symlink(),
                        created_at: metadata.created().ok().and_then(|t| 
                            t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())),
                        modified_at: metadata.modified().ok().and_then(|t| 
                            t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())),
                        readonly: metadata.permissions().readonly(),
                    })
                },
                Err(e) => {
                    println!("Failed to get metadata for {:?}: {}", path, e);
                    None
                }
            }
        }

        let result = match event.kind {
            notify::EventKind::Create(_) => {
                let path = &event.paths[0];
                match get_metadata(path).await {
                    Some(metadata) => Some(FileEvent::Created {
                        path: path.clone(),
                        timestamp_ms,
                        metadata,
                    }),
                    None => None,
                }
            },
            
            notify::EventKind::Modify(modify_kind) => {
                let path = &event.paths[0];
                println!("Processing modify event for path: {:?}, kind: {:?}", path, modify_kind);
                
                // Special handling for Name modifications which might indicate deletion
                if matches!(modify_kind, notify::event::ModifyKind::Name(_)) {
                    match get_metadata(path).await {
                        Some(new_metadata) => Some(FileEvent::Modified {
                            path: path.clone(),
                            timestamp_ms,
                            modification_type: ModificationType::Name,
                            new_metadata,
                        }),
                        None => {
                            // If we can't get metadata, treat it as a deletion
                            println!("Name modification with no metadata - treating as deletion: {:?}", path);
                            Some(FileEvent::Deleted {
                                path: path.clone(),
                                timestamp_ms,
                            })
                        }
                    }
                } else {
                    // Handle other modifications normally
                    match get_metadata(path).await {
                        Some(new_metadata) => {
                            let modification_type = match modify_kind {
                                notify::event::ModifyKind::Data(_) => ModificationType::Content,
                                notify::event::ModifyKind::Metadata(_) => ModificationType::Metadata,
                                _ => ModificationType::Other,
                            };
                            
                            Some(FileEvent::Modified {
                                path: path.clone(),
                                timestamp_ms,
                                modification_type,
                                new_metadata,
                            })
                        },
                        None => None,
                    }
                }
            },
            
            notify::EventKind::Remove(_) => {
                println!("Processing removal event for: {:?}", &event.paths[0]);
                Some(FileEvent::Deleted {
                    path: event.paths[0].clone(),
                    timestamp_ms,
                })
            },
            
            _ => {
                println!("Unhandled event kind: {:?}", event.kind);
                None
            }
        };

        println!("Processed event result: {:?}", result);
        result
    }
}