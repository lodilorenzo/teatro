use std::{sync::Arc, time::Duration};

use thiserror::Error;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};

use super::auth;

#[derive(Debug, Error)]
pub enum PasswordServiceError {
    #[error("password worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),

    #[error(transparent)]
    Hash(#[from] argon2::password_hash::Error),

    #[error("password work queue is full")]
    Busy,

    #[error("password work queue wait timed out")]
    WaitTimeout,
}

impl PasswordServiceError {
    pub fn is_capacity_error(&self) -> bool {
        matches!(self, Self::Busy | Self::WaitTimeout)
    }
}

#[derive(Debug)]
pub struct PasswordAdmission {
    admission: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub struct PasswordPermit {
    worker: OwnedSemaphorePermit,
    admission: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub struct PasswordService {
    worker_semaphore: Arc<Semaphore>,
    admission_semaphore: Arc<Semaphore>,
    queue_wait_timeout: Duration,
}

impl PasswordService {
    pub fn new(
        max_concurrency: usize,
        max_queue_depth: usize,
        queue_wait_timeout: Duration,
    ) -> Self {
        let max_concurrency = max_concurrency.max(1);
        let admission_capacity = max_concurrency.saturating_add(max_queue_depth);
        Self {
            worker_semaphore: Arc::new(Semaphore::new(max_concurrency)),
            admission_semaphore: Arc::new(Semaphore::new(admission_capacity)),
            queue_wait_timeout,
        }
    }

    pub async fn hash(&self, password: &str) -> Result<String, PasswordServiceError> {
        let admission = self.try_admit()?;
        let permit = self.acquire(admission).await?;
        let password = password.to_string();
        Ok(self
            .run_blocking_with_permit(permit, move || auth::hash_password(&password))
            .await??)
    }

    /// Reserve one of the bounded active-or-waiting admission slots without waiting.
    pub fn try_admit(&self) -> Result<PasswordAdmission, PasswordServiceError> {
        self.admission_semaphore
            .clone()
            .try_acquire_owned()
            .map(|admission| PasswordAdmission { admission })
            .map_err(|_| PasswordServiceError::Busy)
    }

    /// Wait for an Argon2 worker only after admission has bounded the queue.
    pub async fn acquire(
        &self,
        admission: PasswordAdmission,
    ) -> Result<PasswordPermit, PasswordServiceError> {
        let worker = if self.queue_wait_timeout.is_zero() {
            self.worker_semaphore
                .clone()
                .try_acquire_owned()
                .map_err(|_| PasswordServiceError::WaitTimeout)?
        } else {
            timeout(
                self.queue_wait_timeout,
                self.worker_semaphore.clone().acquire_owned(),
            )
            .await
            .map_err(|_| PasswordServiceError::WaitTimeout)?
            .expect("password worker semaphore is never closed")
        };
        Ok(PasswordPermit {
            worker,
            admission: admission.admission,
        })
    }

    pub async fn verify_with_permit(
        &self,
        permit: PasswordPermit,
        password: &str,
        password_hash: &str,
    ) -> Result<bool, PasswordServiceError> {
        let password = password.to_string();
        let password_hash = password_hash.to_string();
        Ok(self
            .run_blocking_with_permit(permit, move || {
                auth::verify_password(&password, &password_hash)
            })
            .await?)
    }

    async fn run_blocking_with_permit<T, F>(
        &self,
        permit: PasswordPermit,
        work: F,
    ) -> Result<T, tokio::task::JoinError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            // Both permits must live in the blocking task: cancelling the request does
            // not cancel spawn_blocking work, so releasing either permit early could
            // exceed the worker bound or admit too many queued replacements.
            let _worker = permit.worker;
            let _admission = permit.admission;
            work()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use tokio::time::{Duration, sleep};

    use super::{PasswordService, PasswordServiceError};

    #[tokio::test]
    async fn password_work_uses_a_bounded_waiting_queue() {
        let service = Arc::new(PasswordService::new(1, 1, Duration::from_secs(1)));
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first_permit = service.acquire(service.try_admit().unwrap()).await.unwrap();
        let first = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .run_blocking_with_permit(first_permit, move || {
                        first_started_tx.send(()).unwrap();
                        release_first_rx.recv().unwrap();
                    })
                    .await
                    .unwrap();
            })
        };
        tokio::task::spawn_blocking(move || first_started_rx.recv().unwrap())
            .await
            .unwrap();

        let queued_admission = service.try_admit().unwrap();
        let queued = {
            let service = service.clone();
            tokio::spawn(async move { service.acquire(queued_admission).await })
        };
        sleep(Duration::from_millis(20)).await;
        assert!(!queued.is_finished());
        assert!(matches!(
            service.try_admit(),
            Err(PasswordServiceError::Busy)
        ));

        release_first_tx.send(()).unwrap();
        first.await.unwrap();
        drop(queued.await.unwrap().unwrap());
        assert!(service.try_admit().is_ok());
    }

    #[tokio::test]
    async fn queued_password_work_has_a_wait_timeout() {
        let service = PasswordService::new(1, 1, Duration::from_millis(20));
        let active = service.acquire(service.try_admit().unwrap()).await.unwrap();
        let queued = service.try_admit().unwrap();

        assert!(matches!(
            service.acquire(queued).await,
            Err(PasswordServiceError::WaitTimeout)
        ));
        drop(active);
        assert!(service.try_admit().is_ok());
    }

    #[tokio::test]
    async fn cancellation_keeps_permits_until_blocking_work_finishes() {
        let service = Arc::new(PasswordService::new(1, 0, Duration::from_secs(1)));
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first_permit = service.acquire(service.try_admit().unwrap()).await.unwrap();
        let first = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .run_blocking_with_permit(first_permit, move || {
                        first_started_tx.send(()).unwrap();
                        release_first_rx.recv().unwrap();
                    })
                    .await
                    .unwrap();
            })
        };
        tokio::task::spawn_blocking(move || first_started_rx.recv().unwrap())
            .await
            .unwrap();
        first.abort();

        assert!(matches!(
            service.try_admit(),
            Err(PasswordServiceError::Busy)
        ));
        release_first_tx.send(()).unwrap();
        for _ in 0..100 {
            if service.try_admit().is_ok() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("blocking password work did not release its permits");
    }
}
