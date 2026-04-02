use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    #[serde(default = "default_backoff_factor")]
    pub backoff_factor: f64,
    #[serde(default = "default_backoff_ceiling")]
    pub backoff_ceiling: Duration,
    #[serde(default = "default_execution_timeout")]
    pub execution_timeout: Duration,
}

fn default_backoff_factor() -> f64 {
    2.0
}

fn default_backoff_ceiling() -> Duration {
    Duration::from_secs(300)
}

fn default_execution_timeout() -> Duration {
    Duration::from_secs(1800)
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_factor: default_backoff_factor(),
            backoff_ceiling: default_backoff_ceiling(),
            execution_timeout: default_execution_timeout(),
        }
    }
}

impl RetryPolicy {
    /// Calculate backoff duration for a given attempt (0-based).
    pub fn backoff_duration(&self, attempt: u32) -> Duration {
        let base = Duration::from_secs(5);
        let factor = self.backoff_factor.powi(attempt as i32);
        let backoff = base.mul_f64(factor);
        backoff.min(self.backoff_ceiling)
    }

    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_exponential() {
        let policy = RetryPolicy::default();
        let d0 = policy.backoff_duration(0);
        let d1 = policy.backoff_duration(1);
        let d2 = policy.backoff_duration(2);

        assert_eq!(d0, Duration::from_secs(5));
        assert_eq!(d1, Duration::from_secs(10));
        assert_eq!(d2, Duration::from_secs(20));
    }

    #[test]
    fn backoff_ceiling() {
        let policy = RetryPolicy {
            backoff_ceiling: Duration::from_secs(15),
            ..Default::default()
        };
        let d5 = policy.backoff_duration(5); // 5 * 2^5 = 160s, but ceiling is 15s
        assert_eq!(d5, Duration::from_secs(15));
    }

    #[test]
    fn should_retry() {
        let policy = RetryPolicy {
            max_retries: 2,
            ..Default::default()
        };
        assert!(policy.should_retry(0));
        assert!(policy.should_retry(1));
        assert!(!policy.should_retry(2));
    }
}
