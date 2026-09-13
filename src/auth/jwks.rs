#![forbid(unsafe_code)]

//! A tiny, bounded JWKS cache for the federated (Supabase / Neon) JWT path.
//!
//! JWKS documents are parsed from raw JSON into `jsonwebtoken` decoding keys
//! built from their public components, so this module never has to agree with
//! `jsonwebtoken`'s own JWK model — only with RFC 7517 field names.
//!
//! The cache is keyed by JWKS URL, holds one entry per URL, and refetches when
//! the entry is older than `GHA_INDIE_WORKER_JWKS_TTL_SECONDS`. A `kid` that is
//! not in the cached set forces exactly one refetch, so key rotation is picked
//! up without a restart and without turning every request into a fetch.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::session::{Actor, ActorSource};

#[derive(Debug, Error)]
pub enum JwksError {
    #[error("the token header is malformed")]
    MalformedToken,
    #[error("no key in the set matches the token's kid")]
    UnknownKey,
    #[error("the signature or claims did not validate")]
    Invalid,
    #[error("the JWKS endpoint could not be reached")]
    Unavailable,
}

/// Claims this server cares about. Everything else in the token is ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct FederatedClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub user_roles: Vec<String>,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub acr: Option<String>,
}

impl FederatedClaims {
    /// Supabase puts a single `role` claim on the token; Neon uses `user_roles`.
    #[must_use]
    pub fn roles(&self) -> Vec<String> {
        let mut roles = self.user_roles.clone();
        if let Some(role) = &self.role {
            if !roles.iter().any(|existing| existing == role) {
                roles.push(role.clone());
            }
        }
        roles
    }

    #[must_use]
    pub fn into_actor(self, token: &str) -> Actor {
        let roles = self.roles();
        Actor {
            sub: self.sub,
            email: self.email,
            org_id: self.org_id,
            roles,
            scopes: self
                .scope
                .as_deref()
                .map(|scope| scope.split_ascii_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
            acr: self.acr,
            source: ActorSource::FederatedJwt,
            access_token: token.to_owned(),
        }
    }
}

struct CachedSet {
    fetched_at: Instant,
    keys: HashMap<String, Arc<KeyEntry>>,
}

struct KeyEntry {
    key: DecodingKey,
    algorithm: Algorithm,
}

pub struct JwksCache {
    http: reqwest::Client,
    ttl: Duration,
    sets: RwLock<HashMap<String, CachedSet>>,
}

impl JwksCache {
    #[must_use]
    pub fn new(http: reqwest::Client, ttl_seconds: u64) -> Self {
        Self {
            http,
            ttl: Duration::from_secs(ttl_seconds.max(30)),
            sets: RwLock::new(HashMap::new()),
        }
    }

    /// Verifies `token` against the key set at `url`.
    pub async fn verify(
        &self,
        url: &str,
        token: &str,
        issuers: &[String],
        audiences: &[String],
    ) -> Result<FederatedClaims, JwksError> {
        let header = decode_header(token).map_err(|_| JwksError::MalformedToken)?;
        let kid = header.kid.clone().unwrap_or_default();

        if let Some(entry) = self.lookup(url, &kid).await {
            if let Ok(claims) = decode_with(token, &entry, header.alg, issuers, audiences) {
                return Ok(claims);
            }
        }
        // Either the key is unknown or the cached copy no longer verifies: one
        // refetch, then a final attempt. Never more than one fetch per request.
        self.refresh(url).await?;
        let entry = self.lookup(url, &kid).await.ok_or(JwksError::UnknownKey)?;
        decode_with(token, &entry, header.alg, issuers, audiences)
    }

    async fn lookup(&self, url: &str, kid: &str) -> Option<Arc<KeyEntry>> {
        let sets = self.sets.read().await;
        let set = sets.get(url)?;
        if set.fetched_at.elapsed() > self.ttl {
            return None;
        }
        set.keys
            .get(kid)
            // A set with exactly one key is usable even when the token omits kid.
            .or_else(|| (set.keys.len() == 1).then(|| set.keys.values().next()).flatten())
            .cloned()
    }

