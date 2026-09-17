use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;

use crate::{
    domain::rom::{DependencyKind, FileGroupKind, FileRole},
    services::ingest::filename,
};

use super::{
    super::{
        normalization::{
            file_key, generated_file_key, generated_m3u_file_name, is_disc_image_extension,
            sanitize_upload_file_name, slugify,
        },
        types::{
            IngestPlanDependency, IngestPlanError, IngestPlanFile, IngestPlanGroup, IngestPlanRom,
            PreparedPlanFile,
        },
    },
    descriptors::add_descriptor_dependencies,
    planner::{ingest_error, plan_file, resolve_manifest_reference},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DiscSetKey {
    title: String,
    regions: Vec<String>,
    revision: Option<i32>,
    version: Option<String>,
    languages: Vec<String>,
    special_hardware: Vec<String>,
    unknown_tokens: Vec<String>,
    dump_flags_json: String,
}

pub(super) fn generated_m3u_candidate_groups(
    prepared: &[PreparedPlanFile],
    used: &HashSet<usize>,
) -> Vec<Vec<usize>> {
    let mut groups: HashMap<DiscSetKey, Vec<usize>> = HashMap::new();

    for file in prepared {
        if let Some(key) = generated_m3u_candidate_key(file, used) {
            groups.entry(key).or_default().push(file.input.index);
        }
    }

    let mut groups: Vec<Vec<usize>> = groups
        .into_values()
        .filter(|indices| {
            indices.len() > 1
                || indices.iter().any(|index| {
                    prepared[*index].parsed.disc.as_ref().is_some_and(|disc| {
                        disc.count.is_some_and(|count| count > 1)
                            || disc.index.is_some_and(|index| index > 1)
                    })
                })
        })
        .collect();

    for indices in &mut groups {
        indices.sort_by(|left, right| compare_disc_candidates(&prepared[*left], &prepared[*right]));
    }
    groups.sort_by(|left, right| {
        let left_first = &prepared[left[0]];
        let right_first = &prepared[right[0]];
        left_first
            .parsed
            .clean_title
            .to_ascii_lowercase()
            .cmp(&right_first.parsed.clean_title.to_ascii_lowercase())
            .then_with(|| left_first.input.index.cmp(&right_first.input.index))
    });

    groups
}

fn generated_m3u_candidate_key(
    file: &PreparedPlanFile,
    used: &HashSet<usize>,
) -> Option<DiscSetKey> {
    if used.contains(&file.input.index) || file.extension == "m3u" {
        return None;
    }

    let disc = file.parsed.disc.as_ref()?;
    disc.index?;

    let is_launchable_disc = is_disc_image_extension(&file.extension)
        || (matches!(file.extension.as_str(), "cue" | "gdi")
            && file.manifest.has_descriptor_references());
    if !is_launchable_disc {
        return None;
    }

    Some(DiscSetKey {
        title: file.parsed.clean_title.to_ascii_lowercase(),
        regions: file.parsed.regions.clone(),
        revision: file.parsed.revision,
        version: file.parsed.version.clone(),
        languages: file.parsed.languages.clone(),
        special_hardware: file.parsed.special_hardware.clone(),
        unknown_tokens: file.parsed.unknown_tokens.clone(),
        dump_flags_json: serde_json::to_string(&file.parsed.dump_flags)
            .expect("dump flags serialize as JSON"),
    })
}

fn compare_disc_candidates(
    left: &PreparedPlanFile,
    right: &PreparedPlanFile,
) -> std::cmp::Ordering {
    disc_index(left)
        .cmp(&disc_index(right))
        .then_with(|| left.input.index.cmp(&right.input.index))
        .then_with(|| {
            left.input
                .original_file_name
                .to_ascii_lowercase()
                .cmp(&right.input.original_file_name.to_ascii_lowercase())
        })
}

fn disc_index(file: &PreparedPlanFile) -> i64 {
    file.parsed
        .disc
        .as_ref()
        .and_then(|disc| disc.index)
        .map(i64::from)
        .unwrap_or(i64::MAX)
}

pub(super) fn plan_m3u_rom(
    prepared: &[PreparedPlanFile],
    name_to_index: &HashMap<String, usize>,
    m3u_index: usize,
    used: &mut HashSet<usize>,
    rom_index: usize,
) -> Result<IngestPlanRom, Vec<IngestPlanError>> {
    let m3u_file = &prepared[m3u_index];
    let references = m3u_file.manifest.m3u.as_deref().unwrap_or_default();
    if references.is_empty() {
        return Err(vec![ingest_error(
            "empty_manifest",
            "m3u manifest does not reference any files",
            Some(&m3u_file.input.original_file_name),
        )]);
    }

    let plan_id = format!("rom-{}", rom_index + 1);
    let title = m3u_file.parsed.clean_title.clone();
    let slug = slugify(&title);
    let playlist_group_key = format!("{plan_id}:playlist");
    let mut plan = UserM3uPlan {
        plan_id,
        title: title.clone(),
        slug: slug.clone(),
        regions: m3u_file.parsed.regions.clone(),
        groups: vec![IngestPlanGroup {
            key: playlist_group_key.clone(),
            kind: FileGroupKind::Playlist,
            display_name: title,
            group_key: Some(format!("{slug}:playlist")),
            disc_index: None,
            disc_count: Some(references.len() as i64),
            launchable: true,
            metadata: json!({"preferred_launch": true}),
        }],
        files: vec![plan_file(
            m3u_file,
            Some(&playlist_group_key),
            FileRole::LaunchManifest,
            0,
            None,
            None,
            true,
            json!({"source": "user_supplied"}),
        )],
        dependencies: Vec::new(),
        errors: Vec::new(),
    };
    used.insert(m3u_index);
    let context = UserM3uContext {
        prepared,
        name_to_index,
        m3u_index,
        disc_count: references.len() as i64,
    };
    for (index, reference) in references.iter().enumerate() {
        context.add_reference(index, reference, used, &mut plan);
    }
    plan.finish()
}

struct UserM3uPlan {
    plan_id: String,
    title: String,
    slug: String,
    regions: Vec<String>,
    groups: Vec<IngestPlanGroup>,
    files: Vec<IngestPlanFile>,
    dependencies: Vec<IngestPlanDependency>,
    errors: Vec<IngestPlanError>,
}

impl UserM3uPlan {
    fn finish(self) -> Result<IngestPlanRom, Vec<IngestPlanError>> {
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        Ok(IngestPlanRom {
            plan_id: self.plan_id,
            title: self.title,
            slug: self.slug,
            regions: self.regions,
            groups: self.groups,
            files: self.files,
            dependencies: self.dependencies,
        })
    }
}

struct UserM3uContext<'a> {
    prepared: &'a [PreparedPlanFile],
    name_to_index: &'a HashMap<String, usize>,
    m3u_index: usize,
    disc_count: i64,
}

