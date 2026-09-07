use super::{AuthConfig, AuthError, ForwardedToken, InstanceContext};
use axum::{
    extract::Request,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures::future::BoxFuture;
use pep::{
    oidc::types::{JwtClaims, JwtValidationOptions},
    oidc_resource_server::ResourceServerClient,
};
use std::{
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct AuthLayer {
    config: AuthConfig,
    client: Arc<ResourceServerClient>,
}

impl AuthLayer {
    pub fn new(config: AuthConfig, client: ResourceServerClient) -> Self {
        Self {
            config,
            client: Arc::new(client),
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            config: self.config.clone(),
            client: self.client.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    config: AuthConfig,
    client: Arc<ResourceServerClient>,
}

impl<S> Service<Request> for AuthMiddleware<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let config = self.config.clone();
        let client = self.client.clone();
        let mut inner = self.inner.clone();

        // Extract instance context from headers and store in extensions
        let instance_id = req
            .headers()
            .get("X-Instance-Id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        req.extensions_mut().insert(InstanceContext { instance_id });

        Box::pin(async move {
            if !config.enabled {
                // If auth is disabled, we inject mock claims if dev_mode is true
                if config.dev_mode {
                    inject_mock_claims(&mut req);
                }
                req.extensions_mut().insert(ForwardedToken(None));
                tracing::info!("Auth disabled, passing through");
                return inner.call(req).await;
            }

            let token = match extract_bearer_token(req.headers()) {
                Some(t) => {
                    tracing::debug!("Auth: extracted Bearer token (len={})", t.len());
                    req.extensions_mut().insert(ForwardedToken(Some(t.clone())));
                    t
                }
                None => {
                    tracing::warn!("Auth: no Bearer token found in request");
                    if config.dev_mode {
                        tracing::info!("Auth: dev_mode enabled, injecting mock claims");
                        inject_mock_claims(&mut req);
                        req.extensions_mut().insert(ForwardedToken(None));
                        return inner.call(req).await;
                    }
                    return Ok(AuthError::MissingToken.into_response());
                }
            };

            let mut options = JwtValidationOptions::default();
            let audience = if let Some(aud) = &config.expected_audience {
                aud.as_str()
            } else {
                options.skip_audience_validation = true;
                ""
            };

            match client
                .validate_jwt_with_options(&token, &config.issuer_url, audience, &options)
                .await
            {
                Ok(mut claims) => {
                    tracing::debug!(
                        "Auth: JWT validated successfully sub={} exp={}",
                        claims.sub,
                        claims.exp
                    );
                    // Adaptive claims enrichment: fill missing groups/role from OIDC userinfo
                    // This makes NGHR work with Kanidm (no groups in AT) and Keycloak/Auth0 (groups in AT)
                    //
                    // b82a1925 guard-1 (401-fail-loud): enrichment failure means the
                    // token cannot be resolved to a principal with authorization
                    // attributes. That is an AUTHENTICATION failure → 401 with an
                    // explicit retry signal, journaled at WARN. The previous behavior
                    // logged it as "non-fatal" and inserted role-less claims, which
                    // the extractor then defaulted to role="viewer" → Cedar
                    // default-deny → a silent 403 that lied about authz and masked
                    // recurring token age-out windows.
                    if let Err(e) = client
                        .enrich_claims_with_userinfo(
                            &mut claims,
                            &token,
                            &config.issuer_url,
                            config.userinfo_url.as_deref(),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Auth: userinfo enrichment FAILED — answering 401 with retry \
                             signal (default-role fallback removed, request fails loudly): {}",
                            e
                        );
                        return Ok(AuthError::EnrichmentFailed(format!("{}", e)).into_response());
                    }
                    req.extensions_mut().insert(claims);
                    inner.call(req).await
                }
                Err(e) => {
                    tracing::error!("Auth: JWT validation FAILED: {}", e);
                    Ok(AuthError::PepError(e).into_response())
                }
            }
        })
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(|token| token.to_string()))
}

fn inject_mock_claims(req: &mut Request) {
    let mut extra = std::collections::HashMap::new();
    extra.insert(
        "role".to_string(),
        serde_json::Value::String("admin".to_string()),
    );
    extra.insert(
        "groups".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::String("developers".to_string())]),
    );

    let claims = JwtClaims {
        sub: "dev-user-id".to_string(),
        iss: "dev-issuer".to_string(),
        aud: Some("dev-audience".to_string()),
        exp: 9999999999,
        iat: Some(0),
        email: Some("dev@example.com".to_string()),
        name: Some("Dev User".to_string()),
        preferred_username: Some("dev-user".to_string()),
        extra,
    };
    req.extensions_mut().insert(claims);
}
