use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::io::{BufReader, AsyncBufReadExt};
use std::path::PathBuf;
use std::sync::Arc;

use super::types::{SearchOptions, SearchResult, SearchStatus};

pub struct SearchManager {
    workspace_path: PathBuf,
    current_search: tokio::sync::RwLock<Option<tokio::task::JoinHandle<()>>>,
    event_sender: broadcast::Sender<SearchResult>,
}

impl SearchManager {
    pub fn new(workspace_path: PathBuf) -> Self {
        let (event_sender, _) = broadcast::channel(100);
        
        Self {
            workspace_path,
            current_search: tokio::sync::RwLock::new(None),
            event_sender,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchResult> {
        self.event_sender.subscribe()
    }

    pub async fn start_search(
        &self,
        query: String,
        case_sensitive: bool,
    ) -> Result<()> {
        // Cancel any existing search
        self.cancel_search().await;

        let workspace_path = self.workspace_path.clone();
        let event_sender = self.event_sender.clone();

        // Spawn the search task
        let handle = tokio::spawn(async move {
            let mut cmd = Command::new("rg");
            
            cmd.arg("--fixed-strings")     
               .arg("--no-heading")        
               .arg("--with-filename")     
               .arg("--line-number")       
               .arg("--color").arg("never")
               .arg("--no-require-git")    
               .current_dir(&workspace_path);

            if !case_sensitive {
                cmd.arg("--ignore-case");
            }

            cmd.arg(&query);

            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            if let Ok(mut child) = cmd.spawn() {
                if let Some(stdout) = child.stdout.take() {
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some((file_path, remainder)) = line.split_once(':') {
                            if let Some((line_number_str, line_content)) = remainder.split_once(':') {
                                if let Ok(line_number) = line_number_str.parse::<u32>() {
                                    let result = SearchResult {
                                        file_path: PathBuf::from(file_path),
                                        line_number,
                                        line_content: line_content.to_string(),
                                    };
                                    
                                    // Use _ to ignore errors when there are no subscribers
                                    let _ = event_sender.send(result);
                                }
                            }
                        }
                    }
                }
            }
        });

        // Store the search handle
        *self.current_search.write().await = Some(handle);

        Ok(())
    }

    pub async fn cancel_search(&self) {
        if let Some(handle) = self.current_search.write().await.take() {
            handle.abort();
        }
    }
}
