use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{HeaderMap, HeaderValue, header::AUTHORIZATION, request::Parts},
    middleware::Next,
    response::Response,
};
use base64::{Engine, engine::general_purpose};
use chrono::Utc;

use crate::{
    domain::{
        api_token::ApiTokenScope,
        user::{PublicUser, UserRole},
    },
    error::ApiError,
    repositories::{api_tokens, audit, browser_sessions, users},
    services::{
        api_tokens as api_token_service,
        auth_rate_limit::{RateLimitOutcome, RateLimitRejection},
        password::PasswordServiceError,
    },
    state::AppState,
};

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    user: PublicUser,
    pub(super) verified_password_hash: Option<String>,
}

impl AuthenticatedUser {
    pub fn public_user(&self) -> &PublicUser {
        &self.user
    }

    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.user.role.is_admin() {
            return Ok(());
        }

        Err(ApiError::forbidden("admin role required"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Basic,
    Bearer,
}

impl AuthMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Bearer => "bearer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminUser(AuthenticatedUser);

impl AdminUser {
    pub fn public_user(&self) -> &PublicUser {
        self.0.public_user()
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;
        user.require_admin()?;
        Ok(Self(user))
    }
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let existing_user = parts.extensions.get::<AuthenticatedUser>().cloned();
        let authorization = parts.headers.get(AUTHORIZATION).cloned();
        let context = AuthRequestContext::from_request(
            &parts.headers,
            parts.extensions.get::<ConnectInfo<SocketAddr>>(),
            &state.config().auth,
        );
        let state = state.clone();

        async move {
            if let Some(user) = existing_user {
                return Ok(user);
            }

            authenticate(&state, authorization.as_ref(), &context).await
        }
    }
}

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let authorization = request.headers().get(AUTHORIZATION).cloned();
    let context = AuthRequestContext::from_request(
        request.headers(),
        request.extensions().get::<ConnectInfo<SocketAddr>>(),
        &state.config().auth,
    );
    let user = authenticate(&state, authorization.as_ref(), &context).await?;

    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

async fn authenticate(
    state: &AppState,
    authorization: Option<&HeaderValue>,
    context: &AuthRequestContext,
) -> Result<AuthenticatedUser, ApiError> {
    let Some(parsed_authorization) = authorization.and_then(parse_authorization) else {
        return fail_authentication(
            state,
            AuthFailure {
                method: None,
                username: None,
                token_prefix: None,
                reason: "missing_or_invalid_authorization",
                rate_limit_key: invalid_authorization_rate_limit_key(context),
                context,
                actor_user_id: None,
            },
        )
        .await;
    };

    let basic_credentials_too_long = match &parsed_authorization {
        ParsedAuthorization::Basic(credentials) => {
            credentials.username.len() > state.config().auth.max_username_bytes
                || credentials.password.len() > state.config().auth.max_password_bytes
        }
        ParsedAuthorization::Bearer(_) => false,
    };
    if basic_credentials_too_long {
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Basic),
                username: None,
                token_prefix: None,
                reason: "credentials_too_long",
                rate_limit_key: invalid_authorization_rate_limit_key(context),
                context,
                actor_user_id: None,
            },
        )
        .await;
    }

    let rate_limit_key = parsed_authorization.rate_limit_key(context);
    if let Some(rejection) =
        state
            .auth_rate_limiter()
            .check(&state.config().auth, &rate_limit_key, Utc::now())
    {
        record_auth_failure(
            state,
            AuthFailureAudit {
                method: parsed_authorization.method(),
                username: parsed_authorization.username(),
                token_prefix: parsed_authorization.token_prefix().as_deref(),
                reason: "rate_limited",
                rate_limit_key: &rate_limit_key,
                context,
                actor_user_id: None,
                outcome: None,
                rejection: Some(&rejection),
            },
        )
        .await;

        return Err(ApiError::too_many_requests(
            "too many failed authentication attempts",
            rejection.retry_after_seconds,
        ));
    }

    match parsed_authorization {
        ParsedAuthorization::Basic(credentials) => {
            authenticate_basic(state, credentials, context, rate_limit_key).await
        }
        ParsedAuthorization::Bearer(credentials) => {
            authenticate_bearer(state, credentials, context, rate_limit_key).await
        }
    }
}

