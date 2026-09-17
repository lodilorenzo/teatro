use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::{
    domain::rom::{FileGroupKind, FileRole},
    services::ingest::{filename, manifests},
};

use super::{
    super::{
        normalization::{
            extension_lower, file_key, independent_role, is_manifest_extension,
            normalized_edit_title, sanitize_upload_file_name, slugify,
        },
        types::{
            IngestPlan, IngestPlanError, IngestPlanFile, IngestPlanGroup, IngestPlanRom,
            IngestPlanWarning, LibraryServiceError, ManifestReferences, PlanInputFile,
            PlannedRomTitle, PreparedPlanFile,
        },
    },
    descriptors::plan_descriptor_rom,
    disc_sets::{generated_m3u_candidate_groups, plan_generated_m3u_rom, plan_m3u_rom},
};

pub(crate) fn build_ingest_plan(
    platform_id: i64,
    platform_slug: &str,
    inputs: Vec<PlanInputFile>,
    title_override: Option<String>,
) -> IngestPlan {
    let PreparedInputs {
        files,
        name_to_index,
        mut warnings,
        mut errors,
    } = prepare_inputs(inputs);

    let mut roms = if errors.is_empty() {
        let (roms, mut planning_errors) = plan_prepared_inputs(&files, &name_to_index);
        errors.append(&mut planning_errors);
        roms
    } else {
        Vec::new()
    };
    apply_title_override(&mut roms, title_override.as_deref(), &mut warnings);

    IngestPlan {
        platform_id,
        platform_slug: platform_slug.to_string(),
        roms,
        warnings,
        errors,
    }
}

struct PreparedInputs {
    files: Vec<PreparedPlanFile>,
    name_to_index: HashMap<String, usize>,
    warnings: Vec<IngestPlanWarning>,
    errors: Vec<IngestPlanError>,
}

