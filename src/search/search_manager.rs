// src/search/search_manager.rs
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tokio::time::interval;
use nucleo::{Config, Nucleo, Utf32String};
use nucleo::pattern::{CaseMatching, Normalization};
use anyhow::Result;

use crate::search::{SearchMessage, SearchResultItem};

const BATCH_SIZE: usize = 50;
const TICK_TIMEOUT_MS: u64 = 10;
const POLL_INTERVAL_MS: u64 = 100;
const SEARCH_TIMEOUT_SECS: u64 = 10;

pub struct SearchManager {
    workspace_path: PathBuf,
    searcher: Arc<RwLock<Nucleo<PathBuf>>>,
    event_sender: broadcast::Sender<SearchMessage>,
    last_query: Arc<RwLock<Option<String>>>,
    is_searching: Arc<RwLock<bool>>,
}

impl SearchManager {
    pub fn new(workspace_path: PathBuf) -> Arc<Self> {
        let (event_sender, _ ) = broadcast::channel(100);

        // Notify is now only used for file system changes
        let notify = Arc::new(|| {
            //println!("Nucleo notifying of file system changes"); // why not printing
        });

        let searcher = Nucleo::new(
            Config::DEFAULT.match_paths(),
            notify,
            None,
            1
        );

        let manager = Arc::new(Self {
            workspace_path,
            searcher: Arc::new(RwLock::new(searcher)),
            event_sender,
            last_query: Arc::new(RwLock::new(None)),
            is_searching: Arc::new(RwLock::new(false)),
        });

        // Create polling task for search results
        let manager_clone = Arc::clone(&manager);
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
            let mut search_start: Option<std::time::Instant> = None;
            
            loop {
                interval.tick().await;
                let is_searching = *manager_clone.is_searching.read().await;
                
                if is_searching {
                    // Initialize search start time
                    if search_start.is_none() {
                        search_start = Some(std::time::Instant::now());
                    }

                    // Check for timeout
                    if let Some(start) = search_start {
                        if start.elapsed() > Duration::from_secs(SEARCH_TIMEOUT_SECS) {
                            println!("Search timed out after {} seconds", SEARCH_TIMEOUT_SECS);
                            *manager_clone.is_searching.write().await = false;
                            continue;
                        }
                    }

                    if let Err(e) = manager_clone.process_results().await {
                        eprintln!("Error processing results: {}", e);
                    }
                } else {
                    search_start = None;
                }
            }
        });

        manager
    }


    async fn initialize_files(&self) -> Result<()> {
        let searcher = self.searcher.read().await;
        let injector = searcher.injector();
        let mut count = 0;
        
        // Walk the workspace and inject files
        for entry in walkdir::WalkDir::new(&self.workspace_path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !Self::is_ignored(e.path())) 
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path().to_path_buf();
            
            // Debug: Print the path being injected
            // println!("Injecting file: {:?}", path);
            
            // Inject the path, converting it to the format nucleo expects
            injector.push(path, |path, columns| {
                let path_str = path.to_string_lossy().to_string();
                // println!("Converting path to matcher column: {}", path_str);
                columns[0] = path_str.into();
            });
            count += 1;
        }

        println!("Injected {} files", count);
        
        // Debug: Print injector stats
        println!("Total items in injector: {}", injector.injected_items());
        
        Ok(())
    }


    fn is_ignored(path: &Path) -> bool {
        path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s == ".git" || s == "node_modules" || s == "target"
        })
    }

    pub async fn create_search(
        self: Arc<Self>,
        query: String,
        search_filename_only: bool,
    ) -> Result<()> {
        // Start async initialization if needed
        let mut last_query = self.last_query.write().await;

        let mut initialization_needed = !*self.is_searching.read().await;

        let should_reparse = if let Some(last) = last_query.as_ref() {
            let begins_with = query.starts_with(last);
            initialization_needed = !begins_with;
            begins_with
        } else {
            false
        };

        if initialization_needed {
            println!("Starting new search");
            self.searcher.write().await.restart(true);
            let manager_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = manager_clone.initialize_files().await { 
                    // this is some pretty horrid code. Will fix in the future. I just want to initialize the files here
                    eprintln!("Failed to initialize files: {}", e);
                    *manager_clone.is_searching.write().await = false;
                    return;
                }
                
                // Set up search pattern after initialization
            });
            let mut searcher = self.searcher.write().await;
            searcher.pattern.reparse(0, &query, CaseMatching::Smart, Normalization::Smart, false);

            *last_query = Some(query.to_string());
            *self.is_searching.write().await = true;
        } else {
            // If already initialized, just update the pattern

            println!("Continuing search");
            let mut searcher = self.searcher.write().await;


            searcher.pattern.reparse(0, &query, CaseMatching::Smart, Normalization::Smart, should_reparse);
            
            *last_query = Some(query.to_string());
            *self.is_searching.write().await = true;
        }
        
        Ok(())
    }


    pub async fn close_search(&self) {
        *self.is_searching.write().await = false;
        let mut searcher = self.searcher.write().await;
        searcher.restart(true);
    }

    async fn process_results(&self) -> Result<()> {
        let mut searcher = self.searcher.write().await;
        
        let status = searcher.tick(TICK_TIMEOUT_MS);
        let snapshot = searcher.snapshot();
        let matched_count = snapshot.matched_item_count();
        let is_done = !status.running;
        println!("Found {} matches", matched_count);

        // Send any matches found
        if matched_count > 0 {
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);
            for item in snapshot.matched_items(0..matched_count) {
                let path = item.data.to_string_lossy().to_string();
                current_batch.push(SearchResultItem {
                    path,
                    line_number: 0,
                    content: String::new(),
                });

                if current_batch.len() >= BATCH_SIZE {
                    let message = SearchMessage::Results {
                        search_id: String::new(),
                        items: current_batch,
                        is_complete: false,
                    };
                    let _ = self.event_sender.send(message);
                    current_batch = Vec::with_capacity(BATCH_SIZE);
                }
            }

            // Send remaining results
            if !current_batch.is_empty() {
                let message = SearchMessage::Results {
                    search_id: String::new(),
                    items: current_batch,
                    is_complete: is_done,
                };
                let _ = self.event_sender.send(message);
            }
        } else if is_done {
            // If no matches but search is done, send empty complete message
            let message = SearchMessage::Results {
                search_id: String::new(),
                items: vec![],
                is_complete: true,
            };
            let _ = self.event_sender.send(message);
        }

        // Update search status if done
        if is_done {
            *self.is_searching.write().await = false;
        }

        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchMessage> {
        self.event_sender.subscribe()
    }
}