async fn authenticate_basic(
    state: &AppState,
    credentials: BasicCredentials,
    context: &AuthRequestContext,
    rate_limit_key: String,
) -> Result<AuthenticatedUser, ApiError> {
    let user = users::find_by_username(state.db(), &credentials.username)
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to look up user during authentication");
            ApiError::internal("authentication failed")
        })?;

    let Some(user) = user else {
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Basic),
                username: Some(credentials.username.as_str()),
                token_prefix: None,
                reason: "unknown_user",
                rate_limit_key,
                context,
                actor_user_id: None,
            },
        )
        .await;
    };

    let password_admission = state.password_service().try_admit().map_err(|error| {
        debug_assert!(matches!(error, PasswordServiceError::Busy));
        tracing::warn!(?error, "password verification queue is full");
        ApiError::too_many_requests("password verification is busy; retry shortly", 1)
    })?;
    let password_permit = state
        .password_service()
        .acquire(password_admission)
        .await
        .map_err(|error| {
            debug_assert!(error.is_capacity_error());
            tracing::warn!(?error, "password verification queue wait ended");
            ApiError::too_many_requests("password verification is busy; retry shortly", 1)
        })?;

    // A request can pass the first limiter check, wait behind an earlier failed
    // attempt, and acquire a worker after that attempt establishes a lockout.
    // Recheck after the bounded wait and before starting Argon2.
    if let Some(rejection) =
        state
            .auth_rate_limiter()
            .check(&state.config().auth, &rate_limit_key, Utc::now())
    {
        record_auth_failure(
            state,
            AuthFailureAudit {
                method: Some(AuthMethod::Basic),
                username: Some(credentials.username.as_str()),
                token_prefix: None,
                reason: "rate_limited",
                rate_limit_key: &rate_limit_key,
                context,
                actor_user_id: Some(user.id),
                outcome: None,
                rejection: Some(&rejection),
            },
        )
        .await;
        return Err(ApiError::too_many_requests(
            "too many failed authentication attempts",
            rejection.retry_after_seconds,
        ));
    }

    let password_valid = state
        .password_service()
        .verify_with_permit(password_permit, &credentials.password, &user.password_hash)
        .await
        .map_err(|error| {
            tracing::error!(?error, "password verification worker failed");
            ApiError::internal("authentication failed")
        })?;
    if !password_valid {
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Basic),
                username: Some(credentials.username.as_str()),
                token_prefix: None,
                reason: "invalid_password",
                rate_limit_key,
                context,
                actor_user_id: Some(user.id),
            },
        )
        .await;
    }

    state.auth_rate_limiter().record_success(&rate_limit_key);

    Ok(AuthenticatedUser {
        user: user.public(),
        verified_password_hash: Some(user.password_hash),
    })
}

async fn authenticate_bearer(
    state: &AppState,
    credentials: BearerCredentials,
    context: &AuthRequestContext,
    rate_limit_key: String,
) -> Result<AuthenticatedUser, ApiError> {
    let token_hash = api_token_service::hash_token(&credentials.token);
    if credentials.token.starts_with("teatro_session_") {
        let user = browser_sessions::find_active_user(state.db(), &token_hash)
            .await
            .map_err(|error| {
                tracing::error!(?error, "failed to look up browser session");
                ApiError::internal("authentication failed")
            })?;
        if let Some(user) = user {
            state.auth_rate_limiter().record_success(&rate_limit_key);
            return Ok(AuthenticatedUser {
                user,
                verified_password_hash: None,
            });
        }
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Bearer),
                username: None,
                token_prefix: Some(&credentials.token_prefix),
                reason: "invalid_browser_session",
                rate_limit_key,
                context,
                actor_user_id: None,
            },
        )
        .await;
    }
    let token = api_tokens::find_active_by_hash(state.db(), &token_hash)
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to look up API token during authentication");
            ApiError::internal("authentication failed")
        })?;

    let Some(token) = token else {
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Bearer),
                username: None,
                token_prefix: Some(credentials.token_prefix.as_str()),
                reason: "invalid_api_token",
                rate_limit_key,
                context,
                actor_user_id: None,
            },
        )
        .await;
    };

    if !token.has_scope(ApiTokenScope::Read) && !token.has_scope(ApiTokenScope::Admin) {
        return fail_authentication(
            state,
            AuthFailure {
                method: Some(AuthMethod::Bearer),
                username: Some(token.user.username.as_str()),
                token_prefix: Some(token.token_prefix.as_str()),
                reason: "api_token_without_usable_scope",
                rate_limit_key,
                context,
                actor_user_id: Some(token.user.id),
            },
        )
        .await;
    }

    api_tokens::touch_last_used_if_stale(
        state.db(),
        token.id,
        state.config().auth.token_touch_interval_seconds,
    )
    .await
    .map_err(|error| {
        tracing::error!(?error, "failed to update API token last_used_at");
        ApiError::internal("authentication failed")
    })?;

    state.auth_rate_limiter().record_success(&rate_limit_key);

    let mut user = token.user.clone();
    if !token.has_scope(ApiTokenScope::Admin) || !user.role.is_admin() {
        user.role = UserRole::ReadOnly;
    }

    Ok(AuthenticatedUser {
        user,
        verified_password_hash: None,
    })
}

