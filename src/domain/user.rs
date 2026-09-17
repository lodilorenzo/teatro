use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    #[serde(alias = "read_only", alias = "read-only")]
    ReadOnly,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::ReadOnly => "readonly",
        }
    }

    pub fn is_admin(self) -> bool {
        matches!(self, Self::Admin)
    }
}

impl fmt::Display for UserRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = InvalidUserRole;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "admin" => Ok(Self::Admin),
            "readonly" | "read_only" | "read-only" => Ok(Self::ReadOnly),
            _ => Err(InvalidUserRole(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, Error)]
#[error("invalid user role {0:?}")]
pub struct InvalidUserRole(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
}

impl User {
    pub fn public(&self) -> PublicUser {
        PublicUser {
            id: self.id,
            username: self.username.clone(),
            role: self.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicUser {
    pub id: i64,
    pub username: String,
    pub role: UserRole,
}
