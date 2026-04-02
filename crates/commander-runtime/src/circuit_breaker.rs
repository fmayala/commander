use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::adapter::{AdapterError, LlmAdapter, LlmRequest, LlmResponse};

/// State of the circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Tripped — requests are rejected immediately without calling the inner adapter.
    Open,
    /// Cooldown elapsed — one probe request is allowed through.
    HalfOpen,
}

struct Inner {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

/// A circuit breaker wrapper around any `LlmAdapter`.
///
/// Trips to `Open` after `failure_threshold` consecutive `Api` or `Network` errors.
/// After `cooldown` elapses, transitions to `HalfOpen` to probe with one request.
/// On probe success → `Closed`; on probe failure → `Open` again.
///
/// `RateLimited` and `ContextTooLarge` errors do NOT trip the breaker.
pub struct CircuitBreaker<A> {
    inner: A,
    state: Arc<Mutex<Inner>>,
    failure_threshold: u32,
    cooldown: Duration,
}

impl<A: LlmAdapter> CircuitBreaker<A> {
    pub fn new(inner: A, failure_threshold: u32, cooldown: Duration) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(Inner {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                opened_at: None,
            })),
            failure_threshold,
            cooldown,
        }
    }

    /// Returns the current circuit state.
    pub fn circuit_state(&self) -> CircuitState {
        self.state.lock().unwrap().state
    }
}

