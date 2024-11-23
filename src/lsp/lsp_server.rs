// src/lsp/lsp_server.rs

use lsp_types::*;
use tokio::io::{BufReader, BufWriter, AsyncWriteExt, AsyncBufReadExt, AsyncReadExt};
use std::sync::Arc;
use anyhow::Result;
use serde_json::Value;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::RwLock;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use crate::lsp::capabilities::get_client_capabilities;
use lsp_types::ServerCapabilities;


pub struct LspServer {
    _process: Child,
    client_capabilities: ClientCapabilities,
    server_capabilities: RwLock<Option<ServerCapabilities>>,
    request_counter: AtomicU64,
    pending_requests: RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    writer: Arc<tokio::sync::Mutex<BufWriter<ChildStdin>>>,  // Changed to Mutex
    message_handler: Arc<MessageHandler>,
}

// Separate struct for message handling
struct MessageHandler {
    reader: tokio::sync::Mutex<BufReader<ChildStdout>>,
}

impl MessageHandler {
    async fn read_message(&self) -> Result<String> {
        let mut reader = self.reader.lock().await;
        let mut content_length: Option<usize> = None;

        // Read headers
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("EOF while reading headers"));
            }

            println!("Read header line: {:?}", line);  // Debug log

            // Remove trailing \r\n
            let line = line.trim();
            if line.is_empty() {
                break;
            }

            if let Some(length) = line.strip_prefix("Content-Length: ") {
                content_length = Some(length.parse()?);
            }
        }

        // Read content
        let content_length = content_length.ok_or_else(|| 
            anyhow::anyhow!("No Content-Length header found"))?;

        let mut content = vec![0; content_length];
        reader.read_exact(&mut content).await?;

        let message = String::from_utf8(content)?;
        println!("Read message: {}", message);  // Debug log
        Ok(message)
    }
}

