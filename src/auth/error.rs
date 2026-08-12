use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
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
    #[error("PEP error: {0}")]
    PepError(#[from] pep::error::PepError),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match self {
            AuthError::MissingToken | AuthError::InvalidToken(_) => {
                (StatusCode::UNAUTHORIZED, self.to_string()).into_response()
            }
            AuthError::InvalidClaims(_) => {
                (StatusCode::FORBIDDEN, self.to_string()).into_response()
            }
            AuthError::PepError(ref e) => {
                (e.status_code(), self.to_string()).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response(),
        }
    }
}
