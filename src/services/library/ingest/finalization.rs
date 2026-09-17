//! Filesystem reservation and repository payload construction for validated ingest plans.
//!
//! This stage runs while the caller owns the library-root lock. It assigns every path,
//! rejects collisions, materializes generated manifests, and returns declarative file
//! operations plus one persistence graph per planned ROM.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    domain::{ingest::PlanFileKey, platform::Platform, rom::DependencyKind},
    repositories::roms,
    state::AppState,
    storage::paths::{self, PathSafetyError},
};

use super::super::{
    normalization::{
        available_file_name, join_relative_path, m3u_directory_name, sanitize_upload_file_name,
        upload_filename_metadata,
    },
    types::{
        BatchFileOperation, BatchFinalization, InPlaceSourceFile, IngestPlan, IngestPlanDependency,
        IngestPlanFile, IngestPlanRom, LibraryServiceError, PlanInputFile,
    },
};

pub(crate) async fn prepare_batch_finalization(
    state: &AppState,
    root_path: &Path,
    root_id: i64,
    platform: &Platform,
    plan: &IngestPlan,
    inputs: &[PlanInputFile],
) -> Result<BatchFinalization, LibraryServiceError> {
    let mut context =
        BatchFinalizationContext::new(state, root_path, root_id, platform, inputs).await?;
    let mut roms = Vec::with_capacity(plan.roms.len());
    for planned_rom in &plan.roms {
        roms.push(context.prepare_rom(planned_rom).await?);
    }
    Ok(BatchFinalization {
        roms,
        operations: context.operations,
    })
}

pub(crate) async fn prepare_in_place_finalization(
    state: &AppState,
    root_path: &Path,
    root_id: i64,
    platform: &Platform,
    planned_rom: &IngestPlanRom,
    sources: &[InPlaceSourceFile],
) -> Result<BatchFinalization, LibraryServiceError> {
    let source_by_index: HashMap<usize, &InPlaceSourceFile> = sources
        .iter()
        .map(|source| (source.index, source))
        .collect();
    let parent = sources
        .first()
        .and_then(|source| Path::new(&source.relative_path).parent())
        .ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan("in-place ingest has no source parent".into())
        })?;
    if sources
        .iter()
        .any(|source| Path::new(&source.relative_path).parent() != Some(parent))
    {
        return Err(LibraryServiceError::InvalidIngestPlan(
            "in-place ingest sources must share one directory".into(),
        ));
    }

    let mut file_name_by_key = HashMap::with_capacity(planned_rom.files.len());
    for planned_file in &planned_rom.files {
        let file_name = match planned_file.key {
            PlanFileKey::Uploaded(index) => source_by_index
                .get(&index)
                .map(|source| source.file_name.clone())
                .ok_or_else(|| {
                    LibraryServiceError::InvalidIngestPlan(format!(
                        "planned file source {} is missing",
                        planned_file.key
                    ))
                })?,
            PlanFileKey::GeneratedM3u(_) => {
                sanitize_upload_file_name(&planned_file.original_file_name)?
            }
        };
        file_name_by_key.insert(planned_file.key, file_name);
    }

    let mut operations = Vec::new();
    let mut files = Vec::with_capacity(planned_rom.files.len());
    for planned_file in &planned_rom.files {
        let file_name = file_name_by_key
            .get(&planned_file.key)
            .expect("every planned file was assigned")
            .clone();
        let (relative_path, file_size_bytes) = match planned_file.key {
            PlanFileKey::Uploaded(index) => {
                let source = source_by_index.get(&index).expect("source was assigned");
                (
                    source.relative_path.clone(),
                    i64::try_from(source.file_size_bytes)
                        .map_err(|_| LibraryServiceError::UploadTooLarge)?,
                )
            }
            PlanFileKey::GeneratedM3u(_) => {
                let relative_path = join_relative_path(parent, &file_name);
                if state.file_store().exists(root_path, &relative_path).await?
                    || roms::relative_path_exists(state.db(), root_id, &relative_path).await?
                {
                    return Err(LibraryServiceError::NoAvailableFileName);
                }
                let contents =
                    generated_m3u_contents(planned_rom, &planned_file.key, &file_name_by_key)?;
                let size = i64::try_from(contents.len())
                    .map_err(|_| LibraryServiceError::UploadTooLarge)?;
                operations.push(BatchFileOperation::WriteGenerated {
                    relative_path: relative_path.clone(),
                    contents,
                });
                (relative_path, size)
            }
        };
        files.push(roms::CreateRomFile {
            key: planned_file.key,
            group_key: planned_file.group_key.clone(),
            root_id,
            relative_path,
            file_name,
            original_file_name: planned_file.original_file_name.clone(),
            file_size_bytes,
            is_primary: planned_file.launchable && planned_file.sort_index == 0,
            role: planned_file.role,
            sort_index: planned_file.sort_index,
            disc_index: planned_file.disc_index,
            track_index: planned_file.track_index,
            launchable: planned_file.launchable,
            metadata_json: planned_file.metadata.to_string(),
        });
    }

    let mut used_slugs = HashSet::new();
    let slug =
        unique_slug_for_batch(state, platform.id, &planned_rom.slug, &mut used_slugs).await?;
    Ok(BatchFinalization {
        roms: vec![build_grouped_rom(platform.id, planned_rom, slug, files)?],
        operations,
    })
}

