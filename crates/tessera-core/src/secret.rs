//! Safe-by-construction secret primitives (house rule: build the recurring
//! security concern once, then reuse it everywhere).
//!
//! Two families of secret exist in tessera:
//!
//! * High-entropy bearer secrets (API tokens, session cookies): 256 bits of
//!   randomness, so brute force is infeasible regardless of hash speed. These
//!   are stored as `blake3(secret)` and verified by a fast indexed lookup plus
//!   a constant-time comparison. Using a slow password hash here would buy
//!   nothing and make every request expensive.
//! * The single user password: low-entropy, human-chosen, so it MUST use a slow
//!   memory-hard hash. That is argon2id with OWASP-recommended parameters.
//!
//! All secret-bearing types carry a redacted [`std::fmt::Debug`] and zeroize
//! their material on drop, so a stray `{:?}` or a leftover buffer can never
//! print or linger with a live secret.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Error, ErrorKind};

/// Printable prefix identifying an API token: 4 random bytes as 8 hex chars.
/// Stored in plaintext so a presented token can be looked up in O(1) before the
/// constant-time hash comparison.
const TOKEN_PREFIX_BYTES: usize = 4;
/// Secret length for both API tokens and sessions: 256 bits.
const SECRET_BYTES: usize = 32;
/// The user-facing scheme marker; anchors parsing and namespaces our tokens.
const TOKEN_SCHEME: &str = "tessera";

fn fill_random(buf: &mut [u8]) {
    // `rand::rng()` is a ChaCha-based CSPRNG seeded from the OS and periodically
    // reseeded; correct for generating unguessable secrets.
    rand::rng().fill_bytes(buf);
}

/// A 32-byte secret value that zeroizes on drop and never prints its contents.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Secret32([u8; SECRET_BYTES]);

impl Secret32 {
    fn random() -> Self {
        let mut b = [0u8; SECRET_BYTES];
        fill_random(&mut b);
        Self(b)
    }

    /// The at-rest form: `blake3(secret)`, 32 bytes, safe to store in the DB.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        blake3::hash(&self.0).as_bytes().to_vec()
    }
}

impl std::fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret32(<redacted>)")
    }
}

/// A freshly minted API token: the plaintext to show the user exactly once, plus
/// the values to persist (`prefix` for lookup, `hash` for verification).
pub struct NewApiToken {
    /// The full token string to display once: `tessera_<prefix>_<secret>`.
    pub plaintext: String,
    /// The 8-char hex prefix, stored plaintext and uniquely indexed.
    pub prefix: String,
    /// `blake3(secret)`, the only token material stored at rest.
    pub hash: Vec<u8>,
}

/// Generate a new API token. The plaintext is returned to the caller and must
/// never be persisted; only `prefix` and `hash` go to the database.
#[must_use]
pub fn generate_api_token() -> NewApiToken {
    let mut prefix_bytes = [0u8; TOKEN_PREFIX_BYTES];
    fill_random(&mut prefix_bytes);
    let prefix = hex_lower(&prefix_bytes);

    let secret = Secret32::random();
    let secret_b64 = URL_SAFE_NO_PAD.encode(secret.0);
    let plaintext = format!("{TOKEN_SCHEME}_{prefix}_{secret_b64}");
    let hash = secret.hash();

    NewApiToken {
        plaintext,
        prefix,
        hash,
    }
}

/// The parsed components of a presented API token, ready for a DB lookup.
pub struct PresentedToken {
    /// The 8-char prefix to look up.
    pub prefix: String,
    /// `blake3(secret)` of the presented secret, to constant-time compare with
    /// the stored hash.
    pub presented_hash: Vec<u8>,
}

/// Parse a bearer token string into its lookup prefix and hashed secret.
///
/// The secret is base64url which can itself contain `_`, so we split into at
/// most three parts and keep the remainder whole.
pub fn parse_api_token(token: &str) -> Result<PresentedToken, Error> {
    let mut parts = token.splitn(3, '_');
    let scheme = parts.next().unwrap_or_default();
    let prefix = parts.next().unwrap_or_default();
    let secret_b64 = parts.next().unwrap_or_default();

    if scheme != TOKEN_SCHEME
        || prefix.len() != TOKEN_PREFIX_BYTES * 2
        || !prefix.bytes().all(|b| b.is_ascii_hexdigit())
        || secret_b64.is_empty()
    {
        return Err(Error::new(ErrorKind::Unauthorized, "malformed api token"));
    }

    let secret_bytes = URL_SAFE_NO_PAD
        .decode(secret_b64)
        .map_err(|_| Error::new(ErrorKind::Unauthorized, "malformed api token secret"))?;
    if secret_bytes.len() != SECRET_BYTES {
        return Err(Error::new(
            ErrorKind::Unauthorized,
            "malformed api token secret",
        ));
    }

    let presented_hash = blake3::hash(&secret_bytes).as_bytes().to_vec();
    Ok(PresentedToken {
        prefix: prefix.to_string(),
        presented_hash,
    })
}

