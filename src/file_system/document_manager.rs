use std::path::PathBuf;
use std::collections::{HashMap, VecDeque};
use tokio::sync::RwLock;
use anyhow::{Result, Context, bail};
use serde::{Serialize, Deserialize};
use tokio::fs;
use encoding_rs::{Encoding, UTF_8};

// File size thresholds and configuration
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB default limit
const CACHE_SIZE_LIMIT: u64 = 1024 * 1024; // 1MB cache limit per file

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VersionedDocument {
    pub uri: PathBuf,
    pub version: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FileType {
    Text,
    Binary,
    SymLink,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEncoding {
    pub encoding: String,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocumentMetadata {
    pub size: u64,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub created_at: Option<u64>,
    pub modified_at: Option<u64>,
    pub readonly: bool,
    pub file_type: FileType,
    pub encoding: FileEncoding,
    pub line_ending: LineEnding,
}

#[derive(Debug, Clone)]
pub struct DocumentState {
    pub is_open: bool,
    pub version: i32,      // For LSP synchronization
    pub last_modification: u64,
    pub is_dirty: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum LineEnding {
    CRLF,
    LF,
    Mixed,
}

#[derive(Debug)]
struct CacheEntry {
    content: String,
    metadata: DocumentMetadata,
    last_accessed: std::time::Instant, // For LRU cache TODO
}

#[derive(Debug)]
pub struct DocumentManager {
    workspace_path: PathBuf, // to check if document is within workspace TODO
    // open_files is a way to check if a file is already open
    document_states: RwLock<HashMap<PathBuf, DocumentState>>,
    cache: RwLock<HashMap<PathBuf, CacheEntry>>,
    cache_queue: RwLock<VecDeque<PathBuf>>,
    max_cache_size: u64,
    current_cache_size: RwLock<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiffChange {
    pub value: String,
    pub added: bool,
    pub removed: bool,
}



impl DocumentManager {
    pub fn new(workspace_path: PathBuf) -> Result<Self> {
        let workspace_path = workspace_path.canonicalize()?;
        println!("Initialized document manager at: {:?}", workspace_path);
        
        Ok(Self {
            workspace_path,
            document_states: RwLock::new(HashMap::new()),
            cache: RwLock::new(HashMap::new()),
            cache_queue: RwLock::new(VecDeque::new()),
            max_cache_size: CACHE_SIZE_LIMIT,
            current_cache_size: RwLock::new(0),
        })
    }

    // Detect file type (binary or text)
    async fn detect_file_type(&self, path: &PathBuf) -> Result<FileType> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut buffer = vec![0; 512];
        let n = tokio::io::AsyncReadExt::read(&mut file, &mut buffer).await?;
        buffer.truncate(n);

        // Check for null bytes which usually indicate binary content
        if buffer.iter().take(n).any(|&byte| byte == 0) {
            return Ok(FileType::Binary);
        }

        // Check if it's a symlink
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Ok(FileType::SymLink);
        }

        Ok(FileType::Text)
    }

    // Detect file encoding
    fn detect_encoding(&self, data: &[u8]) -> FileEncoding {
        let mut detector = chardetng::EncodingDetector::new();
        detector.feed(data, true);
        let encoding = detector.guess(None, true);
        
        FileEncoding {
            encoding: encoding.name().to_string(),
            confidence: 0.9, // chardetng doesn't provide confidence, so we use a default
        }
    }

    // Detect line endings
    fn detect_line_ending(&self, content: &str) -> LineEnding {
        let mut has_crlf = false;
        let mut has_lf = false;

        for line in content.lines() {
            if line.ends_with('\r') {
                has_crlf = true;
            } else {
                has_lf = true;
            }

            if has_crlf && has_lf {
                return LineEnding::Mixed;
            }
        }

        if has_crlf {
            LineEnding::CRLF
        } else {
            LineEnding::LF
        }
    }

    // file is closed
    pub async fn close_file(&self, path: &PathBuf) {
        println!("Closing file: {:?}", path);
        if let Some(state) = self.document_states.write().await.get_mut(path) {
            state.is_open = false;
        }
        // make lsp call here TODO
    }

    // Main file reading function

    pub async fn change_document(
        &self,
        doc: &VersionedDocument,
        changes: Vec<DiffChange>,
    ) -> Result<VersionedDocument> {
        let path = &doc.uri;
        let mut states = self.document_states.write().await;
        
        if let Some(state) = states.get_mut(path) {
            // Version check
            if state.version >= doc.version {
                return Err(anyhow::anyhow!(
                    "Version conflict: document has been modified. Server: {}, client: {}", 
                    state.version, doc.version
                ));
            }

            // Get current content
            let current_content = {
                let cache = self.cache.read().await;
                if let Some(cache_entry) = cache.get(path) {
                    cache_entry.content.clone()
                } else {
                    tokio::fs::read_to_string(path).await?
                }
            };

            // Build new content by applying changes sequentially
            let mut result = String::new();
            let mut last_position = 0;
            let chars: Vec<char> = current_content.chars().collect();
            
            println!("Applying changes to document:");
            println!("Original content: {}", current_content);
            
            for change in changes {
                println!("Processing change: {:?}", change);
                
                if change.removed {
                    // Skip the content that's being removed
                    last_position += change.value.chars().count();
                } else if !change.added && !change.removed {
                    // Copy unchanged content
                    let unchanged_len = change.value.chars().count();
                    if last_position + unchanged_len > chars.len() {
                        return Err(anyhow::anyhow!(
                            "Invalid change: position {} exceeds content length {}", 
                            last_position + unchanged_len, 
                            chars.len()
                        ));
                    }
                    
                    // Append the unchanged content
                    result.extend(chars[last_position..last_position + unchanged_len].iter());
                    last_position += unchanged_len;
                } else if change.added {
                    // Insert new content
                    result.push_str(&change.value);
                }
            }

            println!("Final content: {}", result);

            // Update cache with new content
            let metadata = tokio::fs::metadata(path).await?;
            let doc_metadata = DocumentMetadata {
                size: metadata.len(),
                is_directory: metadata.is_dir(),
                is_symlink: metadata.file_type().is_symlink(),
                created_at: metadata.created().ok().and_then(|t| 
                    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())),
                modified_at: metadata.modified().ok().and_then(|t| 
                    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())),
                readonly: metadata.permissions().readonly(),
                file_type: FileType::Text,
                encoding: FileEncoding {
                    encoding: "UTF-8".to_string(),
                    confidence: 1.0,
                },
                line_ending: self.detect_line_ending(&result),
            };

            self.cache_content(path.clone(), result, doc_metadata).await?;

            // Update state
            state.version += 1;
            state.is_dirty = true;
            state.last_modification = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            Ok(VersionedDocument {
                uri: path.clone(),
                version: state.version,
            })
        } else {
            Err(anyhow::anyhow!("Document not found in states"))
        }
    }

    pub async fn save_document(
        &self,
        doc: &VersionedDocument,
    ) -> Result<VersionedDocument> {
        let path = &doc.uri;
        let mut states = self.document_states.write().await;
        
        if let Some(state) = states.get_mut(path) {
            if state.version >= doc.version {
                return Err(anyhow::anyhow!(
                    "Version conflict: document has been modified. Server: {}, client: {}", 
                    state.version, doc.version
                ));
            }

            // Get content from cache
            let content = {
                let cache = self.cache.read().await;
                if let Some(cache_entry) = cache.get(path) {
                    cache_entry.content.clone()
                } else {
                    return Err(anyhow::anyhow!("Document content not found in cache"));
                }
            };

            // Write to file
            tokio::fs::write(&path, &content).await?;
            
            // Update state
            state.is_dirty = false;
            state.last_modification = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            Ok(VersionedDocument {
                uri: path.clone(),
                version: state.version,
            })
        } else {
            Err(anyhow::anyhow!("Document not found in states"))
        }
    }

    pub async fn get_document_content(&self, path: &PathBuf) -> Result<String> {
        // Try cache first
        {
            let cache = self.cache.read().await;
            if let Some(cache_entry) = cache.get(path) {
                return Ok(cache_entry.content.clone());
            }
        }

        // Not in cache, read from file
        let metadata = fs::metadata(path)
            .await
            .with_context(|| format!("Failed to read metadata for file: {:?}", path))?;
            
        if metadata.len() > MAX_FILE_SIZE {
            bail!("File is too large to load (size: {} bytes, max: {} bytes)", 
                  metadata.len(), MAX_FILE_SIZE);
        }

        // Detect file type before reading
        let file_type = self.detect_file_type(path).await?;
        match file_type {
            FileType::Binary => {
                bail!("Cannot read binary file as text: {:?}", path);
            }
            FileType::SymLink => {
                bail!("Cannot read symlink directly: {:?}", path);
            }
            _ => {}
        }

        // Read and decode content
        let content = fs::read(path)
            .await
            .with_context(|| format!("Failed to read file content: {:?}", path))?;

        // Detect encoding
        let encoding = self.detect_encoding(&content);
        
        // Convert to string using detected encoding
        let (content, _, had_errors) = Encoding::for_label(encoding.encoding.as_bytes())
            .unwrap_or(UTF_8)
            .decode(&content);
            
        if had_errors {
            println!("Warning: Some characters couldn't be decoded in file: {:?}", path);
        }

        let content = content.into_owned();

        // Cache the content with metadata
        let doc_metadata = DocumentMetadata {
            size: metadata.len(),
            is_directory: metadata.is_dir(),
            is_symlink: metadata.file_type().is_symlink(),
            created_at: metadata.created().ok().and_then(|t| 
                t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())),
            modified_at: metadata.modified().ok().and_then(|t| 
                t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())),
            readonly: metadata.permissions().readonly(),
            file_type,
            encoding,
            line_ending: self.detect_line_ending(&content),
        };

        // Cache if size is within limit
        if metadata.len() <= CACHE_SIZE_LIMIT {
            self.cache_content(path.clone(), content.clone(), doc_metadata.clone()).await?;
        }

        Ok(content)
    }

    // Get current content (useful for LSP operations)
    pub async fn open_file(&self, path: &PathBuf) -> Result<(String, DocumentMetadata, i32)> {
        // Check if document is already open
        let version = {
            let mut document_states = self.document_states.write().await;
            if let Some(state) = document_states.get(path) {
                state.version
            } else {
                // Initialize new document state
                let metadata = fs::metadata(path)
                    .await
                    .with_context(|| format!("Failed to read metadata for file: {:?}", path))?;

                document_states.insert(path.clone(), DocumentState {
                    version: 0,
                    is_open: true,
                    last_modification: metadata.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    is_dirty: false,
                });
                0
            }
        };

        // Get or read content
        let content = self.get_document_content(path).await?;

        // Get metadata from cache or create new
        let metadata = {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(path) {
                entry.metadata.clone()
            } else {
                // If not in cache, create metadata (shouldn't happen as get_document_content caches)
                let fs_metadata = fs::metadata(path).await?;
                let file_type = self.detect_file_type(path).await?;
                DocumentMetadata {
                    size: fs_metadata.len(),
                    is_directory: fs_metadata.is_dir(),
                    is_symlink: fs_metadata.file_type().is_symlink(),
                    created_at: fs_metadata.created()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                    modified_at: fs_metadata.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                    readonly: fs_metadata.permissions().readonly(),
                    file_type,
                    encoding: FileEncoding {
                        encoding: "UTF-8".to_string(),
                        confidence: 1.0,
                    },
                    line_ending: self.detect_line_ending(&content),
                }
            }
        };

        Ok((content, metadata, version))
    }

    // Cache management
    async fn cache_content(
        &self,
        path: PathBuf,
        content: String,
        metadata: DocumentMetadata
    ) -> Result<()> {
        let mut cache = self.cache.write().await;
        let mut cache_queue = self.cache_queue.write().await;
        let mut current_size = self.current_cache_size.write().await;

        // Evict old entries if necessary
        while *current_size + content.len() as u64 > self.max_cache_size {
            if let Some(old_path) = cache_queue.pop_front() {
                if let Some(old_entry) = cache.remove(&old_path) {
                    *current_size -= old_entry.content.len() as u64;
                }
            } else {
                break;
            }
        }

        // Add new entry
        cache.insert(path.clone(), CacheEntry {
            content,
            metadata,
            last_accessed: std::time::Instant::now(),
        });
        
        cache_queue.push_back(path);
        Ok(())
    }

    pub async fn invalidate_cache_for_file(&self, path: &PathBuf) {
        let mut cache = self.cache.write().await;
        if let Some(entry) = cache.remove(path) {
            *self.current_cache_size.write().await -= entry.content.len() as u64;
            self.cache_queue.write().await.retain(|p| p != path);
        }
    }

    pub async fn get_document_state(&self, path: &PathBuf) -> Result<DocumentState> {
        let states = self.document_states.read().await;
        states.get(path).cloned().ok_or_else(|| anyhow::anyhow!("Document state not found"))
    }

}