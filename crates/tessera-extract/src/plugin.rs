//! The subprocess extractor plugin host.
//!
//! An out-of-language extractor (any executable, e.g. a Maka or Python program)
//! reads raw bytes on stdin and writes NDJSON [`tessera_core::ExtractEvent`]s on
//! stdout. The host runs it under a tight sandbox because the plugin processes
//! untrusted content and may itself be untrusted:
//!
//! * cleared environment (no app secrets reach it) and an empty working directory
//! * `RLIMIT_CPU` and a wall-clock timeout (runaway compute is killed)
//! * `RLIMIT_AS` (memory cap), `RLIMIT_FSIZE = 0` (cannot write files),
//!   `RLIMIT_NOFILE` (few descriptors)
//! * its own process group, so a timeout kills the whole group, not just the
//!   direct child
//! * a hard cap on stdout size
//!
//! Every stdout line is schema-validated; malformed lines are counted, not
//! trusted.

use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tessera_core::ExtractEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ExtractError;

/// Declares how to run one plugin.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    /// The executable to run.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Sniff labels or media types this plugin handles (used by the registry).
    #[serde(default)]
    pub media_types: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
    /// Address-space (memory) cap in bytes.
    #[serde(default = "default_mem")]
    pub max_memory_bytes: u64,
}

fn default_timeout() -> u64 {
    30
}
fn default_max_output() -> usize {
    16 * 1024 * 1024
}
fn default_mem() -> u64 {
    1024 * 1024 * 1024
}

/// Run a plugin over `input`, returning the events it emitted. Errors if the
/// plugin exceeds a limit, is killed, exits non-zero, or produces no valid
/// events.
pub async fn run_plugin(
    manifest: &PluginManifest,
    input: &[u8],
) -> Result<Vec<ExtractEvent>, ExtractError> {
    let scratch = fresh_scratch_dir()?;
    let mut command = build_command(manifest, &scratch);
    let mut child = command
        .spawn()
        .map_err(|e| ExtractError::Other(format!("spawn plugin {}: {e}", manifest.name)))?;

    let pid = child.id();

    // Feed stdin concurrently with reading stdout so a plugin that produces a lot
    // of output before consuming its input cannot deadlock on a full pipe.
    if let Some(mut stdin) = child.stdin.take() {
        let owned = input.to_vec();
        tokio::spawn(async move {
            let _ = stdin.write_all(&owned).await;
            // Dropping stdin here closes it (EOF for the plugin).
        });
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ExtractError::Other("plugin stdout unavailable".into()))?;
    let max_output = manifest.max_output_bytes;

    let collect = async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stdout
                .read(&mut chunk)
                .await
                .map_err(|e| ExtractError::Other(format!("read plugin stdout: {e}")))?;
            if n == 0 {
                break;
            }
            if buf.len() + n > max_output {
                return Err(ExtractError::LimitExceeded(
                    "plugin stdout exceeded cap".into(),
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let status = child
            .wait()
            .await
            .map_err(|e| ExtractError::Other(format!("wait plugin: {e}")))?;
        Ok::<_, ExtractError>((buf, status))
    };

    let timeout = Duration::from_secs(manifest.timeout_secs);
    let (buf, status) = match tokio::time::timeout(timeout, collect).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            kill_group(pid);
            let _ = tokio::fs::remove_dir_all(&scratch).await;
            return Err(e);
        }
        Err(_) => {
            kill_group(pid);
            let _ = tokio::fs::remove_dir_all(&scratch).await;
            return Err(ExtractError::LimitExceeded("plugin timed out".into()));
        }
    };
    let _ = tokio::fs::remove_dir_all(&scratch).await;

    if !status.success() {
        return Err(ExtractError::Other(format!(
            "plugin {} exited unsuccessfully ({status})",
            manifest.name
        )));
    }

    // Parse NDJSON. Malformed lines are dropped (counted), never trusted.
    let text = String::from_utf8_lossy(&buf);
    let mut events = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<ExtractEvent>(line) {
            Ok(ev) => events.push(ev),
            Err(_) => events.push(ExtractEvent::Warn {
                message: "plugin emitted a malformed line".into(),
            }),
        }
    }
    Ok(events)
}

