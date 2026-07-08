//! SSRF-resistant URL fetching for URL ingestion.
//!
//! Server-side fetches are a classic SSRF surface: a caller could point the
//! server at `http://169.254.169.254/` (cloud metadata) or an internal service.
//! `UrlGuard` enforces: http(s) only; resolve the host and reject if ANY resolved
//! address is private, loopback, link-local, or in the tailnet CGNAT range;
//! re-validate on every redirect hop; and cap the response size and time.
//!
//! Residual risk (documented): there is a TOCTOU window between the DNS check and
//! the connection, since reqwest does not pin the connection to the checked IP.
//! A full fix would resolve-then-connect to the pinned address; that is a
//! follow-up. The private-range denial still blocks the common SSRF targets.

use std::net::IpAddr;
use std::time::Duration;

use futures::StreamExt;
use tessera_core::error::{Error, ErrorKind};

use crate::error::ApiError;

const MAX_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;

/// Result of a guarded fetch: the bytes and the server-reported content type.
pub struct Fetched {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub final_url: String,
}

fn invalid(msg: &str) -> ApiError {
    ApiError(Error::new(ErrorKind::Invalid, msg.to_string()))
}

/// True if connecting to this address could reach internal infrastructure.
fn is_disallowed(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // Tailnet / CGNAT 100.64.0.0/10.
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 0b0100_0000)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // Unique local fc00::/7.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link local fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped: check the embedded v4.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_disallowed(&IpAddr::V4(v4)))
        }
    }
}

/// Resolve `host:port` and reject if any resolved address is disallowed. Rejects
/// the whole name if any record is internal (defends against split-horizon and
/// DNS-rebinding multi-records).
async fn check_host(host: &str, port: u16) -> Result<(), ApiError> {
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| invalid(&format!("cannot resolve host: {e}")))?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_disallowed(&addr.ip()) {
            return Err(ApiError(Error::new(
                ErrorKind::Forbidden,
                "refusing to fetch a private, loopback, or link-local address",
            )));
        }
    }
    if !any {
        return Err(invalid("host did not resolve to any address"));
    }
    Ok(())
}

/// Validate a URL: http(s) scheme, and a host that resolves only to public IPs.
async fn validate(url_str: &str) -> Result<url::Url, ApiError> {
    let url = url::Url::parse(url_str).map_err(|_| invalid("invalid url"))?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(invalid("only http and https urls are allowed"));
    }
    let host = url.host_str().ok_or_else(|| invalid("url has no host"))?;
    // A literal IP host is checked directly; a name is resolved.
    let port = url.port_or_known_default().unwrap_or(80);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_disallowed(&ip) {
            return Err(ApiError(Error::new(
                ErrorKind::Forbidden,
                "refusing to fetch a private, loopback, or link-local address",
            )));
        }
    } else {
        check_host(host, port).await?;
    }
    Ok(url)
}

/// Fetch a URL with SSRF protection and size/time caps, following redirects
/// manually so each hop is re-validated.
pub async fn fetch(url_str: &str) -> Result<Fetched, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ApiError(Error::new(ErrorKind::Internal, format!("http client: {e}"))))?;

    let mut current = url_str.to_string();
    for _hop in 0..=MAX_REDIRECTS {
        let url = validate(&current).await?;
        let resp = client.get(url.clone()).send().await.map_err(|e| {
            ApiError(Error::new(
                ErrorKind::Provider,
                format!("fetch failed: {e}"),
            ))
        })?;

        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| invalid("redirect without a location"))?;
            // Resolve relative redirects against the current URL, then re-validate.
            current = url
                .join(location)
                .map_err(|_| invalid("bad redirect target"))?
                .to_string();
            continue;
        }
        if !status.is_success() {
            return Err(ApiError(Error::new(
                ErrorKind::Provider,
                format!("upstream returned {status}"),
            )));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string);

        // Stream with a hard byte cap so a huge or lying content-length cannot
        // exhaust memory.
        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ApiError(Error::new(ErrorKind::Provider, format!("read body: {e}")))
            })?;
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(ApiError(Error::new(
                    ErrorKind::TooLarge,
                    "response exceeds size limit",
                )));
            }
            bytes.extend_from_slice(&chunk);
        }

        return Ok(Fetched {
            bytes,
            content_type,
            final_url: current,
        });
    }
    Err(invalid("too many redirects"))
}

#[cfg(test)]
mod tests {
    use super::is_disallowed;
    use std::net::IpAddr;

    #[test]
    fn blocks_internal_addresses() {
        let block = [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // tailnet CGNAT
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
        ];
        for ip in block {
            assert!(
                is_disallowed(&ip.parse::<IpAddr>().unwrap()),
                "should block {ip}"
            );
        }
    }

    #[test]
    fn allows_public_addresses() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "185.220.101.44",
            "2606:4700:4700::1111",
        ] {
            assert!(
                !is_disallowed(&ip.parse::<IpAddr>().unwrap()),
                "should allow {ip}"
            );
        }
    }
}
