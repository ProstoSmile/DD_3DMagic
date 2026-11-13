//! Checked, D:-only workspace storage for staged reference-system outputs.
//!
//! This module manages paths and durable file publication only. It does not
//! invoke Blender or launch child processes.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("journal JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("UNSUPPORTED_OPERATION: managed storage requires Windows and drive D:")]
    UnsupportedPlatform,
    #[error("PATH_OUTSIDE_D: workspace root must be an absolute path on D:\\: {0}")]
    InvalidRoot(String),
    #[error("relative path must contain only safe normal components: {0}")]
    UnsafePath(String),
    #[error("path is outside the managed workspace: {0}")]
    OutsideWorkspace(String),
    #[error("path component is not a directory: {0}")]
    NotDirectory(String),
    #[error("path component is not a regular file: {0}")]
    NotFile(String),
    #[error("output already exists: {0}")]
    Collision(String),
    #[error("workspace is busy with another publication, recovery, or retention operation")]
    Busy,
    #[error("insufficient free space: need {required} bytes, have {available}")]
    InsufficientSpace { required: u64, available: u64 },
    #[error("invalid managed name: {0}")]
    InvalidName(String),
    #[error("invalid backup policy: max_versions must be at least 1")]
    InvalidBackupPolicy,
    #[error("publication journal is invalid: {0}")]
    InvalidJournal(String),
    #[error("SAVE_VALIDATION_FAILED: final scene or manifest changed during the external job")]
    BaselineChanged,
    #[error("publication failed: {cause}; recovery journal: {journal}")]
    PublicationRecovery { cause: String, journal: PathBuf },
    #[error("simulated publication interruption after step {0}")]
    SimulatedFailure(usize),
}

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone)]
pub struct ManagedWorkspace {
    root: PathBuf,
    backup_policy: BackupPolicy,
    expected_publication: Option<PublicationBaseline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationBaseline {
    pub scene_sha256: Option<String>,
    pub manifest_sha256: Option<String>,
}

struct WorkspaceLock {
    _file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPolicy {
    /// Number of completed backup versions to retain, including the newest one.
    pub max_versions: usize,
    /// Optional byte limit for retained backup files and metadata.
    pub max_total_bytes: Option<u64>,
}

impl Default for BackupPolicy {
    fn default() -> Self {
        Self {
            max_versions: 5,
            max_total_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BackupRetentionReport {
    pub retained_versions: usize,
    pub retained_bytes: u64,
    pub pruned_versions: usize,
    pub pending_versions: usize,
    pub over_byte_limit: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Publication {
    pub scene_path: PathBuf,
    pub manifest_path: PathBuf,
    /// Present when publication preserved at least one previous final file.
    pub backup_directory: Option<PathBuf>,
    pub retention: BackupRetentionReport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RecoveryReport {
    pub rolled_back: usize,
    pub committed_cleanups: usize,
    pub retention: BackupRetentionReport,
}

#[derive(Debug, Serialize, Deserialize)]
struct PublicationJournal {
    version: u32,
    id: String,
    category: String,
    subcategory: String,
    slug: String,
    staged_scene: String,
    staged_manifest: String,
    scene_existed: bool,
    manifest_existed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupMetadata {
    version: u32,
    id: String,
    completed_at_unix_nanos: u64,
    scene_backup: bool,
    manifest_backup: bool,
}

#[derive(Debug, Clone)]
struct BackupCandidate {
    id: String,
    directory: PathBuf,
    completed_at_unix_nanos: u64,
    total_bytes: u64,
    scene_backup: bool,
    manifest_backup: bool,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDiskFreeSpaceExW(
        directory_name: *const u16,
        free_bytes_available_to_caller: *mut u64,
        total_number_of_bytes: *mut u64,
        total_number_of_free_bytes: *mut u64,
    ) -> i32;
}

impl ManagedWorkspace {
    /// Creates and canonicalizes an explicit D:-drive workspace root.
    ///
    /// On non-Windows platforms this API is intentionally unsupported.
    pub fn new(root: &Path) -> StorageResult<Self> {
        #[cfg(windows)]
        {
            let root = ensure_d_root(root)?;
            Ok(Self {
                root,
                backup_policy: BackupPolicy::default(),
                expected_publication: None,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = root;
            Err(StorageError::UnsupportedPlatform)
        }
    }

    /// Returns the canonical managed root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Capture a compare-and-swap baseline before a long external job.
    pub fn publication_baseline(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
    ) -> StorageResult<PublicationBaseline> {
        let _lock = self.acquire_workspace_lock()?;
        self.baseline_unlocked(category, subcategory, slug)
    }

    fn baseline_unlocked(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
    ) -> StorageResult<PublicationBaseline> {
        let scene = scene_relative(category, subcategory, slug)?;
        validate_name(slug)?;
        let manifest = PathBuf::from("assets").join(slug).join("asset.toml");
        let digest = |relative: &Path| -> StorageResult<Option<String>> {
            if !self.safe_exists(relative)? {
                return Ok(None);
            }
            let path = self.existing(relative)?;
            require_regular_file(&path)?;
            let mut file = File::open(path)?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            Ok(Some(
                hash.finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            ))
        };
        Ok(PublicationBaseline {
            scene_sha256: digest(&scene)?,
            manifest_sha256: digest(&manifest)?,
        })
    }

    /// Recheck both finals under the publication lock; artist edits made during Blender
    /// execution must never be silently replaced.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_scene_checked(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
        scene: &Path,
        manifest: &Path,
        baseline: &PublicationBaseline,
    ) -> StorageResult<Publication> {
        let mut workspace = self.clone();
        workspace.expected_publication = Some(baseline.clone());
        workspace.publish_scene(category, subcategory, slug, scene, manifest)
    }

    /// Returns the current per-workspace backup retention policy.
    pub fn backup_policy(&self) -> BackupPolicy {
        self.backup_policy
    }

    /// Sets a retention policy; at least one completed backup is always preserved.
    pub fn set_backup_policy(&mut self, policy: BackupPolicy) -> StorageResult<()> {
        if policy.max_versions == 0 {
            return Err(StorageError::InvalidBackupPolicy);
        }
        self.backup_policy = policy;
        Ok(())
    }

    /// Safely creates a directory beneath the workspace and returns its canonical path.
    pub fn directory(&self, relative: &Path) -> StorageResult<PathBuf> {
        let parts = relative_parts(relative)?;
        let mut current = self.root.clone();
        for part in parts {
            current.push(part);
            ensure_directory_component(&current)?;
            ensure_inside_root(&self.root, &current)?;
        }
        Ok(current.canonicalize()?)
    }

    /// Reserve a fresh staging directory in one filesystem operation.
    pub fn new_staging_directory(&self, relative: &Path) -> StorageResult<PathBuf> {
        let parts = relative_parts(relative)?;
        if parts.len() < 3
            || parts[0] != OsStr::new(".assetlab")
            || parts[1] != OsStr::new("staging")
        {
            return Err(StorageError::UnsafePath(relative.display().to_string()));
        }
        let parent = self.directory(&path_from_parts(&parts[..parts.len() - 1]))?;
        let destination = parent.join(parts.last().expect("validated path"));
        fs::create_dir(&destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StorageError::Collision(destination.display().to_string())
            } else {
                StorageError::Io(error)
            }
        })?;
        self.existing(relative)
    }

    /// Resolves an existing regular file or directory after checking every path component.
    pub fn existing(&self, relative: &Path) -> StorageResult<PathBuf> {
        let parts = relative_parts(relative)?;
        let mut current = self.root.clone();
        for (index, part) in parts.iter().enumerate() {
            current.push(part);
            let metadata = checked_metadata(&current)?;
            if index + 1 < parts.len() && !metadata.is_dir() {
                return Err(StorageError::NotDirectory(current.display().to_string()));
            }
        }
        let canonical = current.canonicalize()?;
        ensure_inside_root(&self.root, &canonical)?;
        Ok(canonical)
    }

    /// Writes a new file beneath `.assetlab/staging`; existing outputs are never replaced.
    pub fn write_new(&self, relative: &Path, bytes: &[u8]) -> StorageResult<PathBuf> {
        let parts = relative_parts(relative)?;
        if parts.len() < 3
            || parts[0] != OsStr::new(".assetlab")
            || parts[1] != OsStr::new("staging")
        {
            return Err(StorageError::UnsafePath(format!(
                "new outputs must be beneath .assetlab/staging: {}",
                relative.display()
            )));
        }
        self.ensure_free_space(bytes.len() as u64)?;
        let file_name = parts.last().expect("validated non-empty path");
        let parent = path_from_parts(&parts[..parts.len() - 1]);
        let parent_path = if parent.as_os_str().is_empty() {
            self.root.clone()
        } else {
            self.directory(&parent)?
        };
        let destination = parent_path.join(file_name);
        ensure_new_target_parent(self, &destination)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    StorageError::Collision(destination.display().to_string())
                } else {
                    StorageError::Io(error)
                }
            })?;
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(StorageError::Io(error));
        }
        let canonical = destination.canonicalize()?;
        ensure_inside_root(&self.root, &canonical)?;
        Ok(canonical)
    }

    /// Returns the managed final scene path, creating its parent directories safely.
    pub fn final_scene(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
    ) -> StorageResult<PathBuf> {
        let relative = scene_relative(category, subcategory, slug)?;
        self.target_path(&relative)
    }

    /// Returns the v2 asset manifest path associated with a final scene.
    pub fn final_manifest(&self, slug: &str) -> StorageResult<PathBuf> {
        validate_name(slug)?;
        let relative = PathBuf::from("assets").join(slug).join("asset.toml");
        self.target_path(&relative)
    }

    /// Promote a complete reference bundle to its immutable per-asset revision path.
    pub fn promote_reference_revision(
        &self,
        staged: &Path,
        slug: &str,
        revision_key: &str,
    ) -> StorageResult<PathBuf> {
        validate_name(slug)?;
        validate_name(revision_key)?;
        let staged = staged_relative(&self.root, staged)?;
        let source = self.existing(&staged)?;
        if !source.is_dir() {
            return Err(StorageError::NotDirectory(source.display().to_string()));
        }
        for entry in fs::read_dir(&source)? {
            require_regular_file(&entry?.path())?;
        }
        let relative = PathBuf::from("assets")
            .join(slug)
            .join("references")
            .join("technical")
            .join(revision_key);
        let target_parent = self.directory(relative.parent().expect("validated revision path"))?;
        let target = target_parent.join(revision_key);
        let _lock = self.acquire_workspace_lock()?;
        if self.safe_exists(&relative)? {
            let existing = self.existing(&relative)?;
            if existing.is_dir() && same_regular_directory_contents(&source, &existing)? {
                return Ok(relative);
            }
            return Err(StorageError::Collision(target.display().to_string()));
        }
        fs::rename(source, target)?;
        Ok(relative)
    }

    /// Creates isolated child-process directories and returns environment pairs.
    /// The caller chooses whether and when to apply them to a child process.
    pub fn child_environment(&self) -> StorageResult<Vec<(String, PathBuf)>> {
        let mapping = [
            ("TEMP", PathBuf::from(".assetlab/tmp")),
            ("TMP", PathBuf::from(".assetlab/tmp")),
            ("CARGO_TARGET_DIR", PathBuf::from(".assetlab/cargo-target")),
            (
                "BLENDER_USER_CONFIG",
                PathBuf::from(".assetlab/blender/config"),
            ),
            (
                "BLENDER_USER_SCRIPTS",
                PathBuf::from(".assetlab/blender/scripts"),
            ),
            (
                "BLENDER_USER_DATAFILES",
                PathBuf::from(".assetlab/blender/datafiles"),
            ),
        ];
        mapping
            .into_iter()
            .map(|(key, relative)| self.directory(&relative).map(|path| (key.to_owned(), path)))
            .collect()
    }

    /// Returns free bytes available to this process on the managed volume.
    pub fn free_space_bytes(&self) -> StorageResult<u64> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let path: Vec<u16> = self
                .root
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut available = 0u64;
            // SAFETY: `path` is NUL terminated and all output pointers reference live u64s.
            let succeeded = unsafe {
                GetDiskFreeSpaceExW(
                    path.as_ptr(),
                    &mut available,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if succeeded == 0 {
                return Err(StorageError::Io(std::io::Error::last_os_error()));
            }
            Ok(available)
        }
        #[cfg(not(windows))]
        {
            Err(StorageError::UnsupportedPlatform)
        }
    }

    /// Fails before writing when the managed volume lacks the requested free space.
    pub fn ensure_free_space(&self, required: u64) -> StorageResult<()> {
        let available = self.free_space_bytes()?;
        if available < required {
            Err(StorageError::InsufficientSpace {
                required,
                available,
            })
        } else {
            Ok(())
        }
    }

    /// Removes only completed, recognized backup versions outside the retention policy.
    /// Incomplete journals, unknown directories and directories with unexpected contents remain.
    pub fn prune_backups(&self) -> BackupRetentionReport {
        let _lock = match self.acquire_workspace_lock() {
            Ok(lock) => lock,
            Err(error) => {
                return BackupRetentionReport {
                    warnings: vec![error.to_string()],
                    ..BackupRetentionReport::default()
                };
            }
        };
        self.prune_backups_unlocked()
    }

    fn prune_backups_unlocked(&self) -> BackupRetentionReport {
        let mut report = BackupRetentionReport::default();
        let backup_root_relative = Path::new(".assetlab/backups");
        let backup_root_exists = match self.safe_exists(backup_root_relative) {
            Ok(exists) => exists,
            Err(error) => {
                report.warnings.push(error.to_string());
                return report;
            }
        };
        if !backup_root_exists {
            return report;
        }
        let backup_root = match self.existing(backup_root_relative) {
            Ok(path) => path,
            Err(error) => {
                report.warnings.push(error.to_string());
                return report;
            }
        };
        let pending_ids = match self.pending_backup_ids() {
            Ok(ids) => ids,
            Err(error) => {
                report.warnings.push(error.to_string());
                return report;
            }
        };
        let entries = match fs::read_dir(&backup_root) {
            Ok(entries) => entries,
            Err(error) => {
                report.warnings.push(error.to_string());
                return report;
            }
        };

        let mut candidates = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    report.warnings.push(error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match checked_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    report.warnings.push(error.to_string());
                    continue;
                }
            };
            if !metadata.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if !safe_id(id) {
                continue;
            }
            if pending_ids.contains(id) {
                report.pending_versions += 1;
                continue;
            }
            match self.backup_candidate(id, &path) {
                Ok(Some(candidate)) => candidates.push(candidate),
                Ok(None) => {}
                Err(error) => report.warnings.push(error.to_string()),
            }
        }

