use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "content")]
pub enum TerminalMessage {
    Input {
        terminal_id: String,
        data: Vec<u8>,
    },
    Output {
        terminal_id: String,
        data: Vec<u8>,
    },
    Resize {
        terminal_id: String,
        size: TerminalSize,
    },
    Error {
        terminal_id: String,
        error: String,
    },
}