/// Constant-time comparison of two hashes. Both sides are non-secret hashes of
/// high-entropy input, but we still compare in constant time to avoid handing an
/// attacker any timing signal.
#[must_use]
pub fn hashes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// A freshly minted session secret: the cookie value plus its at-rest hash.
pub struct NewSession {
    /// The opaque cookie value to send to the browser (base64url, 43 chars).
    pub cookie_value: String,
    /// `blake3(secret)`, the only session material stored at rest.
    pub hash: Vec<u8>,
}

/// Generate a new session secret for a logged-in user.
#[must_use]
pub fn generate_session() -> NewSession {
    let secret = Secret32::random();
    NewSession {
        cookie_value: URL_SAFE_NO_PAD.encode(secret.0),
        hash: secret.hash(),
    }
}

/// Hash a presented session cookie value for lookup, or reject if malformed.
pub fn hash_session_cookie(cookie_value: &str) -> Result<Vec<u8>, Error> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cookie_value)
        .map_err(|_| Error::new(ErrorKind::Unauthorized, "malformed session"))?;
    if bytes.len() != SECRET_BYTES {
        return Err(Error::new(ErrorKind::Unauthorized, "malformed session"));
    }
    Ok(blake3::hash(&bytes).as_bytes().to_vec())
}

/// argon2id tuned to OWASP guidance (m=19456 KiB, t=2, p=1).
fn argon2() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("static argon2 params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hash the single user password to a PHC string for storage.
pub fn hash_password(password: &str) -> Result<String, Error> {
    // Draw the salt from our own CSPRNG (rand's ChaCha-based thread rng) and
    // encode it, rather than argon2's optional OsRng feature.
    let mut salt_bytes = [0u8; 16];
    fill_random(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| Error::internal(format!("salt encode failed: {e}")))?;
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::internal(format!("password hash failed: {e}")))
}

/// Verify a candidate password against a stored PHC hash. Returns `false` (never
/// an error) for a wrong password; errors only on a corrupt stored hash.
pub fn verify_password(password: &str, phc: &str) -> Result<bool, Error> {
    let parsed = PasswordHash::new(phc)
        .map_err(|e| Error::internal(format!("corrupt password hash: {e}")))?;
    Ok(argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrips_and_verifies() {
        let t = generate_api_token();
        assert!(t.plaintext.starts_with("tessera_"));
        assert_eq!(t.prefix.len(), 8);
        assert_eq!(t.hash.len(), 32);

        let presented = parse_api_token(&t.plaintext).expect("valid token parses");
        assert_eq!(presented.prefix, t.prefix);
        assert!(hashes_equal(&presented.presented_hash, &t.hash));
    }

    #[test]
    fn tampered_token_secret_does_not_verify() {
        let t = generate_api_token();
        // Flip the last character of the secret; must not match the stored hash.
        let mut bad = t.plaintext.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == 'A' { 'B' } else { 'A' });
        let presented = parse_api_token(&bad).expect("still well-formed");
        assert!(!hashes_equal(&presented.presented_hash, &t.hash));
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        assert!(parse_api_token("").is_err());
        assert!(parse_api_token("tessera_").is_err());
        assert!(parse_api_token("tessera_zzzzzzzz_secret").is_err()); // non-hex prefix
        assert!(parse_api_token("other_ab12cd34_secret").is_err()); // wrong scheme
        assert!(parse_api_token("tessera_ab12cd34_!!!").is_err()); // bad base64
    }

    #[test]
    fn session_roundtrips() {
        let s = generate_session();
        let looked_up = hash_session_cookie(&s.cookie_value).expect("valid cookie hashes");
        assert!(hashes_equal(&looked_up, &s.hash));
        assert!(hash_session_cookie("not-base64!!").is_err());
    }

    #[test]
    fn password_hash_verifies_and_rejects() {
        let phc = hash_password("correct horse battery staple").expect("hash ok");
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &phc).unwrap());
        assert!(!verify_password("wrong password", &phc).unwrap());
    }

    #[test]
    fn hashes_equal_rejects_length_mismatch() {
        assert!(!hashes_equal(&[1, 2, 3], &[1, 2]));
        assert!(hashes_equal(&[1, 2, 3], &[1, 2, 3]));
    }
}