fn prepare_inputs(inputs: Vec<PlanInputFile>) -> PreparedInputs {
    let mut prepared = PreparedInputs {
        files: Vec::with_capacity(inputs.len()),
        name_to_index: HashMap::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    for input in inputs {
        prepare_input(input, &mut prepared);
    }
    prepared
}

fn prepare_input(input: PlanInputFile, prepared: &mut PreparedInputs) {
    match sanitize_upload_file_name(&input.original_file_name) {
        Ok(file_name) => {
            if prepared
                .name_to_index
                .insert(file_name.clone(), input.index)
                .is_some()
            {
                prepared.errors.push(ingest_error(
                    "duplicate_file_name",
                    "batch uploads cannot contain duplicate filenames",
                    Some(&file_name),
                ));
            }
        }
        Err(_) => prepared.errors.push(ingest_error(
            "invalid_file_name",
            "upload filename is invalid",
            Some(&input.original_file_name),
        )),
    }

    let parsed = filename::parse(&input.original_file_name);
    if parsed.split_archive.is_some() {
        prepared.errors.push(ingest_error(
            "unsupported_split_archive",
            "split archive uploads are not supported yet",
            Some(&input.original_file_name),
        ));
    }

    let extension = extension_lower(&input.original_file_name);
    let manifest = parse_manifest_references(&extension, input.manifest_contents.as_deref())
        .unwrap_or_else(|error| {
            prepared.errors.push(ingest_error(
                "invalid_manifest",
                &error.to_string(),
                Some(&input.original_file_name),
            ));
            ManifestReferences::default()
        });
    if is_manifest_extension(&extension) && input.manifest_contents.is_none() {
        prepared.warnings.push(ingest_warning(
            "manifest_not_validated",
            "manifest dependencies require file contents and will be validated during upload",
            Some(&input.original_file_name),
        ));
    }

    prepared.files.push(PreparedPlanFile {
        input,
        parsed,
        extension,
        manifest,
    });
}

fn plan_prepared_inputs(
    prepared: &[PreparedPlanFile],
    name_to_index: &HashMap<String, usize>,
) -> (Vec<IngestPlanRom>, Vec<IngestPlanError>) {
    let mut used = HashSet::new();
    let mut roms = Vec::new();
    let mut errors = Vec::new();

    for file in prepared {
        if file.extension != "m3u"
            || file.manifest.m3u.is_none()
            || used.contains(&file.input.index)
        {
            continue;
        }
        collect_planned_rom(
            plan_m3u_rom(
                prepared,
                name_to_index,
                file.input.index,
                &mut used,
                roms.len(),
            ),
            &mut roms,
            &mut errors,
        );
    }

    let reserved_file_names: HashSet<String> = name_to_index
        .keys()
        .map(|file_name| file_name.to_ascii_lowercase())
        .collect();
    for candidate_indices in generated_m3u_candidate_groups(prepared, &used) {
        collect_planned_rom(
            plan_generated_m3u_rom(
                prepared,
                name_to_index,
                &candidate_indices,
                &reserved_file_names,
                &mut used,
                roms.len(),
            ),
            &mut roms,
            &mut errors,
        );
    }

    for file in prepared {
        if !matches!(file.extension.as_str(), "cue" | "gdi")
            || used.contains(&file.input.index)
            || !file.manifest.has_descriptor_references()
        {
            continue;
        }
        collect_planned_rom(
            plan_descriptor_rom(
                prepared,
                name_to_index,
                file.input.index,
                &mut used,
                roms.len(),
            ),
            &mut roms,
            &mut errors,
        );
    }

    for file in prepared {
        if used.insert(file.input.index) {
            roms.push(plan_single_rom(file, roms.len()));
        }
    }
    (roms, errors)
}

fn collect_planned_rom(
    planned: Result<IngestPlanRom, Vec<IngestPlanError>>,
    roms: &mut Vec<IngestPlanRom>,
    errors: &mut Vec<IngestPlanError>,
) {
    match planned {
        Ok(rom) => roms.push(rom),
        Err(mut plan_errors) => errors.append(&mut plan_errors),
    }
}

fn apply_title_override(
    roms: &mut [IngestPlanRom],
    title_override: Option<&str>,
    warnings: &mut Vec<IngestPlanWarning>,
) {
    let Some(title) = title_override
        .map(str::trim)
        .filter(|title| !title.is_empty())
    else {
        return;
    };
    if let [rom] = roms {
        rom.title = title.to_string();
        rom.slug = slugify(title);
    } else {
        warnings.push(ingest_warning(
            "title_override_ignored",
            "title override is ignored for batch uploads that create multiple ROMs",
            None,
        ));
    }
}

pub(crate) fn apply_planned_titles(
    plan: &mut IngestPlan,
    planned_titles: &[PlannedRomTitle],
) -> Result<(), LibraryServiceError> {
    if planned_titles.is_empty() {
        return Ok(());
    }
    if planned_titles.len() != plan.roms.len() {
        return Err(LibraryServiceError::InvalidIngestPlan(
            "planned titles do not match the regenerated upload plan".to_string(),
        ));
    }

    let mut titles_by_plan = HashMap::with_capacity(planned_titles.len());
    for planned_title in planned_titles {
        let plan_id = planned_title.plan_id.trim();
        if plan_id.is_empty() {
            return Err(LibraryServiceError::InvalidIngestPlan(
                "planned title has an empty plan_id".to_string(),
            ));
        }
        let title = normalized_edit_title(&planned_title.title)?;
        if titles_by_plan.insert(plan_id.to_string(), title).is_some() {
            return Err(LibraryServiceError::InvalidIngestPlan(format!(
                "planned title {plan_id} was supplied more than once"
            )));
        }
    }

    for rom in &mut plan.roms {
        let title = titles_by_plan.remove(&rom.plan_id).ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan(format!(
                "planned title for {} is missing",
                rom.plan_id
            ))
        })?;
        let previous_title = std::mem::replace(&mut rom.title, title.clone());
        let previous_slug = std::mem::replace(&mut rom.slug, slugify(&title));

        for group in &mut rom.groups {
            if group.display_name == previous_title {
                group.display_name = title.clone();
            }
            if let Some(group_key) = group.group_key.as_mut()
                && let Some(suffix) = group_key.strip_prefix(&previous_slug)
                && (suffix.is_empty() || suffix.starts_with(':'))
            {
                *group_key = format!("{}{suffix}", rom.slug);
            }
        }
    }

    if let Some(unmatched_plan_id) = titles_by_plan.keys().next() {
        return Err(LibraryServiceError::InvalidIngestPlan(format!(
            "planned title {unmatched_plan_id} is not present in the regenerated upload plan"
        )));
    }

    Ok(())
}

