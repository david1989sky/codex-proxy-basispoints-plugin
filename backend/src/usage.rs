use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

#[allow(dead_code)]
pub(crate) struct UsageStats {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    last_request_at_ms: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct UsageSnapshot {
    pub(crate) total_requests: u64,
    pub(crate) successful_requests: u64,
    pub(crate) failed_requests: u64,
    pub(crate) last_request_at_ms: Option<u64>,
}

#[allow(dead_code)]
impl UsageStats {
    pub(crate) const fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            last_request_at_ms: AtomicU64::new(0),
        }
    }

    pub(crate) fn begin(&self, now_ms: u64) {
        self.update_latest_request_time(now_ms);
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_success(&self) {
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_failure(&self) {
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> UsageSnapshot {
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        UsageSnapshot {
            total_requests,
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            last_request_at_ms: (total_requests > 0)
                .then(|| self.last_request_at_ms.load(Ordering::Relaxed)),
        }
    }

    fn update_latest_request_time(&self, now_ms: u64) {
        let mut latest = self.last_request_at_ms.load(Ordering::Relaxed);
        while now_ms > latest {
            match self.last_request_at_ms.compare_exchange_weak(
                latest,
                now_ms,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => latest = observed,
            }
        }
    }
}

impl Default for UsageStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use serde_json::json;

    use super::{UsageSnapshot, UsageStats};

    #[test]
    fn starts_with_a_zero_snapshot() {
        let stats = UsageStats::new();

        assert_eq!(
            stats.snapshot(),
            UsageSnapshot {
                total_requests: 0,
                successful_requests: 0,
                failed_requests: 0,
                last_request_at_ms: None,
            }
        );
    }

    #[test]
    fn records_a_successful_request() {
        let stats = UsageStats::new();

        stats.begin(1_700_000_000_123);
        stats.record_success();

        assert_eq!(
            stats.snapshot(),
            UsageSnapshot {
                total_requests: 1,
                successful_requests: 1,
                failed_requests: 0,
                last_request_at_ms: Some(1_700_000_000_123),
            }
        );
    }

    #[test]
    fn records_a_failed_request() {
        let stats = UsageStats::new();

        stats.begin(1_700_000_000_456);
        stats.record_failure();

        assert_eq!(
            stats.snapshot(),
            UsageSnapshot {
                total_requests: 1,
                successful_requests: 0,
                failed_requests: 1,
                last_request_at_ms: Some(1_700_000_000_456),
            }
        );
    }

    #[test]
    fn keeps_the_latest_request_timestamp_when_updates_arrive_out_of_order() {
        let stats = UsageStats::new();

        stats.begin(2_000);
        stats.begin(1_000);
        stats.begin(3_000);

        assert_eq!(stats.snapshot().last_request_at_ms, Some(3_000));
    }

    #[test]
    fn snapshot_serializes_using_camel_case_fields() {
        let stats = UsageStats::new();

        stats.begin(42);
        stats.record_success();

        assert_eq!(
            serde_json::to_value(stats.snapshot()).expect("snapshot should serialize"),
            json!({
                "totalRequests": 1,
                "successfulRequests": 1,
                "failedRequests": 0,
                "lastRequestAtMs": 42,
            })
        );
    }

    #[test]
    fn concurrent_updates_preserve_the_total_request_count() {
        const THREADS: usize = 8;
        const REQUESTS_PER_THREAD: usize = 1_000;
        let stats = Arc::new(UsageStats::new());

        let handles = (0..THREADS)
            .map(|thread_id| {
                let stats = Arc::clone(&stats);
                thread::spawn(move || {
                    for request_id in 0..REQUESTS_PER_THREAD {
                        stats.begin((thread_id * REQUESTS_PER_THREAD + request_id + 1) as u64);
                        if request_id % 2 == 0 {
                            stats.record_success();
                        } else {
                            stats.record_failure();
                        }
                    }
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().expect("worker thread should finish");
        }

        let snapshot = stats.snapshot();
        assert_eq!(
            snapshot.total_requests,
            (THREADS * REQUESTS_PER_THREAD) as u64
        );
        assert_eq!(
            snapshot.total_requests,
            snapshot.successful_requests + snapshot.failed_requests
        );
    }
}
