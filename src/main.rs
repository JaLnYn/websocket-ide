// src/main.rs
mod server;
mod file_system;
mod lsp;
mod utils;
mod terminal;
mod search;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    workspace: String,
    
    #[arg(short, long, default_value = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let workspace_path = PathBuf::from(args.workspace);
    
    let server = server::Server::new(workspace_path, args.port)?;
    server.start().await
}
