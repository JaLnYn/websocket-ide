use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LspConfiguration {
    pub name: String,
    pub file_extensions: Vec<String>,
    pub server_path: PathBuf,
    pub server_args: Vec<String>,
    pub initialization_options: Option<serde_json::Value>,
}

// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct Position {
//     pub line: u32,
//     pub character: u32,
// }