fn plan_single_rom(file: &PreparedPlanFile, rom_index: usize) -> IngestPlanRom {
    let title = file.parsed.clean_title.clone();
    let slug = slugify(&title);
    let plan_id = format!("rom-{}", rom_index + 1);
    let group_key = format!("{plan_id}:single");
    IngestPlanRom {
        plan_id,
        title: title.clone(),
        slug: slug.clone(),
        regions: file.parsed.regions.clone(),
        groups: vec![IngestPlanGroup {
            key: group_key.clone(),
            kind: FileGroupKind::Single,
            display_name: title,
            group_key: Some(slug),
            disc_index: file
                .parsed
                .disc
                .as_ref()
                .and_then(|disc| disc.index)
                .map(i64::from),
            disc_count: file
                .parsed
                .disc
                .as_ref()
                .and_then(|disc| disc.count)
                .map(i64::from),
            launchable: true,
            metadata: json!({}),
        }],
        files: vec![plan_file(
            file,
            Some(&group_key),
            independent_role(&file.extension),
            0,
            file.parsed
                .disc
                .as_ref()
                .and_then(|disc| disc.index)
                .map(i64::from),
            None,
            true,
            json!({}),
        )],
        dependencies: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn plan_file(
    file: &PreparedPlanFile,
    group_key: Option<&str>,
    role: FileRole,
    sort_index: i64,
    disc_index: Option<i64>,
    track_index: Option<i64>,
    launchable: bool,
    metadata: Value,
) -> IngestPlanFile {
    IngestPlanFile {
        key: file_key(file.input.index),
        group_key: group_key.map(ToOwned::to_owned),
        original_file_name: file.input.original_file_name.clone(),
        file_size_bytes: file.input.file_size_bytes,
        role,
        sort_index,
        disc_index,
        track_index,
        launchable,
        metadata,
        parsed_filename: file.parsed.clone(),
    }
}

fn parse_manifest_references(
    extension: &str,
    contents: Option<&str>,
) -> Result<ManifestReferences, manifests::ManifestParseError> {
    let Some(contents) = contents else {
        return Ok(ManifestReferences::default());
    };

    match extension {
        "m3u" => Ok(ManifestReferences {
            m3u: Some(manifests::parse_m3u_lines(contents)?),
            ..ManifestReferences::default()
        }),
        "cue" => Ok(ManifestReferences {
            cue: Some(manifests::parse_cue_file_references(contents)?),
            ..ManifestReferences::default()
        }),
        "gdi" => Ok(ManifestReferences {
            gdi: Some(manifests::parse_gdi_track_references(contents)?),
            ..ManifestReferences::default()
        }),
        _ => Ok(ManifestReferences::default()),
    }
}

impl ManifestReferences {
    pub(super) fn has_descriptor_references(&self) -> bool {
        self.cue.as_ref().is_some_and(|refs| !refs.is_empty())
            || self.gdi.as_ref().is_some_and(|refs| !refs.is_empty())
    }
}

pub(super) fn resolve_manifest_reference(
    reference: &str,
    name_to_index: &HashMap<String, usize>,
) -> Option<usize> {
    if reference.contains(['/', '\\']) {
        return None;
    }
    name_to_index.get(reference).copied()
}

pub(crate) fn summarize_ingest_errors(errors: &[IngestPlanError]) -> String {
    errors
        .iter()
        .map(|error| match error.file_name.as_deref() {
            Some(file_name) => format!("{}: {}", file_name, error.message),
            None => error.message.clone(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn ingest_error(code: &str, message: &str, file_name: Option<&str>) -> IngestPlanError {
    IngestPlanError {
        code: code.to_string(),
        message: message.to_string(),
        file_name: file_name.map(ToOwned::to_owned),
    }
}

fn ingest_warning(code: &str, message: &str, file_name: Option<&str>) -> IngestPlanWarning {
    IngestPlanWarning {
        code: code.to_string(),
        message: message.to_string(),
        file_name: file_name.map(ToOwned::to_owned),
    }
}