        candidates.sort_by(|left, right| {
            right
                .completed_at_unix_nanos
                .cmp(&left.completed_at_unix_nanos)
                .then_with(|| right.id.cmp(&left.id))
        });
        let mut retained: Vec<BackupCandidate> = candidates
            .iter()
            .take(self.backup_policy.max_versions)
            .cloned()
            .collect();
        let mut retained_bytes = retained
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.total_bytes));
        if let Some(limit) = self.backup_policy.max_total_bytes {
            while retained_bytes > limit && retained.len() > 1 {
                if let Some(oldest) = retained.pop() {
                    retained_bytes = retained_bytes.saturating_sub(oldest.total_bytes);
                }
            }
        }
        let retained_ids: HashSet<&str> = retained
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect();
        for candidate in &candidates {
            if retained_ids.contains(candidate.id.as_str()) {
                continue;
            }
            match self.remove_backup_candidate(candidate) {
                Ok(()) => report.pruned_versions += 1,
                Err(error) => report.warnings.push(error.to_string()),
            }
        }
        report.retained_versions = retained.len();
        report.retained_bytes = retained_bytes;
        report.over_byte_limit = self
            .backup_policy
            .max_total_bytes
            .is_some_and(|limit| retained_bytes > limit);
        report
    }

    fn acquire_workspace_lock(&self) -> StorageResult<WorkspaceLock> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            let lock_relative = Path::new(".assetlab/workspace.lock");
            let lock_directory = self.directory(Path::new(".assetlab"))?;
            let lock_path = lock_directory.join("workspace.lock");
            let open_options = || {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .write(true)
                    .share_mode(0)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
                options
            };
            let file = match open_options().create_new(true).open(&lock_path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let checked = match self.existing(lock_relative) {
                        Ok(path) => path,
                        Err(error) if is_storage_lock_contention(&error) => {
                            return Err(StorageError::Busy);
                        }
                        Err(error) => return Err(error),
                    };
                    require_regular_file(&checked)?;
                    match open_options().open(&lock_path) {
                        Ok(file) => file,
                        Err(error) if is_lock_contention(&error) => {
                            return Err(StorageError::Busy);
                        }
                        Err(error) => return Err(StorageError::Io(error)),
                    }
                }
                Err(error) if is_lock_contention(&error) => return Err(StorageError::Busy),
                Err(error) => return Err(StorageError::Io(error)),
            };
            let metadata = file.metadata()?;
            reject_reparse(&metadata, &lock_path)?;
            if !metadata.is_file() {
                return Err(StorageError::NotFile(lock_path.display().to_string()));
            }
            Ok(WorkspaceLock { _file: file })
        }
        #[cfg(not(windows))]
        {
            Err(StorageError::UnsupportedPlatform)
        }
    }

    fn pending_backup_ids(&self) -> StorageResult<HashSet<String>> {
        let relative = Path::new(".assetlab/journals");
        if !self.safe_exists(relative)? {
            return Ok(HashSet::new());
        }
        let directory = self.existing(relative)?;
        let mut ids = HashSet::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = checked_metadata(&path)?;
            if !metadata.is_file() || path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(OsStr::to_str)
                && safe_id(id)
            {
                ids.insert(id.to_owned());
            }
        }
        Ok(ids)
    }

    /// Publishes already validated staged scene and manifest files with recoverable backups.
    ///
    /// Both input paths must resolve beneath `.assetlab/staging` in this workspace.
    /// The two final files cannot be made visible atomically as a pair; a journal supports
    /// rollback after interruption and `recover_publications` completes that recovery.
    pub fn publish_scene(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
        staged_scene: &Path,
        staged_manifest: &Path,
    ) -> StorageResult<Publication> {
        self.publish_scene_inner(
            category,
            subcategory,
            slug,
            staged_scene,
            staged_manifest,
            None,
        )
    }

    /// Rolls back interrupted publications and clears completed transaction journals.
    pub fn recover_publications(&self) -> StorageResult<RecoveryReport> {
        let _lock = self.acquire_workspace_lock()?;
        let journal_directory = self.directory(Path::new(".assetlab/journals"))?;
        let mut report = RecoveryReport::default();
        for entry in fs::read_dir(&journal_directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = checked_metadata(&path)?;
            if !metadata.is_file() || path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            let journal_path = path
                .strip_prefix(&self.root)
                .map_err(|_| StorageError::OutsideWorkspace(path.display().to_string()))?;
            let journal_path = self.existing(journal_path)?;
            let mut bytes = Vec::new();
            File::open(&journal_path)?.read_to_end(&mut bytes)?;
            let journal: PublicationJournal = serde_json::from_slice(&bytes)?;
            if path.file_stem() != Some(OsStr::new(&journal.id)) {
                return Err(StorageError::InvalidJournal(format!(
                    "journal filename does not match transaction ID: {}",
                    path.display()
                )));
            }
            self.validate_journal(&journal)?;
            let marker = self.commit_marker(&journal.id)?;
            if self.safe_exists_absolute(&marker)? {
                self.ensure_backup_metadata(&journal)?;
                self.remove_journal_file(&journal.id)?;
                if !journal.scene_existed && !journal.manifest_existed {
                    let backup_dir = self.root.join(".assetlab/backups").join(&journal.id);
                    let _ = fs::remove_dir(backup_dir);
                }
                report.committed_cleanups += 1;
            } else {
                self.rollback(&journal)?;
                report.rolled_back += 1;
            }
        }
        report.retention = self.prune_backups_unlocked();
        Ok(report)
    }

    fn publish_scene_inner(
        &self,
        category: &str,
        subcategory: &str,
        slug: &str,
        staged_scene: &Path,
        staged_manifest: &Path,
        fail_after_step: Option<usize>,
    ) -> StorageResult<Publication> {
        let _lock = self.acquire_workspace_lock()?;
        if let Some(expected) = &self.expected_publication
            && self.baseline_unlocked(category, subcategory, slug)? != *expected
        {
            return Err(StorageError::BaselineChanged);
        }
        validate_name(category)?;
        validate_name(subcategory)?;
        validate_name(slug)?;
        let stage_scene_rel = staged_relative(&self.root, staged_scene)?;
        let stage_manifest_rel = staged_relative(&self.root, staged_manifest)?;
        if stage_scene_rel == stage_manifest_rel {
            return Err(StorageError::UnsafePath(
                "scene and manifest staging paths must differ".into(),
            ));
        }
        let stage_scene = self.existing(&stage_scene_rel)?;
        let stage_manifest = self.existing(&stage_manifest_rel)?;
        require_regular_file(&stage_scene)?;
        require_regular_file(&stage_manifest)?;

        let scene_rel = scene_relative(category, subcategory, slug)?;
        let manifest_rel = PathBuf::from("assets").join(slug).join("asset.toml");
        let scene_final = self.target_path(&scene_rel)?;
        let manifest_final = self.target_path(&manifest_rel)?;
        let scene_existed = self.safe_exists(&scene_rel)?;
        let manifest_existed = self.safe_exists(&manifest_rel)?;
        let id = next_id();
        let journal = PublicationJournal {
            version: 1,
            id: id.clone(),
            category: category.to_owned(),
            subcategory: subcategory.to_owned(),
            slug: slug.to_owned(),
            staged_scene: path_to_string(&stage_scene_rel),
            staged_manifest: path_to_string(&stage_manifest_rel),
            scene_existed,
            manifest_existed,
        };

        let backup_root = self.directory(Path::new(".assetlab/backups"))?;
        let backup_directory = backup_root.join(&id);
        fs::create_dir(&backup_directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StorageError::Collision(backup_directory.display().to_string())
            } else {
                StorageError::Io(error)
            }
        })?;
        ensure_directory_component(&backup_directory)?;
        let journal_directory = self.directory(Path::new(".assetlab/journals"))?;
        let journal_path = journal_directory.join(format!("{id}.json"));
        if let Err(error) = self.write_journal(&journal_path, &journal) {
            let _ = fs::remove_dir(&backup_directory);
            return Err(error);
        }

        let operation = self.apply_publication(
            &journal,
            &stage_scene,
            &stage_manifest,
            &scene_final,
            &manifest_final,
            fail_after_step,
        );
        if let Err(error) = operation {
            if fail_after_step.is_some() {
                return Err(error);
            }
            return match self.rollback(&journal) {
                Ok(()) => Err(error),
                Err(recovery_error) => Err(StorageError::PublicationRecovery {
                    cause: format!("{error}; rollback failed: {recovery_error}"),
                    journal: journal_path,
                }),
            };
        }

        if let Err(error) = self.write_commit_marker(&id) {
            return match self.rollback(&journal) {
                Ok(()) => Err(error),
                Err(recovery_error) => Err(StorageError::PublicationRecovery {
                    cause: format!("{error}; rollback failed: {recovery_error}"),
                    journal: journal_path,
                }),
            };
        }
        if let Err(error) = self.ensure_backup_metadata(&journal) {
            return Err(StorageError::PublicationRecovery {
                cause: format!("publication committed; backup metadata write failed: {error}"),
                journal: journal_path,
            });
        }
        let has_backup = journal.scene_existed || journal.manifest_existed;
        if let Err(error) = self.remove_journal_file(&id) {
            return Err(StorageError::PublicationRecovery {
                cause: format!("publication committed; journal cleanup failed: {error}"),
                journal: journal_path,
            });
        }
        if !has_backup {
            let _ = fs::remove_dir(&backup_directory);
        }
        let retention = self.prune_backups_unlocked();
        Ok(Publication {
            scene_path: scene_final,
            manifest_path: manifest_final,
            backup_directory: has_backup.then_some(backup_directory),
            retention,
        })
    }

    fn apply_publication(
        &self,
        journal: &PublicationJournal,
        staged_scene: &Path,
        staged_manifest: &Path,
        scene_final: &Path,
        manifest_final: &Path,
        fail_after_step: Option<usize>,
    ) -> StorageResult<()> {
        let backups = self.backup_paths(&journal.id)?;
        let mut step = 0usize;
        if journal.scene_existed {
            self.ensure_target_is_file(scene_final)?;
            fs::rename(scene_final, &backups.0)?;
            step += 1;
            maybe_fail(fail_after_step, step)?;
        }
        if journal.manifest_existed {
            self.ensure_target_is_file(manifest_final)?;
            fs::rename(manifest_final, &backups.1)?;
            step += 1;
            maybe_fail(fail_after_step, step)?;
        }
        self.ensure_target_parent(scene_final)?;
        fs::rename(staged_scene, scene_final)?;
        step += 1;
        maybe_fail(fail_after_step, step)?;
        self.ensure_target_parent(manifest_final)?;
        fs::rename(staged_manifest, manifest_final)?;
        step += 1;
        maybe_fail(fail_after_step, step)?;
        Ok(())
    }

    fn validate_journal(&self, journal: &PublicationJournal) -> StorageResult<()> {
        if journal.version != 1 || !safe_id(&journal.id) {
            return Err(StorageError::InvalidJournal(
                "unsupported version or ID".into(),
            ));
        }
        validate_name(&journal.category)
            .and_then(|()| validate_name(&journal.subcategory))
            .and_then(|()| validate_name(&journal.slug))
            .map_err(|error| StorageError::InvalidJournal(error.to_string()))?;
        let stage_scene = staged_relative(&self.root, Path::new(&journal.staged_scene))
            .map_err(|error| StorageError::InvalidJournal(error.to_string()))?;
        let stage_manifest = staged_relative(&self.root, Path::new(&journal.staged_manifest))
            .map_err(|error| StorageError::InvalidJournal(error.to_string()))?;
        if stage_scene == stage_manifest {
            return Err(StorageError::InvalidJournal(
                "scene and manifest staging paths are identical".into(),
            ));
        }
        let _ = scene_relative(&journal.category, &journal.subcategory, &journal.slug)?;
        let _ = PathBuf::from("assets")
            .join(&journal.slug)
            .join("asset.toml");
        let backup_dir = Path::new(".assetlab/backups").join(&journal.id);
        let _ = relative_parts(&backup_dir)?;
        Ok(())
    }

    fn rollback(&self, journal: &PublicationJournal) -> StorageResult<()> {
        let scene_rel = scene_relative(&journal.category, &journal.subcategory, &journal.slug)?;
        let manifest_rel = PathBuf::from("assets")
            .join(&journal.slug)
            .join("asset.toml");
        let scene_final = self.target_path(&scene_rel)?;
        let manifest_final = self.target_path(&manifest_rel)?;
        let backups = self.backup_paths(&journal.id)?;
        self.restore_one(&scene_final, &backups.0, journal.scene_existed)?;
        self.restore_one(&manifest_final, &backups.1, journal.manifest_existed)?;
        self.remove_journal_file(&journal.id)?;
        let backup_dir = self.root.join(".assetlab/backups").join(&journal.id);
        let _ = fs::remove_dir(backup_dir);
        Ok(())
    }

    fn restore_one(
        &self,
        target: &Path,
        backup: &Path,
        previously_existed: bool,
    ) -> StorageResult<()> {
        let target_exists = self.safe_exists_absolute(target)?;
        let backup_exists = self.safe_exists_absolute(backup)?;
        if backup_exists {
            if target_exists {
                self.ensure_target_is_file(target)?;
                fs::remove_file(target)?;
            }
            self.ensure_target_parent(target)?;
            fs::rename(backup, target)?;
        } else if !previously_existed && target_exists {
            self.ensure_target_is_file(target)?;
            fs::remove_file(target)?;
        }
        Ok(())
    }

    fn target_path(&self, relative: &Path) -> StorageResult<PathBuf> {
        let parts = relative_parts(relative)?;
        let file_name = parts.last().expect("validated non-empty path");
        let parent_rel = path_from_parts(&parts[..parts.len() - 1]);
        let parent = if parent_rel.as_os_str().is_empty() {
            self.root.clone()
        } else {
            self.directory(&parent_rel)?
        };
        let target = parent.join(file_name);
        if self.safe_exists_absolute(&target)? {
            let metadata = checked_metadata(&target)?;
            if metadata.is_dir() {
                return Err(StorageError::NotFile(target.display().to_string()));
            }
            target.canonicalize().map_err(StorageError::Io)
        } else {
            ensure_inside_root(&self.root, &target)?;
            Ok(target)
        }
    }

    fn safe_exists(&self, relative: &Path) -> StorageResult<bool> {
        let parts = relative_parts(relative)?;
        let mut current = self.root.clone();
        for (index, part) in parts.iter().enumerate() {
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    reject_reparse(&metadata, &current)?;
                    if index + 1 < parts.len() && !metadata.is_dir() {
                        return Err(StorageError::NotDirectory(current.display().to_string()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(StorageError::Io(error)),
            }
        }
        Ok(true)
    }

    fn safe_exists_absolute(&self, path: &Path) -> StorageResult<bool> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| StorageError::OutsideWorkspace(path.display().to_string()))?;
        self.safe_exists(relative)
    }

    fn ensure_target_parent(&self, target: &Path) -> StorageResult<()> {
        let relative = target
            .strip_prefix(&self.root)
            .map_err(|_| StorageError::OutsideWorkspace(target.display().to_string()))?;
        let parent = relative
            .parent()
            .ok_or_else(|| StorageError::UnsafePath(relative.display().to_string()))?;
        self.directory(parent).map(|_| ())
    }

    fn ensure_target_is_file(&self, target: &Path) -> StorageResult<()> {
        let relative = target
            .strip_prefix(&self.root)
            .map_err(|_| StorageError::OutsideWorkspace(target.display().to_string()))?;
        let checked = self.existing(relative)?;
        require_regular_file(&checked)
    }

    fn backup_paths(&self, id: &str) -> StorageResult<(PathBuf, PathBuf)> {
        if !safe_id(id) {
            return Err(StorageError::InvalidJournal("invalid backup ID".into()));
        }
        let relative = Path::new(".assetlab/backups").join(id);
        let directory = self.directory(&relative)?;
        Ok((
            directory.join("scene.backup"),
            directory.join("manifest.backup"),
        ))
    }

    fn ensure_backup_metadata(&self, journal: &PublicationJournal) -> StorageResult<()> {
        if !journal.scene_existed && !journal.manifest_existed {
            return Ok(());
        }
        let (scene_backup, manifest_backup) = self.backup_paths(&journal.id)?;
        for (path, expected) in [
            (&scene_backup, journal.scene_existed),
            (&manifest_backup, journal.manifest_existed),
        ] {
            let exists = self.safe_exists_absolute(path)?;
            if exists != expected {
                return Err(StorageError::InvalidJournal(format!(
                    "backup file state does not match journal: {}",
                    path.display()
                )));
            }
            if exists {
                require_regular_file(path)?;
            }
        }
        let directory = scene_backup.parent().expect("backup path has parent");
        let metadata_path = directory.join("metadata.json");
        if self.safe_exists_absolute(&metadata_path)? {
            let checked =
                self.existing(metadata_path.strip_prefix(&self.root).map_err(|_| {
                    StorageError::OutsideWorkspace(metadata_path.display().to_string())
                })?)?;
            let metadata: BackupMetadata = serde_json::from_slice(&fs::read(checked)?)?;
            if metadata.version != 1
                || metadata.id != journal.id
                || metadata.scene_backup != journal.scene_existed
                || metadata.manifest_backup != journal.manifest_existed
            {
                return Err(StorageError::InvalidJournal(format!(
                    "backup metadata does not match journal {}",
                    journal.id
                )));
            }
            return Ok(());
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| StorageError::InvalidJournal(error.to_string()))?
            .as_nanos() as u64;
        let metadata = BackupMetadata {
            version: 1,
            id: journal.id.clone(),
            completed_at_unix_nanos: timestamp,
            scene_backup: journal.scene_existed,
            manifest_backup: journal.manifest_existed,
        };
        let bytes = serde_json::to_vec(&metadata)?;
        self.ensure_free_space(bytes.len() as u64)?;
        write_synced_new(&metadata_path, &bytes)
    }

    fn backup_candidate(
        &self,
        id: &str,
        directory: &Path,
    ) -> StorageResult<Option<BackupCandidate>> {
        if !safe_id(id) {
            return Ok(None);
        }
        let relative_directory = directory
            .strip_prefix(&self.root)
            .map_err(|_| StorageError::OutsideWorkspace(directory.display().to_string()))?;
        let metadata_relative = relative_directory.join("metadata.json");
        if !self.safe_exists(&metadata_relative)? {
            return Ok(None);
        }
        let metadata_path = self.existing(&metadata_relative)?;
        require_regular_file(&metadata_path)?;
        let metadata: BackupMetadata = serde_json::from_slice(&fs::read(&metadata_path)?)?;
        if metadata.version != 1
            || metadata.id != id
            || metadata.completed_at_unix_nanos == 0
            || (!metadata.scene_backup && !metadata.manifest_backup)
        {
            return Ok(None);
        }
        let mut scene_seen = false;
        let mut manifest_seen = false;
        let mut metadata_seen = false;
        let mut total_bytes = 0u64;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_metadata = checked_metadata(&path)?;
            if !file_metadata.is_file() {
                return Ok(None);
            }
            match path.file_name().and_then(OsStr::to_str) {
                Some("scene.backup") => scene_seen = true,
                Some("manifest.backup") => manifest_seen = true,
                Some("metadata.json") => metadata_seen = true,
                _ => return Ok(None),
            }
            total_bytes = total_bytes.saturating_add(file_metadata.len());
        }
        if !metadata_seen
            || scene_seen != metadata.scene_backup
            || manifest_seen != metadata.manifest_backup
        {
            return Ok(None);
        }
        Ok(Some(BackupCandidate {
            id: id.to_owned(),
            directory: directory.to_path_buf(),
            completed_at_unix_nanos: metadata.completed_at_unix_nanos,
            total_bytes,
            scene_backup: metadata.scene_backup,
            manifest_backup: metadata.manifest_backup,
        }))
    }

    fn remove_backup_candidate(&self, candidate: &BackupCandidate) -> StorageResult<()> {
        let checked = self
            .backup_candidate(&candidate.id, &candidate.directory)?
            .ok_or_else(|| {
                StorageError::InvalidJournal(format!(
                    "backup {} changed before pruning",
                    candidate.id
                ))
            })?;
        if checked.completed_at_unix_nanos != candidate.completed_at_unix_nanos
            || checked.scene_backup != candidate.scene_backup
            || checked.manifest_backup != candidate.manifest_backup
        {
            return Err(StorageError::InvalidJournal(format!(
                "backup {} changed before pruning",
                candidate.id
            )));
        }
        for name in ["scene.backup", "manifest.backup", "metadata.json"] {
            let path = candidate.directory.join(name);
            if self.safe_exists_absolute(&path)? {
                require_regular_file(&path)?;
                fs::remove_file(path)?;
            }
        }
        fs::remove_dir(&candidate.directory)?;
        Ok(())
    }

    fn write_journal(&self, path: &Path, journal: &PublicationJournal) -> StorageResult<()> {
        let bytes = serde_json::to_vec(journal)?;
        self.ensure_free_space(bytes.len() as u64 + 4096)?;
        write_synced_new(path, &bytes)
    }

    fn write_commit_marker(&self, id: &str) -> StorageResult<()> {
        let marker = self.commit_marker(id)?;
        write_synced_new(&marker, id.as_bytes())
    }

    fn commit_marker(&self, id: &str) -> StorageResult<PathBuf> {
        if !safe_id(id) {
            return Err(StorageError::InvalidJournal(
                "invalid commit marker ID".into(),
            ));
        }
        let directory = self.directory(Path::new(".assetlab/journals"))?;
        Ok(directory.join(format!("{id}.committed")))
    }

    fn remove_journal_file(&self, id: &str) -> StorageResult<()> {
        if !safe_id(id) {
            return Err(StorageError::InvalidJournal("invalid journal ID".into()));
        }
        let directory = self.directory(Path::new(".assetlab/journals"))?;
        for extension in ["json", "committed"] {
            let path = directory.join(format!("{id}.{extension}"));
            if self.safe_exists_absolute(&path)? {
                require_regular_file(&path)?;
            }
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(StorageError::Io(error)),
            }
        }
        Ok(())
    }
}

