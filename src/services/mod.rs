//! Business workflows composed from domain, repository, and managed-storage boundaries.

pub mod api_tokens;
pub mod auth;
pub(crate) mod auth_rate_limit;
pub(crate) mod background_transfers;
pub mod file_operations;
pub(crate) mod gog_import;
pub(crate) mod igdb;
pub(crate) mod ingest;
pub mod integrity;
pub(crate) mod job_registry;
pub(crate) mod lan_discovery;
pub mod library;
pub(crate) mod password;
pub(crate) mod romm_source;

pub use crate::ops::report;

fn bounded_text(input: &str, max_bytes: usize) -> (String, bool) {
    let mut output = String::with_capacity(input.len().min(max_bytes));
    for character in input.chars() {
        let character = if character.is_control() {
            '\u{fffd}'
        } else {
            character
        };
        if output.len().saturating_add(character.len_utf8()) > max_bytes {
            return (output, true);
        }
        output.push(character);
    }
    (output, false)
}
