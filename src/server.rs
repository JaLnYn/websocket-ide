use lsp_types::{Position, CompletionList, Hover};
// src/server.rs
use serde::{Serialize, Deserialize};
use tokio::{
    net::{TcpListener, TcpStream}, sync::mpsc, time::Instant
};
use futures_util::{
    SinkExt,
    StreamExt,
};
use tokio_tungstenite::{
    accept_async,
    tungstenite::Message,
};
use std::{path::PathBuf, time::Duration};
use anyhow::Result;
use std::sync::Arc;

use crate::{file_system::{DiffChange, DocumentMetadata}, search::{SearchManager, SearchOptions, SearchResultItem}};
use crate::lsp::{types::LspConfiguration, lsp_manager::LspManager};

use crate::file_system::{FileSystem, FileNode, FileEvent, VersionedDocument};
use crate::utils::path_utils::{get_full_path, canonicalize_document_path};

use crate::terminal::{
    types::{TerminalMessage, TerminalSize},
    terminal_manager::TerminalManager,
};

use crate::search::{SearchMessage, SearchStatus};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "content")]
pub enum ClientMessage {
    GetDirectory { path: String },
    RefreshDirectory { path: String },
    OpenFile { path: String },
    CloseFile { path: String },
    ChangeFile {
        document: VersionedDocument,
        changes: Vec<DiffChange>,
    },
    SaveFile {
        document: VersionedDocument,
    },
    // New LSP messages
    Completion {
        path: String,
        position: Position,
    },
    Hover {
        path: String,
        position: Position,
    },
    Definition {
        path: String,
        position: Position,
    },

    CreateTerminal {
        cols: u16,
        rows: u16,
    },
    ResizeTerminal {
        id: String,
        cols: u16,
        rows: u16,
    },
    WriteTerminal {
        id: String,
        data: Vec<u8>,
    },
    CloseTerminal {
        id: String,
    },
    Search {
        id: String,
        query: String,
        search_filename_only: bool,
    },
    CancelSearch{
        id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "content")]
pub enum ServerMessage {
    Success {},
    DirectoryContent { path: PathBuf, content: Vec<FileNode> },
    FileSystemEvents { events: Vec<FileEvent> },
    DocumentPreview { 
        path: PathBuf, 
        content: String,
        metadata: DocumentMetadata,
    },
    DocumentChunk { 
        path: PathBuf, 
        content: Vec<u8>,
        offset: u64,
    },
    DocumentContent { 
        path: PathBuf, 
        content: String,
        metadata: DocumentMetadata,
        version: i32,
    },
    SaveSuccess { 
        document: VersionedDocument 
    },
    ChangeSuccess {
        document: VersionedDocument
    },
    CompletionResponse {
        completions: lsp_types::CompletionList,
    },
    HoverResponse {
        hover: lsp_types::Hover,
    },
    DefinitionResponse {
        locations: Vec<lsp_types::Location>,
    },
    Error { message: String },
    TerminalCreated { terminal_id: String },
    TerminalOutput { 
        terminal_id: String,
        data: Vec<u8>,
    },
    TerminalClosed { 
        id: String 
    },
    TerminalError {
        terminal_id: String,
        error: String,
    },
    SearchStatus {
        status: SearchStatus,
    },
    SearchResults {
        search_id: String,
        items: Vec<SearchResultItem>,
        is_complete: bool,
    },
}

pub struct Server {
    port: u16,
    file_system: Arc<FileSystem>,
    lsp_manager: Arc<LspManager>,
    terminal_manager: Arc<TerminalManager>,
    search_manager: Arc<SearchManager>,
}


