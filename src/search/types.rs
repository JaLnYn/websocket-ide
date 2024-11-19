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

pub struct ActiveSearch {
    receiver: mpsc::Receiver<ServerMessage>,
    _task: tokio::task::JoinHandle<()>,
}