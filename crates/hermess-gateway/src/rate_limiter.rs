// crates/hermess-gateway/src/rate_limiter.rs
// Token bucket rate limiter for API Gateway.
// Supports per-user and global rate limits, sliding window counters.

use std::collections::HashMap;
use std::time::Instant;
use parking_lot::Mutex;

/// Token bucket rate limiter. Tokens refill at a constant rate up to a maximum
/// burst size. Each request consumes one token.
pub struct TokenBucket {
    /// Maximum tokens in the bucket (burst capacity).
    capacity: f64,
    /// Tokens added per second (sustained rate).
    rate: f64,
    /// Current token count.
    tokens: f64,
    /// Last refill timestamp.
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, rate_per_sec: f64) -> Self {
        Self {
            capacity: capacity as f64,
            rate: rate_per_sec,
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    /// Attempt to consume one token. Returns true if allowed.
    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Check if a request would be allowed without consuming a token.
    pub fn would_allow(&self) -> bool {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        let new_tokens = elapsed * self.rate;
        (self.tokens + new_tokens).min(self.capacity) >= 1.0
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last_refill = now;
    }
}

/// Rate limit configuration for a tier.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests per minute.
    pub rpm: u32,
    /// Requests per hour.
    pub rph: u32,
    /// Maximum concurrent requests.
    pub max_concurrent: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            rpm: 60,
            rph: 1000,
            max_concurrent: 10,
        }
    }
}

/// Preset tiers for common use cases.
impl RateLimitConfig {
    pub fn free_tier() -> Self {
        Self {
            rpm: 10,
            rph: 200,
            max_concurrent: 2,
        }
    }

    pub fn standard_tier() -> Self {
        Self {
            rpm: 60,
            rph: 2000,
            max_concurrent: 10,
        }
    }

    pub fn premium_tier() -> Self {
        Self {
            rpm: 300,
            rph: 10000,
            max_concurrent: 50,
        }
    }

    pub fn unlimited() -> Self {
        Self {
            rpm: u32::MAX,
            rph: u32::MAX,
            max_concurrent: u32::MAX,
        }
    }
}

/// Per-user rate limiter state.
struct UserLimiter {
    minute_bucket: TokenBucket,
    hour_bucket: TokenBucket,
    concurrent: u32,
}

/// Result of a rate limit check.
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub remaining_rpm: u32,
    pub remaining_rph: u32,
    pub retry_after_secs: Option<u64>,
    pub limit_type: Option<String>,
}

/// Multi-user rate limiter with configurable tiers.
pub struct RateLimiter {
    configs: HashMap<String, RateLimitConfig>,
    users: Mutex<HashMap<String, UserLimiter>>,
    global_minute: Mutex<TokenBucket>,
    #[allow(dead_code)]
    global_hour: Mutex<TokenBucket>,
    global_concurrent: Mutex<u32>,
    max_global_concurrent: u32,
}

impl RateLimiter {
    pub fn new(max_global_concurrent: u32) -> Self {
        Self {
            configs: HashMap::new(),
            users: Mutex::new(HashMap::new()),
            global_minute: Mutex::new(TokenBucket::new(10_000, 10_000.0 / 60.0)),
            global_hour: Mutex::new(TokenBucket::new(100_000, 100_000.0 / 3600.0)),
            global_concurrent: Mutex::new(0),
            max_global_concurrent,
        }
    }

    /// Register a rate limit tier.
    pub fn register_tier(&mut self, tier: &str, config: RateLimitConfig) {
        self.configs.insert(tier.to_string(), config);
    }

    /// Set rate limit for a specific user.
    pub fn set_user_tier(&self, user_id: &str, tier: &str) {
        let config = self
            .configs
            .get(tier)
            .cloned()
            .unwrap_or_default();
        let mut users = self.users.lock();
        users.insert(
            user_id.to_string(),
            UserLimiter {
                minute_bucket: TokenBucket::new(config.rpm, config.rpm as f64 / 60.0),
                hour_bucket: TokenBucket::new(config.rph, config.rph as f64 / 3600.0),
                concurrent: 0,
            },
        );
    }