impl LspServer {
    pub async fn initialize(
        mut process: Child,
        workspace_path: PathBuf,
        initialization_options: Option<serde_json::Value>,
    ) -> Result<Arc<Self>> {
        println!("Starting LSP server initialization");

        // Capture stderr for debugging
        let stderr = process.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stderr handle"))?;

        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 { break; }
                eprintln!("LSP stderr: {}", line.trim());
                line.clear();
            }
        });

        let stdin = process.stdin.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdin handle"))?;
        let stdout = process.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdout handle"))?;

        let writer = Arc::new(tokio::sync::Mutex::new(BufWriter::new(stdin)));
        let message_handler = Arc::new(MessageHandler {
            reader: tokio::sync::Mutex::new(BufReader::new(stdout)),
        });

        let server = Arc::new(Self {
            _process: process,
            client_capabilities: get_client_capabilities(),
            server_capabilities: RwLock::new(None),
            request_counter: AtomicU64::new(0),
            pending_requests: RwLock::new(HashMap::new()),
            writer,
            message_handler,
        });

        // Start message handler before sending initialize
        let server_clone = Arc::clone(&server);
        tokio::spawn(async move {
            if let Err(e) = server_clone.handle_messages().await {
                eprintln!("Message handler error: {}", e);
            }
        });

        let workspace_uri = url::Url::from_file_path(&workspace_path)
        .map_err(|_| anyhow::anyhow!("Failed to create URL from workspace path: {:?}", workspace_path))?
        .to_string();

        // Now create the URI from the proper file:// URL string
        let workspace_uri = Uri::from_str(&workspace_uri)
            .map_err(|e| anyhow::anyhow!("Failed to create URI from URL: {}", e))?;

        let workspace_folders = vec![WorkspaceFolder {
            uri: workspace_uri.clone(),
            name: workspace_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace")
                .to_string(),
        }];

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            initialization_options,
            capabilities: server.client_capabilities.clone(),
            trace: Some(TraceValue::Verbose),
            workspace_folders: Some(workspace_folders),
            client_info: Some(ClientInfo {
                name: String::from("rust-editor"),
                version: Some(String::from("0.1.0")),
            }),
            locale: Some(String::from("en-us")),
            ..Default::default()
        };

        // Send initialize request with longer timeout
        let response = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            server.send_request("initialize", serde_json::to_value(params)?)
        ).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => return Err(anyhow::anyhow!("Initialize request failed: {}", e)),
            Err(_) => return Err(anyhow::anyhow!("Initialize request timed out")),
        };
    
        println!("Received initialize response: {:?}", response);

        let init_result: serde_json::Value = serde_json::from_value(response.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse initialize response: {} - Response was: {:?}", e, response))?;

        // Extract the capabilities from the result
        let server_capabilities = match init_result.get("result") {
            Some(result) => match serde_json::from_value::<ServerCapabilities>(result.clone()) {
                Ok(caps) => Some(caps),
                Err(e) => {
                    eprintln!("Failed to parse server capabilities: {}", e);
                    None
                }
            },
            None => {
                eprintln!("Missing 'result' field in initialize response");
                None
            }
        };
        // Store server capabilities
        {
            let mut caps = server.server_capabilities.write().await;
            *caps = server_capabilities;
        }
    
        println!("Successfully stored server capabilities");
    
        // Send initialized notification
        match server.send_notification("initialized", serde_json::json!({})).await {
            Ok(_) => println!("Sent initialized notification successfully"),
            Err(e) => {
                eprintln!("Warning: Failed to send initialized notification: {}", e);
                // Don't fail initialization for this
            }
        }
    
        println!("LSP Server initialization completed successfully");
        Ok(server)
    }

    async fn send_message(&self, msg: String) -> Result<()> {
        let content_length = msg.len();
        let header = format!("Content-Length: {}\r\n\r\n{}", content_length, msg);
        
        //println!("Sending message: {}", header);  // Debug log
        
        let mut writer = self.writer.lock().await;
        writer.write_all(header.as_bytes()).await?;
        writer.flush().await?;
        
        Ok(())
    }

    async fn handle_messages(&self) -> Result<()> {
        loop {
            match self.message_handler.read_message().await {
                Ok(message) => {
                    let parsed: Value = match serde_json::from_str(&message) {
                        Ok(value) => value,
                        Err(e) => {
                            eprintln!("Failed to parse message: {}\nMessage was: {}", e, message);
                            continue;
                        }
                    };

                    println!("Received message: {:?}", parsed);  // Debug log

                    if let Some(id) = parsed.get("id").and_then(|id| id.as_u64()) {
                        // This is a response
                        if let Some(sender) = self.pending_requests.write().await.remove(&id) {
                            if let Some(error) = parsed.get("error") {
                                eprintln!("LSP error response: {:?}", error);
                            }
                            let _ = sender.send(parsed);
                        }
                    } else if parsed.get("method").is_some() {
                        // This is a notification
                        self.handle_notification(parsed).await?;
                    }
                },
                Err(e) => {
                    eprintln!("Error reading message: {}", e);
                    return Err(e);
                }
            }
        }
    }

    pub async fn send_request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_counter.fetch_add(1, Ordering::SeqCst);
        
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        // Use oneshot channel for this specific request
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        self.pending_requests.write().await.insert(id, response_tx);

        // Send the request
        self.send_message(request.to_string()).await?;

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(anyhow::anyhow!("Response channel closed")),
            Err(_) => Err(anyhow::anyhow!("Request timed out")),
        }
    }

    

    async fn handle_notification(&self, notification: Value) -> Result<()> {
        if let Some(method) = notification.get("method").and_then(|m| m.as_str()) {
            match method {
                "textDocument/publishDiagnostics" => {
                    println!("Received diagnostics: {:?}", notification);
                }
                _ => {
                    println!("Received notification: {}", method);
                }
            }
        }
        Ok(())
    }

    pub async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.send_message(notification.to_string()).await
    }
}

impl Drop for LspServer {
    fn drop(&mut self) {
        // Best effort to shutdown gracefully
        let _ = self.send_notification("shutdown", serde_json::json!({}));
        let _ = self.send_notification("exit", serde_json::json!({}));
    }
}