async fn fail_authentication(
    state: &AppState,
    failure: AuthFailure<'_>,
) -> Result<AuthenticatedUser, ApiError> {
    let outcome = state.auth_rate_limiter().record_failure(
        &state.config().auth,
        failure.rate_limit_key.clone(),
        Utc::now(),
    );

    record_auth_failure(
        state,
        AuthFailureAudit {
            method: failure.method,
            username: failure.username,
            token_prefix: failure.token_prefix,
            reason: failure.reason,
            rate_limit_key: &failure.rate_limit_key,
            context: failure.context,
            actor_user_id: failure.actor_user_id,
            outcome: Some(&outcome),
            rejection: None,
        },
    )
    .await;

    if let Some(retry_after_seconds) = outcome.retry_after_seconds {
        return Err(ApiError::too_many_requests(
            "too many failed authentication attempts",
            retry_after_seconds,
        ));
    }

    Err(ApiError::unauthorized(
        "invalid username, password, or API token",
    ))
}

fn parse_authorization(header: &HeaderValue) -> Option<ParsedAuthorization> {
    let value = header.to_str().ok()?;
    let (scheme, credentials) = value.split_once(' ')?;
    let credentials = credentials.trim();

    if scheme.eq_ignore_ascii_case("Basic") {
        parse_basic_credentials_value(credentials).map(ParsedAuthorization::Basic)
    } else if scheme.eq_ignore_ascii_case("Bearer") {
        parse_bearer_credentials_value(credentials).map(ParsedAuthorization::Bearer)
    } else {
        None
    }
}

#[cfg(test)]
fn parse_basic_credentials(header: &HeaderValue) -> Option<BasicCredentials> {
    let ParsedAuthorization::Basic(credentials) = parse_authorization(header)? else {
        return None;
    };

    Some(credentials)
}

#[cfg(test)]
fn parse_bearer_credentials(header: &HeaderValue) -> Option<BearerCredentials> {
    let ParsedAuthorization::Bearer(credentials) = parse_authorization(header)? else {
        return None;
    };

    Some(credentials)
}

fn parse_basic_credentials_value(encoded: &str) -> Option<BasicCredentials> {
    let decoded = general_purpose::STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;

    if username.is_empty() {
        return None;
    }

    Some(BasicCredentials {
        username: username.to_string(),
        password: password.to_string(),
    })
}

fn parse_bearer_credentials_value(token: &str) -> Option<BearerCredentials> {
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }

    Some(BearerCredentials {
        token: token.to_string(),
        token_prefix: api_token_service::token_prefix(token),
    })
}

fn invalid_authorization_rate_limit_key(context: &AuthRequestContext) -> String {
    format!("invalid:{}", context.client_id)
}

