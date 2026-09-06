#![forbid(unsafe_code)]

//! Rate limiting, wired into ores-middleware's `RateLimiter` hook.
//!
//! The decision itself comes from `ores-rl-lib-core`'s deterministic
//! [`transition`] state machine, so this crate holds no limiter algebra of its
//! own — only the bounded state store and the key derivation.
//!
//! **Keys are opaque.** ores-middleware already hands us a derived principal
//! digest; we HMAC it again with a server-held secret before it becomes a map
//! key, so no raw IP, subject, email, cookie or token value ever reaches the
//! limiter state, its eviction log, or a metric label.
//!
//! The store is bounded (`max_entries`) and time-evicted (`entry_ttl`) so an
//! unauthenticated flood across many principals cannot grow memory without
//! limit. Enable the `rate-limit-redis` feature to turn on `ores-rl-lib-core`'s
//! Redis authority for multi-replica deployments; the in-process store then acts
//! as the local fallback described by the fleet's fail-closed posture.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use ores_middleware::RateLimiter;
use ores_rl_lib_core::{transition, Decision, LimitPolicy, LimitState};
use sha2::Sha256;
use tokio::sync::Mutex;

use crate::session::to_hex;

type HmacSha256 = Hmac<Sha256>;

/// Default policy: a burst of 120 with 30 refilled every 10 seconds. Generous
/// for a page that fires several htmx fragments, tight enough to matter.
pub const DEFAULT_CAPACITY: u64 = 120;
pub const DEFAULT_REFILL_TOKENS: u64 = 30;
pub const DEFAULT_REFILL_INTERVAL_MS: u64 = 10_000;
/// Upper bound on distinct tracked principals.
pub const DEFAULT_MAX_ENTRIES: usize = 20_000;
/// How long an idle principal's state survives.
pub const DEFAULT_ENTRY_TTL: Duration = Duration::from_secs(600);

struct Entry {
    state: LimitState,
    last_seen: Instant,
}

/// In-process limiter over opaque keys.
pub struct OpaqueRateLimiter {
    policy: LimitPolicy,
    secret: Vec<u8>,
    max_entries: usize,
    entry_ttl: Duration,
    started: Instant,
    entries: Mutex<HashMap<String, Entry>>,
}

impl std::fmt::Debug for OpaqueRateLimiter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpaqueRateLimiter")
            .field("max_entries", &self.max_entries)
            .field("entry_ttl", &self.entry_ttl)
            .finish_non_exhaustive()
    }
}

impl OpaqueRateLimiter {
    #[must_use]
    pub fn new(secret: &str) -> Self {
        Self::with_policy(
            secret,
            LimitPolicy::token_bucket(DEFAULT_CAPACITY, DEFAULT_REFILL_TOKENS, DEFAULT_REFILL_INTERVAL_MS),
            DEFAULT_MAX_ENTRIES,
            DEFAULT_ENTRY_TTL,
        )
    }

    #[must_use]
    pub fn with_policy(secret: &str, policy: LimitPolicy, max_entries: usize, entry_ttl: Duration) -> Self {
        Self {
            policy,
            secret: secret.as_bytes().to_vec(),
            max_entries: max_entries.max(1),
            entry_ttl,
            started: Instant::now(),
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// HMAC-SHA256 of the caller's key material, truncated to 128 bits of hex.
    /// Truncation is safe here: this is a bucket label, not an authenticator.
    #[must_use]
    pub fn opaque_key(&self, material: &str) -> String {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&self.secret).expect("hmac accepts any key length");
        mac.update(material.as_bytes());
        let digest = to_hex(&mac.finalize().into_bytes());
        digest[..32].to_owned()
    }

    fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// One decision. Returns the token-bucket outcome for the opaque key.
    pub async fn decide(&self, material: &str, cost: u64) -> Decision {
        let key = self.opaque_key(material);
        let now_ms = self.monotonic_ms();
        let mut entries = self.entries.lock().await;
        evict(&mut entries, self.entry_ttl, self.max_entries);

        let previous = entries.get(&key).map_or(LimitState::Empty, |entry| entry.state);
        match transition(self.policy, previous, now_ms, cost.max(1)) {
            Ok((next, decision)) => {
                entries.insert(
                    key,
                    Entry {
                        state: next,
                        last_seen: Instant::now(),
                    },
                );
                decision
            }
            // A rejected transition (mismatched stored state, clock skew) drops
            // the entry and lets the next request start from a clean window.
            // Fail-open here is deliberate: the middleware's own failure mode,
            // which is fail-closed in production, is the authority on denial.
            Err(_) => {
                entries.remove(&key);
                Decision::Bypass {
                    reason: "limiter-state-reset",
                }
            }
        }
    }

    #[cfg(test)]
    async fn tracked(&self) -> usize {
        self.entries.lock().await.len()
    }
}

/// Time-evicts expired entries, then bounds the map by dropping the oldest.
fn evict(entries: &mut HashMap<String, Entry>, ttl: Duration, max_entries: usize) {
    entries.retain(|_, entry| entry.last_seen.elapsed() < ttl);
    while entries.len() >= max_entries {
        let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_seen)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        entries.remove(&oldest);
    }
}

impl RateLimiter for OpaqueRateLimiter {
    fn allow<'a>(
        &'a self,
        key: &'a str,
        _capacity: u32,
        _refill_per_second: f64,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        // The configured policy is the authority; the middleware's suggested
        // capacity/refill is advisory and deliberately ignored so one policy
        // governs every replica of this service.
        Box::pin(async move { !matches!(self.decide(key, 1).await, Decision::Deny { .. }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(capacity: u64) -> OpaqueRateLimiter {
        OpaqueRateLimiter::with_policy(
            "unit-secret",
            LimitPolicy::token_bucket(capacity, 1, 60_000),
            16,
            Duration::from_secs(60),
        )
    }

    #[test]
    fn keys_are_opaque_and_stable() {
        let limiter = limiter(10);
        let key = limiter.opaque_key("user:alex@example.com|route:/runs");
        assert_eq!(key.len(), 32);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!key.contains("alex"));
        assert_eq!(key, limiter.opaque_key("user:alex@example.com|route:/runs"));
        assert_ne!(key, limiter.opaque_key("user:sam@example.com|route:/runs"));
    }

    #[test]
    fn a_different_secret_produces_a_different_key() {
        let a = limiter(10).opaque_key("same");
        let b = OpaqueRateLimiter::new("another-secret").opaque_key("same");
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn a_burst_is_denied_once_the_bucket_empties() {
        let limiter = limiter(3);
        for attempt in 0..3 {
            assert!(
                limiter.allow("principal", 0, 0.0).await,
                "attempt {attempt} should pass"
            );
        }
        assert!(!limiter.allow("principal", 0, 0.0).await);
    }

    #[tokio::test]
    async fn principals_do_not_share_a_bucket() {
        let limiter = limiter(1);
        assert!(limiter.allow("a", 0, 0.0).await);
        assert!(!limiter.allow("a", 0, 0.0).await);
        assert!(limiter.allow("b", 0, 0.0).await);
    }

    #[tokio::test]
    async fn the_store_stays_bounded_under_many_principals() {
        let limiter = limiter(10);
        for index in 0..200 {
            let _ = limiter.allow(&format!("principal-{index}"), 0, 0.0).await;
        }
        assert!(limiter.tracked().await <= 16, "store grew past its bound");
    }
}