struct BatchFinalizationContext<'a> {
    state: &'a AppState,
    root_path: &'a Path,
    root_id: i64,
    platform: &'a Platform,
    platform_relative: PathBuf,
    input_by_index: HashMap<usize, &'a PlanInputFile>,
    used_slugs: HashSet<String>,
    operations: Vec<BatchFileOperation>,
}

impl<'a> BatchFinalizationContext<'a> {
    async fn new(
        state: &'a AppState,
        root_path: &'a Path,
        root_id: i64,
        platform: &'a Platform,
        inputs: &'a [PlanInputFile],
    ) -> Result<Self, LibraryServiceError> {
        let platform_relative = paths::clean_relative_path(&platform.fs_slug)?;
        state
            .file_store()
            .create_dir_all(root_path, &platform_relative)
            .await?;
        Ok(Self {
            state,
            root_path,
            root_id,
            platform,
            platform_relative,
            input_by_index: inputs.iter().map(|input| (input.index, input)).collect(),
            used_slugs: HashSet::new(),
            operations: Vec::new(),
        })
    }

    async fn prepare_rom(
        &mut self,
        planned_rom: &IngestPlanRom,
    ) -> Result<roms::CreateGroupedRomParams, LibraryServiceError> {
        let (slug, directory_name) = self.reserve_rom_location(planned_rom).await?;
        let (assigned_files, file_name_by_key) = self
            .assign_files(planned_rom, directory_name.as_deref())
            .await?;
        let create_files =
            self.materialize_files(planned_rom, assigned_files, &file_name_by_key)?;
        build_grouped_rom(self.platform.id, planned_rom, slug, create_files)
    }

    async fn reserve_rom_location(
        &mut self,
        planned_rom: &IngestPlanRom,
    ) -> Result<(String, Option<String>), LibraryServiceError> {
        let slug = unique_slug_for_batch(
            self.state,
            self.platform.id,
            &planned_rom.slug,
            &mut self.used_slugs,
        )
        .await?;
        let directory_name = if planned_rom.files.len() > 1 {
            Some(
                if let Some(directory_name) = m3u_directory_name(planned_rom)? {
                    let relative_path =
                        join_relative_path(&self.platform_relative, &directory_name);
                    if self
                        .state
                        .file_store()
                        .exists(self.root_path, &relative_path)
                        .await?
                    {
                        return Err(LibraryServiceError::NoAvailableFileName);
                    }
                    directory_name
                } else {
                    available_directory_name(
                        self.state,
                        self.root_path,
                        &self.platform_relative,
                        &slug,
                    )
                    .await?
                },
            )
        } else {
            None
        };
        if let Some(directory_name) = directory_name.as_deref() {
            self.state
                .file_store()
                .create_dir_all(self.root_path, &self.platform_relative.join(directory_name))
                .await?;
        }
        Ok((slug, directory_name))
    }