async fn record_auth_failure(state: &AppState, audit: AuthFailureAudit<'_>) {
    let (failure_count, locked_until, retry_after_seconds) = if let Some(outcome) = audit.outcome {
        (
            Some(outcome.failure_count),
            outcome
                .locked_until
                .map(|locked_until| locked_until.to_rfc3339()),
            outcome.retry_after_seconds,
        )
    } else if let Some(rejection) = audit.rejection {
        (
            Some(rejection.failure_count),
            Some(rejection.locked_until.to_rfc3339()),
            Some(rejection.retry_after_seconds),
        )
    } else {
        (None, None, None)
    };

    let metadata_json = serde_json::json!({
        "auth_method": audit.method.map(AuthMethod::as_str),
        "username": audit.username,
        "token_prefix": audit.token_prefix,
        "reason": audit.reason,
        "client_id": audit.context.client_id,
        "rate_limit_key": audit.rate_limit_key,
        "failure_count": failure_count,
        "locked_until": locked_until,
        "retry_after_seconds": retry_after_seconds,
    })
    .to_string();

    if let Err(error) = audit::record(
        state.db(),
        audit::AuditEvent {
            actor_user_id: audit.actor_user_id,
            action: "auth.failed",
            entity_type: Some("user"),
            entity_id: audit.actor_user_id,
            metadata_json: Some(&metadata_json),
        },
    )
    .await
    {
        tracing::warn!(
            ?error,
            "failed to record authentication failure audit event"
        );
    }
}

#[derive(Debug, Clone)]
struct AuthRequestContext {
    client_id: String,
}

impl AuthRequestContext {
    fn from_request(
        headers: &HeaderMap,
        peer: Option<&ConnectInfo<SocketAddr>>,
        config: &crate::config::AuthConfig,
    ) -> Self {
        let peer_ip = peer.map(|ConnectInfo(address)| address.ip());
        let client_ip = peer_ip.map(|peer_ip| {
            if !config.trusted_proxy_ips.contains(&peer_ip) {
                return peer_ip;
            }

            match parse_forwarded_for(headers) {
                Ok(Some(forwarded_chain)) => {
                    let mut current = peer_ip;
                    for forwarded_ip in forwarded_chain.into_iter().rev() {
                        if !config.trusted_proxy_ips.contains(&current) {
                            break;
                        }
                        current = forwarded_ip;
                    }
                    current
                }
                Ok(None) => parse_single_ip_header(headers, "x-real-ip").unwrap_or(peer_ip),
                // A malformed chain is not allowed to fall through to a second,
                // potentially conflicting client header.
                Err(()) => peer_ip,
            }
        });
        let client_id = client_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Self { client_id }
    }
}

fn parse_forwarded_for(headers: &HeaderMap) -> Result<Option<Vec<std::net::IpAddr>>, ()> {
    let values = headers.get_all("x-forwarded-for");
    let mut addresses = Vec::new();
    let mut present = false;
    for value in values.iter() {
        present = true;
        let value = value.to_str().map_err(|_| ())?;
        for part in value.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err(());
            }
            addresses.push(part.parse().map_err(|_| ())?);
        }
    }

    if present && !addresses.is_empty() {
        Ok(Some(addresses))
    } else {
        Ok(None)
    }
}

fn parse_single_ip_header(headers: &HeaderMap, name: &str) -> Option<std::net::IpAddr> {
    let all_values = headers.get_all(name);
    let mut values = all_values.iter();
    let value = values.next()?.to_str().ok()?.trim().parse().ok()?;
    values.next().is_none().then_some(value)
}

struct AuthFailure<'a> {
    method: Option<AuthMethod>,
    username: Option<&'a str>,
    token_prefix: Option<&'a str>,
    reason: &'static str,
    rate_limit_key: String,
    context: &'a AuthRequestContext,
    actor_user_id: Option<i64>,
}

struct AuthFailureAudit<'a> {
    method: Option<AuthMethod>,
    username: Option<&'a str>,
    token_prefix: Option<&'a str>,
    reason: &'static str,
    rate_limit_key: &'a str,
    context: &'a AuthRequestContext,
    actor_user_id: Option<i64>,
    outcome: Option<&'a RateLimitOutcome>,
    rejection: Option<&'a RateLimitRejection>,
}

enum ParsedAuthorization {
    Basic(BasicCredentials),
    Bearer(BearerCredentials),
}

