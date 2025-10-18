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

    /// Upper bound for the delay if provided in the config
    pub max_interval: Option<Duration>,
}

impl From<DestinationRetry> for RetryState {
    fn from(value: DestinationRetry) -> Self {
        let base_interval = Duration::from_millis(value.interval_ms);
        let max_interval = value.max_interval.map(Duration::from_millis);
        let initial_delay = max_interval
            .map(|max| base_interval.min(max))
            .unwrap_or(base_interval);

        RetryState {
            current_attempt: 0,
            max_attempts: value.max_attempts.min(u32::MAX as u64) as u32,
            base_interval,
            last_attempt_at: None,
            next_delay: initial_delay,
            strategy: match value.backoff_type {
                DestinationRetryBackoffType::Exponential => BackoffStrategy::Exponential,
                DestinationRetryBackoffType::Linear => BackoffStrategy::Linear,
                DestinationRetryBackoffType::None => BackoffStrategy::None,
            },
            max_interval,
        }
    }
}

impl RetryState {
    pub fn register_attempt(&mut self) {
        self.current_attempt += 1;
        self.last_attempt_at = Some(Instant::now());

        // Compute the next delay based on the strategy
        let mut next_delay = match self.strategy {
            BackoffStrategy::None => self.base_interval,
            BackoffStrategy::Linear => self
                .base_interval
                .saturating_mul(self.current_attempt.max(1)),
            BackoffStrategy::Exponential => {
                let shift = (self.current_attempt - 1).min(31);
                let multiplier = 1u32 << shift;
                self.base_interval.saturating_mul(multiplier)
            }
        };

        if let Some(max_interval) = self.max_interval {
            if next_delay > max_interval {
                next_delay = max_interval;
            }
        }

        self.next_delay = next_delay;
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
            max_interval: None,
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
            max_interval: None,
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
            max_interval: None,
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
            max_interval: None,
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
            max_interval: None,
        };

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.register_attempt();
        }));

        assert!(state.next_delay > Duration::ZERO);
    }

    #[test]
    fn retry_state_from_clamps_initial_delay() {
        let retry = DestinationRetry {
            max_attempts: 3,
            interval_ms: 200,
            backoff_type: DestinationRetryBackoffType::Linear,
            max_interval: Some(150),
        };

        let state = RetryState::from(retry);

        assert_eq!(state.next_delay, Duration::from_millis(150));
    }

    #[test]
    fn max_interval_caps_backoff_growth() {
        let mut state = RetryState {
            current_attempt: 0,
            max_attempts: 4,
            base_interval: Duration::from_millis(100),
            last_attempt_at: None,
            next_delay: Duration::from_millis(100),
            strategy: BackoffStrategy::Exponential,
            max_interval: Some(Duration::from_millis(250)),
        };

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(100));

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(200));

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(250));

        state.register_attempt();
        assert_eq!(state.next_delay, Duration::from_millis(250));
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
            max_interval: None,
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
            max_interval: None,
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
            max_interval: None,
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

    #[test]
    fn retry_state_from_clamps_max_attempts_to_u32_max() {
        let retry = DestinationRetry {
            max_attempts: (u32::MAX as u64) + 42,
            interval_ms: 50,
            backoff_type: DestinationRetryBackoffType::Linear,
            max_interval: None,
        };

        let state = RetryState::from(retry);

        assert_eq!(state.max_attempts, u32::MAX);
    }

    #[test]
    fn exponential_backoff_from_config_respects_max_interval_sequence() {
        let retry = DestinationRetry {
            max_attempts: 5,
            interval_ms: 100,
            backoff_type: DestinationRetryBackoffType::Exponential,
            max_interval: Some(250),
        };

        let mut state = RetryState::from(retry);
        let mut observed = Vec::new();

        for _ in 0..state.max_attempts {
            state.register_attempt();
            observed.push(state.next_delay);
        }

        let expected = [
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(250),
            Duration::from_millis(250),
            Duration::from_millis(250),
        ];

        assert_eq!(&observed[..], &expected);
        assert!(!state.can_retry());
    }

    #[tokio::test]
    async fn sleep_if_retry_allowed_returns_false_when_no_attempts_left() {
        let retry = DestinationRetry {
            max_attempts: 2,
            interval_ms: 10,
            backoff_type: DestinationRetryBackoffType::Linear,
            max_interval: Some(10),
        };

        let mut state = RetryState::from(retry);

        state.register_attempt();
        assert!(state.can_retry());

        state.register_attempt();
        assert!(!state.can_retry());

        let slept = state.sleep_if_retry_allowed().await;
        assert!(!slept);
        assert_eq!(state.current_attempt, state.max_attempts);
    }

    #[test]
    fn exponential_backoff_does_not_overflow_large_base_interval() {
        let retry = DestinationRetry {
            max_attempts: 100,
            interval_ms: u64::MAX / 4,
            backoff_type: DestinationRetryBackoffType::Exponential,
            max_interval: None,
        };

        let mut state = RetryState::from(retry);

        for _ in 0..40 {
            state.register_attempt();
        }

        assert_eq!(state.next_delay, Duration::MAX);
        assert!(state.can_retry());
    }

    #[tokio::test]
    async fn retry_loop_with_real_waits_respects_limits() {
        let retry = DestinationRetry {
            max_attempts: 4,
            interval_ms: 90,
            backoff_type: DestinationRetryBackoffType::Exponential,
            max_interval: Some(200),
        };

        let mut state = RetryState::from(retry);
        let mut observed_delays = Vec::new();
        let mut sleep_calls = 0;

        while state.can_retry() {
            state.register_attempt();
            observed_delays.push(state.next_delay);

            if !state.can_retry() {
                assert!(!state.sleep_if_retry_allowed().await);
                break;
            }

            assert!(state.next_delay <= Duration::from_millis(200));
            assert!(state.sleep_if_retry_allowed().await);
            sleep_calls += 1;
        }

        assert_eq!(state.current_attempt, state.max_attempts);
        assert_eq!(sleep_calls, (state.max_attempts - 1) as usize);
        assert_eq!(
            observed_delays,
            vec![
                Duration::from_millis(90),
                Duration::from_millis(180),
                Duration::from_millis(200),
                Duration::from_millis(200),
            ]
        );
    }
}