    async fn assign_files<'p>(
        &self,
        planned_rom: &'p IngestPlanRom,
        directory_name: Option<&str>,
    ) -> Result<(Vec<AssignedBatchFile<'p>>, HashMap<PlanFileKey, String>), LibraryServiceError>
    {
        let mut assigned = Vec::with_capacity(planned_rom.files.len());
        let mut file_name_by_key = HashMap::new();
        let mut relative_paths = HashSet::new();
        for planned_file in &planned_rom.files {
            let (staged_path, uploaded_size) = self.upload_source(planned_file)?;
            let original_file_name = sanitize_upload_file_name(&planned_file.original_file_name)?;
            let file_name = if directory_name.is_some() {
                original_file_name.clone()
            } else {
                available_file_name(
                    self.state,
                    self.root_id,
                    self.root_path,
                    &self.platform_relative,
                    &original_file_name,
                )
                .await?
            };
            let parent = directory_name
                .map(|directory| self.platform_relative.join(directory))
                .unwrap_or_else(|| self.platform_relative.clone());
            let relative_path = join_relative_path(&parent, &file_name);
            self.ensure_path_available(&relative_path, &mut relative_paths)
                .await?;
            file_name_by_key.insert(planned_file.key, file_name.clone());
            assigned.push(AssignedBatchFile {
                planned_file,
                staged_path,
                relative_path,
                file_name,
                original_file_name,
                uploaded_size,
            });
        }
        Ok((assigned, file_name_by_key))
    }

    fn upload_source(
        &self,
        planned_file: &IngestPlanFile,
    ) -> Result<(Option<PathBuf>, Option<i64>), LibraryServiceError> {
        let PlanFileKey::Uploaded(source_index) = planned_file.key else {
            return Ok((None, None));
        };
        let input = self.input_by_index.get(&source_index).ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan(format!(
                "planned file source {} is missing",
                planned_file.key
            ))
        })?;
        let staged_path = input.staged_path.clone().ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan(format!(
                "planned file {} has no staged upload",
                planned_file.original_file_name
            ))
        })?;
        let size = i64::try_from(input.file_size_bytes.unwrap_or_default())
            .map_err(|_| LibraryServiceError::UploadTooLarge)?;
        Ok((Some(staged_path), Some(size)))
    }

    async fn ensure_path_available(
        &self,
        relative_path: &str,
        batch_paths: &mut HashSet<String>,
    ) -> Result<(), LibraryServiceError> {
        let available = batch_paths.insert(relative_path.to_string())
            && !self
                .state
                .file_store()
                .exists(self.root_path, relative_path)
                .await?
            && !roms::relative_path_exists(self.state.db(), self.root_id, relative_path).await?;
        if available {
            Ok(())
        } else {
            Err(LibraryServiceError::NoAvailableFileName)
        }
    }

    fn materialize_files(
        &mut self,
        planned_rom: &IngestPlanRom,
        assigned_files: Vec<AssignedBatchFile<'_>>,
        file_name_by_key: &HashMap<PlanFileKey, String>,
    ) -> Result<Vec<roms::CreateRomFile>, LibraryServiceError> {
        assigned_files
            .into_iter()
            .map(|assigned| {
                let file_size_bytes =
                    self.materialize_file(planned_rom, &assigned, file_name_by_key)?;
                Ok(roms::CreateRomFile {
                    key: assigned.planned_file.key,
                    group_key: assigned.planned_file.group_key.clone(),
                    root_id: self.root_id,
                    relative_path: assigned.relative_path,
                    file_name: assigned.file_name,
                    original_file_name: assigned.original_file_name,
                    file_size_bytes,
                    is_primary: assigned.planned_file.launchable
                        && assigned.planned_file.sort_index == 0,
                    role: assigned.planned_file.role,
                    sort_index: assigned.planned_file.sort_index,
                    disc_index: assigned.planned_file.disc_index,
                    track_index: assigned.planned_file.track_index,
                    launchable: assigned.planned_file.launchable,
                    metadata_json: assigned.planned_file.metadata.to_string(),
                })
            })
            .collect()
    }

    fn materialize_file(
        &mut self,
        planned_rom: &IngestPlanRom,
        assigned: &AssignedBatchFile<'_>,
        file_name_by_key: &HashMap<PlanFileKey, String>,
    ) -> Result<i64, LibraryServiceError> {
        if let Some(staged_path) = assigned.staged_path.clone() {
            self.operations.push(BatchFileOperation::Move {
                staged_path,
                relative_path: assigned.relative_path.clone(),
            });
            return assigned.uploaded_size.ok_or_else(|| {
                LibraryServiceError::InvalidIngestPlan(format!(
                    "planned file {} has no uploaded size",
                    assigned.original_file_name
                ))
            });
        }

        let contents =
            generated_m3u_contents(planned_rom, &assigned.planned_file.key, file_name_by_key)?;
        let size =
            i64::try_from(contents.len()).map_err(|_| LibraryServiceError::UploadTooLarge)?;
        self.operations.push(BatchFileOperation::WriteGenerated {
            relative_path: assigned.relative_path.clone(),
            contents,
        });
        Ok(size)
    }
}

struct AssignedBatchFile<'a> {
    planned_file: &'a IngestPlanFile,
    staged_path: Option<PathBuf>,
    relative_path: String,
    file_name: String,
    original_file_name: String,
    uploaded_size: Option<i64>,
}

