use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("unsupported manifest schema version {0}")]
    Schema(u32),
    #[error("relative path must contain only normal components: {0}")]
    UnsafePath(String),
    #[error("path escapes project directory: {0}")]
    OutsideProject(String),
    #[error("missing project file: .assetlab/project.toml")]
    NoProject,
    #[error("no DD_3DMagic projects found in workspace `{0}`")]
    NoProjects(String),
    #[error("project `{0}` was not found in the selected workspace")]
    ProjectNotFound(String),
    #[error("workspace has {0} projects; select one by project_id")]
    ProjectSelectionRequired(usize),
    #[error("duplicate project_id `{0}` in workspace")]
    DuplicateProjectId(String),
    #[error("missing {kind} profile `{id}`; create `{path}` or choose an existing profile")]
    MissingProfile {
        kind: String,
        id: String,
        path: String,
    },
    #[error("asset manifest not found for `{0}`")]
    MissingAsset(String),
    #[error("configuration error: {0}")]
    Invalid(String),
    #[error("output name already exists (case-insensitive): {0}")]
    NameCollision(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureProfile {
    pub width: u32,
    pub height: u32,
    pub target_td_px_per_m: f64,
    pub td_tolerance_pct: f64,
    pub padding_px: u32,
    #[serde(default)]
    pub palette: Vec<[u8; 3]>,
    #[serde(default = "default_true")]
    pub nearest_filter: bool,
}
fn default_true() -> bool {
    true
}
impl Default for TextureProfile {
    fn default() -> Self {
        Self {
            width: 256,
            height: 256,
            target_td_px_per_m: 128.0,
            td_tolerance_pct: 15.0,
            padding_px: 4,
            palette: vec![
                [14, 20, 25],
                [46, 59, 58],
                [93, 106, 78],
                [175, 163, 110],
                [225, 211, 156],
            ],
            nearest_filter: true,
        }
    }
}
impl TextureProfile {
    pub fn validate(&self) -> Result<(), ContextError> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            return Err(ContextError::Invalid(
                "texture dimensions must be 1..=8192".into(),
            ));
        }
        if !self.target_td_px_per_m.is_finite() || self.target_td_px_per_m <= 0.0 {
            return Err(ContextError::Invalid(
                "target TD must be positive and finite".into(),
            ));
        }
        if !self.td_tolerance_pct.is_finite() || !(0.0..=100.0).contains(&self.td_tolerance_pct) {
            return Err(ContextError::Invalid(
                "TD tolerance must be 0..=100%".into(),
            ));
        }
        if self.padding_px.saturating_mul(4) >= self.width.min(self.height) {
            return Err(ContextError::Invalid("padding too large for atlas".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextureOverrides {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub target_td_px_per_m: Option<f64>,
    pub td_tolerance_pct: Option<f64>,
    pub padding_px: Option<u32>,
    pub palette: Option<Vec<[u8; 3]>>,
    pub nearest_filter: Option<bool>,
}
impl TextureOverrides {
    pub fn apply_to(&self, p: &mut TextureProfile) {
        if let Some(v) = self.width {
            p.width = v;
        }
        if let Some(v) = self.height {
            p.height = v;
        }
        if let Some(v) = self.target_td_px_per_m {
            p.target_td_px_per_m = v;
        }
        if let Some(v) = self.td_tolerance_pct {
            p.td_tolerance_pct = v;
        }
        if let Some(v) = self.padding_px {
            p.padding_px = v;
        }
        if let Some(v) = &self.palette {
            p.palette = v.clone();
        }
        if let Some(v) = self.nearest_filter {
            p.nearest_filter = v;
        }
    }

    fn apply_with_provenance(
        &self,
        profile: &mut TextureProfile,
        provenance: &mut BTreeMap<String, String>,
        source: &str,
    ) {
        macro_rules! apply_field {
            ($field:ident) => {
                if let Some(value) = self.$field {
                    profile.$field = value;
                    provenance.insert(
                        concat!("texture.", stringify!($field)).into(),
                        source.into(),
                    );
                }
            };
        }
        apply_field!(width);
        apply_field!(height);
        apply_field!(target_td_px_per_m);
        apply_field!(td_tolerance_pct);
        apply_field!(padding_px);
        apply_field!(nearest_filter);
        if let Some(value) = &self.palette {
            profile.palette = value.clone();
            provenance.insert("texture.palette".into(), source.into());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// The in-memory representation is always normalized to v2.
    pub schema_version: u32,
    pub project_id: String,
    pub project_uid: String,
    pub project_version: u32,
    pub display_name: String,
    pub units: String,
    pub meters_per_unit: f64,
    pub style: String,
    pub texture: TextureProfile,
    #[serde(skip)]
    pub defaulted_texture_fields: Vec<String>,
    pub naming: NamingSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migrated_from_schema: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingSettings {
    pub texture_pattern: String,
    pub material_pattern: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamingOverrides {
    #[serde(default)]
    pub texture_pattern: Option<String>,
    #[serde(default)]
    pub material_pattern: Option<String>,
}
impl NamingOverrides {
    fn apply_to(
        &self,
        target: &mut NamingSettings,
        provenance: &mut BTreeMap<String, String>,
        source: &str,
    ) {
        if let Some(pattern) = &self.texture_pattern {
            target.texture_pattern.clone_from(pattern);
            provenance.insert("naming.texture_pattern".into(), source.into());
        }
        if let Some(pattern) = &self.material_pattern {
            target.material_pattern.clone_from(pattern);
            provenance.insert("naming.material_pattern".into(), source.into());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetManifest {
    /// The in-memory representation is always normalized to v2.
    pub schema_version: u32,
    pub asset_id: String,
    pub asset_uid: String,
    pub asset_version: u32,
    pub category: String,
    pub blend_path: Option<String>,
    pub uv_map: Option<String>,
    #[serde(default)]
    pub protected_components: Vec<String>,
    #[serde(default)]
    pub texture: TextureOverrides,
    #[serde(default)]
    pub naming: NamingOverrides,
    #[serde(default)]
    pub face_ids: Vec<FaceIdentifier>,
    #[serde(default)]
    pub materials: Vec<MaterialIdentifier>,
    #[serde(default)]
    pub references: Vec<ReferenceArtifact>,
    #[serde(default)]
    pub source_artifacts: Vec<SourceArtifact>,
    /// Optional reference production state; absent in legacy v1/v2 manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<TechnicalReferenceState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migrated_from_schema: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnicalReferenceState {
    pub schema_version: u32,
    pub geometry_path: String,
    pub geometry_sha256: String,
    pub revision: u64,
    pub subcategory: String,
    pub validation_status: String,
    #[serde(default)]
    pub revision_history: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceIdentifier {
    pub face_id: String,
    pub component_id: String,
    pub material_id: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialIdentifier {
    pub material_id: String,
    /// Semantic artist role such as `wood`, `metal`, or `glass`.
    pub role: String,
    pub display_name: String,
    pub slot_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceArtifact {
    pub reference_id: String,
    pub role: String,
    pub path: String,
    pub reference_version: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceArtifact {
    pub artifact_id: String,
    pub role: String,
    pub path: String,
    /// Missing only on a v1 in-memory migration. Migrations never write files.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Match,
    Changed,
    Missing,
    Unpinned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactIntegrity {
    pub artifact_id: String,
    pub kind: String,
    pub role: String,
    pub path: String,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub state: ArtifactState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub project_root: PathBuf,
    pub project_schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_migrated_from_schema: Option<u32>,
    pub project_id: String,
    pub project_uid: String,
    pub project_version: u32,
    pub display_name: String,
    pub units: String,
    pub meters_per_unit: f64,
    pub style: String,
    pub asset: Option<AssetManifest>,
    pub texture: TextureProfile,
    pub naming: NamingSettings,
    pub artifact_integrity: Vec<ArtifactIntegrity>,
    /// Effective value origin keyed by stable property paths such as `texture.width`.
    pub provenance: BTreeMap<String, String>,
    /// Kept for the v0.1 UI contract; provenance is now available per property above.
    pub resolution_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDescriptor {
    pub project_id: String,
    pub project_uid: String,
    pub project_version: u32,
    pub display_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDescriptor {
    pub asset_id: String,
    pub asset_uid: String,
    pub asset_version: u32,
    pub category: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProfilePatch {
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    texture: TextureOverrides,
    #[serde(default)]
    naming: NamingOverrides,
}

#[derive(Debug, Deserialize)]
struct RawProjectSettings {
    schema_version: u32,
    project_id: String,
    #[serde(default)]
    project_uid: Option<String>,
    #[serde(default)]
    project_version: Option<u32>,
    display_name: String,
    units: String,
    style: String,
    texture: RawTextureProfile,
    naming: NamingSettings,
}

#[derive(Debug, Deserialize)]
struct RawTextureProfile {
    width: u32,
    height: u32,
    target_td_px_per_m: f64,
    td_tolerance_pct: f64,
    padding_px: u32,
    #[serde(default)]
    palette: Option<Vec<[u8; 3]>>,
    #[serde(default)]
    nearest_filter: Option<bool>,
}

impl RawTextureProfile {
    fn normalize(self) -> (TextureProfile, Vec<String>) {
        let defaults = TextureProfile::default();
        let mut defaulted_fields = Vec::new();
        let palette = self.palette.unwrap_or_else(|| {
            defaulted_fields.push("palette".into());
            defaults.palette
        });
        let nearest_filter = self.nearest_filter.unwrap_or_else(|| {
            defaulted_fields.push("nearest_filter".into());
            defaults.nearest_filter
        });
        (
            TextureProfile {
                width: self.width,
                height: self.height,
                target_td_px_per_m: self.target_td_px_per_m,
                td_tolerance_pct: self.td_tolerance_pct,
                padding_px: self.padding_px,
                palette,
                nearest_filter,
            },
            defaulted_fields,
        )
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawAssetManifest {
    #[serde(default)]
    schema_version: Option<u32>,
    asset_id: String,
    #[serde(default)]
    asset_uid: Option<String>,
    #[serde(default)]
    asset_version: Option<u32>,
    category: String,
    #[serde(default)]
    blend_path: Option<String>,
    #[serde(default)]
    uv_map: Option<String>,
    #[serde(default)]
    protected_components: Vec<String>,
    #[serde(default)]
    texture: TextureOverrides,
    #[serde(default)]
    naming: NamingOverrides,
    #[serde(default)]
    face_ids: Vec<FaceIdentifier>,
    #[serde(default)]
    materials: Vec<MaterialIdentifier>,
    #[serde(default)]
    references: Vec<ReferenceArtifact>,
    #[serde(default)]
    source_artifacts: Vec<SourceArtifact>,
    #[serde(default)]
    technical: Option<TechnicalReferenceState>,
}

pub fn read_project(root: &Path) -> Result<ProjectSettings, ContextError> {
    let root = root.canonicalize()?;
    let rel = Path::new(".assetlab/project.toml");
    if !root.join(rel).exists() {
        return Err(ContextError::NoProject);
    }
    let path = safe_existing(&root, rel)?;
    let raw: RawProjectSettings = toml::from_str(&fs::read_to_string(path)?)?;
    normalize_project(raw)
}

fn normalize_project(raw: RawProjectSettings) -> Result<ProjectSettings, ContextError> {
    if !matches!(raw.schema_version, 1 | 2) {
        return Err(ContextError::Schema(raw.schema_version));
    }
    checked_id(&raw.project_id)?;
    checked_id(&raw.style)?;
    if raw.display_name.trim().is_empty() {
        return Err(ContextError::Invalid(
            "display_name must not be empty".into(),
        ));
    }
    if raw.schema_version == 1 && raw.units != "meters" {
        return Err(ContextError::Invalid(
            "units must be meters in project schema v1".into(),
        ));
    }
    let meters_per_unit = meters_per_unit(&raw.units)?;
    if raw.schema_version == 2 && (raw.project_uid.is_none() || raw.project_version.is_none()) {
        return Err(ContextError::Invalid(
            "project schema v2 requires project_uid and project_version".into(),
        ));
    }
    let project_uid = raw
        .project_uid
        .unwrap_or_else(|| legacy_uid("project", &raw.project_id));
    checked_id(&project_uid)?;
    let project_version = raw.project_version.unwrap_or(1);
    if project_version == 0 {
        return Err(ContextError::Invalid(
            "project_version must be positive".into(),
        ));
    }
    let (texture, defaulted_texture_fields) = raw.texture.normalize();
    texture.validate()?;
    validate_naming(&raw.naming)?;
    Ok(ProjectSettings {
        schema_version: 2,
        project_id: raw.project_id,
        project_uid,
        project_version,
        display_name: raw.display_name,
        units: raw.units,
        meters_per_unit,
        style: raw.style,
        texture,
        defaulted_texture_fields,
        naming: raw.naming,
        migrated_from_schema: (raw.schema_version == 1).then_some(1),
    })
}

pub fn discover_projects(workspace: &Path) -> Result<Vec<ProjectDescriptor>, ContextError> {
    let workspace = workspace.canonicalize()?;
    let mut roots = Vec::new();
    if workspace.join(".assetlab/project.toml").exists() {
        roots.push(workspace.clone());
    } else {
        for entry in fs::read_dir(&workspace)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let candidate = entry.path();
            if candidate.join(".assetlab/project.toml").exists() {
                roots.push(candidate.canonicalize()?);
            }
        }
    }
    if roots.is_empty() {
        return Err(ContextError::NoProjects(workspace.display().to_string()));
    }
    let mut projects = Vec::with_capacity(roots.len());
    let mut ids = HashSet::new();
    for root in roots {
        let settings = read_project(&root)?;
        if !ids.insert(settings.project_id.to_ascii_lowercase()) {
            return Err(ContextError::DuplicateProjectId(settings.project_id));
        }
        projects.push(ProjectDescriptor {
            project_id: settings.project_id,
            project_uid: settings.project_uid,
            project_version: settings.project_version,
            display_name: settings.display_name,
            path: root,
        });
    }
    projects.sort_by_key(|project| project.project_id.to_ascii_lowercase());
    Ok(projects)
}

pub fn select_project(
    workspace: &Path,
    project_id: &str,
) -> Result<ProjectDescriptor, ContextError> {
    checked_id(project_id)?;
    discover_projects(workspace)?
        .into_iter()
        .find(|project| project.project_id == project_id)
        .ok_or_else(|| ContextError::ProjectNotFound(project_id.to_owned()))
}

pub fn discover_assets(project_root: &Path) -> Result<Vec<AssetDescriptor>, ContextError> {
    let root = project_root.canonicalize()?;
    read_project(&root)?;
    let assets_rel = Path::new("assets");
    let assets_path = root.join(assets_rel);
    if !assets_path.exists() {
        return Ok(Vec::new());
    }
    let assets_path = safe_existing(&root, assets_rel)?;
    if !assets_path.is_dir() {
        return Err(ContextError::Invalid("assets must be a directory".into()));
    }
    let mut assets = Vec::new();
    let mut ids = HashSet::new();
    for entry in fs::read_dir(assets_path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let directory_id = entry.file_name().to_string_lossy().into_owned();
        checked_id(&directory_id)?;
        let manifest_rel = PathBuf::from("assets")
            .join(&directory_id)
            .join("asset.toml");
        if !root.join(&manifest_rel).exists() {
            continue;
        }
        let manifest = read_asset_manifest_from_project(&root, &directory_id)?;
        if !ids.insert(directory_id.to_ascii_lowercase()) {
            return Err(ContextError::Invalid(format!(
                "asset directory collision for `{directory_id}`"
            )));
        }
        assets.push(AssetDescriptor {
            asset_id: manifest.asset_id,
            asset_uid: manifest.asset_uid,
            asset_version: manifest.asset_version,
            category: manifest.category,
            path: entry.path(),
        });
    }
    assets.sort_by_key(|asset| asset.asset_id.to_ascii_lowercase());
    Ok(assets)
}

pub fn read_asset_manifest(
    project_root: &Path,
    asset_id: &str,
) -> Result<AssetManifest, ContextError> {
    let root = project_root.canonicalize()?;
    let project = read_project(&root)?;
    read_asset_manifest_with_project(&root, &project, asset_id)
}

fn read_asset_manifest_from_project(
    root: &Path,
    asset_id: &str,
) -> Result<AssetManifest, ContextError> {
    let project = read_project(root)?;
    read_asset_manifest_with_project(root, &project, asset_id)
}

fn read_asset_manifest_with_project(
    root: &Path,
    project: &ProjectSettings,
    asset_id: &str,
) -> Result<AssetManifest, ContextError> {
    checked_id(asset_id)?;
    let rel = PathBuf::from("assets").join(asset_id).join("asset.toml");
    if !root.join(&rel).exists() {
        return Err(ContextError::MissingAsset(asset_id.to_owned()));
    }
    let path = safe_existing(root, &rel)?;
    let raw: RawAssetManifest = toml::from_str(&fs::read_to_string(path)?)?;
    normalize_asset(raw, project, asset_id)
}

fn normalize_asset(
    mut raw: RawAssetManifest,
    project: &ProjectSettings,
    expected_id: &str,
) -> Result<AssetManifest, ContextError> {
    let schema = raw.schema_version.unwrap_or(1);
    if !matches!(schema, 1 | 2) {
        return Err(ContextError::Schema(schema));
    }
    checked_id(&raw.asset_id)?;
    if raw.asset_id != expected_id {
        return Err(ContextError::Invalid(
            "manifest asset_id does not match its directory".into(),
        ));
    }
    checked_id(&raw.category)?;
    if schema == 2 && (raw.asset_uid.is_none() || raw.asset_version.is_none()) {
        return Err(ContextError::Invalid(
            "asset schema v2 requires asset_uid and asset_version".into(),
        ));
    }
    let asset_uid = raw.asset_uid.take().unwrap_or_else(|| {
        legacy_uid("asset", &format!("{}:{}", project.project_uid, expected_id))
    });
    checked_id(&asset_uid)?;
    let asset_version = raw.asset_version.unwrap_or(1);
    if asset_version == 0 {
        return Err(ContextError::Invalid(
            "asset_version must be positive".into(),
        ));
    }
    for component in &raw.protected_components {
        checked_id(component)?;
    }
    if let Some(path) = &raw.blend_path {
        validated_relative(Path::new(path))?;
    }
    if let Some(uv_map) = &raw.uv_map {
        validate_display_value(uv_map, "uv_map")?;
    }
    for face in &raw.face_ids {
        checked_id(&face.face_id)?;
        checked_id(&face.component_id)?;
        validate_display_value(&face.display_name, "face display_name")?;
        if let Some(material_id) = &face.material_id {
            checked_id(material_id)?;
        }
    }
    ensure_unique(
        raw.face_ids.iter().map(|face| face.face_id.as_str()),
        "face_id",
    )?;
    let mut material_ids = HashSet::new();
    for material in &raw.materials {
        checked_id(&material.material_id)?;
        checked_id(&material.role)?;
        validate_display_value(&material.display_name, "material display_name")?;
        validate_display_value(&material.slot_name, "material slot_name")?;
        material_ids.insert(material.material_id.to_ascii_lowercase());
    }
    ensure_unique(
        raw.materials
            .iter()
            .map(|material| material.material_id.as_str()),
        "material_id",
    )?;
    for face in &raw.face_ids {
        if face
            .material_id
            .as_ref()
            .is_some_and(|id| !material_ids.contains(&id.to_ascii_lowercase()))
        {
            return Err(ContextError::Invalid(format!(
                "face `{}` references an unknown material_id",
                face.face_id
            )));
        }
    }
    for reference in &mut raw.references {
        checked_id(&reference.reference_id)?;
        validate_reference_role(&reference.role)?;
        validated_relative(Path::new(&reference.path))?;
        if reference.reference_version == 0 {
            return Err(ContextError::Invalid(format!(
                "reference `{}` version must be positive",
                reference.reference_id
            )));
        }
        reference.sha256 = normalize_sha256(&reference.sha256)?;
    }
    ensure_unique(
        raw.references
            .iter()
            .map(|reference| reference.reference_id.as_str()),
        "reference_id",
    )?;
    for source in &mut raw.source_artifacts {
        checked_id(&source.artifact_id)?;
        checked_id(&source.role)?;
        validated_relative(Path::new(&source.path))?;
        if schema == 2 && source.sha256.is_none() {
            return Err(ContextError::Invalid(format!(
                "source artifact `{}` needs a pinned sha256 in schema v2",
                source.artifact_id
            )));
        }
        if let Some(hash) = &source.sha256 {
            source.sha256 = Some(normalize_sha256(hash)?);
        }
    }
    ensure_unique(
        raw.source_artifacts
            .iter()
            .map(|source| source.artifact_id.as_str()),
        "artifact_id",
    )?;
    if schema == 1
        && raw.source_artifacts.is_empty()
        && let Some(path) = &raw.blend_path
    {
        raw.source_artifacts.push(SourceArtifact {
            artifact_id: "legacy_blend".into(),
            role: "source_model".into(),
            path: path.clone(),
            sha256: None,
        });
    }
    if let Some(technical) = &raw.technical {
        if technical.schema_version != 1 {
            return Err(ContextError::Schema(technical.schema_version));
        }
        validated_relative(Path::new(&technical.geometry_path))?;
        normalize_sha256(&technical.geometry_sha256)?;
        checked_id(&technical.subcategory)?;
        for path in &technical.revision_history {
            validated_relative(Path::new(path))?;
        }
    }
    Ok(AssetManifest {
        schema_version: 2,
        asset_id: raw.asset_id,
        asset_uid,
        asset_version,
        category: raw.category,
        blend_path: raw.blend_path,
        uv_map: raw.uv_map,
        protected_components: raw.protected_components,
        texture: raw.texture,
        naming: raw.naming,
        face_ids: raw.face_ids,
        materials: raw.materials,
        references: raw.references,
        source_artifacts: raw.source_artifacts,
        technical: raw.technical,
        migrated_from_schema: (schema == 1).then_some(1),
    })
}

/// Resolution order: project < style < category < asset < explicit override.
/// Missing style/category profiles are actionable errors; neither is inferred.
pub fn resolve_context(
    root: &Path,
    asset_id: Option<&str>,
    explicit: Option<&TextureOverrides>,
) -> Result<ProjectContext, ContextError> {
    let root = root.canonicalize()?;
    let project = read_project(&root)?;
    let project_rel = ".assetlab/project.toml";
    let mut profile = project.texture.clone();
    let mut naming = project.naming.clone();
    let mut provenance = BTreeMap::new();
    seed_provenance(
        &mut provenance,
        project_rel,
        &project.defaulted_texture_fields,
    );
    for key in [
        "project_id",
        "project_uid",
        "project_version",
        "display_name",
        "units",
        "style",
    ] {
        provenance.insert(format!("project.{key}"), project_rel.into());
    }
    let mut sources = vec![project_rel.to_owned()];

    let style_rel = format!(".assetlab/styles/{}.toml", checked_id(&project.style)?);
    let style_patch = read_required_patch(&root, &style_rel, "style", &project.style)?;
    style_patch
        .texture
        .apply_with_provenance(&mut profile, &mut provenance, &style_rel);
    style_patch
        .naming
        .apply_to(&mut naming, &mut provenance, &style_rel);
    sources.push(style_rel.clone());

    let mut asset = None;
    let mut artifact_integrity = Vec::new();
    if let Some(id) = asset_id {
        let parsed = read_asset_manifest_with_project(&root, &project, id)?;
        let category_rel = format!(
            ".assetlab/categories/{}.toml",
            checked_id(&parsed.category)?
        );
        let category_patch =
            read_required_patch(&root, &category_rel, "category", &parsed.category)?;
        category_patch
            .texture
            .apply_with_provenance(&mut profile, &mut provenance, &category_rel);
        category_patch
            .naming
            .apply_to(&mut naming, &mut provenance, &category_rel);
        sources.push(category_rel.clone());

        parsed.texture.apply_with_provenance(
            &mut profile,
            &mut provenance,
            &format!("assets/{id}/asset.toml"),
        );
        parsed.naming.apply_to(
            &mut naming,
            &mut provenance,
            &format!("assets/{id}/asset.toml"),
        );
        let asset_rel = format!("assets/{id}/asset.toml");
        sources.push(asset_rel.clone());
        add_asset_provenance(&parsed, &asset_rel, &mut provenance);
        for source in &parsed.source_artifacts {
            artifact_integrity.push(inspect_artifact(
                &root,
                &source.artifact_id,
                "source",
                &source.role,
                &source.path,
                source.sha256.as_deref(),
            )?);
        }
        for reference in &parsed.references {
            artifact_integrity.push(inspect_artifact(
                &root,
                &reference.reference_id,
                "reference",
                &reference.role,
                &reference.path,
                Some(&reference.sha256),
            )?);
        }
        if let Some(technical) = &parsed.technical {
            artifact_integrity.push(inspect_artifact(
                &root,
                "geometry_spec",
                "reference",
                "technical_geometry",
                &technical.geometry_path,
                Some(&technical.geometry_sha256),
            )?);
        }
        asset = Some(parsed);
    }
    if let Some(override_) = explicit {
        override_.apply_with_provenance(&mut profile, &mut provenance, "explicit request");
        sources.push("explicit request".into());
    }
    profile.validate()?;
    validate_naming(&naming)?;
    sources.dedup();
    Ok(ProjectContext {
        project_root: root,
        project_schema_version: project.schema_version,
        project_migrated_from_schema: project.migrated_from_schema,
        project_id: project.project_id,
        project_uid: project.project_uid,
        project_version: project.project_version,
        display_name: project.display_name,
        units: project.units,
        meters_per_unit: project.meters_per_unit,
        style: project.style,
        asset,
        texture: profile,
        naming,
        artifact_integrity,
        provenance,
        resolution_sources: sources,
    })
}

pub fn resolve_workspace_context(
    workspace: &Path,
    project_id: &str,
    asset_id: Option<&str>,
    explicit: Option<&TextureOverrides>,
) -> Result<ProjectContext, ContextError> {
    let project = select_project(workspace, project_id)?;
    resolve_context(&project.path, asset_id, explicit)
}

fn read_required_patch(
    root: &Path,
    relative: &str,
    kind: &str,
    id: &str,
) -> Result<ProfilePatch, ContextError> {
    let rel = Path::new(relative);
    if !root.join(rel).exists() {
        let directory = match kind {
            "style" => "styles",
            "category" => "categories",
            _ => "profiles",
        };
        return Err(ContextError::MissingProfile {
            kind: kind.into(),
            id: id.into(),
            path: format!(".assetlab/{directory}/{id}.toml"),
        });
    }
    let path = safe_existing(root, rel)?;
    let patch: ProfilePatch = toml::from_str(&fs::read_to_string(path)?)?;
    if let Some(version) = patch.schema_version
        && !matches!(version, 1 | 2)
    {
        return Err(ContextError::Schema(version));
    }
    Ok(patch)
}

fn inspect_artifact(
    root: &Path,
    artifact_id: &str,
    kind: &str,
    role: &str,
    relative: &str,
    expected: Option<&str>,
) -> Result<ArtifactIntegrity, ContextError> {
    let rel = Path::new(relative);
    validated_relative(rel)?;
    let candidate = root.join(rel);
    let present = match fs::symlink_metadata(&candidate) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let actual = if present {
        let path = safe_existing(root, rel)?;
        Some(hash_file(&path)?)
    } else {
        None
    };
    let expected = expected.map(str::to_owned);
    let state = match (&actual, &expected) {
        (None, _) => ArtifactState::Missing,
        (Some(_), None) => ArtifactState::Unpinned,
        (Some(actual), Some(expected)) if actual == expected => ArtifactState::Match,
        (Some(_), Some(_)) => ArtifactState::Changed,
    };
    Ok(ArtifactIntegrity {
        artifact_id: artifact_id.into(),
        kind: kind.into(),
        role: role.into(),
        path: relative.into(),
        expected_sha256: expected,
        actual_sha256: actual,
        state,
    })
}

fn add_asset_provenance(
    asset: &AssetManifest,
    source: &str,
    provenance: &mut BTreeMap<String, String>,
) {
    for key in [
        "asset_id",
        "asset_uid",
        "asset_version",
        "category",
        "uv_map",
    ] {
        provenance.insert(format!("asset.{key}"), source.into());
    }
    for material in &asset.materials {
        provenance.insert(
            format!("asset.materials.{}.role", material.material_id),
            source.into(),
        );
    }
    for face in &asset.face_ids {
        provenance.insert(
            format!("asset.faces.{}.component_id", face.face_id),
            source.into(),
        );
        if face.material_id.is_some() {
            provenance.insert(
                format!("asset.faces.{}.material_id", face.face_id),
                source.into(),
            );
        }
    }
    for reference in &asset.references {
        provenance.insert(
            format!("asset.references.{}.role", reference.reference_id),
            source.into(),
        );
    }
    for artifact in &asset.source_artifacts {
        provenance.insert(
            format!("asset.source_artifacts.{}.sha256", artifact.artifact_id),
            source.into(),
        );
    }
}

fn seed_provenance(
    provenance: &mut BTreeMap<String, String>,
    source: &str,
    defaulted_texture_fields: &[String],
) {
    for field in [
        "width",
        "height",
        "target_td_px_per_m",
        "td_tolerance_pct",
        "padding_px",
        "palette",
        "nearest_filter",
    ] {
        let origin = if defaulted_texture_fields
            .iter()
            .any(|defaulted| defaulted == field)
        {
            "built-in TextureProfile defaults"
        } else {
            source
        };
        provenance.insert(format!("texture.{field}"), origin.into());
    }
    provenance.insert("naming.texture_pattern".into(), source.into());
    provenance.insert("naming.material_pattern".into(), source.into());
}

fn validate_naming(naming: &NamingSettings) -> Result<(), ContextError> {
    crate::naming::validate_pattern(
        &naming.texture_pattern,
        &["asset_id", "surface", "channel", "width", "height"],
        &["asset_id", "surface", "channel", "width"],
    )?;
    crate::naming::validate_pattern(
        &naming.material_pattern,
        &["asset_id", "surface"],
        &["asset_id", "surface"],
    )?;
    Ok(())
}

fn validate_reference_role(role: &str) -> Result<(), ContextError> {
    const ROLES: &[&str] = &[
        "asset_master",
        "concept",
        "dimension_guide",
        "orthographic_guide",
        "texture_motif",
        "projection_source",
        "design_reference",
        "silhouette_reference",
        "graphic_reference",
    ];
    if ROLES.contains(&role) {
        Ok(())
    } else {
        Err(ContextError::Invalid(format!(
            "unsupported reference role `{role}`"
        )))
    }
}

fn normalize_sha256(value: &str) -> Result<String, ContextError> {
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ContextError::Invalid(
            "sha256 must contain exactly 64 hexadecimal characters".into(),
        ));
    }
    Ok(format!("sha256:{}", hex.to_ascii_lowercase()))
}

fn hash_file(path: &Path) -> Result<String, ContextError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("sha256:{}", hex_digest(&hasher.finalize())))
}

fn legacy_uid(kind: &str, identity: &str) -> String {
    let digest = Sha256::digest(format!("dd-3dmagic/v1/{kind}/{identity}").as_bytes());
    format!("legacy-{kind}-{}", &hex_digest(&digest)[..32])
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ensure_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), ContextError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value.to_ascii_lowercase()) {
            return Err(ContextError::Invalid(format!("duplicate {kind} `{value}`")));
        }
    }
    Ok(())
}

fn validate_display_value(value: &str, field: &str) -> Result<(), ContextError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(ContextError::Invalid(format!(
            "{field} must be nonempty text"
        )))
    } else {
        Ok(())
    }
}

fn checked_id(value: &str) -> Result<&str, ContextError> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ContextError::UnsafePath(value.to_string()));
    }
    Ok(value)
}

fn validated_relative(path: &Path) -> Result<(), ContextError> {
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(ContextError::UnsafePath(path.display().to_string()));
    }
    Ok(())
}

pub fn meters_per_unit(units: &str) -> Result<f64, ContextError> {
    match units {
        "meters" | "meter" | "m" => Ok(1.0),
        "centimeters" | "centimeter" | "cm" => Ok(0.01),
        "millimeters" | "millimeter" | "mm" => Ok(0.001),
        "kilometers" | "kilometer" | "km" => Ok(1000.0),
        "feet" | "foot" | "ft" => Ok(0.3048),
        "inches" | "inch" | "in" => Ok(0.0254),
        _ => Err(ContextError::Invalid(format!(
            "unsupported project units `{units}`; use meters, centimeters, millimeters, feet, or inches"
        ))),
    }
}

pub fn convert_to_meters(value: f64, units: &str) -> Result<f64, ContextError> {
    if !value.is_finite() {
        return Err(ContextError::Invalid("unit value must be finite".into()));
    }
    Ok(value * meters_per_unit(units)?)
}

/// Read-only operations reject path traversal and symlinks escaping the root.
pub fn safe_existing(root: &Path, relative: &Path) -> Result<PathBuf, ContextError> {
    validated_relative(relative)?;
    let base = root.canonicalize()?;
    let actual = base.join(relative).canonicalize()?;
    if !actual.starts_with(&base) {
        return Err(ContextError::OutsideProject(relative.display().to_string()));
    }
    Ok(actual)
}

/// Writes require an existing parent inside the project and reject destination symlinks.
pub fn safe_output(root: &Path, relative: &Path) -> Result<PathBuf, ContextError> {
    crate::storage::require_d_path(root)
        .map_err(|error| ContextError::Invalid(format!("PATH_OUTSIDE_D: {error}")))?;
    validated_relative(relative)?;
    let base = root.canonicalize()?;
    let full = base.join(relative);
    let parent = full
        .parent()
        .ok_or_else(|| ContextError::UnsafePath(relative.display().to_string()))?
        .canonicalize()?;
    if !parent.starts_with(&base) {
        return Err(ContextError::OutsideProject(relative.display().to_string()));
    }
    if full
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(ContextError::OutsideProject(relative.display().to_string()));
    }
    Ok(full)
}

/// Create the staging directory only after proving `.assetlab` resolves inside the project.
pub fn ensure_staging_dir(root: &Path) -> Result<PathBuf, ContextError> {
    crate::storage::require_d_path(root)
        .map_err(|error| ContextError::Invalid(format!("PATH_OUTSIDE_D: {error}")))?;
    let base = root.canonicalize()?;
    let assetlab = safe_existing(&base, Path::new(".assetlab"))?;
    if !assetlab.is_dir() {
        return Err(ContextError::Invalid(
            ".assetlab must be a directory".into(),
        ));
    }
    let staging = assetlab.join("staging");
    match fs::symlink_metadata(&staging) {
        Ok(_) => {
            let resolved = safe_existing(&base, Path::new(".assetlab/staging"))?;
            if !resolved.is_dir() {
                return Err(ContextError::Invalid(
                    ".assetlab/staging must be a directory".into(),
                ));
            }
            Ok(resolved)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&staging)?;
            safe_existing(&base, Path::new(".assetlab/staging"))
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dd3d-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".assetlab/styles")).unwrap();
        fs::create_dir_all(root.join(".assetlab/categories")).unwrap();
        write_project(&root, "meters");
        fs::write(
            root.join(".assetlab/styles/lowpoly_pixel.toml"),
            "schema_version = 2\n[texture]\nwidth = 256\n[naming]\ntexture_pattern = \"S_{asset_id}_{surface}_{channel}_{width}.png\"\n",
        )
        .unwrap();
        fs::write(
            root.join(".assetlab/categories/prop.toml"),
            "schema_version = 2\n[texture]\nwidth = 512\n",
        )
        .unwrap();
        root
    }

    fn write_project(root: &Path, units: &str) {
        fs::create_dir_all(root.join(".assetlab")).unwrap();
        fs::write(
            root.join(".assetlab/project.toml"),
            format!(
                "schema_version = 2\nproject_id = \"demo\"\nproject_uid = \"prj_demo_uid\"\nproject_version = 3\ndisplay_name = \"Demo\"\nunits = \"{units}\"\nstyle = \"lowpoly_pixel\"\n\n[texture]\nwidth = 128\nheight = 128\ntarget_td_px_per_m = 96.0\ntd_tolerance_pct = 15.0\npadding_px = 4\n\n[naming]\ntexture_pattern = \"T_{{asset_id}}_{{surface}}_{{channel}}_{{width}}.png\"\nmaterial_pattern = \"M_{{asset_id}}_{{surface}}\"\n"
            ),
        )
        .unwrap();
    }

    fn write_asset(root: &Path, source_hash: &str) -> PathBuf {
        let asset_dir = root.join("assets/demo_prop");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::create_dir_all(asset_dir.join("textures")).unwrap();
        fs::write(asset_dir.join("textures/source.bin"), b"source").unwrap();
        let manifest = format!(
            "schema_version = 2\nasset_id = \"demo_prop\"\nasset_uid = \"ast_demo_prop_uid\"\nasset_version = 4\ncategory = \"prop\"\nuv_map = \"UV_Main\"\n\n[[materials]]\nmaterial_id = \"wood\"\nrole = \"wood\"\ndisplay_name = \"Aged wood\"\nslot_name = \"wood_section\"\n\n[[face_ids]]\nface_id = \"wood_front\"\ncomponent_id = \"stock\"\nmaterial_id = \"wood\"\ndisplay_name = \"Wood front\"\n\n[[source_artifacts]]\nartifact_id = \"source_texture\"\nrole = \"source_texture\"\npath = \"assets/demo_prop/textures/source.bin\"\nsha256 = \"{source_hash}\"\n"
        );
        fs::write(asset_dir.join("asset.toml"), manifest).unwrap();
        asset_dir.join("asset.toml")
    }

    #[test]
    fn resolves_precedence_roles_and_property_provenance() {
        let root = temp_root("precedence");
        let hash = hash_file(&root.join(".assetlab/project.toml")).unwrap();
        let manifest = write_asset(&root, &hash);
        fs::write(
            manifest,
            format!(
                "{}\n[texture]\nwidth = 1024\n",
                fs::read_to_string(root.join("assets/demo_prop/asset.toml")).unwrap()
            ),
        )
        .unwrap();
        let request = TextureOverrides {
            width: Some(64),
            ..Default::default()
        };
        let context = resolve_context(&root, Some("demo_prop"), Some(&request)).unwrap();
        assert_eq!(context.texture.width, 64);
        assert_eq!(context.texture.height, 128);
        assert_eq!(context.provenance["texture.width"], "explicit request");
        assert_eq!(
            context.provenance["texture.height"],
            ".assetlab/project.toml"
        );
        assert_eq!(
            context.provenance["texture.palette"],
            "built-in TextureProfile defaults"
        );
        assert_eq!(
            context.provenance["naming.texture_pattern"],
            ".assetlab/styles/lowpoly_pixel.toml"
        );
        let asset = context.asset.unwrap();
        assert_eq!(asset.asset_uid, "ast_demo_prop_uid");
        assert_eq!(asset.materials[0].role, "wood");
        assert_eq!(asset.face_ids[0].material_id.as_deref(), Some("wood"));
        assert_eq!(asset.asset_version, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_profiles_are_actionable() {
        let root = temp_root("missing-profile");
        fs::remove_file(root.join(".assetlab/styles/lowpoly_pixel.toml")).unwrap();
        assert!(matches!(
            resolve_context(&root, None, None),
            Err(ContextError::MissingProfile { kind, id, .. }) if kind == "style" && id == "lowpoly_pixel"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_category_profile_is_reported_without_inference() {
        let root = temp_root("missing-category");
        let hash = hash_file(&root.join(".assetlab/project.toml")).unwrap();
        write_asset(&root, &hash);
        fs::remove_file(root.join(".assetlab/categories/prop.toml")).unwrap();
        assert!(matches!(
            resolve_context(&root, Some("demo_prop"), None),
            Err(ContextError::MissingProfile { kind, id, .. })
                if kind == "category" && id == "prop"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_malformed_values_and_unknown_materials() {
        let root = temp_root("malformed");
        fs::write(
            root.join(".assetlab/project.toml"),
            fs::read_to_string(root.join(".assetlab/project.toml"))
                .unwrap()
                .replace("target_td_px_per_m = 96.0", "target_td_px_per_m = 0.0"),
        )
        .unwrap();
        assert!(matches!(
            resolve_context(&root, None, None),
            Err(ContextError::Invalid(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v1_migration_is_read_only_and_stable() {
        let root = temp_root("migration");
        write_project(&root, "meters");
        let project_path = root.join(".assetlab/project.toml");
        fs::write(
            &project_path,
            "schema_version = 1\nproject_id = \"legacy\"\ndisplay_name = \"Legacy\"\nunits = \"meters\"\nstyle = \"lowpoly_pixel\"\n[texture]\nwidth = 128\nheight = 128\ntarget_td_px_per_m = 96.0\ntd_tolerance_pct = 15.0\npadding_px = 4\n[naming]\ntexture_pattern = \"T_{asset_id}_{surface}_{channel}_{width}.png\"\nmaterial_pattern = \"M_{asset_id}_{surface}\"\n",
        )
        .unwrap();
        let before = fs::read(&project_path).unwrap();
        let first = read_project(&root).unwrap();
        let second = read_project(&root).unwrap();
        assert_eq!(first.schema_version, 2);
        assert_eq!(first.migrated_from_schema, Some(1));
        assert_eq!(first.project_uid, second.project_uid);
        assert_eq!(fs::read(&project_path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v1_asset_migration_is_read_only_and_generates_stable_identity() {
        let root = temp_root("asset-migration");
        let asset_dir = root.join("assets/demo_prop");
        fs::create_dir_all(&asset_dir).unwrap();
        let manifest = asset_dir.join("asset.toml");
        fs::write(
            &manifest,
            "asset_id = \"demo_prop\"\ncategory = \"prop\"\nuv_map = \"UV_Main\"\nprotected_components = [\"body\"]\n[texture]\npadding_px = 4\n",
        )
        .unwrap();
        let before = fs::read(&manifest).unwrap();
        let first = read_asset_manifest(&root, "demo_prop").unwrap();
        let second = read_asset_manifest(&root, "demo_prop").unwrap();
        assert_eq!(first.schema_version, 2);
        assert_eq!(first.migrated_from_schema, Some(1));
        assert_eq!(first.asset_version, 1);
        assert!(first.asset_uid.starts_with("legacy-asset-"));
        assert_eq!(first.asset_uid, second.asset_uid);
        assert_eq!(fs::read(&manifest).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_face_ids_that_reference_unknown_materials() {
        let root = temp_root("unknown-material");
        let manifest = write_asset(
            &root,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        );
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str(
            "\n[[face_ids]]\nface_id = \"unknown_surface\"\ncomponent_id = \"stock\"\nmaterial_id = \"missing_material\"\ndisplay_name = \"Bad reference\"\n",
        );
        fs::write(&manifest, text).unwrap();
        assert!(matches!(
            read_asset_manifest(&root, "demo_prop"),
            Err(ContextError::Invalid(message)) if message.contains("unknown material_id")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_source_is_reported_without_rewriting_manifest() {
        let root = temp_root("source-hash");
        let source_hash = hash_file(&root.join(".assetlab/project.toml")).unwrap();
        let manifest = write_asset(&root, &source_hash);
        let before = fs::read(&manifest).unwrap();
        let first = resolve_context(&root, Some("demo_prop"), None).unwrap();
        assert!(matches!(
            first.artifact_integrity[0].state,
            ArtifactState::Changed
        ));
        let actual = hash_file(&root.join("assets/demo_prop/textures/source.bin")).unwrap();
        fs::write(
            &manifest,
            fs::read_to_string(&manifest)
                .unwrap()
                .replace(&source_hash, &actual),
        )
        .unwrap();
        let pinned_manifest = fs::read(&manifest).unwrap();
        let matched = resolve_context(&root, Some("demo_prop"), None).unwrap();
        assert!(matches!(
            matched.artifact_integrity[0].state,
            ArtifactState::Match
        ));
        fs::write(
            root.join("assets/demo_prop/textures/source.bin"),
            b"artist edit",
        )
        .unwrap();
        let changed = resolve_context(&root, Some("demo_prop"), None).unwrap();
        assert!(matches!(
            changed.artifact_integrity[0].state,
            ArtifactState::Changed
        ));
        assert_eq!(fs::read(&manifest).unwrap(), pinned_manifest);
        assert_ne!(before, pinned_manifest);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_parent_traversal_and_symlink_escape() {
        let root = temp_root("path-guard");
        let outside = root.parent().unwrap().join(format!(
            "dd3d-outside-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("outside.txt"), b"outside").unwrap();
        assert!(matches!(
            safe_existing(&root, Path::new("../outside.txt")),
            Err(ContextError::UnsafePath(_))
        ));
        let link = root.join(".assetlab/outside-link");
        #[cfg(windows)]
        {
            let link_literal = link.to_string_lossy().replace('\'', "''");
            let outside_literal = outside.to_string_lossy().replace('\'', "''");
            let command = format!(
                "New-Item -ItemType Junction -Path '{link_literal}' -Target '{outside_literal}' | Out-Null"
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
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(matches!(
            safe_existing(&root, Path::new(".assetlab/outside-link/outside.txt")),
            Err(ContextError::OutsideProject(_))
        ));
        #[cfg(windows)]
        fs::remove_dir(link).unwrap();
        #[cfg(unix)]
        fs::remove_file(link).unwrap();
        fs::remove_dir_all(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_and_selects_project_from_workspace() {
        let workspace = temp_root("workspace");
        fs::remove_file(workspace.join(".assetlab/project.toml")).unwrap();
        let first = workspace.join("project_a");
        let second = workspace.join("project_b");
        fs::create_dir_all(first.join(".assetlab")).unwrap();
        fs::create_dir_all(second.join(".assetlab")).unwrap();
        write_project(&first, "meters");
        write_project(&second, "meters");
        let second_project = fs::read_to_string(second.join(".assetlab/project.toml"))
            .unwrap()
            .replace("project_id = \"demo\"", "project_id = \"second\"")
            .replace(
                "project_uid = \"prj_demo_uid\"",
                "project_uid = \"prj_second_uid\"",
            );
        fs::write(second.join(".assetlab/project.toml"), second_project).unwrap();
        let found = discover_projects(&workspace).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            select_project(&workspace, "second").unwrap().path,
            second.canonicalize().unwrap()
        );
        assert!(matches!(
            select_project(&workspace, "unknown"),
            Err(ContextError::ProjectNotFound(_))
        ));
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn workspace_selection_and_direct_resolution_are_identical() {
        let workspace = temp_root("same-context");
        fs::remove_file(workspace.join(".assetlab/project.toml")).unwrap();
        let project_root = workspace.join("artist_project");
        fs::create_dir_all(project_root.join(".assetlab/styles")).unwrap();
        fs::create_dir_all(project_root.join(".assetlab/categories")).unwrap();
        write_project(&project_root, "centimeters");
        fs::write(
            project_root.join(".assetlab/styles/lowpoly_pixel.toml"),
            "schema_version = 2\n[texture]\nwidth = 256\n",
        )
        .unwrap();
        fs::write(
            project_root.join(".assetlab/categories/prop.toml"),
            "schema_version = 2\n[texture]\ntarget_td_px_per_m = 128.0\n",
        )
        .unwrap();
        write_asset(
            &project_root,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        );
        let direct = resolve_context(&project_root, Some("demo_prop"), None).unwrap();
        let selected =
            resolve_workspace_context(&workspace, "demo", Some("demo_prop"), None).unwrap();
        assert_eq!(
            serde_json::to_value(direct).unwrap(),
            serde_json::to_value(selected).unwrap()
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn creates_only_in_project_staging() {
        let root = temp_root("staging-dir");
        fs::remove_dir_all(root.join(".assetlab/staging")).unwrap_or(());
        let staging = ensure_staging_dir(&root).unwrap();
        assert!(staging.is_dir());
        assert!(staging.starts_with(root.canonicalize().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unit_conversion_normalizes_artist_units_to_meters() {
        assert_eq!(convert_to_meters(250.0, "centimeters").unwrap(), 2.5);
        assert!((convert_to_meters(12.0, "inches").unwrap() - 0.3048).abs() < 1e-12);
        assert!(convert_to_meters(f64::INFINITY, "meters").is_err());
    }

    #[test]
    fn overrides_do_not_mutate_other_fields_and_zero_density_is_rejected() {
        let mut base = TextureProfile::default();
        TextureOverrides {
            width: Some(128),
            ..Default::default()
        }
        .apply_to(&mut base);
        assert_eq!(base.width, 128);
        assert_eq!(base.height, 256);
        let invalid = TextureProfile {
            target_td_px_per_m: 0.0,
            ..TextureProfile::default()
        };
        assert!(invalid.validate().is_err());
    }
}
