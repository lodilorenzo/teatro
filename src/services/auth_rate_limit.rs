use std::{collections::HashMap, sync::Mutex};

use chrono::{DateTime, Duration, Utc};

use crate::config::AuthConfig;

#[derive(Debug, Default)]
pub struct AuthRateLimiter {
    entries: Mutex<HashMap<String, RateLimitEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitOutcome {
    pub failure_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
    pub retry_after_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitRejection {
    pub failure_count: u32,
    pub locked_until: DateTime<Utc>,
    pub retry_after_seconds: u64,
}

#[derive(Debug, Clone)]
struct RateLimitEntry {
    failure_count: u32,
    window_started_at: DateTime<Utc>,
    locked_until: Option<DateTime<Utc>>,
    last_seen_at: DateTime<Utc>,
}

impl AuthRateLimiter {
    pub fn check(
        &self,
        config: &AuthConfig,
        key: &str,
        now: DateTime<Utc>,
    ) -> Option<RateLimitRejection> {
        if config.rate_limit_max_failures == 0 {
            return None;
        }

        let mut entries = self.entries.lock().expect("rate limiter mutex poisoned");
        let entry = entries.get_mut(key)?;

        if window_expired(config, entry, now) {
            entries.remove(key);
            return None;
        }

        entry.last_seen_at = now;
        if let Some(locked_until) = entry
            .locked_until
            .filter(|locked_until| *locked_until > now)
        {
            return Some(RateLimitRejection {
                failure_count: entry.failure_count,
                locked_until,
                retry_after_seconds: retry_after_seconds(now, locked_until),
            });
        }

        None
    }

    pub fn record_failure(
        &self,
        config: &AuthConfig,
        key: String,
        now: DateTime<Utc>,
    ) -> RateLimitOutcome {
        if config.rate_limit_max_failures == 0 || config.rate_limit_max_entries == 0 {
            return RateLimitOutcome {
                failure_count: 0,
                locked_until: None,
                retry_after_seconds: None,
            };
        }

        let mut entries = self.entries.lock().expect("rate limiter mutex poisoned");
        entries.retain(|_, entry| !window_expired(config, entry, now));
        if !entries.contains_key(&key) && entries.len() >= config.rate_limit_max_entries {
            let oldest_unlocked_key = entries
                .iter()
                .filter(|(_, entry)| {
                    entry
                        .locked_until
                        .is_none_or(|locked_until| locked_until <= now)
                })
                .min_by(|(left_key, left), (right_key, right)| {
                    left.last_seen_at
                        .cmp(&right.last_seen_at)
                        .then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _)| key.clone());
            if let Some(oldest_unlocked_key) = oldest_unlocked_key {
                entries.remove(&oldest_unlocked_key);
            } else {
                let locked_until = entries
                    .values()
                    .filter_map(|entry| entry.locked_until)
                    .min()
                    .expect("a full map without an unlocked entry contains a lockout");
                return RateLimitOutcome {
                    failure_count: config.rate_limit_max_failures,
                    locked_until: Some(locked_until),
                    retry_after_seconds: Some(retry_after_seconds(now, locked_until)),
                };
            }
        }
        let entry = entries.entry(key).or_insert_with(|| RateLimitEntry {
            failure_count: 0,
            window_started_at: now,
            locked_until: None,
            last_seen_at: now,
        });

        if window_expired(config, entry, now) {
            entry.failure_count = 0;
            entry.window_started_at = now;
            entry.locked_until = None;
        }

        entry.last_seen_at = now;
        entry.failure_count = entry.failure_count.saturating_add(1);

        if entry.failure_count >= config.rate_limit_max_failures {
            let lockout = Duration::seconds(config.rate_limit_lockout_seconds as i64);
            entry.locked_until = Some(now + lockout);
        }

        RateLimitOutcome {
            failure_count: entry.failure_count,
            locked_until: entry.locked_until,
            retry_after_seconds: entry
                .locked_until
                .filter(|locked_until| *locked_until > now)
                .map(|locked_until| retry_after_seconds(now, locked_until)),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("rate limiter mutex poisoned")
            .len()
    }

    pub fn record_success(&self, key: &str) {
        let mut entries = self.entries.lock().expect("rate limiter mutex poisoned");
        entries.remove(key);
    }
}

fn window_expired(config: &AuthConfig, entry: &RateLimitEntry, now: DateTime<Utc>) -> bool {
    let window = Duration::seconds(config.rate_limit_window_seconds as i64);
    now.signed_duration_since(entry.window_started_at) > window
}

fn retry_after_seconds(now: DateTime<Utc>, locked_until: DateTime<Utc>) -> u64 {
    locked_until.signed_duration_since(now).num_seconds().max(1) as u64
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::AuthRateLimiter;
    use crate::config::AuthConfig;

    #[test]
    fn locks_after_configured_failures_and_clears_on_success() {
        let limiter = AuthRateLimiter::default();
        let config = AuthConfig {
            rate_limit_max_failures: 2,
            rate_limit_window_seconds: 60,
            rate_limit_lockout_seconds: 30,
            ..AuthConfig::default()
        };
        let now = Utc::now();

        let first = limiter.record_failure(&config, "basic:admin".to_string(), now);
        assert_eq!(first.failure_count, 1);
        assert!(limiter.check(&config, "basic:admin", now).is_none());

        let second = limiter.record_failure(&config, "basic:admin".to_string(), now);
        assert_eq!(second.failure_count, 2);
        assert!(second.locked_until.is_some());
        assert!(limiter.check(&config, "basic:admin", now).is_some());

        limiter.record_success("basic:admin");
        assert!(limiter.check(&config, "basic:admin", now).is_none());
    }

    #[test]
    fn bounds_arbitrary_rate_limit_keys() {
        let limiter = AuthRateLimiter::default();
        let config = AuthConfig {
            rate_limit_max_entries: 3,
            ..AuthConfig::default()
        };
        let now = Utc::now();
        for index in 0..100 {
            limiter.record_failure(&config, format!("attacker-{index}"), now);
        }
        assert_eq!(limiter.len(), 3);
    }

    #[test]
    fn capacity_pressure_does_not_evict_active_lockouts() {
        let limiter = AuthRateLimiter::default();
        let config = AuthConfig {
            rate_limit_max_failures: 1,
            rate_limit_max_entries: 2,
            rate_limit_lockout_seconds: 60,
            ..AuthConfig::default()
        };
        let now = Utc::now();
        limiter.record_failure(&config, "protected".to_string(), now);

        for index in 0..20 {
            limiter.record_failure(
                &config,
                format!("overflow-{index}"),
                now + Duration::milliseconds(index),
            );
        }

        assert!(limiter.check(&config, "protected", now).is_some());
        assert_eq!(limiter.len(), 2);
    }

    #[test]
    fn expires_windows() {
        let limiter = AuthRateLimiter::default();
        let config = AuthConfig {
            rate_limit_max_failures: 2,
            rate_limit_window_seconds: 10,
            rate_limit_lockout_seconds: 30,
            ..AuthConfig::default()
        };
        let now = Utc::now();

        limiter.record_failure(&config, "basic:admin".to_string(), now);
        limiter.record_failure(
            &config,
            "basic:admin".to_string(),
            now + Duration::seconds(11),
        );

        assert!(
            limiter
                .check(&config, "basic:admin", now + Duration::seconds(11))
                .is_none()
        );
    }
}
