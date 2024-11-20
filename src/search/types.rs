use serde::{Serialize, Deserialize};
use tokio::sync::mpsc;
use std::path::PathBuf;

use crate::server::ServerMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOptions {
    pub query: String,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub file_path: PathBuf,
    pub line_number: u32,
    pub line_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchStatus {
    Started,
    Completed,
    Error { message: String },
}

struct ActiveSearch {
    receiver: mpsc::Receiver<ServerMessage>,
    _task: tokio::task::JoinHandle<()>,
}


#[derive(Clone)]
pub struct SearchItem {
    pub path: String,
    pub line_number: u32,
    pub content: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SearchResultItem {
    pub path: String,
    pub line_number: u32,
    pub content: String,
}

#[derive(Clone)]
pub enum SearchMessage {
    Results {
        search_id: String,
        items: Vec<SearchResultItem>, // Vec of matching results
        is_complete: bool,  // indicates if this is the final batch
    },
    Error {
        search_id: String,
        error: String,
    },
}
