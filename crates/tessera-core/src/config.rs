//! Layered configuration: built-in defaults, then `tessera.toml`, then
//! `TESSERA__*` environment overrides, with the database URL taken from the
//! conventional `DATABASE_URL` secret env var.
//!
//! Secrets (the DB URL, the initial admin password) come from the environment,
//! never a committed file.

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind};

/// Root configuration for `tesserad`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub cas: CasConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address the HTTP server binds to. Default is loopback; production sets
    /// this to the host's tailnet IP via env so the port is never public.
    pub bind: SocketAddr,
    /// Public base URL used when emitting absolute links (optional).
    #[serde(default)]
    pub public_url: Option<String>,
    /// Set the `Secure` attribute on session cookies. Default false so local
    /// HTTP development works; production (behind Caddy TLS) sets this true.
    #[serde(default)]
    pub secure_cookies: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Postgres connection URL. Sourced from `DATABASE_URL`; the placeholder
    /// default fails fast if neither env nor file provides a real one.
    pub url: String,
    /// Max connections for the interactive (API) pool.
    #[serde(default = "default_api_conns")]
    pub max_connections: u32,
    /// Max connections for the background worker pool (kept separate so bulk
    /// pipeline work can never starve interactive queries).
    #[serde(default = "default_worker_conns")]
    pub worker_max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasConfig {
    /// Filesystem root of the content-addressed store for raw ingested bytes.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// `json` for production, `pretty` for local development.
    #[serde(default = "default_log_format")]
    pub format: String,
    /// A `tracing_subscriber` env-filter directive.
    #[serde(default = "default_log_filter")]
    pub filter: String,
}

fn default_api_conns() -> u32 {
    10
}
fn default_worker_conns() -> u32 {
    6
}
fn default_log_format() -> String {
    "pretty".to_string()
}
fn default_log_filter() -> String {
    "info,tessera=debug,sqlx=warn".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                bind: "127.0.0.1:8080".parse().expect("static addr"),
                public_url: None,
                secure_cookies: false,
            },
            database: DatabaseConfig {
                url: "postgres://tessera:tessera@127.0.0.1:5432/tessera".to_string(),
                max_connections: default_api_conns(),
                worker_max_connections: default_worker_conns(),
            },
            cas: CasConfig {
                path: PathBuf::from("./data/cas"),
            },
            log: LogConfig {
                format: default_log_format(),
                filter: default_log_filter(),
            },
        }
    }
}

impl Config {
    /// Load configuration from defaults, an optional TOML file, and the
    /// environment. `DATABASE_URL`, if set, always wins for the DB URL.
    pub fn load(toml_path: Option<&std::path::Path>) -> Result<Self, Error> {
        let mut fig = Figment::from(Serialized::defaults(Config::default()));
        if let Some(path) = toml_path {
            fig = fig.merge(Toml::file(path));
        } else {
            // Conventionally look for a tessera.toml in the working directory.
            fig = fig.merge(Toml::file("tessera.toml"));
        }
        fig = fig.merge(Env::prefixed("TESSERA__").split("__"));

        let mut config: Config = fig
            .extract()
            .map_err(|e| Error::new(ErrorKind::Invalid, format!("invalid configuration: {e}")))?;

        if let Ok(url) = std::env::var("DATABASE_URL") {
            if !url.is_empty() {
                config.database.url = url;
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn defaults_are_valid() {
        let c = Config::default();
        assert_eq!(c.server.bind.port(), 8080);
        assert_eq!(c.database.max_connections, 10);
        assert_eq!(c.log.format, "pretty");
    }
}
