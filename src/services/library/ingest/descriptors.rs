use std::collections::{HashMap, HashSet};

use serde_json::json;

use crate::domain::rom::{FileGroupKind, FileRole};

use super::{
    super::{
        normalization::{descriptor_dependency_kind, file_key, slugify},
        types::{
            IngestPlanDependency, IngestPlanError, IngestPlanFile, IngestPlanGroup, IngestPlanRom,
            PreparedPlanFile,
        },
    },
    planner::{ingest_error, plan_file, resolve_manifest_reference},
};

pub(super) fn plan_descriptor_rom(
    prepared: &[PreparedPlanFile],
    name_to_index: &HashMap<String, usize>,
    descriptor_index: usize,
    used: &mut HashSet<usize>,
    rom_index: usize,
) -> Result<IngestPlanRom, Vec<IngestPlanError>> {
    let descriptor = &prepared[descriptor_index];
    let title = descriptor.parsed.clean_title.clone();
    let slug = slugify(&title);
    let plan_id = format!("rom-{}", rom_index + 1);
    let group_key = format!("{plan_id}:track-set");
    let disc_index = descriptor
        .parsed
        .disc
        .as_ref()
        .and_then(|disc| disc.index)
        .map(i64::from);
    let disc_count = descriptor
        .parsed
        .disc
        .as_ref()
        .and_then(|disc| disc.count)
        .map(i64::from);
    let mut groups = vec![IngestPlanGroup {
        key: group_key.clone(),
        kind: FileGroupKind::TrackSet,
        display_name: disc_index
            .map(|index| format!("Disc {index} track set"))
            .unwrap_or_else(|| title.clone()),
        group_key: Some(format!("{slug}:track-set")),
        disc_index,
        disc_count,
        launchable: true,
        metadata: json!({"descriptor_format": descriptor.extension}),
    }];
    let mut files = vec![plan_file(
        descriptor,
        Some(&group_key),
        FileRole::Descriptor,
        0,
        disc_index,
        None,
        true,
        json!({"descriptor_format": descriptor.extension}),
    )];
    let mut dependencies = Vec::new();
    let mut errors = Vec::new();
    used.insert(descriptor_index);

    add_descriptor_dependencies(
        prepared,
        name_to_index,
        descriptor_index,
        &group_key,
        disc_index,
        1,
        used,
        &mut files,
        &mut dependencies,
        &mut errors,
    );

    if errors.is_empty() {
        Ok(IngestPlanRom {
            plan_id,
            title,
            slug,
            regions: descriptor.parsed.regions.clone(),
            groups: std::mem::take(&mut groups),
            files,
            dependencies,
        })
    } else {
        Err(errors)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add_descriptor_dependencies(
    prepared: &[PreparedPlanFile],
    name_to_index: &HashMap<String, usize>,
    descriptor_index: usize,
    group_key: &str,
    disc_index: Option<i64>,
    base_sort_index: i64,
    used: &mut HashSet<usize>,
    files: &mut Vec<IngestPlanFile>,
    dependencies: &mut Vec<IngestPlanDependency>,
    errors: &mut Vec<IngestPlanError>,
) {
    let descriptor = &prepared[descriptor_index];
    let track_refs = descriptor_track_references(descriptor);
    if track_refs.is_empty() {
        errors.push(ingest_error(
            "empty_manifest",
            "descriptor does not reference any track files",
            Some(&descriptor.input.original_file_name),
        ));
        return;
    }

    for (index, track_ref) in track_refs.iter().enumerate() {
        let Some(track_index) = resolve_manifest_reference(&track_ref.file_name, name_to_index)
        else {
            errors.push(ingest_error(
                "missing_manifest_dependency",
                &format!(
                    "descriptor dependency {} is missing from the batch",
                    track_ref.file_name
                ),
                Some(&descriptor.input.original_file_name),
            ));
            continue;
        };
        if used.contains(&track_index) {
            errors.push(ingest_error(
                "file_used_by_multiple_groups",
                "a batch file is referenced by more than one planned ROM",
                Some(&prepared[track_index].input.original_file_name),
            ));
            continue;
        }

        let track_file = &prepared[track_index];
        let track_number = track_ref.track_index.unwrap_or(index as i64 + 1);
        files.push(plan_file(
            track_file,
            Some(group_key),
            FileRole::Track,
            base_sort_index + index as i64,
            disc_index,
            Some(track_number),
            false,
            json!({}),
        ));
        dependencies.push(IngestPlanDependency {
            parent_file_key: file_key(descriptor_index),
            child_file_key: file_key(track_index),
            dependency_kind: descriptor_dependency_kind(&descriptor.extension),
            sort_index: index as i64,
        });
        used.insert(track_index);
    }
}

#[derive(Debug, Clone)]
struct DescriptorTrackRef {
    file_name: String,
    track_index: Option<i64>,
}

fn descriptor_track_references(descriptor: &PreparedPlanFile) -> Vec<DescriptorTrackRef> {
    if let Some(cue_refs) = descriptor.manifest.cue.as_ref() {
        return cue_refs
            .iter()
            .enumerate()
            .map(|(index, file_name)| DescriptorTrackRef {
                file_name: file_name.clone(),
                track_index: Some(index as i64 + 1),
            })
            .collect();
    }

    if let Some(gdi_refs) = descriptor.manifest.gdi.as_ref() {
        return gdi_refs
            .iter()
            .map(|reference| DescriptorTrackRef {
                file_name: reference.file_name.clone(),
                track_index: Some(i64::from(reference.track_number)),
            })
            .collect();
    }

    Vec::new()
}
