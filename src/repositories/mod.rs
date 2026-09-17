//! SQL ownership boundary for application persistence.
//!
//! Operational report queries remain in `ops::report` as an explicit read-model
//! exception; business services should use repositories rather than issue SQL.

pub mod api_tokens;
pub(crate) mod audit;
pub(crate) mod browser_sessions;
pub mod file_operations;
pub(crate) mod igdb_settings;
pub mod integrity;
pub mod library_roots;
pub mod platforms;
pub(crate) mod romm_source;
pub mod roms;
pub mod users;