impl ParsedAuthorization {
    fn method(&self) -> Option<AuthMethod> {
        Some(match self {
            Self::Basic(_) => AuthMethod::Basic,
            Self::Bearer(_) => AuthMethod::Bearer,
        })
    }

    fn username(&self) -> Option<&str> {
        match self {
            Self::Basic(credentials) => Some(credentials.username.as_str()),
            Self::Bearer(_) => None,
        }
    }

    fn token_prefix(&self) -> Option<String> {
        match self {
            Self::Basic(_) => None,
            Self::Bearer(credentials) => Some(credentials.token_prefix.clone()),
        }
    }

    fn rate_limit_key(&self, context: &AuthRequestContext) -> String {
        match self {
            Self::Basic(credentials) => format!(
                "basic:{}:{}",
                credentials.username.to_ascii_lowercase(),
                context.client_id
            ),
            Self::Bearer(credentials) => {
                format!("bearer:{}:{}", credentials.token_prefix, context.client_id)
            }
        }
    }
}

struct BasicCredentials {
    username: String,
    password: String,
}

struct BearerCredentials {
    token: String,
    token_prefix: String,
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{
        extract::ConnectInfo,
        http::{HeaderMap, HeaderValue},
    };
    use base64::{Engine, engine::general_purpose};

    use crate::config::AuthConfig;

    use super::{AuthRequestContext, parse_basic_credentials, parse_bearer_credentials};

    #[test]
    fn parses_basic_auth_credentials() {
        let encoded = general_purpose::STANDARD.encode("admin:secret");
        let header = HeaderValue::from_str(&format!("Basic {encoded}")).unwrap();
        let credentials = parse_basic_credentials(&header).unwrap();

        assert_eq!(credentials.username, "admin");
        assert_eq!(credentials.password, "secret");
    }

    #[test]
    fn rejects_non_basic_auth() {
        let header = HeaderValue::from_static("Bearer token");

        assert!(parse_basic_credentials(&header).is_none());
    }

    #[test]
    fn forwarding_headers_require_an_explicit_trusted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.8"));
        let peer = ConnectInfo("127.0.0.1:4000".parse::<SocketAddr>().unwrap());

        let untrusted =
            AuthRequestContext::from_request(&headers, Some(&peer), &AuthConfig::default());
        assert_eq!(untrusted.client_id, "127.0.0.1");

        let trusted_config = AuthConfig {
            trusted_proxy_ips: vec!["127.0.0.1".parse().unwrap()],
            ..AuthConfig::default()
        };
        let trusted = AuthRequestContext::from_request(&headers, Some(&peer), &trusted_config);
        assert_eq!(trusted.client_id, "203.0.113.8");
    }

    #[test]
    fn forwarded_chain_stops_at_the_first_untrusted_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.99, 203.0.113.8, 10.0.0.2"),
        );
        let peer = ConnectInfo("127.0.0.1:4000".parse::<SocketAddr>().unwrap());
        let config = AuthConfig {
            trusted_proxy_ips: vec!["127.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap()],
            ..AuthConfig::default()
        };

        let context = AuthRequestContext::from_request(&headers, Some(&peer), &config);

        assert_eq!(context.client_id, "203.0.113.8");
    }

    #[test]
    fn malformed_forwarded_chain_does_not_fall_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.8, not-an-ip"),
        );
        headers.insert("x-real-ip", HeaderValue::from_static("198.51.100.4"));
        let peer = ConnectInfo("127.0.0.1:4000".parse::<SocketAddr>().unwrap());
        let config = AuthConfig {
            trusted_proxy_ips: vec!["127.0.0.1".parse().unwrap()],
            ..AuthConfig::default()
        };

        let context = AuthRequestContext::from_request(&headers, Some(&peer), &config);

        assert_eq!(context.client_id, "127.0.0.1");
    }

    #[test]
    fn parses_bearer_auth_credentials() {
        let header = HeaderValue::from_static("Bearer teatro_pat_token");
        let credentials = parse_bearer_credentials(&header).unwrap();

        assert_eq!(credentials.token, "teatro_pat_token");
        assert_eq!(credentials.token_prefix, "teatro_pat_token");
    }
}