fn ensure_new_target_parent(workspace: &ManagedWorkspace, destination: &Path) -> StorageResult<()> {
    let relative = destination
        .strip_prefix(&workspace.root)
        .map_err(|_| StorageError::OutsideWorkspace(destination.display().to_string()))?;
    let parent = relative
        .parent()
        .ok_or_else(|| StorageError::UnsafePath(relative.display().to_string()))?;
    workspace.directory(parent).map(|_| ())
}

fn write_synced_new(path: &Path, bytes: &[u8]) -> StorageResult<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(StorageError::Io(error));
    }
    Ok(())
}

fn relative_parts(relative: &Path) -> StorageResult<Vec<PathBuf>> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(StorageError::UnsafePath(relative.display().to_string()));
    }
    let raw = relative.to_string_lossy();
    if raw.starts_with(['\\', '/']) || raw.contains(':') {
        return Err(StorageError::UnsafePath(relative.display().to_string()));
    }
    for segment in raw.split(['\\', '/']) {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || unsafe_windows_segment(segment)
        {
            return Err(StorageError::UnsafePath(relative.display().to_string()));
        }
    }
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(PathBuf::from(part)),
            _ => return Err(StorageError::UnsafePath(relative.display().to_string())),
        }
    }
    if parts.is_empty() {
        return Err(StorageError::UnsafePath(relative.display().to_string()));
    }
    Ok(parts)
}

