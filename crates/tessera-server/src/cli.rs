//! CLI definition and command implementations for `tesserad`.

use std::io::{IsTerminal, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use tessera_api::events::EventBus;
use tessera_api::{AppState, AppStateParts};
use tessera_core::config::Config;
use tessera_db::cas::CasStore;
use tessera_db::repos::embeddings;
use tessera_db::Db;
use tokio::task::JoinHandle;

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
    /// Run the HTTP API and the pipeline workers.
    Serve,
    /// Run the MCP server over stdio (for local AI agents).
    McpStdio,
    /// Apply database migrations and exit.
    Migrate,
    /// Print resolved config and check DB + CAS health.
    Doctor,
    /// Backfill entity embeddings and rebuild all global semantic correlation
    /// edges (run once after enabling correlation on an existing corpus).
    Recorrelate,
    /// Recluster all chunk embeddings with HDBSCAN (density-based; resists the
    /// mega-cluster that centroid drift produces on tightly-packed embeddings).
    Recluster,
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

    // Logs go to stderr so they never corrupt a stdout protocol channel (the
    // MCP stdio server writes JSON-RPC to stdout).
    let registry = tracing_subscriber::registry().with(filter);
    if config.log.format == "json" {
        registry
            .with(fmt::layer().json().with_writer(std::io::stderr))
            .init();
    } else {
        registry
            .with(fmt::layer().compact().with_writer(std::io::stderr))
            .init();
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
#[allow(clippy::too_many_lines)]
pub async fn serve(config: Config) -> Result<()> {
    let db = connect(&config).await?;
    db.migrate()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("applying migrations")?;
    ensure_cas_writable(&config).context("checking content store")?;

    let cas = CasStore::open(&config.cas.path).map_err(|e| anyhow!(e.to_string()))?;

    // Build the active embedding provider and register its space (+ HNSW index).
    let embedder = tessera_providers::build_embedder(&config.providers)
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("initializing embedding provider")?;
    let info = embedder.space().clone();
    let mut space = embeddings::ensure(
        &db.api,
        &info.name,
        &config.providers.embedder,
        i32::try_from(info.dim).unwrap_or(i32::MAX),
        info.metric,
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    embeddings::ensure_hnsw_index(&db.api, space.id, space.dim)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    embeddings::ensure_entity_hnsw_index(&db.api, space.id, space.dim)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    embeddings::set_active(&db.api, space.id)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    space.active = true;
    tracing::info!(space = %space.name, dim = space.dim, "embedding space active");

    let llm = tessera_providers::build_llm(&config.providers)
        .map_err(|e| anyhow!(e.to_string()))
        .context("initializing llm provider")?;

    let bind = config.server.bind;
    let workers = config.pipeline.workers;
    let db_url = config.database.url.clone();

    // Load extractor plugins (if any) before `config` is moved into the state.
    let plugins = std::sync::Arc::new(tessera_extract::plugin::PluginRegistry::load_from_dir(
        config.plugins.dir.as_deref(),
    ));

    // Pipeline context values (read before `config` is moved into the state).
    let pipeline_ctx = tessera_pipeline::PipelineContext {
        db: db.clone(),
        cas: cas.clone(),
        embedder: embedder.clone(),
        plugins,
        llm: llm.clone(),
        space_id: space.id,
        space_dim: space.dim,
        embed_batch: config.pipeline.embed_batch,
        cluster_max_distance: config.pipeline.cluster_max_distance,
        cluster_dirty_threshold: config.pipeline.cluster_dirty_threshold,
        synth_debounce_secs: config.pipeline.synth_debounce_secs,
        semantic_k: config.pipeline.semantic_k,
        semantic_min_sim: config.pipeline.semantic_min_sim,
        temporal_window_days: config.pipeline.temporal_window_days,
        temporal_tau_days: config.pipeline.temporal_tau_days,
        community_hub_degree: config.pipeline.community_hub_degree,
    };

    // Install the Prometheus recorder (global) and keep its render handle.
    let metrics = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| anyhow!("install metrics recorder: {e}"))?;

    // A source for documents ingested by agents over the HTTP MCP transport.
    let mcp_source = tessera_db::repos::sources::create(
        &db.api,
        "agent",
        "mcp-http",
        &serde_json::json!({ "transport": "http" }),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;

    let state = AppState::new(AppStateParts {
        db: db.clone(),
        config: Arc::new(config),
        cas: cas.clone(),
        embedder: embedder.clone(),
        llm,
        space: space.clone(),
        metrics,
        mcp_source_id: mcp_source.id,
    });

    // Forward Postgres NOTIFY progress into the SSE event bus.
    let forwarder = spawn_event_forwarder(db_url, state.events.clone());

    // Start the pipeline workers.
    let pipeline = tessera_pipeline::run_pipeline(pipeline_ctx, workers);

    let app = tessera_api::build_router(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, "tesserad listening");

    // Connect-info make service so handlers can see the client IP (login limit).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server error")?;

    tracing::info!("shutting down pipeline");
    pipeline.shutdown().await;
    forwarder.abort();
    tracing::info!("tesserad stopped");
    Ok(())
}

/// Spawn a task that keeps a Postgres LISTEN connection open and republishes
/// every NOTIFY payload to the in-process event bus, reconnecting on failure.
fn spawn_event_forwarder(url: String, events: EventBus) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match tessera_db::listen(&url, tessera_db::EVENTS_CHANNEL).await {
                Ok(mut listener) => loop {
                    match listener.recv().await {
                        Ok(note) => events.publish(note.payload().to_string()),
                        Err(e) => {
                            tracing::warn!(error = %e, "event listener dropped, reconnecting");
                            break;
                        }
                    }
                },
                Err(e) => tracing::warn!(error = %e, "event listener connect failed, retrying"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

/// Run the MCP server over stdio. Logs go to stderr; stdout is the JSON-RPC
/// channel. Connect a local agent with:
///   claude mcp add tessera -- tesserad mcp-stdio
pub async fn mcp_stdio(config: Config) -> Result<()> {
    let state = tessera_mcp::McpState::connect(&config)
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("initializing MCP state")?;
    tracing::info!("tessera MCP server ready on stdio");
    tessera_mcp::run_stdio(state)
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("MCP stdio server")
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

/// Recluster all chunk embeddings with HDBSCAN. Enqueues synthesis (picked up by
/// a running server's workers) for clusters whose membership changed.
pub async fn recluster(config: Config) -> Result<()> {
    let db = connect(&config).await?;
    let space = embeddings::active(&db.api)
        .await
        .map_err(|e| anyhow!(e.to_string()))?
        .ok_or_else(|| anyhow!("no active embedding space; run `serve` once first"))?;
    let r = tessera_pipeline::recluster::run(
        &db,
        space.id,
        config.pipeline.cluster_min_size,
        config.pipeline.synth_debounce_secs,
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    println!(
        "reclustered: {} clusters, {} noise chunks, {} changed",
        r.clusters, r.noise, r.changed
    );
    Ok(())
}

/// Backfill entity embeddings and rebuild every global semantic correlation edge.
/// Idempotent; run once after enabling correlation on an existing corpus.
pub async fn recorrelate(config: Config) -> Result<()> {
    let db = connect(&config).await?;
    let space = embeddings::active(&db.api)
        .await
        .map_err(|e| anyhow!(e.to_string()))?
        .ok_or_else(|| anyhow!("no active embedding space; run `serve` once first"))?;
    embeddings::ensure_entity_hnsw_index(&db.api, space.id, space.dim)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;

    let embedded = tessera_db::repos::entities::recompute_all_entity_embeddings(&db.api, space.id)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    let edges = tessera_db::repos::entities::rebuild_similar_edges(
        &db.api,
        space.id,
        space.dim,
        config.pipeline.semantic_k,
        config.pipeline.semantic_min_sim,
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    let day = 86_400.0;
    let temporal = tessera_db::repos::entities::rebuild_temporal_edges(
        &db.api,
        config.pipeline.temporal_window_days * day,
        config.pipeline.temporal_tau_days * day,
        config.pipeline.semantic_k,
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))?;
    let communities =
        tessera_db::repos::communities::detect(&db.api, config.pipeline.community_hub_degree)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
    println!(
        "recorrelated: {embedded} entity embeddings, {edges} semantic edges, {temporal} temporal edges, {communities} communities"
    );
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

    print!("  embedding provider     ... ");
    std::io::stdout().flush().ok();
    match tessera_providers::build_embedder(&config.providers).await {
        Ok(embedder) => {
            let space = embedder.space();
            println!("ok ({}, dim {})", space.name, space.dim);
        }
        Err(e) => println!("FAIL ({e})"),
    }

    print!("  llm provider           ... ");
    std::io::stdout().flush().ok();
    match tessera_providers::build_llm(&config.providers) {
        Ok(llm) => match llm.health().await {
            tessera_providers::ProviderHealth::Up => println!("ok"),
            tessera_providers::ProviderHealth::Degraded(r) => println!("degraded ({r})"),
            tessera_providers::ProviderHealth::Down(r) => println!("down ({r})"),
        },
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
