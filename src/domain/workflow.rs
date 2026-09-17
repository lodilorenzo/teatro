//! Typed values for persisted integrity and managed-file workflow states.
//!
//! SQLite stores these values as snake-case text for API and schema compatibility.
//! Decoding is deliberately fallible so corrupt or future values cannot silently enter
//! a state machine.

#![warn(missing_docs)]

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

macro_rules! workflow_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(
                #[doc = concat!("Persisted value `", $value, "`.")]
                $variant
            ),+
        }

        impl $name {
            /// Return the stable database and API representation.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = InvalidWorkflowValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(InvalidWorkflowValue {
                        kind: stringify!($name),
                        value: value.to_string(),
                    }),
                }
            }
        }
    };
}

/// Error returned when persisted workflow text is not a supported value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid persisted {kind} value {value:?}")]
pub struct InvalidWorkflowValue {
    kind: &'static str,
    value: String,
}

workflow_enum!(
    /// Kind of recoverable managed-filesystem operation.
    FileOperationKind {
        Upload => "upload",
        Delete => "delete",
        CoverReplace => "cover_replace",
    }
);

workflow_enum!(
    /// Durable state of a recoverable managed-filesystem operation.
    FileOperationState {
        Prepared => "prepared",
        DbCommitted => "db_committed",
        CleanupPending => "cleanup_pending",
        Completed => "completed",
        Failed => "failed",
    }
);

workflow_enum!(
    /// Work performed by an integrity job.
    IntegrityJobKind {
        HashAndMatch => "hash_and_match",
    }
);

workflow_enum!(
    /// Durable lifecycle state of an integrity job.
    IntegrityJobStatus {
        Queued => "queued",
        Running => "running",
        Completed => "completed",
        Failed => "failed",
    }
);

workflow_enum!(
    /// Hashing state persisted for a managed ROM file.
    FileHashStatus {
        Pending => "pending",
        Hashing => "hashing",
        Complete => "complete",
        Failed => "failed",
    }
);

workflow_enum!(
    /// Digest that established a DAT-entry match.
    DatMatchMethod {
        Crc32 => "crc32",
        Md5 => "md5",
        Sha1 => "sha1",
        Sha256 => "sha256",
    }
);

workflow_enum!(
    /// Aggregate integrity state returned for a ROM.
    RomIntegrityStatus {
        Pending => "pending",
        Hashing => "hashing",
        Unmatched => "unmatched",
        Verified => "verified",
        Failed => "failed",
    }
);

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{FileHashStatus, FileOperationState, IntegrityJobStatus};

    #[test]
    fn workflow_values_keep_existing_wire_strings() {
        assert_eq!(
            serde_json::to_string(&FileOperationState::CleanupPending).unwrap(),
            "\"cleanup_pending\""
        );
        assert_eq!(
            serde_json::to_string(&IntegrityJobStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            FileHashStatus::from_str("complete").unwrap(),
            FileHashStatus::Complete
        );
        assert!(FileHashStatus::from_str("unknown").is_err());
    }
}
