//! The `claude` CLI as a generation provider.
//!
//! The CLI is invoked as a pure text function: the prompt is written to stdin
//! (never argv, so it cannot leak into the process table), it runs in an empty
//! working directory with a minimal environment (only HOME and PATH, which the
//! CLI needs to find its own auth), and output is requested as JSON. Tools are
//! not granted; in headless `-p` mode any tool that would need permission simply
//! fails rather than executing. A semaphore bounds concurrency because the CLI
//! rides the user's subscription.
//!
//! All output is treated as untrusted data by the caller (schema-validated,
//! never executed).

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::{GenRequest, GenResponse, LlmProvider, ProviderError, ProviderHealth};

/// Generates text by shelling out to the `claude` CLI in headless mode.
pub struct ClaudeCliLlm {
    bin: String,
    model: Option<String>,
    timeout: Duration,
    semaphore: Arc<Semaphore>,
}

#[derive(Deserialize)]
struct CliJson {
    /// The `-p --output-format json` result payload.
    #[serde(default)]
    result: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    model: Option<String>,
}

impl ClaudeCliLlm {
    #[must_use]
    pub fn new(
        bin: &str,
        model: Option<String>,
        timeout_secs: u64,
        max_concurrency: usize,
    ) -> Self {
        Self {
            bin: bin.to_string(),
            model,
            timeout: Duration::from_secs(timeout_secs),
            semaphore: Arc::new(Semaphore::new(max_concurrency.max(1))),
        }
    }

    fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("-p").arg("--output-format").arg("json");
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        // Minimal environment: keep HOME (CLI auth/config) and PATH, drop
        // everything else (no DATABASE_URL or other app secrets reach the tool).
        cmd.env_clear();
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        // Run in a neutral directory so the tool has no project context to act on.
        cmd.current_dir(std::env::temp_dir());
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

#[async_trait]
impl LlmProvider for ClaudeCliLlm {
    fn id(&self) -> &'static str {
        "claude_cli"
    }

    async fn generate(&self, req: &GenRequest) -> Result<GenResponse, ProviderError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| ProviderError::Other(format!("semaphore closed: {e}")))?;

        // Compose the prompt; the CLI has no separate system-prompt flag in -p
        // mode, so a system instruction is prepended as a delimited preamble.
        let full_prompt = match &req.system {
            Some(system) => format!("{system}\n\n---\n\n{}", req.prompt),
            None => req.prompt.clone(),
        };

        let mut child = self
            .command()
            .spawn()
            .map_err(|e| ProviderError::Unavailable(format!("spawn {}: {e}", self.bin)))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(full_prompt.as_bytes())
                .await
                .map_err(|e| ProviderError::Other(format!("write stdin: {e}")))?;
            stdin.shutdown().await.ok();
        }

        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(ProviderError::Other(format!("cli wait: {e}"))),
            Err(_) => return Err(ProviderError::Timeout(self.timeout)),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProviderError::Other(format!(
                "claude cli exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        let parsed: CliJson = serde_json::from_slice(&output.stdout)
            .map_err(|e| ProviderError::InvalidOutput(format!("cli json: {e}")))?;
        if parsed.is_error {
            return Err(ProviderError::Other(format!(
                "claude cli reported error: {}",
                parsed.result
            )));
        }

        Ok(GenResponse {
            text: parsed.result,
            model: parsed.model.unwrap_or_else(|| "claude".to_string()),
        })
    }

    async fn health(&self) -> ProviderHealth {
        // A cheap version check confirms the binary is present and runnable.
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match tokio::time::timeout(Duration::from_secs(5), cmd.status()).await {
            Ok(Ok(s)) if s.success() => ProviderHealth::Up,
            Ok(Ok(s)) => ProviderHealth::Degraded(format!("claude --version exited {s}")),
            Ok(Err(e)) => ProviderHealth::Down(format!("claude not runnable: {e}")),
            Err(_) => ProviderHealth::Down("claude --version timed out".into()),
        }
    }
}
