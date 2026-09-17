//! Managed-library scan job lifecycle; the shared registry lives in `services::job_registry`.

use super::super::job_registry::{
    JobError, JobProgress, JobRegistry, JobRegistryError, JobReporter, JobSnapshot,
};
use super::types::LibraryScanResult;

pub(crate) type LibraryScanJobRegistry = JobRegistry<LibraryScanResult>;
pub(crate) type LibraryScanJobReporter = JobReporter<LibraryScanResult>;
pub(crate) type LibraryScanJobSnapshot = JobSnapshot<LibraryScanResult>;
pub(crate) type LibraryScanJobProgress = JobProgress;
pub(crate) type LibraryScanJobError = JobError;
pub(crate) type LibraryScanJobRegistryError = JobRegistryError;
