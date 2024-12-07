# WIDE (websocket ide)

A lightweight code server that lets you build custom websocket-based IDEs. Built with Rust for speed and reliability. Perfect for web-based coding environments, self-hosted solutions, or custom IDE implementations.

An example browser IDE is right here: [🍌 JaLnYn/browser-ide](https://github.com/JaLnYn/browser-ide)
![design](https://github.com/user-attachments/assets/004de091-e4c0-40fa-8101-96ceea281f49)


## Features

- ✨ File operations (read/write/watch)
- 🚀 Language Server Protocol support (completion, hover, go-to-def) (only rust for now)
- 🔄 Real-time WebSocket communication
- ⚡ Event batching for performance

## Quick Start

```bash
# Run the server
cargo run -- --workspace /your/code/path
```

### Test front-end

```
# clone the frontend test
git clone https://github.com/JaLnYn/browser-ide
cd browser-ide

# install dependencies and run front end
npm i
npm run dev
```

Note: if you want to test the lsp, you have to install rust-analyzer or it may error. Instructions [HERE](https://rust-analyzer.github.io/manual.html#rust-analyzer-language-server-binary)

```
# snippet from the site

mkdir -p ~/.local/bin
curl -L https://github.com/rust-lang/rust-analyzer/releases/latest/download/rust-analyzer-x86_64-unknown-linux-gnu.gz | gunzip -c - > ~/.local/bin/rust-analyzer
chmod +x ~/.local/bin/rust-analyzer
```

## WebSocket API

### Client Messages

| Type               | Content                                                             | Description                                                                                           |
| ------------------ | ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `OpenFile`         | `{ path: string }`                                                  | Opens a file and returns its content. Validates file existence and readability. Notifies LSP servers. |
| `CloseFile`        | `{ path: string }`                                                  | Closes an open file, cleans up resources, and notifies LSP servers.                                   |
| `GetDirectory`     | `{ path: string }`                                                  | Retrieves directory contents at the specified path.                                                   |
| `RefreshDirectory` | `{ path: string }`                                                  | Force refreshes directory contents, clearing cache.                                                   |
| `ChangeFile`       | `{ document: { uri: string, version: number }, changes: Change[] }` | Applies changes to file content. Validates document version.                                          |
| `SaveFile`         | `{ document: { uri: string, version: number } }`                    | Saves current file content to disk.                                                                   |
| `CreateFile`       | `{ path: string, is_directory: boolean }`                           | Creates a new file or directory at the specified path.                                                |
| `DeleteFile`       | `{ path: string }`                                                  | Deletes the file or directory at the specified path.                                                  |
| `RenameFile`       | `{ old_path: string, new_path: string }`                           | Renames/moves a file or directory from old_path to new_path.                                         |
| `Completion`       | `{ path: string, position: Position }`                              | Requests code completions at position.                                                                |
| `Hover`           | `{ path: string, position: Position }`                              | Requests hover information at position.                                                               |
| `Definition`       | `{ path: string, position: Position }`                              | Requests go-to-definition locations.                                                                  |
| `CreateTerminal`   | `{ cols: number, rows: number }`                                    | Creates a new terminal instance with specified dimensions.                                            |
| `ResizeTerminal`   | `{ id: string, cols: number, rows: number }`                        | Resizes an existing terminal.                                                                         |
| `WriteTerminal`    | `{ id: string, data: number[] }`                                    | Sends input data to terminal.                                                                         |
| `CloseTerminal`    | `{ id: string }`                                                    | Closes a terminal instance.                                                                           |
| `Search`           | `{ query: string, search_content: boolean }`                        | Initiates a search with optional content searching.                                                   |
| `CancelSearch`     | `{}`                                                                | Cancels an ongoing search operation.                                                                  |

### Server Messages

| Type                 | Content                                                                          | Description                   |
| -------------------- | -------------------------------------------------------------------------------- | ----------------------------- |
| `DirectoryContent`   | `{ path: string, content: FileNode[] }`                                          | Directory listing             |
| `DocumentContent`    | `{ path: string, content: string, metadata: DocumentMetadata, version: number }` | File content                  |
| `FileSystemEvents`   | `{ events: FileEvent[] }`                                                        | Real-time file system changes |
| `CompletionResponse` | `{ completions: CompletionList }`                                                | LSP completion items          |
| `HoverResponse`      | `{ hover: Hover }`                                                               | LSP hover information         |
| `DefinitionResponse` | `{ locations: Location[] }`                                                      | LSP definition locations      |
| `ChangeSuccess`      | `{ document: { version: number } }`                                              | Confirms file changes         |
| `SaveSuccess`        | `{ document: { version: number } }`                                              | Confirms file save            |
| `Error`              | `{ message: string }`                                                            | Error details                 |
| `Success`            | `{}`                                                                             | Generic success               |
| `TerminalCreated`    | `{ terminal_id: string }`                                                        | Confirms terminal creation    |
| `TerminalOutput`     | `{ terminal_id: string, data: number[] }`                                        | Terminal output data          |
| `TerminalClosed`     | `{ id: string }`                                                                 | Confirms terminal closure     |
| `TerminalError`      | `{ terminal_id: string, error: string }`                                         | Terminal error details        |
| `SearchResults`      | `{ search_id: string, items: SearchResultItem[], is_complete: boolean }`         | Search results batch          |

## Todo

- [ ] Debugger support
- [x] Search
- [x] Websocket based terminal
- [ ] More LSP features
- [ ] Testing
- [ ] Documentation improvements
- [ ] Better error handling
- [ ] Multi-root workspace support
- [ ] Clean up

## Contributing

I'm actively working on this project and welcome any contributions! Feel free to:

- Implementing new features
- Making things faster
- Adding tests
- Improving docs
- Making the code prettier
- Adding cool stuff we haven't thought of

---

Built with 🦀 by someone who love coding on gpu servers.
