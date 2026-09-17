//! RomM import job lifecycle; the shared registry lives in `services::job_registry`.
//!
//! Snapshots never carry the RomM secret, the base URL, or any credential-bearing value.

use super::super::job_registry::{
    JobError, JobProgress, JobRegistry, JobRegistryError, JobReporter, JobSnapshot,
};
use super::import::RommImportOutcome;

pub(crate) type RommImportJobRegistry = JobRegistry<RommImportOutcome>;
pub(crate) type RommImportJobReporter = JobReporter<RommImportOutcome>;
pub(crate) type RommImportJobSnapshot = JobSnapshot<RommImportOutcome>;
pub(crate) type RommImportJobProgress = JobProgress;
pub(crate) type RommImportJobError = JobError;
pub(crate) type RommImportJobRegistryError = JobRegistryError;
