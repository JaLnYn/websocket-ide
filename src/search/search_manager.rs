// src/search/search_manager.rs
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};
use nucleo::{pattern::{CaseMatching, Normalization}, Config, Nucleo};
use anyhow::{Result, anyhow};
use tokio::fs;
use walkdir::WalkDir;
use serde::{Serialize, Deserialize};

use crate::search::types::{SearchItem, SearchResultItem};

use super::SearchMessage;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
const BATCH_SIZE: u32 = 100;

struct SearchState {
    nucleo: Arc<RwLock<Nucleo<SearchItem>>>,
    last_query: String,
}

// Internal message for tick handling
enum InternalMessage {
    Tick { search_id: String },
}


pub struct SearchManager {
    searches: Arc<RwLock<HashMap<String, SearchState>>>,
    event_sender: broadcast::Sender<SearchMessage>,
    internal_sender: mpsc::UnboundedSender<InternalMessage>,
    workspace_path: PathBuf,
}

impl SearchManager {
    pub fn new(workspace_path: PathBuf) -> Self {
        let (event_sender, _) = broadcast::channel(100);
        let (internal_sender, internal_receiver) = mpsc::unbounded_channel();
        
        let manager = Self {
            searches: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            internal_sender,
            workspace_path,
        };
        
        // Spawn internal message handler
        let event_sender = manager.event_sender.clone();
        let searches = manager.searches.clone();
        tokio::spawn(async move {
            Self::handle_internal_messages(searches, internal_receiver, event_sender).await;
        });
        
        manager
    }

    async fn handle_internal_messages(
        searches: Arc<RwLock<HashMap<String, SearchState>>>,
        mut receiver: mpsc::UnboundedReceiver<InternalMessage>,
        event_sender: broadcast::Sender<SearchMessage>,
    ) {
        while let Some(message) = receiver.recv().await {
            match message {
                InternalMessage::Tick { search_id } => {
                    if let Err(e) = Self::process_search_results(&searches, &event_sender, &search_id).await {
                        eprintln!("Error processing search results: {}", e);
                    }
                }
            }
        }
    }

    async fn process_search_results(
        searches: &RwLock<HashMap<String, SearchState>>,
        event_sender: &broadcast::Sender<SearchMessage>,
        search_id: &str,
    ) -> Result<()> {
        let searches = searches.read().await;
        if let Some(state) = searches.get(search_id) {
            let nucleo = state.nucleo.read().await;
            let snapshot = nucleo.snapshot();
            
            let total_matches = snapshot.matched_item_count();
            let mut current_batch = Vec::with_capacity(BATCH_SIZE as usize);

            for (i, item) in snapshot.matched_items(0..total_matches).enumerate() {
                current_batch.push(SearchResultItem {
                    path: item.data.path.clone(),
                    line_number: item.data.line_number,
                    content: item.data.content.clone(),
                });

                if current_batch.len() >= BATCH_SIZE as usize || i == (total_matches - 1) as usize {
                    let is_complete = i == (total_matches - 1) as usize;
                    let _ = event_sender.send(SearchMessage::Results {
                        search_id: search_id.to_string(),
                        items: std::mem::take(&mut current_batch),
                        is_complete,
                    });
                }
            }
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchMessage> {
        self.event_sender.subscribe()
    }

    pub async fn create_search(
        &self, 
        query: &str, 
        id: Option<String>,
        search_filename_only: bool
    ) -> Result<String> {
        if let Some(id) = id {
            let searches = self.searches.read().await;
            if let Some(state) = searches.get(&id) {
                let is_append = query.starts_with(&state.last_query);
                let mut nucleo_instance = state.nucleo.write().await;
                nucleo_instance.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, is_append);

                let mut searches = self.searches.write().await;
                if let Some(state) = searches.get_mut(&id) {
                    state.last_query = query.to_string();
                }
                return Ok(id);
            } else {
                return Err(anyhow!("Search not found: {}", id));
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let internal_sender = self.internal_sender.clone();
        let search_id = id.clone();

        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = internal_sender.send(InternalMessage::Tick {
                search_id: search_id.clone()
            });
        });

        let config = Config::DEFAULT;
        let mut nucleo_instance = Nucleo::new(config, notify, None, 1);
        nucleo_instance.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, false);

        self.inject_items(&mut nucleo_instance, search_filename_only).await?;
        
        let state = SearchState {
            nucleo: Arc::new(RwLock::new(nucleo_instance)),
            last_query: query.to_string(),
        };
        
        self.searches.write().await.insert(id.clone(), state);
        Ok(id)
    }

    async fn inject_items(
        &self,
        nucleo: &mut Nucleo<SearchItem>,
        search_filename_only: bool,
    ) -> Result<()> {
        let walker = WalkDir::new(&self.workspace_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok());

        let injector = nucleo.injector();

        if search_filename_only {
            for entry in walker {
                let path = entry.path().to_string_lossy().into_owned();
                injector.push(
                    SearchItem {
                        path: path.clone(),
                        line_number: 0,
                        content: String::new(),
                    },
                    |item, columns| {
                        columns[0] = item.path.clone().into();
                    }
                );
            }
        } else {
            for entry in walker.filter(|e| e.file_type().is_file()) {
                let metadata = match entry.metadata() {
                    Ok(m) if m.len() <= MAX_FILE_SIZE => m,
                    _ => continue,
                };

                if let Ok(content) = fs::read_to_string(entry.path()).await {
                    let path = entry.path().to_string_lossy().into_owned();
                    for (idx, line) in content.lines().enumerate() {
                        injector.push(
                            SearchItem {
                                path: path.clone(),
                                line_number: (idx + 1) as u32,
                                content: line.to_string(),
                            },
                            |item, columns| {
                                columns[0] = item.content.clone().into();
                            }
                        );
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn close_search(&self, id: String) -> Result<()> {
        if self.searches.write().await.remove(&id).is_some() {
            Ok(())
        } else {
            Err(anyhow!("Search not found: {}", id))
        }
    }
}