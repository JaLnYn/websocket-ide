# WIDE (websocket ide) 

A lightweight code server that lets you build custom websocket-based IDEs. Built with Rust for speed and reliability. Perfect for web-based coding environments, self-hosted solutions, or custom IDE implementations.

An example browser IDE is right here: [🍌 JaLnYn/browser-ide](https://github.com/JaLnYn/browser-ide)

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
| `Completion`       | `{ path: string, position: Position }`                              | Requests code completions at position.                                                                |
| `Hover`            | `{ path: string, position: Position }`                              | Requests hover information at position.                                                               |
| `Definition`       | `{ path: string, position: Position }`                              | Requests go-to-definition locations.                                                                  |

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

## Todo

- [ ] Debugger support
- [ ] More LSP features
- [ ] Testing
- [ ] Documentation improvements
- [ ] Better error handling
- [ ] Multi-root workspace support

## Contributing

I'm actively working on this project and welcome any contributions! Feel free to:

- Implementing new features
- Making things faster
- Adding tests
- Improving docs
- Making the code prettier
- Adding cool stuff we haven't thought of

## License

Coming soon! (MIT probably)

---

Built with 🦀 by someone who love coding on gpu servers.