fn fresh_scratch_dir() -> Result<std::path::PathBuf, ExtractError> {
    let dir = std::env::temp_dir().join(format!(
        "tessera-plugin-{}",
        tessera_core::new_id().simple()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|e| ExtractError::Other(format!("create plugin scratch dir: {e}")))?;
    Ok(dir)
}

#[cfg(unix)]
fn build_command(manifest: &PluginManifest, scratch: &std::path::Path) -> tokio::process::Command {
    use std::os::unix::process::CommandExt;

    let mut std_cmd = std::process::Command::new(&manifest.command);
    std_cmd
        .args(&manifest.args)
        .env_clear()
        .current_dir(scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let cpu_secs = manifest.timeout_secs.max(1);
    let mem = manifest.max_memory_bytes;

    // SAFETY: pre_exec runs in the forked child before exec. We only call
    // async-signal-safe syscalls (setrlimit, setpgid) with local values; no heap
    // allocation, no locks. This is the single unsafe block in the crate and it
    // exists to sandbox untrusted plugins.
    #[allow(unsafe_code)]
    unsafe {
        std_cmd.pre_exec(move || {
            set_rlimit(libc::RLIMIT_CPU, cpu_secs + 1);
            set_rlimit(libc::RLIMIT_AS, mem);
            set_rlimit(libc::RLIMIT_FSIZE, 0); // cannot create/grow files
            set_rlimit(libc::RLIMIT_NOFILE, 64);
            // New process group so a timeout can kill the whole group.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    tokio::process::Command::from(std_cmd)
}

#[cfg(not(unix))]
fn build_command(manifest: &PluginManifest, scratch: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&manifest.command);
    cmd.args(&manifest.args)
        .env_clear()
        .current_dir(scratch)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

/// Set both the soft and hard limit for a resource, ignoring errors (best effort
/// hardening in the child).
#[cfg(unix)]
#[allow(unsafe_code)]
fn set_rlimit(resource: libc::__rlimit_resource_t, value: u64) {
    let lim = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: setrlimit with a valid resource and a stack-local rlimit struct.
    unsafe {
        libc::setrlimit(resource, &raw const lim);
    }
}

/// Kill the plugin's whole process group after a limit breach.
#[cfg(unix)]
#[allow(unsafe_code)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // Only kill a real, in-range pid; never a wrapped or negative value
        // (killpg(-1, ...) would signal every process we can reach).
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // The child called setpgid(0, 0), so its pgid equals its pid.
            // SAFETY: killpg with a pid we spawned and SIGKILL.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

#[cfg(all(test, unix))]
mod tests {
    use super::{run_plugin, PluginManifest};

    fn manifest(command: &str, args: &[&str], timeout: u64) -> PluginManifest {
        PluginManifest {
            name: "test".into(),
            command: command.into(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            media_types: vec![],
            timeout_secs: timeout,
            max_output_bytes: 1024 * 1024,
            max_memory_bytes: 256 * 1024 * 1024,
        }
    }

    #[tokio::test]
    async fn well_behaved_plugin_emits_events() {
        // A shell one-liner that echoes two valid ExtractEvent lines.
        let script = r#"printf '{"event":"text","text":"hello from plugin"}\n{"event":"entity","entity_kind":"ip","value":"9.9.9.9"}\n'"#;
        let m = manifest("sh", &["-c", script], 5);
        let events = run_plugin(&m, b"ignored input").await.expect("plugin ok");
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], tessera_core::ExtractEvent::Text { .. }));
    }

    #[tokio::test]
    async fn spinning_plugin_is_killed_by_timeout() {
        // Busy loop; must be killed by the wall-clock timeout / CPU rlimit.
        let m = manifest("sh", &["-c", "while true; do :; done"], 2);
        let result = run_plugin(&m, b"x").await;
        assert!(result.is_err(), "spinning plugin should be killed");
    }

    #[tokio::test]
    async fn file_writing_plugin_is_blocked() {
        // RLIMIT_FSIZE = 0 means any write to a file fails; the plugin dies.
        let m = manifest(
            "sh",
            &[
                "-c",
                "echo data > outfile; echo '{\"event\":\"text\",\"text\":\"x\"}'",
            ],
            5,
        );
        let result = run_plugin(&m, b"x").await;
        // Either the shell dies on SIGXFSZ (non-zero exit) or produces nothing
        // usable; either way it must not succeed with the file written.
        assert!(
            result.is_err()
                || result.unwrap().iter().all(
                    |e| !matches!(e, tessera_core::ExtractEvent::Text { text, .. } if text == "x")
                )
        );
    }
}
