// src/terminal/terminal_manager.rs
use std::collections::HashMap;
use tokio::sync::{broadcast, RwLock};
use std::sync::Arc;
use anyhow::{Result, anyhow};
use crate::terminal::types::{TerminalMessage, TerminalSize};
use crate::terminal::terminal_server::TerminalServer;   

pub struct TerminalManager {
    terminals: RwLock<HashMap<String, Arc<TerminalServer>>>,
    event_sender: broadcast::Sender<TerminalMessage>,
}

impl TerminalManager {
    pub fn new() -> Self {
        let (event_sender, _) = broadcast::channel(100);


        Self {
            terminals: RwLock::new(HashMap::new()),
            event_sender,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalMessage> {
        println!("Subscribing to terminal events");
        self.event_sender.subscribe()
    }

    pub async fn create_terminal(&self, size: TerminalSize) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let event_sender = self.event_sender.clone();
        
        let terminal = Arc::new(TerminalServer::new(
            id.clone(),
            size,
            event_sender,
        )?);

        terminal.start().await?;
        self.terminals.write().await.insert(id.clone(), terminal);
        Ok(id)
    }

    pub async fn write_to_terminal(&self, id: &str, data: &[u8]) -> Result<()> {
        let terminals = self.terminals.read().await;
        if let Some(terminal) = terminals.get(id) {
            terminal.write(data).await?;
            Ok(())
        } else {
            Err(anyhow!("Terminal not found: {}", id))
        }
    }

    pub async fn resize_terminal(&self, id: &str, size: TerminalSize) -> Result<()> {
        let terminals = self.terminals.read().await;
        if let Some(terminal) = terminals.get(id) {
            terminal.resize(size).await?;
            Ok(())
        } else {
            Err(anyhow!("Terminal not found: {}", id))
        }
    }

    pub async fn close_terminal(&self, id: &str) -> Result<()> {
        if self.terminals.write().await.remove(id).is_none() {
            Err(anyhow!("Terminal not found: {}", id))
        } else {
            Ok(())
        }
    }
}