impl UserM3uContext<'_> {
    fn add_reference(
        &self,
        ref_index: usize,
        reference: &str,
        used: &mut HashSet<usize>,
        plan: &mut UserM3uPlan,
    ) {
        let Some(child_index) = resolve_manifest_reference(reference, self.name_to_index) else {
            plan.errors.push(ingest_error(
                "missing_manifest_dependency",
                &format!("manifest dependency {reference} is missing from the batch"),
                Some(&self.prepared[self.m3u_index].input.original_file_name),
            ));
            return;
        };
        let child = &self.prepared[child_index];
        if used.contains(&child_index) && child_index != self.m3u_index {
            plan.errors.push(ingest_error(
                "file_used_by_multiple_groups",
                "a batch file is referenced by more than one planned ROM",
                Some(&child.input.original_file_name),
            ));
            return;
        }

        let disc_index = child
            .parsed
            .disc
            .as_ref()
            .and_then(|disc| disc.index)
            .map(i64::from)
            .or(Some(ref_index as i64 + 1));
        let disc_count = child
            .parsed
            .disc
            .as_ref()
            .and_then(|disc| disc.count)
            .map(i64::from)
            .or(Some(self.disc_count));
        let display_index = disc_index.unwrap_or(ref_index as i64 + 1);
        let child_group_key = format!("{}:disc-{}", plan.plan_id, ref_index + 1);
        let descriptor = matches!(child.extension.as_str(), "cue" | "gdi");
        plan.groups.push(IngestPlanGroup {
            key: child_group_key.clone(),
            kind: if descriptor {
                FileGroupKind::TrackSet
            } else {
                FileGroupKind::Disc
            },
            display_name: format!("Disc {display_index}"),
            group_key: Some(format!("{}:disc-{display_index}", plan.slug)),
            disc_index,
            disc_count,
            launchable: true,
            metadata: json!({}),
        });
        let role = if descriptor {
            FileRole::Descriptor
        } else if is_disc_image_extension(&child.extension) {
            FileRole::DiscImage
        } else {
            FileRole::Content
        };
        plan.files.push(plan_file(
            child,
            Some(&child_group_key),
            role,
            10 + ref_index as i64 * 10,
            disc_index,
            None,
            true,
            json!({}),
        ));
        plan.dependencies.push(IngestPlanDependency {
            parent_file_key: file_key(self.m3u_index),
            child_file_key: file_key(child_index),
            dependency_kind: DependencyKind::PlaylistEntry,
            sort_index: ref_index as i64,
        });
        used.insert(child_index);

        if descriptor {
            add_descriptor_dependencies(
                self.prepared,
                self.name_to_index,
                child_index,
                &child_group_key,
                disc_index,
                11 + ref_index as i64 * 10,
                used,
                &mut plan.files,
                &mut plan.dependencies,
                &mut plan.errors,
            );
        }
    }
}

