//! `tesserad`: the tessera daemon and operator CLI.
//!
//! One binary runs the service (`serve`) and the operational commands
//! (`migrate`, `token`, `user`, `doctor`). The pipeline workers and MCP server
//! run inside `serve` alongside the HTTP API (added in later milestones).

mod cli;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = tessera_core::config::Config::load(cli.config.as_deref())
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    cli::init_tracing(&config);

    match cli.command {
        Command::Serve => cli::serve(config).await,
        Command::McpStdio => cli::mcp_stdio(config).await,
        Command::Migrate => cli::migrate(config).await,
        Command::Doctor => cli::doctor(config).await,
        Command::Recorrelate => cli::recorrelate(config).await,
        Command::Token(cmd) => cli::token(config, cmd).await,
        Command::User(cmd) => cli::user(config, cmd).await,
    }
}
