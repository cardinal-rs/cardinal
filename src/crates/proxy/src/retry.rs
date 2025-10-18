use cardinal_config::{DestinationRetry, DestinationRetryBackoffType};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Serialize, Deserialize)]
pub enum BackoffStrategy {
    Exponential,
    Linear,
    None,
}

pub struct RetryState {
    /// How many attempts have been made so far (starts at 0)
    pub current_attempt: u32,

    /// Total allowed attempts from the config
    pub max_attempts: u32,

    /// The base interval between retries
    pub base_interval: Duration,

    /// The timestamp of the last retry attempt
    pub last_attempt_at: Option<Instant>,

    /// The computed delay before the next retry
    pub next_delay: Duration,

    /// Whether exponential or linear backoff is used
    pub strategy: BackoffStrategy,
}

impl From<DestinationRetry> for RetryState {
    fn from(value: DestinationRetry) -> Self {
        RetryState {
            current_attempt: 0,
            max_attempts: value.max_attempts as u32,
            base_interval: Duration::from_millis(value.interval_ms),
            last_attempt_at: None,
            next_delay: Duration::from_millis(value.interval_ms),
            strategy: match value.backoff_type {
                DestinationRetryBackoffType::Exponential => BackoffStrategy::Exponential,
                DestinationRetryBackoffType::Linear => BackoffStrategy::Linear,
                DestinationRetryBackoffType::None => BackoffStrategy::None,
            },
        }
    }
}

impl RetryState {
    pub fn register_attempt(&mut self) {
        self.current_attempt += 1;
        self.last_attempt_at = Some(Instant::now());

        // Compute the next delay based on the strategy
        self.next_delay = match self.strategy {
            BackoffStrategy::None => self.base_interval,
            BackoffStrategy::Linear => self.base_interval * self.current_attempt,
            BackoffStrategy::Exponential => {
                self.base_interval * (1u32 << (self.current_attempt - 1))
            }
        };
    }

    pub fn can_retry(&self) -> bool {
        self.current_attempt < self.max_attempts
    }

    pub async fn sleep_if_retry_allowed(&mut self) -> bool {
        if self.can_retry() {
            tokio::time::sleep(self.next_delay).await;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time;
    use tokio::time::sleep;
    //
    // ──────────────────────────────── UNIT TESTS ────────────────────────────────
    //

    #[test]
    fn none_backoff_increments_and_uses_fixed_interval() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 3,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::None,
        };

        state.register_attempt();
        assert_eq!(state.current_attempt, 1);
        assert_eq!(state.next_delay, Duration::from_millis(100));
        assert!(state.last_attempt_at.is_some());
    }

    #[test]
    fn linear_backoff_grows_linearly() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 3,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Linear,
        };

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(100));

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(200));

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(300));
    }

    #[test]
    fn exponential_backoff_doubles_each_attempt() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 4,
            base_interval: Duration::from_millis(50),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Exponential,
        };

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(50)); // 1x

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(100)); // 2x

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(200)); // 4x

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(400)); // 8x
    }

    #[test]
    fn can_retry_returns_false_when_limit_reached() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 2,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Linear,
        };

        assert!(state.can_retry());
        state.register_attempt();
        assert!(state.can_retry());
        state.register_attempt();
        assert!(!state.can_retry());
    }

    #[test]
    fn exponential_backoff_saturates_safely_at_large_attempts() {
        // Verify no panic when exceeding shift limits in release mode
        let mut state = RetryState {
            current_attempt: 31,
            max_attempts: 32,
            base_interval: Duration::from_millis(1),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Exponential,
        };

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.register_attempt();
        }));

        assert!(state.next_delay > Duration::ZERO);
    }

    async fn fake_request(
        should_succeed_on: u32,
        attempt: u32,
    ) -> Result<&'static str, &'static str> {
        if attempt >= should_succeed_on {
            Ok("success")
        } else {
            Err("failed")
        }
    }

    #[tokio::test]
    async fn retry_loop_with_exponential_backoff_succeeds_after_expected_attempts() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 5,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Exponential,
        };

        let start = Instant::now();
        let mut result = Err("not started");

        while state.can_retry() {
            result = fake_request(3, state.current_attempt).await;
            if result.is_ok() {
                break;
            }

            state.register_attempt();
            sleep(state.next_delay).await;
        }

        let elapsed = start.elapsed();

        assert_eq!(result, Ok("success"));
        assert_eq!(state.current_attempt, 3);

        // Expected 100 + 200 + 400 = ~700ms total wait
        assert!(
            elapsed >= Duration::from_millis(650) && elapsed <= Duration::from_millis(850),
            "elapsed = {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn retry_loop_with_linear_backoff_fails_after_max_attempts() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 4,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::Linear,
        };

        let start = Instant::now();
        let mut result = Err("failed");

        while state.can_retry() {
            result = fake_request(10, state.current_attempt).await; // always fails
            if result.is_ok() {
                break;
            }

            state.register_attempt();
            sleep(state.next_delay).await;
        }

        let elapsed = start.elapsed();

        assert_eq!(result, Err("failed"));
        assert_eq!(state.current_attempt, state.max_attempts);

        // Expected 100 + 200 + 300 + 400 = ~1000ms total
        assert!(
            elapsed >= Duration::from_millis(900) && elapsed <= Duration::from_millis(1100),
            "elapsed = {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn retry_loop_with_none_backoff_retries_immediately() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 3,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::ZERO,
            strategy: BackoffStrategy::None,
        };

        let start = Instant::now();
        let mut result = Err("failed");

        while state.can_retry() {
            result = fake_request(2, state.current_attempt).await;
            if result.is_ok() {
                break;
            }

            state.register_attempt();
            sleep(state.next_delay).await;
        }

        let elapsed = start.elapsed();

        assert_eq!(result, Ok("success"));
        assert_eq!(state.current_attempt, 2);

        // Expected 0 + 100 + 100 = ~200ms total
        assert!(
            elapsed >= Duration::from_millis(150) && elapsed <= Duration::from_millis(300),
            "elapsed = {:?}",
            elapsed
        );
    }
}
