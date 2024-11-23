// src/search/search_manager.rs
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tokio::time::interval;
use nucleo::{Config, Nucleo, Utf32String};
use nucleo::pattern::{CaseMatching, Normalization};
use anyhow::Result;
use tokio::fs;

use crate::search::{SearchMessage, SearchResultItem};

const BATCH_SIZE: usize = 50;
const TICK_TIMEOUT_MS: u64 = 10;
const POLL_INTERVAL_MS: u64 = 100;
const SEARCH_TIMEOUT_SECS: u64 = 10;
const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1MB

#[derive(Clone, PartialEq, Debug)]
enum SearchMode {
    Filename,
    Content,
}


#[derive(Clone)]
struct LineContent {
    path: PathBuf,
    line_number: u32,
    line: String,
}

pub struct SearchManager {
    workspace_path: PathBuf,
    searcher: Arc<RwLock<Nucleo<LineContent>>>,
    event_sender: broadcast::Sender<SearchMessage>,
    last_query: Arc<RwLock<Option<String>>>,
    is_searching: Arc<RwLock<bool>>,
    current_mode: Arc<RwLock<SearchMode>>,
}

impl SearchManager {
    pub fn new(workspace_path: PathBuf) -> Arc<Self> {
        let (event_sender, _) = broadcast::channel(100);

        let notify = Arc::new(|| {});

        // Change to single column
        let searcher = Nucleo::new(
            Config::DEFAULT.match_paths(),
            notify,
            None,
            1  // Single column
        );

        let manager = Arc::new(Self {
            workspace_path,
            searcher: Arc::new(RwLock::new(searcher)),
            event_sender,
            last_query: Arc::new(RwLock::new(None)),
            is_searching: Arc::new(RwLock::new(false)),
            current_mode: Arc::new(RwLock::new(SearchMode::Filename)),
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
                    if search_start.is_none() {
                        search_start = Some(std::time::Instant::now());
                    }

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

    async fn initialize_files(&self, search_mode: &SearchMode) -> Result<()> {
        let searcher = self.searcher.read().await;
        let injector = searcher.injector();
        let mut count = 0;
        
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
            
            match search_mode {
                SearchMode::Content => {
                    // Check file size before reading
                    if let Ok(metadata) = fs::metadata(&path).await {
                        if metadata.len() > MAX_FILE_SIZE {
                            println!("Skipping large file: {:?}", path);
                            continue;
                        }

                        match fs::read_to_string(&path).await {
                            Ok(content) => {
                                for (line_number, line) in content.lines().enumerate() {
                                    let line_content = LineContent {
                                        path: path.clone(),
                                        line_number: (line_number + 1) as u32,
                                        line: line.to_string(),
                                    };

                                    injector.push(line_content, |content, columns| {
                                        // Only use single column - content for content search
                                        columns[0] = content.line.clone().into();
                                    });
                                }
                            }
                            Err(e) => {
                                println!("Error reading file {:?}: {}", path, e);
                                continue;
                            }
                        }
                    }
                }
                SearchMode::Filename => {
                    let line_content = LineContent {
                        path: path.clone(),
                        line_number: 0,
                        line: String::new(),
                    };

                    injector.push(line_content, |content, columns| {
                        // Only use single column - path for filename search
                        columns[0] = content.path.to_string_lossy().to_string().into();
                    });
                }
            }
            count += 1;
        }

        println!("Injected {} files for mode {:?}", count, search_mode);
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
        query: &str,
        search_content: bool,
    ) -> Result<()> {
        let new_mode = if search_content {
            SearchMode::Content
        } else {
            SearchMode::Filename
        };
    
        let mut current_mode = self.current_mode.write().await;
        let mut last_query = self.last_query.write().await;
        let mode_changed = *current_mode != new_mode;
        *current_mode = new_mode.clone();
    
        // Determine if we need to reinitialize
        let initialization_needed = mode_changed;
    
        let should_reparse = if let Some(last) = last_query.as_ref() {
            query.starts_with(last) && !mode_changed
        } else {
            false
        };
    
        if initialization_needed {
            println!("Starting new search with mode: {:?}", new_mode);
            self.searcher.write().await.restart(true);
            
            // Initialize files and wait for completion
            if let Err(e) = self.initialize_files(&new_mode).await {
                eprintln!("Failed to initialize files: {}", e);
                return Err(e);
            }
    
            // After initialization, set up the search pattern
            let mut searcher = self.searcher.write().await;
            searcher.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, false);
            
            *last_query = Some(query.to_string());
            *self.is_searching.write().await = true;
        } else {
            println!("Continuing search");
            let mut searcher = self.searcher.write().await;
            searcher.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, should_reparse);
            
            *last_query = Some(query.to_string());
            *self.is_searching.write().await = true;
        }
        
        Ok(())
    }

    async fn process_results(&self) -> Result<()> {
        let mut searcher = self.searcher.write().await;
        let current_mode = self.current_mode.read().await;
        
        let status = searcher.tick(TICK_TIMEOUT_MS);
        let snapshot = searcher.snapshot();
        let matched_count = snapshot.matched_item_count();
        let is_done = !status.running;

        if matched_count > 0 {
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);
            
            for item in snapshot.matched_items(0..matched_count) {
                let line_content = &item.data;
                
                match *current_mode {
                    SearchMode::Content => {
                        current_batch.push(SearchResultItem {
                            path: line_content.path.to_string_lossy().to_string(),
                            line_number: line_content.line_number,
                            content: line_content.line.clone(),
                        });
                    }
                    SearchMode::Filename => {
                        current_batch.push(SearchResultItem {
                            path: line_content.path.to_string_lossy().to_string(),
                            line_number: 0,
                            content: String::new(),
                        });
                    }
                }

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

            if !current_batch.is_empty() {
                let message = SearchMessage::Results {
                    search_id: String::new(),
                    items: current_batch,
                    is_complete: is_done,
                };
                let _ = self.event_sender.send(message);
            }
        } else if is_done {
            let message = SearchMessage::Results {
                search_id: String::new(),
                items: vec![],
                is_complete: true,
            };
            let _ = self.event_sender.send(message);
        }

        if is_done {
            *self.is_searching.write().await = false;
        }

        Ok(())
    }

    pub async fn close_search(&self) {
        *self.is_searching.write().await = false;
        let mut searcher = self.searcher.write().await;
        searcher.restart(true);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SearchMessage> {
        self.event_sender.subscribe()
    }
}