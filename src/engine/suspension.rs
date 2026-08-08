//! Engine suspension with exponential backoff (ported from SearXNG).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::error::SearchError;

#[derive(Debug, Clone, Default)]
pub struct SuspendedStatus {
    pub continuous_errors: u32,
    pub suspend_end_time: Option<Instant>,
}

impl SuspendedStatus {
    pub fn is_suspended(&self) -> bool {
        self.suspend_end_time
            .map(|end| Instant::now() < end)
            .unwrap_or(false)
    }

    pub fn suspend(&mut self, duration: Duration) {
        self.continuous_errors += 1;
        self.suspend_end_time = Some(Instant::now() + duration);
    }

    /// Reset on successful response.
    pub fn resume(&mut self) {
        self.continuous_errors = 0;
        self.suspend_end_time = None;
    }
}

pub struct EngineSuspensionManager {
    statuses: Mutex<HashMap<String, SuspendedStatus>>,
    ban_time_on_fail: Duration,
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

    pub fn is_suspended(&self, engine_name: &str) -> bool {
        self.statuses
            .lock()
            .unwrap()
            .get(engine_name)
            .map(|s| s.is_suspended())
            .unwrap_or(false)
    }

    pub fn record_success(&self, engine_name: &str) {
        if let Some(status) = self.statuses.lock().unwrap().get_mut(engine_name) {
            status.resume();
        }
    }

    /// Returns suspension duration if the engine was suspended.
    pub fn record_error(&self, engine_name: &str, error: &SearchError) -> Option<Duration> {
        let mut statuses = self.statuses.lock().unwrap();
        let status = statuses.entry(engine_name.to_string()).or_default();

        let duration = suspension_duration(error, status.continuous_errors, self.ban_time_on_fail, self.max_ban_time_on_fail);

        if let Some(dur) = duration {
            status.suspend(dur);
            Some(dur)
        } else {
            status.continuous_errors += 1;
            None
        }
    }
}

/// 403/429: 180s, others: exponential backoff.
fn suspension_duration(
    error: &SearchError,
    continuous_errors: u32,
    ban_time: Duration,
    max_ban_time: Duration,
) -> Option<Duration> {
    let exponential = || {
        let exponent = continuous_errors.min(10);
        (ban_time * 2u32.pow(exponent)).min(max_ban_time)
    };

    match error {
        SearchError::Timeout => None,
        SearchError::HttpStatus(403) | SearchError::HttpStatus(429) => {
            Some(Duration::from_secs(180))
        }
        _ => Some(exponential()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suspension_status() {
        let mut status = SuspendedStatus::default();
        assert!(!status.is_suspended());

        status.suspend(Duration::from_secs(60));
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
            &SearchError::HttpStatus(403),
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

        let d1 = suspension_duration(&SearchError::EmptyResultSet, 0, ban_time, max_ban);
        assert_eq!(d1, Some(Duration::from_secs(5)));

        let d2 = suspension_duration(&SearchError::EmptyResultSet, 1, ban_time, max_ban);
        assert_eq!(d2, Some(Duration::from_secs(10)));

        let d5 = suspension_duration(&SearchError::EmptyResultSet, 4, ban_time, max_ban);
        assert_eq!(d5, Some(Duration::from_secs(80)));

        let d6 = suspension_duration(&SearchError::EmptyResultSet, 5, ban_time, max_ban);
        assert_eq!(d6, Some(Duration::from_secs(120)));
    }
}