fn path_from_parts(parts: &[PathBuf]) -> PathBuf {
    parts.iter().fold(PathBuf::new(), |mut path, part| {
        path.push(part);
        path
    })
}

fn staged_relative(root: &Path, path: &Path) -> StorageResult<PathBuf> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| StorageError::OutsideWorkspace(path.display().to_string()))?
    } else {
        path
    };
    let parts = relative_parts(relative)?;
    if parts.len() < 3 || parts[0] != OsStr::new(".assetlab") || parts[1] != OsStr::new("staging") {
        return Err(StorageError::UnsafePath(format!(
            "staged input must be beneath .assetlab/staging: {}",
            path.display()
        )));
    }
    Ok(path_from_parts(&parts))
}

fn scene_relative(category: &str, subcategory: &str, slug: &str) -> StorageResult<PathBuf> {
    validate_name(category)?;
    validate_name(subcategory)?;
    validate_name(slug)?;
    Ok(Path::new("blender")
        .join("models")
        .join(category)
        .join(subcategory)
        .join(slug)
        .join(format!("{slug}.blend")))
}

fn validate_name(name: &str) -> StorageResult<()> {
    if unsafe_windows_segment(name) || name == "." || name == ".." || name.trim().is_empty() {
        return Err(StorageError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn unsafe_windows_segment(segment: &str) -> bool {
    segment.is_empty()
        || segment.ends_with([' ', '.'])
        || segment
            .chars()
            .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
        || is_reserved_device_name(segment)
}

fn is_reserved_device_name(segment: &str) -> bool {
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn require_regular_file(path: &Path) -> StorageResult<()> {
    let metadata = checked_metadata(path)?;
    if !metadata.is_file() {
        return Err(StorageError::NotFile(path.display().to_string()));
    }
    Ok(())
}

fn same_regular_directory_contents(first: &Path, second: &Path) -> StorageResult<bool> {
    fn contents(directory: &Path) -> StorageResult<Vec<(std::ffi::OsString, Vec<u8>)>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            require_regular_file(&path)?;
            files.push((entry.file_name(), fs::read(path)?));
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }

    Ok(contents(first)? == contents(second)?)
}

fn ensure_directory_component(path: &Path) -> StorageResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_reparse(&metadata, path)?;
            if !metadata.is_dir() {
                return Err(StorageError::NotDirectory(path.display().to_string()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(path)?;
                reject_reparse(&metadata, path)?;
                if !metadata.is_dir() {
                    return Err(StorageError::NotDirectory(path.display().to_string()));
                }
            }
            Err(error) => return Err(StorageError::Io(error)),
        },
        Err(error) => return Err(StorageError::Io(error)),
    }
    Ok(())
}

fn checked_metadata(path: &Path) -> StorageResult<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    reject_reparse(&metadata, path)?;
    Ok(metadata)
}

fn reject_reparse(metadata: &fs::Metadata, path: &Path) -> StorageResult<()> {
    if metadata.file_type().is_symlink() || is_reparse_point(metadata) {
        return Err(StorageError::OutsideWorkspace(path.display().to_string()));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn ensure_inside_root(root: &Path, path: &Path) -> StorageResult<()> {
    let canonical_root = root.canonicalize()?;
    let canonical_path = path.canonicalize().or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(path.to_path_buf())
        } else {
            Err(error)
        }
    })?;
    if canonical_path.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(StorageError::OutsideWorkspace(path.display().to_string()))
    }
}

