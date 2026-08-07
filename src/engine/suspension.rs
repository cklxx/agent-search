//! Engine suspension / rate limiting.
//!
//! Implements exponential backoff suspension based on error type,
//! ported from SearXNG's suspension mechanism.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::error::SearchError;

/// Suspension status for a single engine.
#[derive(Debug, Clone)]
pub struct SuspendedStatus {
    /// Number of consecutive errors.
    pub continuous_errors: u32,
    /// When the suspension ends.
    pub suspend_end_time: Option<Instant>,
    /// Reason for suspension.
    pub suspend_reason: String,
}

impl Default for SuspendedStatus {
    fn default() -> Self {
        Self {
            continuous_errors: 0,
            suspend_end_time: None,
            suspend_reason: String::new(),
        }
    }
}

impl SuspendedStatus {
    /// Check if the engine is currently suspended.
    pub fn is_suspended(&self) -> bool {
        self.suspend_end_time
            .map(|end| Instant::now() < end)
            .unwrap_or(false)
    }

    /// Get remaining suspension duration.
    pub fn remaining(&self) -> Option<Duration> {
        self.suspend_end_time
            .map(|end| end.saturating_duration_since(Instant::now()))
    }

    /// Suspend the engine for a given duration.
    pub fn suspend(&mut self, duration: Duration, reason: &str) {
        self.continuous_errors += 1;
        self.suspend_end_time = Some(Instant::now() + duration);
        self.suspend_reason = reason.to_string();
    }

    /// Reset the suspension status (on successful response).
    pub fn resume(&mut self) {
        self.continuous_errors = 0;
        self.suspend_end_time = None;
        self.suspend_reason.clear();
    }
}

/// Manages suspension status for all engines.
pub struct EngineSuspensionManager {
    statuses: Mutex<HashMap<String, SuspendedStatus>>,
    /// Base ban time on failure (default 5s).
    ban_time_on_fail: Duration,
    /// Maximum ban time (default 120s).
    max_ban_time_on_fail: Duration,
}

impl Default for EngineSuspensionManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(5), Duration::from_secs(120))
    }
}

impl EngineSuspensionManager {
    pub fn new(ban_time_on_fail: Duration, max_ban_time_on_fail: Duration) -> Self {
        Self {
            statuses: Mutex::new(HashMap::new()),
            ban_time_on_fail,
            max_ban_time_on_fail,
        }
    }

    /// Check if an engine is suspended.
    pub fn is_suspended(&self, engine_name: &str) -> bool {
        self.statuses
            .lock()
            .unwrap()
            .get(engine_name)
            .map(|s| s.is_suspended())
            .unwrap_or(false)
    }

    /// Get the suspension reason for an engine.
    pub fn suspend_reason(&self, engine_name: &str) -> String {
        self.statuses
            .lock()
            .unwrap()
            .get(engine_name)
            .map(|s| s.suspend_reason.clone())
            .unwrap_or_default()
    }

    /// Record a successful response from an engine.
    pub fn record_success(&self, engine_name: &str) {
        if let Some(status) = self.statuses.lock().unwrap().get_mut(engine_name) {
            status.resume();
        }
    }

    /// Record an error from an engine and suspend if necessary.
    ///
    /// Returns the suspension duration if the engine was suspended.
    pub fn record_error(&self, engine_name: &str, error: &SearchError) -> Option<Duration> {
        let mut statuses = self.statuses.lock().unwrap();
        let status = statuses
            .entry(engine_name.to_string())
            .or_default();

        let duration = suspension_duration(error, status.continuous_errors, self.ban_time_on_fail, self.max_ban_time_on_fail);

        if let Some(dur) = duration {
            status.suspend(dur, &error.to_string());
            Some(dur)
        } else {
            // Timeout and other non-suspending errors just increment counter
            status.continuous_errors += 1;
            None
        }
    }
}

/// Calculate suspension duration based on error type and error count.
///
/// Ported from SearXNG's suspended_times:
/// - 403 (AccessDenied): 180s
/// - 429 (TooManyRequests): 180s
/// - CAPTCHA: 3600s
/// - Other errors: exponential backoff (ban_time * 2^errors, capped at max_ban_time)
/// - Timeout: no suspension
fn suspension_duration(
    error: &SearchError,
    continuous_errors: u32,
    ban_time: Duration,
    max_ban_time: Duration,
) -> Option<Duration> {
    match error {
        SearchError::Timeout => None, // Timeouts don't suspend
        SearchError::Request(msg) => {
            // Check for specific HTTP status codes in the error message
            if msg.contains("403") {
                Some(Duration::from_secs(180))
            } else if msg.contains("429") {
                Some(Duration::from_secs(180))
            } else if msg.to_lowercase().contains("captcha") {
                Some(Duration::from_secs(3600))
            } else if msg.to_lowercase().contains("cloudflare") {
                Some(Duration::from_secs(86400)) // 1 day
            } else {
                // Exponential backoff
                let exponent = continuous_errors.min(10);
                let duration = ban_time * 2u32.pow(exponent);
                Some(duration.min(max_ban_time))
            }
        }
        _ => {
            // Exponential backoff for other errors
            let exponent = continuous_errors.min(10);
            let duration = ban_time * 2u32.pow(exponent);
            Some(duration.min(max_ban_time))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suspension_status() {
        let mut status = SuspendedStatus::default();
        assert!(!status.is_suspended());

        status.suspend(Duration::from_secs(60), "test");
        assert!(status.is_suspended());
        assert_eq!(status.continuous_errors, 1);

        status.resume();
        assert!(!status.is_suspended());
        assert_eq!(status.continuous_errors, 0);
    }

    #[test]
    fn test_timeout_no_suspension() {
        let duration = suspension_duration(
            &SearchError::Timeout,
            5,
            Duration::from_secs(5),
            Duration::from_secs(120),
        );
        assert!(duration.is_none());
    }

    #[test]
    fn test_403_suspension() {
        let duration = suspension_duration(
            &SearchError::Request("HTTP 403".to_string()),
            0,
            Duration::from_secs(5),
            Duration::from_secs(120),
        );
        assert_eq!(duration, Some(Duration::from_secs(180)));
    }

    #[test]
    fn test_exponential_backoff() {
        let ban_time = Duration::from_secs(5);
        let max_ban = Duration::from_secs(120);

        // 1st error: 5 * 2^0 = 5s
        let d1 = suspension_duration(&SearchError::EmptyResultSet, 0, ban_time, max_ban);
        assert_eq!(d1, Some(Duration::from_secs(5)));

        // 2nd error: 5 * 2^1 = 10s
        let d2 = suspension_duration(&SearchError::EmptyResultSet, 1, ban_time, max_ban);
        assert_eq!(d2, Some(Duration::from_secs(10)));

        // 5th error: 5 * 2^4 = 80s
        let d5 = suspension_duration(&SearchError::EmptyResultSet, 4, ban_time, max_ban);
        assert_eq!(d5, Some(Duration::from_secs(80)));

        // 6th error: 5 * 2^5 = 160s, capped at 120s
        let d6 = suspension_duration(&SearchError::EmptyResultSet, 5, ban_time, max_ban);
        assert_eq!(d6, Some(Duration::from_secs(120)));
    }
}
