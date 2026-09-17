use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::domain::{
    api_token::{ApiTokenScope, InvalidApiTokenScope},
    user::User,
};

const TOKEN_PREFIX: &str = "teatro_pat_";
const TOKEN_RANDOM_BYTES: usize = 32;
const TOKEN_PREVIEW_LEN: usize = 24;

#[derive(Debug, Error)]
pub enum ApiTokenServiceError {
    #[error(transparent)]
    InvalidScope(#[from] InvalidApiTokenScope),

    #[error("API token name cannot be empty")]
    EmptyName,

    #[error("API token name is too long")]
    NameTooLong,

    #[error("admin scope can only be granted to admin users")]
    AdminScopeRequiresAdminUser,
}

pub fn generate_token() -> String {
    let mut bytes = [0_u8; TOKEN_RANDOM_BYTES];
    OsRng.fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let digest = hasher.finalize();

    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }

    output
}

pub fn token_prefix(token: &str) -> String {
    token.chars().take(TOKEN_PREVIEW_LEN).collect()
}

pub fn validate_name(name: &str) -> Result<String, ApiTokenServiceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiTokenServiceError::EmptyName);
    }
    if name.len() > 100 {
        return Err(ApiTokenServiceError::NameTooLong);
    }

    Ok(name.to_string())
}

pub fn normalize_scopes(
    raw_scopes: Option<Vec<String>>,
    user: &User,
) -> Result<Vec<ApiTokenScope>, ApiTokenServiceError> {
    let mut scopes = match raw_scopes {
        Some(scopes) if !scopes.is_empty() => scopes
            .into_iter()
            .map(|scope| scope.parse::<ApiTokenScope>())
            .collect::<Result<Vec<_>, _>>()?,
        _ => vec![ApiTokenScope::Read],
    };

    scopes.sort_by_key(|scope| scope.as_str());
    scopes.dedup();

    if scopes.contains(&ApiTokenScope::Admin) {
        if !user.role.is_admin() {
            return Err(ApiTokenServiceError::AdminScopeRequiresAdminUser);
        }
        if !scopes.contains(&ApiTokenScope::Read) {
            scopes.push(ApiTokenScope::Read);
            scopes.sort_by_key(|scope| scope.as_str());
        }
    }

    Ok(scopes)
}

pub fn scopes_to_json(scopes: &[ApiTokenScope]) -> Result<String, serde_json::Error> {
    let scopes: Vec<_> = scopes.iter().map(|scope| scope.as_str()).collect();
    serde_json::to_string(&scopes)
}

#[derive(Debug, Error)]
pub enum ParseScopesError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    InvalidScope(#[from] InvalidApiTokenScope),
}

pub fn parse_scopes(scopes_json: &str) -> Result<Vec<ApiTokenScope>, ParseScopesError> {
    serde_json::from_str::<Vec<String>>(scopes_json)?
        .into_iter()
        .map(|scope| scope.parse().map_err(ParseScopesError::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        api_token::ApiTokenScope,
        user::{User, UserRole},
    };

    use super::{TOKEN_PREFIX, generate_token, hash_token, normalize_scopes, token_prefix};

    #[test]
    fn generated_tokens_have_expected_prefix_and_hash() {
        let token = generate_token();

        assert!(token.starts_with(TOKEN_PREFIX));
        assert_eq!(hash_token(&token).len(), 64);
        assert!(token.starts_with(&token_prefix(&token)));
    }

    #[test]
    fn normalizes_scopes_and_requires_admin_user_for_admin_scope() {
        let admin = User {
            id: 1,
            username: "admin".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
        };
        let readonly = User {
            id: 2,
            username: "player".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::ReadOnly,
        };

        let scopes = normalize_scopes(Some(vec!["admin".to_string()]), &admin).unwrap();
        assert_eq!(scopes, vec![ApiTokenScope::Admin, ApiTokenScope::Read]);
        assert!(normalize_scopes(Some(vec!["admin".to_string()]), &readonly).is_err());
    }
}
