use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use axum::{
    body::Bytes,
    http::{HeaderMap, Method, StatusCode},
};
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

const TERMINAL_CAPACITY: usize = 64;
const TERMINAL_RETENTION: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
pub(crate) struct CachedTransferResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

pub(crate) enum TransferClaim {
    Owner(TransferLease),
    Running,
    Complete(CachedTransferResponse),
    Cancelled,
}

pub(crate) enum CancelTransferResult {
    Cancelled,
    Complete,
    NotFound,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TransferKey {
    user_id: i64,
    method: Method,
    endpoint: String,
    id: String,
}

impl TransferKey {
    pub(crate) fn new(user_id: i64, method: Method, endpoint: &str, id: &str) -> Self {
        Self {
            user_id,
            method,
            endpoint: endpoint.to_owned(),
            id: id.to_owned(),
        }
    }
}

pub(crate) struct BackgroundTransferRegistry {
    entries: Mutex<HashMap<TransferKey, TransferEntry>>,
}

impl BackgroundTransferRegistry {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn claim(self: &Arc<Self>, key: &TransferKey) -> Option<TransferClaim> {
        if !valid_id(&key.id) {
            return None;
        }
        let now = Utc::now();
        let mut entries = self.lock();
        prune(&mut entries, now);
        match entries.get(key) {
            Some(TransferEntry::Running { .. }) => Some(TransferClaim::Running),
            Some(TransferEntry::Complete { response, .. }) => {
                Some(TransferClaim::Complete(response.clone()))
            }
            Some(TransferEntry::Cancelled { .. }) => Some(TransferClaim::Cancelled),
            None => {
                let cancellation = CancellationToken::new();
                entries.insert(
                    key.clone(),
                    TransferEntry::Running {
                        cancellation: cancellation.clone(),
                    },
                );
                Some(TransferClaim::Owner(TransferLease {
                    registry: Arc::clone(self),
                    key: key.clone(),
                    completed: false,
                    cancellation,
                }))
            }
        }
    }

    pub(crate) fn cancel(&self, key: &TransferKey) -> CancelTransferResult {
        let now = Utc::now();
        let mut entries = self.lock();
        prune(&mut entries, now);
        match entries.get(key) {
            None => CancelTransferResult::NotFound,
            Some(TransferEntry::Complete { .. }) => CancelTransferResult::Complete,
            Some(TransferEntry::Cancelled { .. }) => CancelTransferResult::Cancelled,
            Some(TransferEntry::Running { cancellation }) => {
                let cancellation = cancellation.clone();
                entries.insert(key.clone(), TransferEntry::Cancelled { cancelled_at: now });
                drop(entries);
                cancellation.cancel();
                CancelTransferResult::Cancelled
            }
        }
    }

    fn complete(&self, key: &TransferKey, response: CachedTransferResponse) {
        let now = Utc::now();
        let mut entries = self.lock();
        if matches!(entries.get(key), Some(TransferEntry::Running { .. })) {
            entries.insert(
                key.clone(),
                TransferEntry::Complete {
                    response,
                    completed_at: now,
                },
            );
            prune(&mut entries, now);
        }
    }

