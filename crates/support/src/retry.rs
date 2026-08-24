//! Explicit-factory retry primitives for replayable request bodies (main
//! spec §31; DECISIONS D-impl-retry).
//!
//! Reqwest streaming bodies are one-shot, so generated clients NEVER retry
//! implicitly. Every operation whose request carries streaming content
//! gains a `<op>_replaying(body_factory, policy)` twin that rebuilds the
//! body per attempt through the caller-supplied factory; only failures
//! classified retryable by [`is_retryable_transport`] — strictly PRE-
//! response transport faults — are retried, and backoff sleeps separate the
//! attempts ([`backoff_sleep`], deterministic: no jitter). Once response
//! headers arrive the outcome is final, whatever the status code.
//!
//! Idempotency is the caller's responsibility: PUT-style operations are
//! natural fits; retrying POST may duplicate effects.

/// How hard a `_replaying` client method tries (§31/D-impl-retry).
///
/// `max_attempts` counts EVERY attempt including the first, so `1` disables
/// retries entirely ([`RetryPolicy::none`]). Backoff grows exponentially
/// from `initial_backoff_ms`, doubling per failed attempt and capped at
/// `max_backoff_ms`; jitter is deliberately OFF so tests stay deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts allowed, including the first. Values of `0` and `1`
    /// both mean a single attempt.
    pub max_attempts: u32,
    /// Sleep before the SECOND attempt (after the first failure), in
    /// milliseconds.
    pub initial_backoff_ms: u64,
    /// Upper bound for any single inter-attempt sleep, in milliseconds.
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    /// The single-attempt policy: behave exactly like the base method with
    /// no retries and no sleeps.
    #[must_use]
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
        }
    }

    /// True when more than one attempt may be made.
    #[must_use]
    pub fn is_retry_enabled(&self) -> bool {
        self.max_attempts > 1
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::none()
    }
}

/// True ONLY for pre-response transport failures (§31/D-impl-retry): faults
/// that aborted the exchange BEFORE any response headers arrived, where the
/// request's server-side effects most plausibly never started. Every
/// accessor decision:
///
/// - [`reqwest::Error::status`] is the authoritative gate: once headers
///   arrived the outcome is final (even a 5xx must never be implicitly
///   retried — the request already took effect), so `Some(_)` → `false`.
/// - [`reqwest::Error::is_redirect`] → `false`: a redirect-policy outcome
///   is protocol behavior (§30.1), not a transport fault to paper over.
/// - [`reqwest::Error::is_builder`] → `false`: client-construction misuse
///   would fail identically on every attempt.
/// - [`reqwest::Error::is_upgrade`] → `false`: upgrades concern responses
///   that already arrived.
/// - [`reqwest::Error::is_connect`] → `true`: connection establishment
///   failed; nothing was sent.
/// - [`reqwest::Error::is_timeout`] → `true` (under the `status().is_none()`
///   gate): the connect/request timeout fired before headers arrived.
/// - [`reqwest::Error::is_request`] / [`reqwest::Error::is_body`] → `true`
///   under the same gate: request setup or BODY-WRITE failures that aborted
///   before any response headers. These are retryable only in a
///   `_replaying` twin whose factory rebuilds the body each attempt
///   (`openapi_support::retry` cannot see the body source; callers of this
///   predicate guarantee it) — base methods never consult it.
/// - [`reqwest::Error::is_decode`] → unreachable after the gate in practice
///   (decode errors imply a received response); conservatively excluded by
///   falling through to `false` unless another accessor matched.
#[must_use]
pub fn is_retryable_transport(error: &::reqwest::Error) -> bool {
    // §31: once response headers arrived, the attempt is final regardless
    // of status code — no implicit retries of executed requests.
    if error.status().is_some() {
        return false;
    }
    // Protocol/policy outcomes and construction misuse never retry.
    if error.is_redirect() || error.is_builder() || error.is_upgrade() {
        return false;
    }
    // Pre-response transport faults only: connection establishment,
    // connect/request timeouts, and request/body-write aborts.
    error.is_connect() || error.is_request() || error.is_body() || error.is_timeout()
}

/// Deterministic exponential backoff delay before the retry following
/// `failed_attempt` (1-based count of the failure just observed): the first
/// failure waits `initial_backoff_ms`, each subsequent failure doubles the
/// previous delay, capped at `max_backoff_ms`. Jitter is OFF (§50
/// determinism; tests assert exact values).
#[must_use]
pub fn backoff_delay_ms(policy: RetryPolicy, failed_attempt: u32) -> u64 {
    let doublings = u64::from(failed_attempt.saturating_sub(1)).min(63);
    policy
        .initial_backoff_ms
        .saturating_mul(1_u64 << doublings)
        .min(policy.max_backoff_ms)
}

/// Sleeps the [`backoff_delay_ms`] for `failed_attempt` between replay
/// attempts. A no-op wait when the policy disables retries.
pub async fn backoff_sleep(policy: RetryPolicy, failed_attempt: u32) {
    let delay = backoff_delay_ms(policy, failed_attempt);
    if delay == 0 {
        return;
    }
    ::tokio::time::sleep(::std::time::Duration::from_millis(delay)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_policy_is_single_attempt() {
        let policy = RetryPolicy::none();
        assert_eq!(policy.max_attempts, 1);
        assert!(!policy.is_retry_enabled());
        assert_eq!(policy, RetryPolicy::default());
    }

    #[test]
    fn multi_attempt_policy_reports_enabled() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 10,
            max_backoff_ms: 100,
        };
        assert!(policy.is_retry_enabled());
        assert_eq!(backoff_delay_ms(policy, 1), 10);
        assert_eq!(backoff_delay_ms(policy, 2), 20);
        assert_eq!(backoff_delay_ms(policy, 3), 40);
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let policy = RetryPolicy {
            max_attempts: 8,
            initial_backoff_ms: 5,
            max_backoff_ms: 40,
        };
        assert_eq!(backoff_delay_ms(policy, 1), 5);
        assert_eq!(backoff_delay_ms(policy, 2), 10);
        assert_eq!(backoff_delay_ms(policy, 3), 20);
        assert_eq!(backoff_delay_ms(policy, 4), 40);
        assert_eq!(backoff_delay_ms(policy, 9), 40, "capped at max_backoff_ms");
        // Zero-initial policies sleep zero regardless of attempts.
        assert_eq!(backoff_delay_ms(RetryPolicy::none(), 7), 0);
    }

    #[tokio::test]
    async fn backoff_sleep_with_none_policy_returns_promptly() {
        let started = std::time::Instant::now();
        backoff_sleep(RetryPolicy::none(), 3).await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(50),
            "a disabled policy must not sleep"
        );
    }
}
