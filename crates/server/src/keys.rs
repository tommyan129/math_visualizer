//! Workspace keys: `prefix-random` (random part 128 bit), stored as SHA-256 only.
//! A key both authenticates and selects the workspace (capability token).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

pub const RANDOM_BYTES: usize = 16;

/// Failed attempts allowed per window before all key checks are refused for `BLOCK`.
pub const MAX_FAILURES: usize = 10;
pub const WINDOW: Duration = Duration::from_secs(60);
pub const BLOCK: Duration = Duration::from_secs(60);

pub fn hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Readable prefix: the name itself when it is plain ASCII, else 6 hex of its hash.
pub fn prefix(name: &str) -> String {
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'));
    let slug: String = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if plain && !slug.is_empty() { slug } else { hash(name)[..6].to_string() }
}

pub fn generate(name: &str) -> String {
    let mut buf = [0u8; RANDOM_BYTES];
    getrandom::fill(&mut buf).expect("OS random source");
    format!("{}-{}", prefix(name), hex::encode(buf))
}

/// Global limiter for wrong keys on the public listener (single user, so not per IP).
#[derive(Default)]
pub struct Limiter {
    failures: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}

impl Limiter {
    pub fn blocked(&self, now: Instant) -> bool {
        self.blocked_until.is_some_and(|t| now < t)
    }

    pub fn fail(&mut self, now: Instant) {
        self.failures.push_back(now);
        while self.failures.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
            self.failures.pop_front();
        }
        if self.failures.len() > MAX_FAILURES {
            self.blocked_until = Some(now + BLOCK);
            self.failures.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_rules() {
        assert_eq!(prefix("Linear Algebra"), "linear-algebra");
        assert_eq!(prefix("calc_1"), "calc-1");
        let p = prefix("미적분 1 · 정적분");
        assert_eq!(p.len(), 6);
        assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(p, prefix("미적분 1 · 정적분"), "deterministic");
    }

    #[test]
    fn key_has_128_bit_random_part_and_changes() {
        let a = generate("calc");
        let b = generate("calc");
        assert!(a.starts_with("calc-"));
        assert_eq!(a.len() - "calc-".len(), RANDOM_BYTES * 2);
        assert_ne!(a, b, "replacing must produce a new key");
        assert_ne!(hash(&a), hash(&b));
    }

    #[test]
    fn limiter_blocks_after_burst() {
        let mut l = Limiter::default();
        let t = Instant::now();
        for _ in 0..MAX_FAILURES {
            l.fail(t);
        }
        assert!(!l.blocked(t));
        l.fail(t);
        assert!(l.blocked(t));
        assert!(!l.blocked(t + BLOCK + Duration::from_secs(1)));
    }
}