/// Canonicalizes an existing D:-drive path, or checks the existing parents of a new target.
/// Reparse points, junctions, traversal components and non-D: paths are rejected.
pub fn require_d_path(path: &Path) -> StorageResult<PathBuf> {
    #[cfg(windows)]
    {
        let parts = absolute_d_parts(path)?;
        let mut current = PathBuf::from(r"D:\");
        for (index, part) in parts.iter().enumerate() {
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    reject_reparse(&metadata, &current)?;
                    if index + 1 < parts.len() && !metadata.is_dir() {
                        return Err(StorageError::NotDirectory(current.display().to_string()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let mut candidate = current;
                    for remaining in &parts[index + 1..] {
                        candidate.push(remaining);
                    }
                    return Ok(candidate);
                }
                Err(error) => return Err(StorageError::Io(error)),
            }
        }
        let canonical = current.canonicalize()?;
        if path_is_on_d(&canonical) {
            Ok(canonical)
        } else {
            Err(StorageError::InvalidRoot(path.display().to_string()))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(StorageError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
fn ensure_d_root(root: &Path) -> StorageResult<PathBuf> {
    let parts = absolute_d_parts(root)?;
    let mut current = PathBuf::from(r"D:\");
    let mut missing = false;
    for part in parts {
        current.push(part);
        if missing {
            match fs::create_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure_directory_component(&current)?;
                }
                Err(error) => return Err(StorageError::Io(error)),
            }
        } else {
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    reject_reparse(&metadata, &current)?;
                    if !metadata.is_dir() {
                        return Err(StorageError::NotDirectory(current.display().to_string()));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing = true;
                    fs::create_dir(&current)?;
                }
                Err(error) => return Err(StorageError::Io(error)),
            }
        }
    }
    let canonical = current.canonicalize()?;
    if !path_is_on_d(&canonical) {
        return Err(StorageError::InvalidRoot(root.display().to_string()));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn absolute_d_parts(path: &Path) -> StorageResult<Vec<PathBuf>> {
    let mut components = path.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix)) => prefix,
        _ => return Err(StorageError::InvalidRoot(path.display().to_string())),
    };
    let is_d = match prefix.kind() {
        std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive) => {
            drive.eq_ignore_ascii_case(&b'D')
        }
        _ => false,
    };
    if !is_d || components.next() != Some(Component::RootDir) {
        return Err(StorageError::InvalidRoot(path.display().to_string()));
    }
    let mut parts = Vec::new();
    for component in components {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if unsafe_windows_segment(&text) {
                    return Err(StorageError::InvalidRoot(path.display().to_string()));
                }
                parts.push(PathBuf::from(part));
            }
            _ => return Err(StorageError::InvalidRoot(path.display().to_string())),
        }
    }
    Ok(parts)
}

