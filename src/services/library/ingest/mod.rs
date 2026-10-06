//! Pure ingest planning and persistence finalization boundaries.

mod descriptors;
mod disc_sets;
mod finalization;
mod planner;

pub(super) use finalization::{prepare_batch_finalization, prepare_in_place_finalization};
pub(crate) use planner::build_ingest_plan;
pub(super) use planner::{apply_planned_titles, summarize_ingest_errors};
