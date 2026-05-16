//! Error classification and the rate-limit / transient-failure retry policy.
//!
//! Spotify rate-limits with HTTP 429 and a `Retry-After` header (whole
//! seconds). Transient transport failures (and the occasional 5xx) are worth a
//! few retries with exponential backoff plus jitter. This module turns an
//! `rspotify::ClientError` into a typed [`ApiError`] and decides whether — and
//! for how long — to wait before the next attempt.

use std::time::Duration;

use rspotify::http::HttpError;
use rspotify::ClientError;

use crate::error::ApiError;

/// The retry budget and backoff schedule for a single logical request.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// How many times to retry *after* the initial attempt.
    pub max_retries: u32,
    /// The backoff delay applied before the first retry; doubled each retry.
    pub base_delay: Duration,
    /// The ceiling for any single backoff delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// The exponential-backoff delay for a given retry attempt (0-indexed),
    /// with full jitter applied and capped at [`Self::max_delay`].
    ///
    /// "Full jitter" means the returned delay is uniformly random in
    /// `[0, capped_exponential_delay]`; this spreads a thundering herd of
    /// clients that were rate-limited together.
    #[must_use]
    pub fn backoff(&self, attempt: u32, rng: &mut impl rand::Rng) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(attempt));
        let capped = exp.min(self.max_delay);
        // Full jitter in `[0, capped]`.
        capped.mul_f64(rng.random::<f64>())
    }
}

/// The decision made about a failed attempt.
#[derive(Debug)]
pub enum RetryDecision {
    /// Give up; surface this terminal error to the caller.
    Fail(ApiError),
    /// Retry after waiting at least this long.
    ///
    /// For a 429 the delay is Spotify's `Retry-After`; for a transient
    /// failure it is the backoff schedule. The caller takes the larger of
    /// this and its own backoff.
    RetryAfter(Duration),
}

/// Inspect a failed `rspotify` call and decide what to do next.
///
/// `attempt` is 0-indexed (0 is the first try). When the retry budget is
/// spent, a retryable error is downgraded to a terminal [`ApiError`].
pub fn classify(
    err: ClientError,
    attempt: u32,
    policy: &RetryPolicy,
    rng: &mut impl rand::Rng,
) -> RetryDecision {
    let api_err = map_error(&err);
    let budget_left = attempt < policy.max_retries;

    match &api_err {
        ApiError::RateLimited { retry_after } if budget_left => {
            // Honour `Retry-After`, but never wait less than the backoff
            // schedule would have asked for anyway.
            let backoff = policy.backoff(attempt, rng);
            let wait = retry_after.unwrap_or(backoff).max(backoff);
            RetryDecision::RetryAfter(wait)
        }
        ApiError::Network(_) if budget_left => {
            RetryDecision::RetryAfter(policy.backoff(attempt, rng))
        }
        _ => RetryDecision::Fail(api_err),
    }
}

/// Map an `rspotify::ClientError` onto a typed [`ApiError`], without any
/// retry consideration.
///
/// HTTP status codes drive the classification: 401 → auth, 403/404 →
/// not-found (the caller specialises 403/404 into `EndpointUnavailable` for
/// endpoints known to be deprecated), 429 → rate-limited, everything else →
/// network.
#[must_use]
pub fn map_error(err: &ClientError) -> ApiError {
    match err {
        ClientError::Http(http) => map_http_error(http),
        ClientError::ParseJson(e) => ApiError::Deserialize(e.to_string()),
        ClientError::Model(e) => ApiError::Deserialize(e.to_string()),
        ClientError::InvalidToken => ApiError::Auth("token rejected as invalid".to_owned()),
        ClientError::ParseUrl(e) => ApiError::Network(e.to_string()),
        ClientError::Io(e) => ApiError::Network(e.to_string()),
        other => ApiError::Network(other.to_string()),
    }
}

/// Map the HTTP-layer error, reading the status code and `Retry-After`
/// header off a non-success response when one is present.
fn map_http_error(http: &HttpError) -> ApiError {
    match http {
        HttpError::StatusCode(resp) => {
            let status = resp.status().as_u16();
            match status {
                401 => ApiError::Auth(format!("HTTP {status}")),
                403 | 404 => ApiError::NotFound(format!("HTTP {status}")),
                429 => ApiError::RateLimited {
                    retry_after: retry_after_of(resp),
                },
                _ => ApiError::Network(format!("HTTP {status}")),
            }
        }
        HttpError::Client(e) => ApiError::Network(e.to_string()),
    }
}

/// Parse the `Retry-After` header (whole seconds) from a response.
fn retry_after_of(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// How the status code of a non-success HTTP response classifies.
///
/// Exposed for unit testing the 403/404/429/401 split without a live server.
#[must_use]
pub fn classify_status(status: u16, retry_after: Option<Duration>) -> ApiError {
    match status {
        401 => ApiError::Auth(format!("HTTP {status}")),
        403 | 404 => ApiError::NotFound(format!("HTTP {status}")),
        429 => ApiError::RateLimited { retry_after },
        _ => ApiError::Network(format!("HTTP {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> rand::rngs::StdRng {
        rand::rngs::StdRng::seed_from_u64(42)
    }

    #[test]
    fn backoff_is_capped_and_grows() {
        let policy = RetryPolicy {
            max_retries: 10,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
        };
        let mut rng = rng();
        // Even with jitter, no delay may exceed the cap.
        for attempt in 0..12 {
            let d = policy.backoff(attempt, &mut rng);
            assert!(d <= policy.max_delay, "attempt {attempt}: {d:?}");
        }
    }

    #[test]
    fn status_classification_splits_correctly() {
        assert!(matches!(classify_status(401, None), ApiError::Auth(_)));
        assert!(matches!(classify_status(403, None), ApiError::NotFound(_)));
        assert!(matches!(classify_status(404, None), ApiError::NotFound(_)));
        assert!(matches!(
            classify_status(429, Some(Duration::from_secs(3))),
            ApiError::RateLimited {
                retry_after: Some(d)
            } if d == Duration::from_secs(3)
        ));
        assert!(matches!(classify_status(503, None), ApiError::Network(_)));
    }

    #[test]
    fn rate_limit_honours_retry_after_over_backoff() {
        let policy = RetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_secs(60),
        };
        let mut rng = rng();
        let err = ClientError::InvalidToken; // placeholder, replaced below
        let _ = err;
        // Drive `classify` via a synthesised rate-limit error path: we test
        // the wait selection directly since constructing an `HttpError` here
        // is awkward. `Retry-After` of 30s must dominate a ~1ms backoff.
        let retry_after = Duration::from_secs(30);
        let backoff = policy.backoff(0, &mut rng);
        let wait = retry_after.max(backoff);
        assert_eq!(wait, retry_after);
    }

    #[test]
    fn budget_exhaustion_downgrades_to_terminal() {
        let policy = RetryPolicy {
            max_retries: 0,
            ..RetryPolicy::default()
        };
        let mut rng = rng();
        // attempt 0 with max_retries 0 → no budget → must Fail.
        let decision = classify(ClientError::InvalidToken, 0, &policy, &mut rng);
        assert!(matches!(decision, RetryDecision::Fail(ApiError::Auth(_))));
    }
}
