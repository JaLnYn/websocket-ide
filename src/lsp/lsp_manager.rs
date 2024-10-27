use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::{Result, Context};
use lsp_types::*;
use tokio::process::Command;
use std::ffi::OsStr;
use url::Url;

use super::{lsp_server::LspServer, types::LspConfiguration};

pub struct LspManager {
    workspace_path: PathBuf,
    extension_map: HashMap<String, String>,
    server_configs: HashMap<String, LspConfiguration>,
    active_servers: RwLock<HashMap<String, Arc<LspServer>>>,
}

impl LspManager {
    pub fn new(workspace_path: PathBuf, configs: Vec<LspConfiguration>) -> Self {
        let mut extension_map = HashMap::new();
        let mut server_configs = HashMap::new();

        for config in configs {
            let server_name = config.name.clone();
            for ext in &config.file_extensions {
                extension_map.insert(ext.clone(), server_name.clone());
            }
            server_configs.insert(server_name, config);
        }

        Self {
            workspace_path,
            extension_map,
            server_configs,
            active_servers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_server(&self, path: &PathBuf) -> Result<Option<Arc<LspServer>>> {
        // Get file extension
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .map(String::from);

        let Some(ext) = extension else {
            println!("No extension found for path: {:?}", path);
            return Ok(None);
        };

        let Some(server_name) = self.extension_map.get(&ext) else {
            println!("No server configured for extension: {}", ext);
            return Ok(None);
        };

        // First check active servers
        {
            let active_servers = self.active_servers.read().await;
            println!("Current active servers: {:?}", active_servers.keys().collect::<Vec<_>>());
            if let Some(server) = active_servers.get(server_name) {
                println!("Found existing server for: {}", server_name);
                return Ok(Some(Arc::clone(server)));
            }
        }

        // Initialize new server with proper error handling
        match self.initialize_server(server_name).await {
            Ok(server) => {
                println!("Successfully initialized server for: {}", server_name);
                Ok(Some(server))
            }
            Err(e) => {
                eprintln!("Failed to initialize server for {}: {}", server_name, e);
                // Could add retry logic here
                Ok(None)
            }
        }
    }

    async fn initialize_server(&self, server_name: &str) -> Result<Arc<LspServer>> {
        let config = self.server_configs.get(server_name)
            .ok_or_else(|| anyhow::anyhow!("No config found for server: {}", server_name))?;
    
        println!("Initializing LSP server: {} at path: {:?}", server_name, config.server_path);
    
        // Start server process
        let mut command = Command::new(&config.server_path);
        command
            .args(&config.server_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    
        let process = command.spawn()
            .context(format!("Failed to start LSP server process for {}", server_name))?;
    
        // Initialize server
        let server = match LspServer::initialize(
            process,
            self.workspace_path.clone(),
            config.initialization_options.clone(),
        ).await {
            Ok(server) => {
                println!("Successfully initialized LSP server for {}", server_name);
                server
            },
            Err(e) => {
                eprintln!("Failed to initialize LSP server for {}: {}", server_name, e);
                return Err(e);
            }
        };
    
        // Store in active servers
        {
            let mut active_servers = self.active_servers.write().await;
            println!("Successfully storing server '{}' in active_servers", server_name);
            active_servers.insert(server_name.to_string(), Arc::clone(&server));
        }
    
        Ok(server)
    }

    pub async fn notify_document_opened(
        &self,
        path: &PathBuf,
        content: &str,
        version: i32,
    ) -> Result<()> {
        let server = self.get_server(path).await?;

        let file_uri = Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("Failed to create URI from path: {:?}", path))?
            .to_string();

        let params = serde_json::json!({
            "textDocument": {
                "uri": file_uri,
                "languageId": path.extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or("plaintext"),
                "version": version,
                "text": content
            }
        });

        if let Some(server) = server {
            server.send_notification("textDocument/didOpen", params).await?;
        }
        Ok(())
    }

    pub async fn notify_document_changed(
        &self,
        path: &PathBuf,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) -> Result<()> {
        let server = self.get_server(path).await?;

        let file_uri = Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("Failed to create URI from path: {:?}", path))?
            .to_string();

        let params = serde_json::json!({
            "textDocument": {
                "uri": file_uri,
                "version": version
            },
            "contentChanges": changes
        });

        if let Some(server) = server {
            server.send_notification("textDocument/didOpen", params).await?;
        }
        Ok(())
    }

    pub async fn notify_document_saved(
        &self,
        path: &PathBuf,
        text: &str,
        version: i32,
    ) -> Result<()> {
        let server = self.get_server(path).await?;

        let file_uri = Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("Failed to create URI from path: {:?}", path))?
            .to_string();

        let params = serde_json::json!({
            "textDocument": {
                "uri": file_uri,
                "version": version
            },
            "text": text
        });

        if let Some(server) = server {
            server.send_notification("textDocument/didOpen", params).await?;
        }
        Ok(())
    }

    async fn send_request_with_uri<T: serde::de::DeserializeOwned>(
        &self,
        path: &PathBuf,
        method: &str,
        position: Position,
    ) -> Result<Option<T>> {
        if let Some(server) = self.get_server(path).await? {
            let file_uri = Url::from_file_path(path)
                .map_err(|_| anyhow::anyhow!("Failed to create URI from path: {:?}", path))?
                .to_string();

            let params = serde_json::json!({
                "textDocument": {
                    "uri": file_uri
                },
                "position": position
            });

            let response = server.send_request(method, params).await?;
            
            // Extract result from JSON-RPC response
            if let Some(result) = response.get("result") {
                if result.is_null() {
                    return Ok(None);
                }
                return Ok(Some(serde_json::from_value(result.clone())?));
            }
            
            if let Some(error) = response.get("error") {
                return Err(anyhow::anyhow!("LSP error: {:?}", error));
            }
            
            Ok(None)
        } else {
            Ok(None)
        }
    }

    pub async fn get_completions(
        &self,
        path: &PathBuf,
        position: Position
    ) -> Result<Option<CompletionList>> {
        self.send_request_with_uri(path, "textDocument/completion", position).await
    }

    pub async fn get_hover(
        &self,
        path: &PathBuf,
        position: Position
    ) -> Result<Option<Hover>> {
        self.send_request_with_uri(path, "textDocument/hover", position).await
    }

    pub async fn get_definition(
        &self,
        path: &PathBuf,
        position: Position
    ) -> Result<Option<Vec<Location>>> {
        self.send_request_with_uri(path, "textDocument/definition", position).await
    }
}