pub(super) fn plan_generated_m3u_rom(
    prepared: &[PreparedPlanFile],
    name_to_index: &HashMap<String, usize>,
    candidate_indices: &[usize],
    reserved_file_names: &HashSet<String>,
    used: &mut HashSet<usize>,
    rom_index: usize,
) -> Result<IngestPlanRom, Vec<IngestPlanError>> {
    let first = &prepared[candidate_indices[0]];
    let title = first.parsed.clean_title.clone();
    let slug = slugify(&title);
    let plan_id = format!("rom-{}", rom_index + 1);
    let generated_file_name = generated_m3u_file_name(first, reserved_file_names);
    let generated_file_key = generated_file_key(rom_index);
    let disc_count = generated_disc_count(prepared, candidate_indices);
    let mut errors = validate_generated_disc_set(prepared, candidate_indices, disc_count);

    if !errors.is_empty() {
        return Err(errors);
    }

    let playlist_group_key = format!("{plan_id}:playlist");
    let preview_file_names: Vec<String> = candidate_indices
        .iter()
        .map(|index| {
            sanitize_upload_file_name(&prepared[*index].input.original_file_name)
                .expect("candidate file name was validated before planning")
        })
        .collect();
    let preview_contents = format!("{}\n", preview_file_names.join("\n"));
    let mut groups = vec![IngestPlanGroup {
        key: playlist_group_key.clone(),
        kind: FileGroupKind::Playlist,
        display_name: title.clone(),
        group_key: Some(format!("{slug}:playlist")),
        disc_index: None,
        disc_count: Some(disc_count),
        launchable: true,
        metadata: json!({"preferred_launch": true, "source": "generated"}),
    }];
    let mut files = vec![IngestPlanFile {
        key: generated_file_key,
        group_key: Some(playlist_group_key),
        original_file_name: generated_file_name.clone(),
        file_size_bytes: Some(preview_contents.len() as u64),
        role: FileRole::LaunchManifest,
        sort_index: 0,
        disc_index: None,
        track_index: None,
        launchable: true,
        metadata: json!({"source": "generated", "preferred_launch": true}),
        parsed_filename: filename::parse(&generated_file_name),
    }];
    let mut dependencies = Vec::new();

    for (ref_index, child_index) in candidate_indices.iter().enumerate() {
        let child = &prepared[*child_index];
        let disc_index = child
            .parsed
            .disc
            .as_ref()
            .and_then(|disc| disc.index)
            .map(i64::from)
            .or(Some(ref_index as i64 + 1));
        let child_group_key = format!(
            "{plan_id}:disc-{}",
            disc_index.unwrap_or(ref_index as i64 + 1)
        );
        let child_group_kind = if matches!(child.extension.as_str(), "cue" | "gdi") {
            FileGroupKind::TrackSet
        } else {
            FileGroupKind::Disc
        };
        groups.push(IngestPlanGroup {
            key: child_group_key.clone(),
            kind: child_group_kind,
            display_name: format!("Disc {}", disc_index.unwrap_or(ref_index as i64 + 1)),
            group_key: Some(format!(
                "{slug}:disc-{}",
                disc_index.unwrap_or(ref_index as i64 + 1)
            )),
            disc_index,
            disc_count: Some(disc_count),
            launchable: true,
            metadata: json!({"disc_marker": child.parsed.disc.clone()}),
        });

        let role = if matches!(child.extension.as_str(), "cue" | "gdi") {
            FileRole::Descriptor
        } else {
            FileRole::DiscImage
        };
        files.push(plan_file(
            child,
            Some(&child_group_key),
            role,
            10 + ref_index as i64 * 10,
            disc_index,
            None,
            true,
            json!({}),
        ));
        dependencies.push(IngestPlanDependency {
            parent_file_key: generated_file_key,
            child_file_key: file_key(*child_index),
            dependency_kind: DependencyKind::PlaylistEntry,
            sort_index: ref_index as i64,
        });
        used.insert(*child_index);

        if matches!(child.extension.as_str(), "cue" | "gdi") {
            add_descriptor_dependencies(
                prepared,
                name_to_index,
                *child_index,
                &child_group_key,
                disc_index,
                11 + ref_index as i64 * 10,
                used,
                &mut files,
                &mut dependencies,
                &mut errors,
            );
        }
    }

    if errors.is_empty() {
        Ok(IngestPlanRom {
            plan_id,
            title,
            slug,
            regions: first.parsed.regions.clone(),
            groups,
            files,
            dependencies,
        })
    } else {
        Err(errors)
    }
}