    /// Check if a request from a user is allowed. Returns detailed result.
    pub fn check(&self, user_id: &str) -> RateLimitResult {
        // 1. Global concurrent check
        {
            let global = self.global_concurrent.lock();
            if *global >= self.max_global_concurrent {
                return RateLimitResult {
                    allowed: false,
                    remaining_rpm: 0,
                    remaining_rph: 0,
                    retry_after_secs: Some(1),
                    limit_type: Some("global_concurrent".into()),
                };
            }
        }

        // 2. User-level checks
        let mut users = self.users.lock();
        let user = users.entry(user_id.to_string()).or_insert_with(|| {
            let config = RateLimitConfig::default();
            UserLimiter {
                minute_bucket: TokenBucket::new(config.rpm, config.rpm as f64 / 60.0),
                hour_bucket: TokenBucket::new(config.rph, config.rph as f64 / 3600.0),
                concurrent: 0,
            }
        });

        // Check concurrent
        if user.concurrent >= self.configs.get("default").map(|c| c.max_concurrent).unwrap_or(10) {
            return RateLimitResult {
                allowed: false,
                remaining_rpm: user.minute_bucket.tokens as u32,
                remaining_rph: user.hour_bucket.tokens as u32,
                retry_after_secs: Some(1),
                limit_type: Some("user_concurrent".into()),
            };
        }

        // Check minute bucket
        if !user.minute_bucket.try_consume() {
            return RateLimitResult {
                allowed: false,
                remaining_rpm: 0,
                remaining_rph: user.hour_bucket.tokens as u32,
                retry_after_secs: Some(1),
                limit_type: Some("rpm".into()),
            };
        }

        // Check hour bucket
        if !user.hour_bucket.try_consume() {
            // Refund the minute token since we're denying on hour limit
            user.minute_bucket.tokens += 1.0;
            return RateLimitResult {
                allowed: false,
                remaining_rpm: user.minute_bucket.tokens as u32,
                remaining_rph: 0,
                retry_after_secs: Some(1),
                limit_type: Some("rph".into()),
            };
        }

        // 3. Global rate check
        {
            let mut global = self.global_minute.lock();
            if !global.try_consume() {
                // Refund user tokens
                user.minute_bucket.tokens += 1.0;
                user.hour_bucket.tokens += 1.0;
                return RateLimitResult {
                    allowed: false,
                    remaining_rpm: user.minute_bucket.tokens as u32,
                    remaining_rph: user.hour_bucket.tokens as u32,
                    retry_after_secs: Some(5),
                    limit_type: Some("global_rpm".into()),
                };
            }
        }

        // All checks passed
        user.concurrent += 1;
        RateLimitResult {
            allowed: true,
            remaining_rpm: user.minute_bucket.tokens as u32,
            remaining_rph: user.hour_bucket.tokens as u32,
            retry_after_secs: None,
            limit_type: None,
        }
    }

    /// Release a concurrent slot when a request completes.
    pub fn release(&self, user_id: &str) {
        let mut users = self.users.lock();
        if let Some(user) = users.get_mut(user_id) {
            user.concurrent = user.concurrent.saturating_sub(1);
        }
        let mut global = self.global_concurrent.lock();
        *global = global.saturating_sub(1);
    }

    /// Acquire a global concurrent slot (call before check).
    pub fn acquire_concurrent(&self) -> bool {
        let mut global = self.global_concurrent.lock();
        if *global >= self.max_global_concurrent {
            return false;
        }
        *global += 1;
        true
    }

    /// Remove a user's rate limit state (on session expiry).
    pub fn remove_user(&self, user_id: &str) {
        self.users.lock().remove(user_id);
    }

    /// Get current stats for a user.
    pub fn user_stats(&self, user_id: &str) -> UserRateStats {
        let users = self.users.lock();
        if let Some(user) = users.get(user_id) {
            UserRateStats {
                remaining_rpm: user.minute_bucket.tokens as u32,
                remaining_rph: user.hour_bucket.tokens as u32,
                concurrent_used: user.concurrent,
            }
        } else {
            UserRateStats::default()
        }
    }
}

/// Snapshot of a user's current rate limit state.
#[derive(Debug, Clone, Default)]
pub struct UserRateStats {
    pub remaining_rpm: u32,
    pub remaining_rph: u32,
    pub concurrent_used: u32,
}

// ── Simple API Key Manager ──

/// Manages API keys for gateway authentication.
pub struct ApiKeyManager {
    /// Map of api_key → (user_id, tier).
    keys: Mutex<HashMap<String, (String, String)>>,
}

