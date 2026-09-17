use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::user::PublicUser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiTokenScope {
    Read,
    Admin,
}

impl ApiTokenScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Admin => "admin",
        }
    }
}

impl fmt::Display for ApiTokenScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ApiTokenScope {
    type Err = InvalidApiTokenScope;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Self::Read),
            "admin" => Ok(Self::Admin),
            _ => Err(InvalidApiTokenScope(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, Error)]
#[error("invalid API token scope {0:?}")]
pub struct InvalidApiTokenScope(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: i64,
    pub user: PublicUser,
    pub name: String,
    pub token_prefix: String,
    pub scopes: Vec<ApiTokenScope>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

impl ApiToken {
    pub fn has_scope(&self, scope: ApiTokenScope) -> bool {
        self.scopes.contains(&scope)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedApiToken {
    pub token: String,
    pub record: ApiToken,
}
