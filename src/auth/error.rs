use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Missing authentication token")]
    MissingToken,
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Invalid claims: {0}")]
    InvalidClaims(String),
    /// Claims enrichment against the IdP userinfo endpoint failed — the
    /// presented token cannot be resolved to a principal with authorization
    /// attributes. This is an AUTHENTICATION failure and must answer 401
    /// with an explicit retry signal, never 403-with-defaults.
    ///
    /// b82a1925 guard-1: the previous behavior logged the enrichment error
    /// as "non-fatal" and let a role-less principal fall through to a
    /// `role="viewer"` default — a silent downgrade that default-denied as a
    /// lying 403 and hid recurring token age-out windows for days.
    ///
    /// Wire contract (spec-fixed, not left to taste):
    /// - status 401
    /// - `WWW-Authenticate: Bearer error="invalid_token"` (RFC 6750)
    /// - JSON body: `{"error":"token_expired","retryable":true,"detail":…}`
    ///   so a client can mechanically distinguish retryable authn failure
    ///   from a real authz denial (403 is NEVER retryable).
    /// - occurrence journaled at WARN with the underlying reason.
    #[error("Token enrichment failed: {0}")]
    EnrichmentFailed(String),
    #[error("PEP error: {0}")]
    PepError(#[from] pep::error::PepError),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl AuthError {
    /// The `WWW-Authenticate` challenge value carried by every 401 this
    /// service emits. `error="invalid_token"` is constant per the wire
    /// contract; `error_description` carries the specific reason.
    fn www_authenticate(reason: &str) -> String {
        format!(
            "Bearer error=\"invalid_token\", error_description=\"{}\"",
            reason.replace('"', "'")
        )
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match self {
            AuthError::EnrichmentFailed(reason) => {
                let body = serde_json::json!({
                    // Uniform machine-readable contract: any 401 from this
                    // service is retryable-exactly-once; `detail` preserves
                    // the underlying cause (invalid_token, IdP outage, …)
                    // for operators without changing client handling.
                    "error": "token_expired",
                    "retryable": true,
                    "detail": reason,
                });
                let mut resp = (StatusCode::UNAUTHORIZED, Json(body)).into_response();
                resp.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    Self::www_authenticate("token enrichment failed")
                        .parse()
                        .expect("static header value"),
                );
                resp
            }
            AuthError::MissingToken | AuthError::InvalidToken(_) => {
                let mut resp = (StatusCode::UNAUTHORIZED, self.to_string()).into_response();
                resp.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    Self::www_authenticate("missing or invalid bearer token")
                        .parse()
                        .expect("static header value"),
                );
                resp
            }
            AuthError::InvalidClaims(_) => {
                (StatusCode::FORBIDDEN, self.to_string()).into_response()
            }
            AuthError::PepError(ref e) => (e.status_code(), self.to_string()).into_response(),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::HttpBody;

    /// b82a1925 guard-1 wire contract: enrichment failure answers
    /// 401 + WWW-Authenticate + machine-readable retryable body —
    /// mechanically distinguishable from a real 403 authz denial.
    #[test]
    fn enrichment_failure_is_401_with_retry_signal() {
        let resp = AuthError::EnrichmentFailed(
            "Userinfo endpoint returned error: {\"error\":\"invalid_token\"} status=400"
                .to_string(),
        )
        .into_response();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let www = resp
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .expect("WWW-Authenticate header required")
            .to_str()
            .unwrap();
        assert!(www.starts_with("Bearer error=\"invalid_token\""), "{www}");

        let body = resp.into_body();
        // Body must be the machine-readable JSON (not the Display string).
        let bytes = futures::executor::block_on(axum::body::to_bytes(
            axum::body::Body::new(body),
            4096,
        ))
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON body");
        assert_eq!(v["error"], "token_expired");
        assert_eq!(v["retryable"], true);
        assert!(v["detail"]
            .as_str()
            .unwrap()
            .contains("invalid_token"));
    }

    /// 403 must NEVER appear on the enrichment-failure path — that is the
    /// lying status that masked this bug class (b82a1925).
    #[test]
    fn authz_denial_stays_403_and_enrichment_never_is() {
        let claims_resp = AuthError::InvalidClaims("no permit".to_string()).into_response();
        assert_eq!(claims_resp.status(), StatusCode::FORBIDDEN);

        let enrich_resp = AuthError::EnrichmentFailed("x".to_string()).into_response();
        assert_ne!(enrich_resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn missing_token_carries_challenge_header() {
        let resp = AuthError::MissingToken.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(resp.headers().get(header::WWW_AUTHENTICATE).is_some());
    }
}