fn build_grouped_rom(
    platform_id: i64,
    planned_rom: &IngestPlanRom,
    slug: String,
    files: Vec<roms::CreateRomFile>,
) -> Result<roms::CreateGroupedRomParams, LibraryServiceError> {
    let metadata_json = planned_rom
        .files
        .iter()
        .find(|file| file.launchable)
        .or_else(|| planned_rom.files.first())
        .and_then(|file| upload_filename_metadata(&planned_rom.title, &file.parsed_filename));
    Ok(roms::CreateGroupedRomParams {
        platform_id,
        name: planned_rom.title.clone(),
        slug,
        regions_json: serde_json::to_string(&planned_rom.regions)?,
        metadata_json,
        groups: planned_rom
            .groups
            .iter()
            .map(|group| roms::CreateRomFileGroup {
                key: group.key.clone(),
                kind: group.kind,
                display_name: group.display_name.clone(),
                group_key: group.group_key.clone(),
                disc_index: group.disc_index,
                disc_count: group.disc_count,
                launchable: group.launchable,
                metadata_json: group.metadata.to_string(),
            })
            .collect(),
        files,
        dependencies: planned_rom
            .dependencies
            .iter()
            .map(|dependency| roms::CreateRomFileDependency {
                parent_file_key: dependency.parent_file_key,
                child_file_key: dependency.child_file_key,
                dependency_kind: dependency.dependency_kind,
                sort_index: dependency.sort_index,
            })
            .collect(),
    })
}

fn generated_m3u_contents(
    planned_rom: &IngestPlanRom,
    generated_file_key: &PlanFileKey,
    file_name_by_key: &HashMap<PlanFileKey, String>,
) -> Result<String, LibraryServiceError> {
    let mut playlist_entries: Vec<&IngestPlanDependency> = planned_rom
        .dependencies
        .iter()
        .filter(|dependency| {
            dependency.parent_file_key == *generated_file_key
                && dependency.dependency_kind == DependencyKind::PlaylistEntry
        })
        .collect();
    playlist_entries.sort_by_key(|dependency| dependency.sort_index);

    if playlist_entries.is_empty() {
        return Err(LibraryServiceError::InvalidIngestPlan(format!(
            "generated manifest {generated_file_key} has no playlist entries"
        )));
    }

    let mut lines = Vec::with_capacity(playlist_entries.len());
    for entry in playlist_entries {
        let file_name = file_name_by_key.get(&entry.child_file_key).ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan(format!(
                "generated manifest dependency {} is missing a file name",
                entry.child_file_key
            ))
        })?;
        if sanitize_upload_file_name(file_name).is_err() {
            return Err(LibraryServiceError::InvalidIngestPlan(format!(
                "generated manifest entry {file_name} is not a safe relative file name"
            )));
        }
        lines.push(file_name.clone());
    }

    Ok(format!("{}\n", lines.join("\n")))
}

async fn unique_slug_for_batch(
    state: &AppState,
    platform_id: i64,
    base_slug: &str,
    used_slugs: &mut HashSet<String>,
) -> Result<String, LibraryServiceError> {
    let base_slug = if base_slug.is_empty() {
        "rom"
    } else {
        base_slug
    };

    for attempt in 0..1000 {
        let slug = if attempt == 0 {
            base_slug.to_string()
        } else {
            format!("{base_slug}-{}", attempt + 1)
        };

        if used_slugs.contains(&slug) {
            continue;
        }
        if !roms::slug_exists(state.db(), platform_id, &slug).await? {
            used_slugs.insert(slug.clone());
            return Ok(slug);
        }
    }

    Err(LibraryServiceError::NoAvailableFileName)
}

async fn available_directory_name(
    state: &AppState,
    root_path: &Path,
    platform_relative: &Path,
    slug: &str,
) -> Result<String, LibraryServiceError> {
    let root_path = root_path.canonicalize()?;
    let platform_dir = root_path.join(platform_relative).canonicalize()?;
    if !platform_dir.starts_with(&root_path) {
        return Err(LibraryServiceError::PathSafety(
            PathSafetyError::EscapesRoot,
        ));
    }

    for attempt in 0..1000 {
        let directory_name = if attempt == 0 {
            slug.to_string()
        } else {
            format!("{slug}-{}", attempt + 1)
        };
        let directory_path = platform_relative.join(&directory_name);
        let directory_relative = directory_path.to_string_lossy().replace('\\', "/");
        if !state
            .file_store()
            .exists(&root_path, &directory_relative)
            .await?
        {
            return Ok(directory_name);
        }
    }

    Err(LibraryServiceError::NoAvailableFileName)
}