impl Server {
    pub fn new(workspace_path: PathBuf, port: u16) -> Result<Self> {
        let file_system = Arc::new(FileSystem::new(workspace_path.clone())?);

        let lsp_configs = vec![
            LspConfiguration {
                name: "rust-analyzer".to_string(),
                file_extensions: vec!["rs".to_string()],
                server_path: PathBuf::from("rust-analyzer"),
                server_args: vec![],
                initialization_options: None,
            },
            // Add more language servers as needed
        ];
        let mut new_path = workspace_path.clone();
        if !new_path.is_absolute() {
            new_path = workspace_path.canonicalize()?;
        }
        
        let lsp_manager = Arc::new(LspManager::new(new_path, lsp_configs));
        let terminal_manager = Arc::new(TerminalManager::new());
        let search_manager = Arc::new(SearchManager::new(workspace_path.clone()));


        Ok(Self {
            port,
            file_system,
            lsp_manager,
            terminal_manager,
            search_manager
        })
    }

    async fn handle_client_message(
        &self,
        message: ClientMessage,
        write: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<TcpStream>,
            tokio_tungstenite::tungstenite::Message
        >,
    ) -> Result<()> {
        let response = match message {
            ClientMessage::GetDirectory { path: relative_path } => {
                println!(  "Received GetDirectory message: {:?}", relative_path);
                match get_full_path(self.file_system.get_workspace_path(), &relative_path) {
                    Ok(full_path) => {
                        match self.file_system.load_directory(&full_path).await {
                            Ok(content) => {
                                println!("Loaded directory: {:?}", full_path);
                                ServerMessage::DirectoryContent { 
                                    path: full_path, 
                                    content 
                                }
                            },
                            Err(e) => ServerMessage::Error { 
                                message: format!("Failed to load directory: {}", e) 
                            },
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },
            ClientMessage::RefreshDirectory { path: relative_path } => {
                match get_full_path(self.file_system.get_workspace_path(), &relative_path) {
                    Ok(full_path) => {
                        match self.file_system.refresh_directory(&full_path).await {
                            Ok(content) => {
                                println!("Refreshed directory: {:?}", full_path);
                                ServerMessage::DirectoryContent { 
                                    path: full_path, 
                                    content 
                                }
                            },
                            Err(e) => ServerMessage::Error { 
                                message: format!("Failed to refresh directory: {}", e) 
                            },
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },
            ClientMessage::CloseFile { path } => {
                match get_full_path(self.file_system.get_workspace_path(), &path) {
                    Ok(full_path) => {
                        // Validate file was open
                        let document_state = self.file_system
                            .get_document_state(&full_path)
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to get document state: {}", e))?;
            
                        if !document_state.is_open {
                            return Ok(write.send(Message::Text(
                                serde_json::to_string(&ServerMessage::Error {
                                    message: format!("File was not open: {}", path)
                                })?
                            )).await?);
                        }
            
                        // Notify LSP first
                        if let Some(server) = self.lsp_manager.get_server(&full_path).await? {
                            if let Err(e) = server
                                .send_notification(
                                    "textDocument/didClose",
                                    serde_json::json!({
                                        "textDocument": {
                                            "uri": full_path.to_str().ok_or_else(|| {
                                                anyhow::anyhow!("Invalid UTF-8 in path")
                                            })?
                                        }
                                    })
                                )
                                .await
                            {
                                eprintln!("LSP close notification failed: {}", e);
                            }
                        }
            
                        // Clean up resources
                        if let Err(e) = self.file_system.invalidate_document_cache(&full_path).await {
                            eprintln!("Failed to invalidate document cache: {}", e);
                        }
            
                        // Close in file system
                        match self.file_system.close_file(&full_path).await {
                            Ok(_) => ServerMessage::Success {},
                            Err(e) => ServerMessage::Error {
                                message: format!("Failed to close file: {}", e)
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },
            ClientMessage::OpenFile { path } => {
                match get_full_path(self.file_system.get_workspace_path(), &path) {
                    Ok(full_path) => {
                        // Validate file exists and is readable before opening
                        if !full_path.exists() {
                            ServerMessage::Error { 
                                message: format!("File does not exist: {}", path) 
                            }
                        } else if !full_path.is_file() {
                            ServerMessage::Error { 
                                message: format!("Path is not a file: {}", path) 
                            }
                        } else {
                            match self.file_system.open_file(&full_path).await {
                                Ok((content, metadata, version)) => {
                                    // First notify LSP before sending content to client
                                    if let Err(e) = self.lsp_manager
                                        .notify_document_opened(&full_path, &content, version)
                                        .await 
                                    {
                                        eprintln!("LSP notification failed: {}", e);
                                    }
                                    
                                    // Track file state for synchronization
                                    ServerMessage::DocumentContent { 
                                        path: full_path,
                                        content,
                                        metadata,
                                        version,
                                    }
                                },
                                Err(e) => ServerMessage::Error { 
                                    message: format!("Failed to open file: {}", e) 
                                },
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },
    
            ClientMessage::ChangeFile { document, changes } => {
                let path = match canonicalize_document_path(self.file_system.get_workspace_path(), &document) {
                    Ok(p) => p,
                    Err(e) => return Ok(write.send(Message::Text(
                        serde_json::to_string(&ServerMessage::Error {
                            message: format!("Invalid document path: {}", e)
                        })?
                    )).await?)
                };
    
                match self.file_system.change_document(document.clone(), changes).await {
                    Ok(new_document) => {
                        // Get updated content for LSP
                        match self.file_system.get_document_content(&path).await {
                            Ok(content) => {
                                // Convert to LSP format - now we send the full content
                                // as a single change since we're working with line-based diffs
                                let lsp_change = lsp_types::TextDocumentContentChangeEvent {
                                    range: None, // Full document update
                                    range_length: None,
                                    text: content.clone(),
                                };
    
                                // Notify LSP of changes
                                if let Err(e) = self.lsp_manager
                                    .notify_document_changed(&path, vec![lsp_change], new_document.version)
                                    .await 
                                {
                                    eprintln!("LSP change notification failed: {}", e);
                                }
                                
                                ServerMessage::ChangeSuccess { 
                                    document: new_document 
                                }
                            },
                            Err(e) => ServerMessage::Error {
                                message: format!("Failed to get document content: {}", e)
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error { 
                        message: format!("Failed to apply changes: {}", e) 
                    },
                }
            },
    
            ClientMessage::SaveFile { document } => {
                let path = match canonicalize_document_path(self.file_system.get_workspace_path(), &document) {
                    Ok(p) => p,
                    Err(e) => return Ok(write.send(Message::Text(
                        serde_json::to_string(&ServerMessage::Error {
                            message: format!("Invalid document path: {}", e)
                        })?
                    )).await?)
                };
                
                // Get content before saving for LSP notification
                match self.file_system.get_document_content(&path).await {
                    Ok(content) => {
                        match self.file_system.save_document(document.clone()).await {
                            Ok(new_document) => {
                                // Notify LSP about save
                                if let Err(e) = self.lsp_manager
                                    .notify_document_saved(&path, &content, new_document.version)
                                    .await 
                                {
                                    eprintln!("LSP save notification failed: {}", e);
                                }
    
                                ServerMessage::SaveSuccess { 
                                    document: new_document 
                                }
                            },
                            Err(e) => ServerMessage::Error { 
                                message: format!("Failed to save document: {}", e) 
                            },
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Failed to get document content: {}", e)
                    }
                }
            },
            ClientMessage::Completion { path, position } => {
                println!("Received completion request: {:?}", path);
                match get_full_path(self.file_system.get_workspace_path(), &path) {
                    Ok(full_path) => {
                        match self.lsp_manager.get_completions(&full_path, position).await {
                            Ok(Some(completions)) => ServerMessage::CompletionResponse { 
                                completions 
                            },
                            Ok(None) => ServerMessage::CompletionResponse { 
                                completions: CompletionList { 
                                    is_incomplete: false, 
                                    items: vec![] 
                                }
                            },
                            Err(e) => ServerMessage::Error {
                                message: e.to_string()
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },

            ClientMessage::Hover { path, position } => {
                println!("Received hover request: {:?}", path);
                match get_full_path(self.file_system.get_workspace_path(), &path) {
                    Ok(full_path) => {
                        match self.lsp_manager.get_hover(&full_path, position).await {
                            Ok(Some(hover)) => ServerMessage::HoverResponse { hover },
                            Ok(None) => ServerMessage::HoverResponse { 
                                hover: Hover { 
                                    contents: lsp_types::HoverContents::Scalar(
                                        lsp_types::MarkedString::String(String::new())
                                    ),
                                    range: None 
                                }
                            },
                            Err(e) => ServerMessage::Error {
                                message: e.to_string()
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },

            ClientMessage::Definition { path, position } => {
                println!("Received definition request: {:?}", path);
                match get_full_path(self.file_system.get_workspace_path(), &path) {
                    Ok(full_path) => {
                        match self.lsp_manager.get_definition(&full_path, position).await {
                            Ok(Some(locations)) => ServerMessage::DefinitionResponse { 
                                locations 
                            },
                            Ok(None) => ServerMessage::DefinitionResponse { 
                                locations: vec![] 
                            },
                            Err(e) => ServerMessage::Error {
                                message: e.to_string()
                            }
                        }
                    },
                    Err(e) => ServerMessage::Error {
                        message: format!("Invalid path: {}", e)
                    }
                }
            },
            ClientMessage::CreateTerminal { cols, rows } => {
                match self.terminal_manager.create_terminal(TerminalSize { cols, rows }).await {
                    Ok(id) => ServerMessage::TerminalCreated { terminal_id: id },
                    Err(e) => ServerMessage::Error { 
                        message: format!("Failed to create terminal: {}", e) 
                    },
                }
            },
            ClientMessage::WriteTerminal { id, data } => {
                match self.terminal_manager.write_to_terminal(&id, &data).await {
                    Ok(_) => ServerMessage::Success {},
                    Err(e) => ServerMessage::Error { 
                        message: format!("Failed to write to terminal: {}", e) 
                    },
                }
            },
            ClientMessage::ResizeTerminal { id, cols, rows } => {
                match self.terminal_manager.resize_terminal(&id, TerminalSize { cols, rows }).await {
                    Ok(_) => ServerMessage::Success {},
                    Err(e) => ServerMessage::Error { 
                        message: format!("Failed to resize terminal: {}", e) 
                    },
                }
            },
            ClientMessage::CloseTerminal { id } => {
                match self.terminal_manager.close_terminal(&id).await {
                    Ok(_) => ServerMessage::TerminalClosed { id },
                    Err(e) => ServerMessage::Error { 
                        message: format!("Failed to close terminal: {}", e) 
                    },
                }
            },
            ClientMessage::Search { id, query, search_filename_only } => {
                match self.search_manager.create_search(&query, Some(id), search_filename_only).await {
                    Ok(_) => ServerMessage::Success {},
                    Err(e) => ServerMessage::Error {
                        message: format!("Search failed: {}", e)
                    }
                }
            },
            ClientMessage::CancelSearch {id} => {
                self.search_manager.close_search(id).await;
                ServerMessage::Success {}
            }, 
        };

        if matches!(response, ServerMessage::Success {}) {
            return Ok(());
        }

        let message = serde_json::to_string(&response)?;
        println!("Sending message: {}", message);
        write.send(Message::Text(message)).await?;
        Ok(())
    }

    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        println!("New connection attempt from: {}", stream.peer_addr()?);

        let ws_stream = accept_async(stream).await?;
        let (mut write, mut read) = ws_stream.split();
        
        let mut fs_events = self.file_system.subscribe();
        let mut terminal_events = self.terminal_manager.subscribe();
        let mut search_events = self.search_manager.subscribe();

        
        // Buffer for collecting events
        let mut event_buffer = Vec::with_capacity(100);
        let mut last_send = Instant::now();
        
        loop {
            println!("Loop iteration");
            tokio::select! {
                Some(msg) = read.next() => {
                    println!("Server received message: {:?}", msg);
                    match msg? {
                        Message::Text(text) => {
                            match serde_json::from_str::<ClientMessage>(&text) {
                                Ok(client_message) => {
                                    if let Err(e) = self.handle_client_message(client_message, &mut write).await {
                                        println!("Invalid message format: {}", e);
                                        let error_message = ServerMessage::Error {
                                            message: format!("Error processing request: {}", e),
                                        };
                                        write.send(Message::Text(serde_json::to_string(&error_message)?)).await?;
                                    }
                                },
                                Err(e) => {
                                    println!("Invalid message format: {}", e);
                                    let error_message = ServerMessage::Error {
                                        message: format!("Invalid message format: {}", e),
                                    };
                                    write.send(Message::Text(serde_json::to_string(&error_message)?)).await?;
                                }
                            }
                        }
                        Message::Close(_) => return Ok(()),
                        _ => continue,
                    }
                }
                Ok(event) = fs_events.recv() => {
                    println!("Server received file system event");
                    event_buffer.push(event);
                    
                    if event_buffer.len() >= 100 || last_send.elapsed() >= Duration::from_millis(100) {
                        if !event_buffer.is_empty() {
                            let message = ServerMessage::FileSystemEvents { 
                                events: std::mem::replace(&mut event_buffer, Vec::with_capacity(100)) 
                            };
                            if let Ok(text) = serde_json::to_string(&message) {
                                let _ = write.send(Message::Text(text)).await;
                            }
                            last_send = Instant::now();
                        }
                    }
                }
                Ok(term_msg) = terminal_events.recv() => {
                    println!("Server received terminal message");
                    match term_msg {
                        TerminalMessage::Output { terminal_id, data } => {
                            println!("Terminal output: {:?}", data);
                            let message = ServerMessage::TerminalOutput { terminal_id, data };
                            if let Ok(text) = serde_json::to_string(&message) {
                                let _ = write.send(Message::Text(text)).await;
                            }
                        }
                        TerminalMessage::Error { terminal_id, error } => {
                            println!("Terminal error: {:?}", error);
                            let message = ServerMessage::TerminalError { terminal_id, error };
                            if let Ok(text) = serde_json::to_string(&message) {
                                let _ = write.send(Message::Text(text)).await;
                            }
                        }
                        _ => {
                            println!("Unhandled terminal message: {:?}", term_msg);
                        }
                    }
                }
                Ok(search_msg) = search_events.recv() => {
                    match search_msg {
                        SearchMessage::Results { search_id, items, is_complete } => {
                            let message = ServerMessage::SearchResults { 
                                search_id,
                                items,
                                is_complete
                            };
                            if let Ok(json) = serde_json::to_string(&message) {
                                write.send(Message::Text(json)).await?;
                            }
                        },
                        SearchMessage::Error { search_id, error } => {
                            let message = ServerMessage::Error { 
                                message: format!("Search error ({}): {}", search_id, error)
                            };
                            if let Ok(json) = serde_json::to_string(&message) {
                                write.send(Message::Text(json)).await?;
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn start(&self) -> Result<()> {
        println!("Initializing file system...");
        self.file_system.init().await?;
        
        // Start the file watcher
        println!("Starting file watcher...");
        self.file_system.start_watching().await?;

        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        println!("WebSocket server listening on: {}", addr);
        
        let server = Arc::new(self.clone());
        
        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            let server = Arc::clone(&server);
            
            tokio::spawn(async move {
                if let Err(e) = server.handle_connection(stream).await {
                    eprintln!("Error handling connection from {}: {}", addr, e);
                }
            });
        }
        
        Ok(())
    }
}

// Make Server cloneable
impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            port: self.port,
            file_system: Arc::clone(&self.file_system),
            lsp_manager: Arc::clone(&self.lsp_manager),
            terminal_manager: Arc::clone(&self.terminal_manager),
            search_manager: Arc::clone(&self.search_manager),
        }
    }
}