    fn release(&self, key: &TransferKey) {
        let mut entries = self.lock();
        if matches!(entries.get(key), Some(TransferEntry::Running { .. })) {
            entries.remove(key);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TransferKey, TransferEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) struct TransferLease {
    registry: Arc<BackgroundTransferRegistry>,
    key: TransferKey,
    completed: bool,
    cancellation: CancellationToken,
}

impl TransferLease {
    pub(crate) async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    pub(crate) fn complete(mut self, response: CachedTransferResponse) {
        self.registry.complete(&self.key, response);
        self.completed = true;
    }
}

impl Drop for TransferLease {
    fn drop(&mut self) {
        if !self.completed {
            self.registry.release(&self.key);
        }
    }
}

enum TransferEntry {
    Running {
        cancellation: CancellationToken,
    },
    Complete {
        response: CachedTransferResponse,
        completed_at: DateTime<Utc>,
    },
    Cancelled {
        cancelled_at: DateTime<Utc>,
    },
}

fn valid_id(id: &str) -> bool {
    id.len() <= 128
        && ["upload_", "gog_"].iter().any(|prefix| {
            id.strip_prefix(prefix)
                .is_some_and(|suffix| !suffix.is_empty())
        })
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn prune(entries: &mut HashMap<TransferKey, TransferEntry>, now: DateTime<Utc>) {
    let retention = chrono::Duration::from_std(TERMINAL_RETENTION)
        .expect("background transfer retention fits chrono duration");
    entries.retain(|_, entry| match entry {
        TransferEntry::Running { .. } => true,
        TransferEntry::Complete { completed_at, .. } => {
            now.signed_duration_since(*completed_at) < retention
        }
        TransferEntry::Cancelled { cancelled_at } => {
            now.signed_duration_since(*cancelled_at) < retention
        }
    });

    let mut terminal = entries
        .iter()
        .filter_map(|(id, entry)| match entry {
            TransferEntry::Running { .. } => None,
            TransferEntry::Complete { completed_at, .. } => Some((*completed_at, id.clone())),
            TransferEntry::Cancelled { cancelled_at } => Some((*cancelled_at, id.clone())),
        })
        .collect::<Vec<_>>();
    if terminal.len() > TERMINAL_CAPACITY {
        terminal.sort_by_key(|(at, _)| *at);
        let remove_count = terminal.len() - TERMINAL_CAPACITY;
        for (_, id) in terminal.into_iter().take(remove_count) {
            entries.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str) -> TransferKey {
        TransferKey::new(1, Method::POST, "/api/admin/upload-batches", id)
    }

    #[test]
    fn claims_are_idempotent_and_failed_owners_release_for_retry() {
        let registry = Arc::new(BackgroundTransferRegistry::new());
        let owner = match registry.claim(&key("upload_test_1")).unwrap() {
            TransferClaim::Owner(owner) => owner,
            _ => panic!("first claim must own the transfer"),
        };
        assert!(matches!(
            registry.claim(&key("upload_test_1")),
            Some(TransferClaim::Running)
        ));
        drop(owner);
        assert!(matches!(
            registry.claim(&key("upload_test_1")),
            Some(TransferClaim::Owner(_))
        ));
        assert!(registry.claim(&key("invalid/path")).is_none());
    }

    #[tokio::test]
    async fn cancellation_notifies_the_owner_and_blocks_retries() {
        let registry = Arc::new(BackgroundTransferRegistry::new());
        let owner = match registry.claim(&key("upload_cancel_1")).unwrap() {
            TransferClaim::Owner(owner) => owner,
            _ => unreachable!(),
        };
        assert!(matches!(
            registry.cancel(&key("upload_cancel_1")),
            CancelTransferResult::Cancelled
        ));
        owner.cancelled().await;
        assert!(matches!(
            registry.claim(&key("upload_cancel_1")),
            Some(TransferClaim::Cancelled)
        ));
    }

    #[test]
    fn keys_isolate_every_identity_component_and_expire_terminal_entries() {
        let registry = Arc::new(BackgroundTransferRegistry::new());
        let original = key("upload_scoped");
        let TransferClaim::Owner(owner) = registry.claim(&original).unwrap() else {
            panic!("first claim must own the transfer");
        };
        for different in [
            TransferKey {
                user_id: 2,
                ..original.clone()
            },
            TransferKey {
                method: Method::PUT,
                ..original.clone()
            },
            TransferKey {
                endpoint: "/api/admin/gog-imports".into(),
                ..original.clone()
            },
            key("upload_other"),
        ] {
            assert!(matches!(
                registry.cancel(&different),
                CancelTransferResult::NotFound
            ));
            assert!(matches!(
                registry.claim(&different),
                Some(TransferClaim::Owner(_))
            ));
        }
        assert!(!owner.cancellation.is_cancelled());
        assert!(matches!(
            registry.claim(&original),
            Some(TransferClaim::Running)
        ));
        assert!(matches!(
            registry.cancel(&original),
            CancelTransferResult::Cancelled
        ));
        drop(owner);
        let expired_at = Utc::now() - chrono::Duration::hours(2);
        registry.lock().insert(
            original.clone(),
            TransferEntry::Cancelled {
                cancelled_at: expired_at,
            },
        );
        assert!(matches!(
            registry.cancel(&original),
            CancelTransferResult::NotFound
        ));
        assert!(matches!(
            registry.claim(&original),
            Some(TransferClaim::Owner(_))
        ));
        registry.lock().insert(
            original.clone(),
            TransferEntry::Complete {
                response: CachedTransferResponse {
                    status: StatusCode::CREATED,
                    headers: HeaderMap::new(),
                    body: Bytes::from_static(b"expired"),
                },
                completed_at: expired_at,
            },
        );
        assert!(matches!(
            registry.claim(&original),
            Some(TransferClaim::Owner(_))
        ));
        assert!(matches!(
            registry.cancel(&original),
            CancelTransferResult::NotFound
        ));
    }

    #[test]
    fn completed_response_is_returned_to_retries() {
        let registry = Arc::new(BackgroundTransferRegistry::new());
        let owner = match registry.claim(&key("gog_test_1")).unwrap() {
            TransferClaim::Owner(owner) => owner,
            _ => unreachable!(),
        };
        owner.complete(CachedTransferResponse {
            status: StatusCode::ACCEPTED,
            headers: HeaderMap::new(),
            body: Bytes::from_static(br#"{"job":{"id":"gog_server"}}"#),
        });
        let TransferClaim::Complete(response) = registry.claim(&key("gog_test_1")).unwrap() else {
            panic!("completed transfer must replay its response");
        };
        assert_eq!(response.status, StatusCode::ACCEPTED);
        assert_eq!(
            response.body,
            Bytes::from_static(br#"{"job":{"id":"gog_server"}}"#)
        );
    }
}
