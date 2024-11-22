// src/search/search_manager.rs
use std::sync::Arc;
use std::path::{Path, PathBuf};
use tokio::sync::{broadcast, RwLock};
use nucleo::{Config, Nucleo, Utf32String};
use nucleo::pattern::{CaseMatching, Normalization};
use anyhow::Result;

use crate::search::{SearchMessage, SearchResultItem};

const BATCH_SIZE: usize = 50;
const TICK_TIMEOUT_MS: u64 = 10;

pub struct SearchManager {
    workspace_path: PathBuf,
    searcher: Arc<RwLock<Nucleo<PathBuf>>>,
    event_sender: broadcast::Sender<SearchMessage>,
    last_query: Arc<RwLock<Option<String>>>,
    is_searching: Arc<RwLock<bool>>,
}

impl SearchManager {
    pub fn new(workspace_path: PathBuf) -> Arc<Self> {
        let (event_sender, _) = broadcast::channel(100);
        
        // Create channel for notifying about new results
        let (notify_tx, _) = broadcast::channel(1);
        
        let notify_tx_clone = notify_tx.clone();
        let notify = Arc::new(move || {
            // SOMETHINGS WRONG HERE. THIS IS NEVER CALLED FOR NEW RESULTS
            println!("Nucleo notifying of new results");
            let tx = notify_tx_clone.clone();
            tokio::spawn(async move {
                let _ = tx.send(());
            });
        });

        // Initialize nucleo with 1 column (just the path for now)
        let searcher = Nucleo::new(
            Config::DEFAULT.match_paths(),
            notify,
            None, // Use default thread count
            1     // One column for path
        );

        let manager = Arc::new(Self {
            workspace_path,
            searcher: Arc::new(RwLock::new(searcher)),
            event_sender,
            last_query: Arc::new(RwLock::new(None)),
            is_searching: Arc::new(RwLock::new(false)),
        });

        // Create a background task to continuously process results
        let manager_clone = Arc::clone(&manager);
        tokio::spawn(async move {
            let mut rx = notify_tx.subscribe();
            println!("Starting result processing loop");
            
            loop {
                // Only process if we get a notification and are actively searching
                if rx.recv().await.is_ok() {
                    let is_searching = *manager_clone.is_searching.read().await;
                    if is_searching {
                        if let Err(e) = manager_clone.process_results().await {
                            eprintln!("Error processing results: {}", e);
                        }
                    }
                }
            }
        });

        // Initialize file collection
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            if let Err(e) = manager_clone.initialize_files().await {
                eprintln!("Failed to initialize files: {}", e);
            }
            println!("File initialization complete");
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
            println!("Injecting file: {:?}", path);
            
            // Inject the path, converting it to the format nucleo expects
            injector.push(path, |path, columns| {
                let path_str = path.to_string_lossy().to_string();
                println!("Converting path to matcher column: {}", path_str);
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
        &self,
        query: &str,
        search_filename_only: bool,
    ) -> Result<()> {
        println!("Creating search for query: {}", query);
        let mut searcher = self.searcher.write().await;
        let mut last_query = self.last_query.write().await;
        
        // Check if we can optimize by using reparse
        let should_reparse = if let Some(last) = last_query.as_ref() {
            query.starts_with(last)
        } else {
            false
        };

        // Update the pattern
        if should_reparse {
            println!("Reparsing existing pattern");
            searcher.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, true);
        } else {
            println!("Creating new pattern");
            searcher.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, false);
        }

        // Debug: Print pattern state
        println!("Pattern after update: {:?}", searcher.pattern);
        
        *last_query = Some(query.to_string());
        *self.is_searching.write().await = true;
        
        // Debug: Print injector state
        let injector = searcher.injector();
        println!("Items in injector before tick: {}", injector.injected_items());
        
        // Trigger initial match and debug
        let status = searcher.tick(TICK_TIMEOUT_MS);
        println!("Initial tick status: {:?}", status);
        
        let snapshot = searcher.snapshot();
        println!("Initial snapshot - matched items: {}", snapshot.matched_item_count());
        
        // Debug: Try to print first few items to see what we're matching against
        // if injector.injected_items() > 0 {
            // for i in 0..std::cmp::min(5, injector.injected_items()) {
                // if let Some(item) = injector.get(i) {
                    // println!("Sample item {}: {:?} with columns: {:?}", i, item.data, item.matcher_columns);
                // }
            // }
        // }
        
        Ok(())
    }


    pub async fn close_search(&self) {
        *self.is_searching.write().await = false;
        let mut searcher = self.searcher.write().await;
        searcher.restart(true);
    }

    async fn process_results(&self) -> Result<()> {
        let mut searcher = self.searcher.write().await;
        
        // Debug: Print state before tick
        println!("Processing results - current pattern: {:?}", searcher.pattern);
        let injector = searcher.injector();
        println!("Total items before tick: {}", injector.injected_items());
        
        let status = searcher.tick(TICK_TIMEOUT_MS);
        println!("Tick status: {:?}", status);
        
        let snapshot = searcher.snapshot();
        let matched_count = snapshot.matched_item_count();
        
        println!("Processing results, found {} matches", matched_count);
        
        if matched_count > 0 {
            // Debug: Print first few matches to see what's matching
            for (i, item) in snapshot.matched_items(0..std::cmp::min(5, matched_count)).enumerate() {
                println!("Match {}: {:?} with columns: {:?}", i, item.data, item.matcher_columns);
            }
        }

        if matched_count == 0 {
            return Ok(());
        }

        // Send results in batches
        let mut current_batch = Vec::with_capacity(BATCH_SIZE);
        
        for item in snapshot.matched_items(0..matched_count) {
            println!("Processing result: {:?}", item.data);
            let path = item.data.to_string_lossy().to_string();
            
            current_batch.push(SearchResultItem {
                path,
                line_number: 0,
                content: String::new(),
            });

            if current_batch.len() >= BATCH_SIZE {
                println!("Sending batch of {} results", current_batch.len());
                let is_complete = false;
                let message = SearchMessage::Results {
                    search_id: String::new(), // Ignored in single search mode
                    items: current_batch,
                    is_complete,
                };
                
                let _ = self.event_sender.send(message);
                current_batch = Vec::with_capacity(BATCH_SIZE);
            }
        }

        // Send any remaining results
        if !current_batch.is_empty() {
            println!("Sending final batch of {} results", current_batch.len());
            let message = SearchMessage::Results {
                search_id: String::new(),
                items: current_batch,
                is_complete: true,
            };
            
            let _ = self.event_sender.send(message);
        }

        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchMessage> {
        self.event_sender.subscribe()
    }
}