fn generated_disc_count(prepared: &[PreparedPlanFile], candidate_indices: &[usize]) -> i64 {
    let explicit_count = candidate_indices
        .iter()
        .filter_map(|index| prepared[*index].parsed.disc.as_ref()?.count)
        .map(i64::from)
        .max();
    let max_index = candidate_indices
        .iter()
        .filter_map(|index| prepared[*index].parsed.disc.as_ref()?.index)
        .map(i64::from)
        .max()
        .unwrap_or(candidate_indices.len() as i64);

    explicit_count
        .unwrap_or(max_index)
        .max(max_index)
        .max(candidate_indices.len() as i64)
}

fn validate_generated_disc_set(
    prepared: &[PreparedPlanFile],
    candidate_indices: &[usize],
    disc_count: i64,
) -> Vec<IngestPlanError> {
    let mut errors = Vec::new();
    let mut present = BTreeSet::new();

    for index in candidate_indices {
        let file = &prepared[*index];
        let Some(disc_index) = file
            .parsed
            .disc
            .as_ref()
            .and_then(|disc| disc.index)
            .map(i64::from)
        else {
            continue;
        };

        if !present.insert(disc_index) {
            errors.push(ingest_error(
                "duplicate_disc_index",
                &format!("multi-disc set contains more than one file for disc {disc_index}"),
                Some(&file.input.original_file_name),
            ));
        }
    }

    let missing: Vec<String> = (1..=disc_count)
        .filter(|disc_index| !present.contains(disc_index))
        .map(|disc_index| disc_index.to_string())
        .collect();
    if !missing.is_empty() {
        errors.push(ingest_error(
            "missing_disc_file",
            &format!(
                "multi-disc set is missing disc {} of {disc_count}",
                missing.join(", ")
            ),
            Some(&prepared[candidate_indices[0]].input.original_file_name),
        ));
    }

    errors
}