#[cfg(windows)]
fn path_is_on_d(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive) => {
                drive.eq_ignore_ascii_case(&b'D')
            }
            _ => false,
        },
        _ => false,
    }
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

#[cfg(windows)]
fn is_lock_contention(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(32) | Some(33))
}

#[cfg(windows)]
fn is_storage_lock_contention(error: &StorageError) -> bool {
    matches!(error, StorageError::Io(io_error) if is_lock_contention(io_error))
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn next_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{}-{timestamp}-{sequence}", std::process::id())
}

fn maybe_fail(fail_after_step: Option<usize>, step: usize) -> StorageResult<()> {
    if fail_after_step == Some(step) {
        Err(StorageError::SimulatedFailure(step))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::process::Command;

    #[cfg(windows)]
    fn test_root(label: &str) -> PathBuf {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("dd_core lives beneath the repository root")
            .to_path_buf();
        let base = workspace.join(".work").join("temp");
        fs::create_dir_all(&base).unwrap();
        let id = next_id();
        let root = base.join(format!("storage-{label}-{id}"));
        fs::create_dir(&root).unwrap();
        root
    }

    #[cfg(windows)]
    fn write_stage(workspace: &ManagedWorkspace, name: &str, bytes: &[u8]) -> PathBuf {
        workspace
            .write_new(Path::new(".assetlab/staging").join(name).as_path(), bytes)
            .unwrap()
    }

    #[test]
    #[cfg(windows)]
    fn root_is_d_only_and_relative_paths_reject_traversal() {
        let root = test_root("paths");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        assert_eq!(workspace.root(), root.canonicalize().unwrap());
        assert!(!root.join(".assetlab").exists());
        assert!(matches!(
            workspace.directory(Path::new("../outside")),
            Err(StorageError::UnsafePath(_))
        ));
        assert!(matches!(
            ManagedWorkspace::new(Path::new(r"C:\temp\not-managed")),
            Err(StorageError::InvalidRoot(_))
        ));
        assert!(matches!(
            require_d_path(Path::new(r"C:\temp\not-managed")),
            Err(StorageError::InvalidRoot(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(windows))]
    fn reports_unsupported_platform() {
        assert!(matches!(
            ManagedWorkspace::new(Path::new("D:/workspace")),
            Err(StorageError::UnsupportedPlatform)
        ));
        assert!(matches!(
            require_d_path(Path::new("D:/workspace")),
            Err(StorageError::UnsupportedPlatform)
        ));
    }

    #[test]
    #[cfg(windows)]
    fn checked_paths_reject_junction_escapes() {
        let root = test_root("junction");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let staging = workspace.directory(Path::new(".assetlab/staging")).unwrap();
        let outside = root
            .parent()
            .unwrap()
            .join(format!("outside-{}", next_id()));
        fs::create_dir(&outside).unwrap();
        let junction = staging.join("escape");
        let command = format!(
            "New-Item -ItemType Junction -Path '{}' -Target '{}' | Out-Null",
            junction.to_string_lossy().replace('\'', "''"),
            outside.to_string_lossy().replace('\'', "''")
        );
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &command,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(matches!(
            workspace.existing(Path::new(".assetlab/staging/escape")),
            Err(StorageError::OutsideWorkspace(_))
        ));
        fs::remove_dir(&junction).unwrap();
        fs::remove_dir_all(&outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn write_new_stages_only_and_rejects_collisions() {
        let root = test_root("collision");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let relative = Path::new(".assetlab/staging/output.bin");
        workspace.write_new(relative, b"first").unwrap();
        assert!(matches!(
            workspace.write_new(relative, b"second"),
            Err(StorageError::Collision(_))
        ));
        assert_eq!(fs::read(root.join(relative)).unwrap(), b"first");
        assert!(matches!(
            workspace.write_new(Path::new("assets/output.bin"), b"no"),
            Err(StorageError::UnsafePath(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn reference_revision_promotion_is_idempotent_only_for_identical_bundles() {
        let root = test_root("reference-promotion");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let first = workspace
            .new_staging_directory(Path::new(".assetlab/staging/reference-first"))
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-first/geometry.json"),
                b"spec-v1",
            )
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-first/side.png"),
                b"image-v1",
            )
            .unwrap();
        let promoted = workspace
            .promote_reference_revision(&first, "long_shotgun", "r1_hash")
            .unwrap();

        let retry = workspace
            .new_staging_directory(Path::new(".assetlab/staging/reference-retry"))
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-retry/geometry.json"),
                b"spec-v1",
            )
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-retry/side.png"),
                b"image-v1",
            )
            .unwrap();
        assert_eq!(
            workspace
                .promote_reference_revision(&retry, "long_shotgun", "r1_hash")
                .unwrap(),
            promoted
        );

        let changed = workspace
            .new_staging_directory(Path::new(".assetlab/staging/reference-changed"))
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-changed/geometry.json"),
                b"spec-v2",
            )
            .unwrap();
        workspace
            .write_new(
                Path::new(".assetlab/staging/reference-changed/side.png"),
                b"image-v1",
            )
            .unwrap();
        assert!(matches!(
            workspace.promote_reference_revision(&changed, "long_shotgun", "r1_hash"),
            Err(StorageError::Collision(_))
        ));
        assert_eq!(
            fs::read(root.join(promoted).join("geometry.json")).unwrap(),
            b"spec-v1"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn child_environment_uses_checked_workspace_directories() {
        let root = test_root("environment");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let environment = workspace.child_environment().unwrap();
        assert_eq!(environment.len(), 6);
        for (_, path) in &environment {
            assert!(path.starts_with(workspace.root()));
            assert!(path.is_dir());
        }
        assert_eq!(environment[0].1, environment[1].1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn publication_lock_is_exclusive_across_workspace_handles() {
        let root = test_root("lock");
        let first = ManagedWorkspace::new(&root).unwrap();
        let second = ManagedWorkspace::new(&root).unwrap();
        let guard = first.acquire_workspace_lock().unwrap();
        assert!(matches!(
            second.recover_publications(),
            Err(StorageError::Busy)
        ));
        drop(guard);
        assert!(second.recover_publications().is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn publication_backups_prior_files_under_assetlab_and_publishes_both_outputs() {
        let root = test_root("publish");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let scene = workspace
            .final_scene("weapon", "shotgun", "field_scatter")
            .unwrap();
        let manifest = workspace.final_manifest("field_scatter").unwrap();
        fs::write(&scene, b"previous scene").unwrap();
        fs::write(&manifest, b"previous manifest").unwrap();
        let stage_scene = write_stage(&workspace, "scene.blend", b"validated scene");
        let stage_manifest = write_stage(&workspace, "asset.toml", b"new manifest");
        let publication = workspace
            .publish_scene(
                "weapon",
                "shotgun",
                "field_scatter",
                Path::new(".assetlab/staging/scene.blend"),
                Path::new(".assetlab/staging/asset.toml"),
            )
            .unwrap();
        assert_eq!(
            fs::read(&publication.scene_path).unwrap(),
            b"validated scene"
        );
        assert_eq!(
            fs::read(&publication.manifest_path).unwrap(),
            b"new manifest"
        );
        let backups = publication.backup_directory.unwrap();
        assert!(backups.starts_with(workspace.root().join(".assetlab/backups")));
        assert_eq!(
            fs::read(backups.join("scene.backup")).unwrap(),
            b"previous scene"
        );
        assert_eq!(
            fs::read(backups.join("manifest.backup")).unwrap(),
            b"previous manifest"
        );
        assert!(!stage_scene.exists());
        assert!(!stage_manifest.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn simulated_interruption_is_recovered_from_journal() {
        let root = test_root("recovery");
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let scene = workspace
            .final_scene("weapon", "shotgun", "test_piece")
            .unwrap();
        let manifest = workspace.final_manifest("test_piece").unwrap();
        fs::write(&scene, b"old scene").unwrap();
        fs::write(&manifest, b"old manifest").unwrap();
        write_stage(&workspace, "scene.blend", b"new scene");
        write_stage(&workspace, "asset.toml", b"new manifest");
        assert!(matches!(
            workspace.publish_scene_inner(
                "weapon",
                "shotgun",
                "test_piece",
                Path::new(".assetlab/staging/scene.blend"),
                Path::new(".assetlab/staging/asset.toml"),
                Some(3),
            ),
            Err(StorageError::SimulatedFailure(3))
        ));
        assert_eq!(fs::read(&scene).unwrap(), b"new scene");
        assert!(!manifest.exists());
        let report = workspace.recover_publications().unwrap();
        assert_eq!(report.rolled_back, 1);
        assert_eq!(fs::read(&scene).unwrap(), b"old scene");
        assert_eq!(fs::read(&manifest).unwrap(), b"old manifest");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn backup_retention_prunes_only_completed_versions_and_keeps_the_latest() {
        let root = test_root("retention");
        let mut workspace = ManagedWorkspace::new(&root).unwrap();
        workspace
            .set_backup_policy(BackupPolicy {
                max_versions: 2,
                max_total_bytes: None,
            })
            .unwrap();
        let scene = workspace
            .final_scene("weapon", "shotgun", "retained_piece")
            .unwrap();
        let manifest = workspace.final_manifest("retained_piece").unwrap();
        fs::write(&scene, b"scene-v0").unwrap();
        fs::write(&manifest, b"manifest-v0").unwrap();

        let mut last_publication = None;
        for revision in 1..=3 {
            let scene_name = format!("scene-{revision}.blend");
            let manifest_name = format!("manifest-{revision}.toml");
            write_stage(
                &workspace,
                &scene_name,
                format!("scene-v{revision}").as_bytes(),
            );
            write_stage(
                &workspace,
                &manifest_name,
                format!("manifest-v{revision}").as_bytes(),
            );
            last_publication = Some(
                workspace
                    .publish_scene(
                        "weapon",
                        "shotgun",
                        "retained_piece",
                        Path::new(".assetlab/staging").join(&scene_name).as_path(),
                        Path::new(".assetlab/staging")
                            .join(&manifest_name)
                            .as_path(),
                    )
                    .unwrap(),
            );
        }
        let publication = last_publication.unwrap();
        assert_eq!(publication.retention.retained_versions, 2);
        assert_eq!(publication.retention.pruned_versions, 1);
        let latest_backup = publication.backup_directory.clone().unwrap();

        let foreign = workspace
            .directory(Path::new(".assetlab/backups/unrelated"))
            .unwrap();
        fs::write(foreign.join("notes.txt"), b"preserve").unwrap();
        let journal_directory = workspace
            .directory(Path::new(".assetlab/journals"))
            .unwrap();
        let journal_path = journal_directory.join(format!(
            "{}.json",
            latest_backup.file_name().unwrap().to_string_lossy()
        ));
        fs::write(&journal_path, b"pending").unwrap();
        workspace
            .set_backup_policy(BackupPolicy {
                max_versions: 5,
                max_total_bytes: Some(1),
            })
            .unwrap();
        let pending_retention = workspace.prune_backups();
        assert_eq!(pending_retention.pending_versions, 1);
        assert!(latest_backup.exists());
        fs::remove_file(&journal_path).unwrap();
        let retention = workspace.prune_backups();
        assert_eq!(retention.retained_versions, 1);
        assert!(retention.over_byte_limit);
        assert!(latest_backup.exists());
        assert_eq!(fs::read(foreign.join("notes.txt")).unwrap(), b"preserve");
        let backup_root = root.join(".assetlab/backups");
        let retained = fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.join("metadata.json").is_file())
            .unwrap();
        assert_eq!(
            fs::read(retained.join("scene.backup")).unwrap(),
            b"scene-v2"
        );
        assert_eq!(
            fs::read(retained.join("manifest.backup")).unwrap(),
            b"manifest-v2"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