    async fn refresh(&self, url: &str) -> Result<(), JwksError> {
        let response = self.http.get(url).send().await.map_err(|_| JwksError::Unavailable)?;
        if !response.status().is_success() {
            return Err(JwksError::Unavailable);
        }
        let document: serde_json::Value = response.json().await.map_err(|_| JwksError::Unavailable)?;
        let keys = parse_key_set(&document);
        if keys.is_empty() {
            return Err(JwksError::Unavailable);
        }
        let mut sets = self.sets.write().await;
        sets.insert(
            url.to_owned(),
            CachedSet {
                fetched_at: Instant::now(),
                keys,
            },
        );
        Ok(())
    }
}

fn decode_with(
    token: &str,
    entry: &KeyEntry,
    header_algorithm: Algorithm,
    issuers: &[String],
    audiences: &[String],
) -> Result<FederatedClaims, JwksError> {
    // The key's declared algorithm wins; a token may not choose its own.
    if header_algorithm != entry.algorithm {
        return Err(JwksError::Invalid);
    }
    let mut validation = Validation::new(entry.algorithm);
    validation.leeway = 30;
    if issuers.is_empty() {
        validation.iss = None;
    } else {
        validation.set_issuer(issuers);
    }
    if audiences.is_empty() {
        validation.validate_aud = false;
    } else {
        validation.set_audience(audiences);
    }
    decode::<FederatedClaims>(token, &entry.key, &validation)
        .map(|data| data.claims)
        .map_err(|_| JwksError::Invalid)
}

/// RFC 7517 key set → decoding keys. Unsupported key types are skipped rather
/// than failing the whole document, so one exotic key cannot lock everyone out.
fn parse_key_set(document: &serde_json::Value) -> HashMap<String, Arc<KeyEntry>> {
    let mut keys = HashMap::new();
    let Some(entries) = document.get("keys").and_then(serde_json::Value::as_array) else {
        return keys;
    };
    for entry in entries {
        let kid = entry
            .get("kid")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let algorithm = entry
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .and_then(algorithm_from_name);
        let key = match entry.get("kty").and_then(serde_json::Value::as_str) {
            Some("RSA") => {
                let modulus = entry.get("n").and_then(serde_json::Value::as_str);
                let exponent = entry.get("e").and_then(serde_json::Value::as_str);
                match (modulus, exponent) {
                    (Some(n), Some(e)) => DecodingKey::from_rsa_components(n, e)
                        .ok()
                        .map(|key| (key, algorithm.unwrap_or(Algorithm::RS256))),
                    _ => None,
                }
            }
            Some("EC") => {
                let x = entry.get("x").and_then(serde_json::Value::as_str);
                let y = entry.get("y").and_then(serde_json::Value::as_str);
                match (x, y) {
                    (Some(x), Some(y)) => DecodingKey::from_ec_components(x, y)
                        .ok()
                        .map(|key| (key, algorithm.unwrap_or(Algorithm::ES256))),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((key, algorithm)) = key {
            keys.insert(kid, Arc::new(KeyEntry { key, algorithm }));
        }
    }
    keys
}

fn algorithm_from_name(name: &str) -> Option<Algorithm> {
    match name {
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "PS256" => Some(Algorithm::PS256),
        "PS384" => Some(Algorithm::PS384),
        "PS512" => Some(Algorithm::PS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_names_map_to_supported_algorithms_only() {
        assert_eq!(algorithm_from_name("RS256"), Some(Algorithm::RS256));
        assert_eq!(algorithm_from_name("ES256"), Some(Algorithm::ES256));
        // HS* is symmetric: it must never come out of a public key set.
        assert_eq!(algorithm_from_name("HS256"), None);
        assert_eq!(algorithm_from_name("none"), None);
    }

    #[test]
    fn a_key_set_with_no_usable_keys_parses_to_nothing() {
        let document = serde_json::json!({ "keys": [ { "kty": "oct", "k": "abc" } ] });
        assert!(parse_key_set(&document).is_empty());
        assert!(parse_key_set(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn claims_merge_supabase_role_and_neon_user_roles() {
        let claims = FederatedClaims {
            sub: "u1".into(),
            email: None,
            role: Some("authenticated".into()),
            user_roles: vec!["member".into()],
            org_id: Some("org_1".into()),
            scope: Some("runs:read runs:write".into()),
            acr: None,
        };
        assert_eq!(claims.roles(), vec!["member".to_owned(), "authenticated".to_owned()]);
        let actor = claims.into_actor("tok");
        assert_eq!(actor.source, ActorSource::FederatedJwt);
        assert_eq!(actor.scopes, vec!["runs:read".to_owned(), "runs:write".to_owned()]);
        assert_eq!(actor.org_id.as_deref(), Some("org_1"));
    }

    #[test]
    fn a_duplicated_role_is_not_repeated() {
        let claims = FederatedClaims {
            sub: "u1".into(),
            email: None,
            role: Some("member".into()),
            user_roles: vec!["member".into()],
            org_id: None,
            scope: None,
            acr: None,
        };
        assert_eq!(claims.roles(), vec!["member".to_owned()]);
    }
}