#[async_trait]
impl<A: LlmAdapter> LlmAdapter for CircuitBreaker<A> {
    async fn complete(&self, request: LlmRequest<'_>) -> Result<LlmResponse, AdapterError> {
        // Determine whether to allow this call, transitioning Open→HalfOpen if cooldown elapsed.
        let allow = {
            let mut guard = self.state.lock().unwrap();
            match guard.state {
                CircuitState::Closed | CircuitState::HalfOpen => true,
                CircuitState::Open => {
                    if guard
                        .opened_at
                        .map(|t| t.elapsed() >= self.cooldown)
                        .unwrap_or(false)
                    {
                        guard.state = CircuitState::HalfOpen;
                        true
                    } else {
                        false
                    }
                }
            }
        };

        if !allow {
            return Err(AdapterError::Api(
                "circuit breaker open — adapter temporarily unavailable".to_string(),
            ));
        }

        let result = self.inner.complete(request).await;

        {
            let mut guard = self.state.lock().unwrap();
            match &result {
                Ok(_) => {
                    guard.consecutive_failures = 0;
                    guard.state = CircuitState::Closed;
                    guard.opened_at = None;
                }
                Err(e) => {
                    let trips = matches!(e, AdapterError::Api(_) | AdapterError::Network(_));
                    if trips {
                        guard.consecutive_failures += 1;
                        let should_open = guard.state == CircuitState::HalfOpen
                            || guard.consecutive_failures >= self.failure_threshold;
                        if should_open {
                            guard.state = CircuitState::Open;
                            guard.opened_at = Some(Instant::now());
                        }
                    }
                }
            }
        }

        result
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn context_window(&self) -> u32 {
        self.inner.context_window()
    }

    fn max_output_tokens(&self) -> u32 {
        self.inner.max_output_tokens()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commander_messages::TokenUsage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::adapter::StopReason;

    struct MockAdapter {
        responses: Mutex<Vec<Result<LlmResponse, AdapterError>>>,
        pub call_count: Arc<AtomicUsize>,
    }

    impl MockAdapter {
        fn new(responses: Vec<Result<LlmResponse, AdapterError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl LlmAdapter for MockAdapter {
        async fn complete(&self, _request: LlmRequest<'_>) -> Result<LlmResponse, AdapterError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Err(AdapterError::Api("no more responses".to_string()))
            } else {
                responses.remove(0)
            }
        }

        fn model_id(&self) -> &str {
            "mock"
        }
        fn context_window(&self) -> u32 {
            4096
        }
        fn max_output_tokens(&self) -> u32 {
            1024
        }
    }

    fn ok_response() -> Result<LlmResponse, AdapterError> {
        Ok(LlmResponse {
            content: vec![],
            usage: TokenUsage::default(),
            stop_reason: StopReason::EndTurn,
        })
    }

    fn api_error() -> Result<LlmResponse, AdapterError> {
        Err(AdapterError::Api("server error".to_string()))
    }

    fn make_request() -> LlmRequest<'static> {
        LlmRequest {
            messages: &[],
            system_prompt: None,
            tools: &[],
            max_tokens: 100,
        }
    }

    #[tokio::test]
    async fn trips_open_after_threshold_failures() {
        let cb = CircuitBreaker::new(
            MockAdapter::new(vec![api_error(), api_error(), api_error()]),
            3,
            Duration::from_secs(60),
        );

        assert_eq!(cb.circuit_state(), CircuitState::Closed);
        for _ in 0..3 {
            let _ = cb.complete(make_request()).await;
        }
        assert_eq!(cb.circuit_state(), CircuitState::Open);
    }

    #[tokio::test]
    async fn open_circuit_rejects_without_calling_inner() {
        let mock = MockAdapter::new(vec![api_error(), api_error(), api_error()]);
        let call_count = mock.call_count.clone();
        let cb = CircuitBreaker::new(mock, 3, Duration::from_secs(60));

        // Trip the breaker
        for _ in 0..3 {
            let _ = cb.complete(make_request()).await;
        }
        assert_eq!(cb.circuit_state(), CircuitState::Open);

        let before = call_count.load(Ordering::SeqCst);
        let result = cb.complete(make_request()).await;
        // Rejected without calling inner
        assert!(matches!(result, Err(AdapterError::Api(_))));
        assert_eq!(call_count.load(Ordering::SeqCst), before);
    }

    #[tokio::test]
    async fn half_open_probe_success_closes_circuit() {
        let cb = CircuitBreaker::new(
            MockAdapter::new(vec![
                api_error(),
                api_error(),
                api_error(),
                ok_response(),
            ]),
            3,
            Duration::from_millis(1),
        );

        // Trip the breaker
        for _ in 0..3 {
            let _ = cb.complete(make_request()).await;
        }
        assert_eq!(cb.circuit_state(), CircuitState::Open);

        tokio::time::sleep(Duration::from_millis(5)).await;

        let result = cb.complete(make_request()).await;
        assert!(result.is_ok());
        assert_eq!(cb.circuit_state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn half_open_probe_failure_reopens_circuit() {
        let cb = CircuitBreaker::new(
            MockAdapter::new(vec![
                api_error(),
                api_error(),
                api_error(),
                api_error(), // probe fails
            ]),
            3,
            Duration::from_millis(1),
        );

        // Trip the breaker
        for _ in 0..3 {
            let _ = cb.complete(make_request()).await;
        }
        assert_eq!(cb.circuit_state(), CircuitState::Open);

        tokio::time::sleep(Duration::from_millis(5)).await;

        let result = cb.complete(make_request()).await;
        assert!(result.is_err());
        assert_eq!(cb.circuit_state(), CircuitState::Open);
    }

    #[tokio::test]
    async fn rate_limited_does_not_trip_circuit() {
        let cb = CircuitBreaker::new(
            MockAdapter::new(vec![
                Err(AdapterError::RateLimited {
                    retry_after_ms: 100,
                }),
                Err(AdapterError::RateLimited {
                    retry_after_ms: 100,
                }),
                Err(AdapterError::RateLimited {
                    retry_after_ms: 100,
                }),
            ]),
            3,
            Duration::from_secs(60),
        );

        for _ in 0..3 {
            let _ = cb.complete(make_request()).await;
        }
        // RateLimited should not trip the breaker
        assert_eq!(cb.circuit_state(), CircuitState::Closed);
    }
}
