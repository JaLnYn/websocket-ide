// src/terminal/terminal_server.rs
use anyhow::Result;
use portable_pty::{native_pty_system, PtyPair, PtySize, CommandBuilder};
use std::io::{Read, Write};
use tokio::sync::{broadcast, Mutex};
use std::sync::Arc;
use crate::terminal::types::{TerminalMessage, TerminalSize};

pub struct TerminalServer {
    id: String,
    pty_pair: Arc<Mutex<PtyPair>>,
    event_sender: broadcast::Sender<TerminalMessage>,
}

impl TerminalServer {
    pub fn new(
        id: String,
        size: TerminalSize,
        event_sender: broadcast::Sender<TerminalMessage>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        
        let pty_pair = pty_system.openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell_cmd = if cfg!(windows) {
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        let mut cmd = CommandBuilder::new(shell_cmd);
        if !cfg!(windows) {
            cmd.env("TERM", "xterm-256color");
        }

        let child = pty_pair.slave.spawn_command(cmd)?;
        std::mem::drop(child);

        Ok(Self {
            id,
            pty_pair: Arc::new(Mutex::new(pty_pair)),
            event_sender,
        })
    }

    pub async fn start(&self) -> Result<()> {
        let id = self.id.clone();
        let pty_pair = Arc::clone(&self.pty_pair);
        let event_sender = self.event_sender.clone();

        let mut reader = {
            let pair = pty_pair.lock().await;
            pair.master.try_clone_reader()?
        };

        tokio::task::spawn_blocking(move || {
            let mut buffer = [0u8; 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        let msg = TerminalMessage::Output {
                            terminal_id: id.clone(),
                            data: buffer[..n].to_vec(),
                        };
                        // Using blocking_send since we're in a blocking task
                        if event_sender.send(msg).is_err() { break; }
                    }
                    Ok(_) => break,  // EOF
                    Err(e) => {
                        let msg = TerminalMessage::Error {
                            terminal_id: id.clone(),
                            error: e.to_string(),
                        };
                        let _ = event_sender.send(msg);
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn write(&self, data: &[u8]) -> Result<()> {
        let pair = self.pty_pair.lock().await;
        let mut writer = pair.master.take_writer()?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub async fn resize(&self, size: TerminalSize) -> Result<()> {
        let pair = self.pty_pair.lock().await;
        pair.master.resize(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

unsafe impl Send for TerminalServer {}
unsafe impl Sync for TerminalServer {}