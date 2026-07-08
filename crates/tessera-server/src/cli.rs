//! CLI definition and command implementations for `tesserad`.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use tessera_api::AppState;
use tessera_core::config::Config;
use tessera_db::Db;

/// tessera daemon and operator CLI.
#[derive(Debug, Parser)]
#[command(name = "tesserad", version, about)]
pub struct Cli {
    /// Path to a tessera.toml config file (otherwise ./tessera.toml + env).
    #[arg(long, short, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP API (and, in later milestones, workers + MCP).
    Serve,
    /// Apply database migrations and exit.
    Migrate,
    /// Print resolved config and check DB + CAS health.
    Doctor,
    /// Manage API tokens.
    #[command(subcommand)]
    Token(TokenCmd),
    /// Manage users.
    #[command(subcommand)]
    User(UserCmd),
}

#[derive(Debug, Subcommand)]
pub enum TokenCmd {
    /// Create a new API token and print it once.
    New(TokenNewArgs),
    /// List a user's tokens.
    List(UserRef),
    /// Revoke a token by id.
    Revoke(TokenRevokeArgs),
}

#[derive(Debug, Args)]
pub struct TokenNewArgs {
    /// Owning username.
    #[arg(long)]
    pub user: String,
    /// A human label for the token.
    #[arg(long)]
    pub name: String,
    /// Comma-separated scopes (read, ingest, mcp, admin).
    #[arg(long, value_delimiter = ',', default_value = "read")]
    pub scopes: Vec<String>,
}

#[derive(Debug, Args)]
pub struct TokenRevokeArgs {
    /// Owning username.
    #[arg(long)]
    pub user: String,
    /// Token id to revoke.
    #[arg(long)]
    pub id: uuid::Uuid,
}

#[derive(Debug, Args)]
pub struct UserRef {
    #[arg(long)]
    pub user: String,
}

#[derive(Debug, Subcommand)]
pub enum UserCmd {
    /// Create a user. Password read from `TESSERA_ADMIN_PASSWORD` or stdin.
    Create(UserRef),
    /// Set a user's password. Password read from `TESSERA_ADMIN_PASSWORD` or stdin.
    SetPassword(UserRef),
}

/// Initialize the tracing subscriber from config (`RUST_LOG` overrides).
pub fn init_tracing(config: &Config) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.log.filter.clone()));

    let registry = tracing_subscriber::registry().with(filter);
    if config.log.format == "json" {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer().compact()).init();
    }
}

async fn connect(config: &Config) -> Result<Db> {
    Db::connect(
        &config.database.url,
        config.database.max_connections,
        config.database.worker_max_connections,
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))
    .context("connecting to Postgres")
}