impl ApiKeyManager {
    pub fn new() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Register an API key for a user at a specific tier.
    pub fn register(&self, api_key: &str, user_id: &str, tier: &str) {
        self.keys.lock().insert(
            api_key.to_string(),
            (user_id.to_string(), tier.to_string()),
        );
    }

    /// Look up a user by API key. Returns (user_id, tier).
    pub fn lookup(&self, api_key: &str) -> Option<(String, String)> {
        self.keys.lock().get(api_key).cloned()
    }

    /// Revoke an API key.
    pub fn revoke(&self, api_key: &str) -> bool {
        self.keys.lock().remove(api_key).is_some()
    }

    /// List all API keys with their user IDs (masked keys).
    pub fn list_masked(&self) -> Vec<(String, String, String)> {
        self.keys
            .lock()
            .iter()
            .map(|(key, (user, tier))| {
                let masked = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    "****".to_string()
                };
                (masked, user.clone(), tier.clone())
            })
            .collect()
    }

    /// Generate a new random API key (hermes_ prefix + 32 random hex chars).
    pub fn generate_key() -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        format!("hermes_{}", hex::encode(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn token_bucket_allows_up_to_capacity() {
        let mut bucket = TokenBucket::new(5, 10.0);
        for _ in 0..5 {
            assert!(bucket.try_consume());
        }
        assert!(!bucket.try_consume());
    }

    #[test]
    fn token_bucket_refills_over_time() {
        let mut bucket = TokenBucket::new(3, 100.0); // 100 tokens/sec
        for _ in 0..3 {
            assert!(bucket.try_consume());
        }
        assert!(!bucket.try_consume());
        thread::sleep(std::time::Duration::from_millis(20));
        // After 20ms at 100 tok/s, ~2 tokens refilled
        assert!(bucket.try_consume());
    }

    #[test]
    fn rate_limiter_allows_and_blocks() {
        let limiter = RateLimiter::new(100);
        limiter.set_user_tier("test-user", "default");

        // Default tier: 60 rpm
        for i in 0..60 {
            let result = limiter.check("test-user");
            assert!(result.allowed, "Request {i} should be allowed");
            limiter.release("test-user");
        }
        let blocked = limiter.check("test-user");
        assert!(!blocked.allowed);
        assert_eq!(blocked.limit_type, Some("rpm".into()));
    }

    #[test]
    fn global_concurrent_limit() {
        let limiter = RateLimiter::new(2);
        limiter.set_user_tier("user1", "default");
        limiter.set_user_tier("user2", "default");

        assert!(limiter.acquire_concurrent());
        assert!(limiter.acquire_concurrent());
        assert!(!limiter.acquire_concurrent());

        limiter.release("user1");
        assert!(limiter.acquire_concurrent());
    }

    #[test]
    fn api_key_generation() {
        let key = ApiKeyManager::generate_key();
        assert!(key.starts_with("hermes_"));
        assert_eq!(key.len(), 7 + 32); // "hermes_" + 32 hex chars
    }

    #[test]
    fn api_key_lookup() {
        let mgr = ApiKeyManager::new();
        let key = ApiKeyManager::generate_key();
        mgr.register(&key, "alice", "premium");
        let (user, tier) = mgr.lookup(&key).unwrap();
        assert_eq!(user, "alice");
        assert_eq!(tier, "premium");
    }

    #[test]
    fn api_key_revoke() {
        let mgr = ApiKeyManager::new();
        let key = ApiKeyManager::generate_key();
        mgr.register(&key, "bob", "standard");
        assert!(mgr.revoke(&key));
        assert!(mgr.lookup(&key).is_none());
    }

    #[test]
    fn api_key_masked_list() {
        let mgr = ApiKeyManager::new();
        let key = "hermes_aaaabbbbccccddddeeeeffff00001111";
        mgr.register(key, "carol", "free");
        let list = mgr.list_masked();
        assert_eq!(list.len(), 1);
        assert!(list[0].0.contains("..."));
    }

    #[test]
    fn user_stats_show_remaining() {
        let limiter = RateLimiter::new(50);
        limiter.set_user_tier("stats-user", "default");
        let _ = limiter.check("stats-user");
        limiter.release("stats-user");
        let stats = limiter.user_stats("stats-user");
        assert!(stats.remaining_rpm < 60);
    }
}