/// Run the service. Applies migrations (idempotent, advisory-locked) then serves.
pub async fn serve(config: Config) -> Result<()> {
    let db = connect(&config).await?;
    db.migrate()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("applying migrations")?;

    // Ensure the CAS root exists and is writable before accepting traffic.
    ensure_cas_writable(&config).context("checking content store")?;

    let bind = config.server.bind;
    let state = AppState::new(db, Arc::new(config));
    let app = tessera_api::build_router(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "tesserad listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    tracing::info!("tesserad stopped");
    Ok(())
}

/// Apply migrations and exit (the one-shot compose `migrate` service).
pub async fn migrate(config: Config) -> Result<()> {
    let db = connect(&config).await?;
    db.migrate()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("applying migrations")?;
    println!("migrations applied");
    Ok(())
}

/// Print resolved config (redacted) and probe dependencies.
pub async fn doctor(config: Config) -> Result<()> {
    println!("resolved configuration:");
    println!("  server.bind            = {}", config.server.bind);
    println!(
        "  server.secure_cookies  = {}",
        config.server.secure_cookies
    );
    println!(
        "  database.url           = {}",
        redact_url(&config.database.url)
    );
    println!(
        "  database.max_conns     = {}",
        config.database.max_connections
    );
    println!("  cas.path               = {}", config.cas.path.display());
    println!("  log.format             = {}", config.log.format);

    print!("  postgres connectivity  ... ");
    std::io::stdout().flush().ok();
    match connect(&config).await {
        Ok(db) => match db.ping().await {
            Ok(()) => println!("ok"),
            Err(e) => println!("FAIL ({e})"),
        },
        Err(e) => println!("FAIL ({e})"),
    }

    print!("  content store writable ... ");
    std::io::stdout().flush().ok();
    match ensure_cas_writable(&config) {
        Ok(()) => println!("ok"),
        Err(e) => println!("FAIL ({e})"),
    }
    Ok(())
}

/// Token subcommands.
pub async fn token(config: Config, cmd: TokenCmd) -> Result<()> {
    let db = connect(&config).await?;
    match cmd {
        TokenCmd::New(args) => {
            // Validate scopes before touching the DB.
            for s in &args.scopes {
                if tessera_api::auth::Scope::parse(s).is_none() {
                    bail!("unknown scope: {s} (valid: read, ingest, mcp, admin)");
                }
            }
            let user = tessera_db::repos::users::by_username(&db.api, &args.user)
                .await
                .map_err(|e| anyhow!(e.to_string()))?
                .ok_or_else(|| {
                    anyhow!(
                        "no such user '{}'; create it with `tesserad user create`",
                        args.user
                    )
                })?;

            let minted = tessera_core::secret::generate_api_token();
            let record = tessera_db::repos::api_tokens::create(
                &db.api,
                user.id,
                &args.name,
                &minted.prefix,
                &minted.hash,
                &args.scopes,
                None,
            )
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

            println!("created token {} ({})", record.id, args.name);
            println!("scopes: {}", args.scopes.join(", "));
            println!();
            println!("{}", minted.plaintext);
            println!();
            println!("This is the only time the token is shown. Store it now.");
        }
        TokenCmd::List(args) => {
            let user = tessera_db::repos::users::by_username(&db.api, &args.user)
                .await
                .map_err(|e| anyhow!(e.to_string()))?
                .ok_or_else(|| anyhow!("no such user '{}'", args.user))?;
            let rows = tessera_db::repos::api_tokens::list(&db.api, user.id)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            if rows.is_empty() {
                println!("(no tokens)");
            }
            for t in rows {
                let state = if t.revoked_at.is_some() {
                    "revoked"
                } else {
                    "active"
                };
                println!(
                    "{}  {:<20}  [{}]  {}  {}",
                    t.id,
                    t.name,
                    t.scopes.join(","),
                    state,
                    t.prefix
                );
            }
        }
        TokenCmd::Revoke(args) => {
            let user = tessera_db::repos::users::by_username(&db.api, &args.user)
                .await
                .map_err(|e| anyhow!(e.to_string()))?
                .ok_or_else(|| anyhow!("no such user '{}'", args.user))?;
            let did = tessera_db::repos::api_tokens::revoke(&db.api, user.id, args.id)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            if did {
                println!("revoked {}", args.id);
            } else {
                bail!("no active token {} for user {}", args.id, args.user);
            }
        }
    }
    Ok(())
}

/// User subcommands.
pub async fn user(config: Config, cmd: UserCmd) -> Result<()> {
    let db = connect(&config).await?;
    match cmd {
        UserCmd::Create(args) => {
            let password = read_password()?;
            let hash = tessera_core::secret::hash_password(&password)
                .map_err(|e| anyhow!(e.to_string()))?;
            let user = tessera_db::repos::users::create(&db.api, &args.user, &hash)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            println!("created user {} ({})", user.username, user.id);
        }
        UserCmd::SetPassword(args) => {
            let password = read_password()?;
            let hash = tessera_core::secret::hash_password(&password)
                .map_err(|e| anyhow!(e.to_string()))?;
            tessera_db::repos::users::set_password(&db.api, &args.user, &hash)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            println!("password updated for {}", args.user);
        }
    }
    Ok(())
}

/// Read a password from `TESSERA_ADMIN_PASSWORD` or, if unset, a single stdin
/// line. Never taken from a CLI flag (which would leak into the process table).
fn read_password() -> Result<String> {
    if let Ok(p) = std::env::var("TESSERA_ADMIN_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    if std::io::stdin().is_terminal() {
        print!("password: ");
        std::io::stdout().flush().ok();
    }
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading password")?;
    let password = line.trim_end_matches(['\n', '\r']).to_string();
    if password.is_empty() {
        bail!("empty password");
    }
    Ok(password)
}

fn ensure_cas_writable(config: &Config) -> Result<()> {
    let root = &config.cas.path;
    std::fs::create_dir_all(root)
        .with_context(|| format!("creating CAS root {}", root.display()))?;
    let probe = root.join(".tessera-write-probe");
    std::fs::write(&probe, b"ok").with_context(|| format!("writing under {}", root.display()))?;
    std::fs::remove_file(&probe).ok();
    Ok(())
}

/// Redact credentials from a Postgres URL for display.
fn redact_url(url: &str) -> String {
    // postgres://user:pass@host/db -> postgres://user:***@host/db
    if let Some((scheme, rest)) = url.split_once("://") {
        if let Some((creds, tail)) = rest.split_once('@') {
            if let Some((user, _pass)) = creds.split_once(':') {
                return format!("{scheme}://{user}:***@{tail}");
            }
        }
    }
    url.to_string()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
