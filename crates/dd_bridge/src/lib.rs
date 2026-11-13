//! Shared filesystem workflows and bounded external Blender execution.
//! Geometry remains in deterministic dd_core; GUI, CLI and MCP call this service.
pub mod drawing;
pub mod graphic;
use crate::graphic::{
    GraphicProvider, GraphicReferenceInput, GraphicReferenceRequest, rasterize_projection,
    render_silhouette_reference, render_with_provider,
};
use dd_core::geometry::{
    GeometryError, GeometrySpec, PixelRenderOptions, ReferenceCommand, ReferenceDocument, View,
    render_pixel_reference, render_technical_sheet_svg,
};
use dd_core::storage::ManagedWorkspace;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, String>;

const SILHOUETTE_WIDTH: u32 = 1440;
const SILHOUETTE_HEIGHT: u32 = 480;
const SILHOUETTE_INK: [u8; 4] = [20, 24, 28, 255];
const SILHOUETTE_PAPER: [u8; 4] = [246, 244, 236, 255];

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewedProxy {
    pub scene_relative: String,
    pub scene_sha256: String,
    pub geometry_sha256: String,
    pub drawing_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceRequest {
    Create {
        output_relative: String,
        #[serde(default)]
        spec: Option<Value>,
    },
    Inspect {
        source_relative: String,
    },
    Apply {
        source_relative: String,
        output_relative: String,
        command: Value,
    },
    Undo {
        source_relative: String,
        output_relative: String,
    },
    Redo {
        source_relative: String,
        output_relative: String,
    },
    Render {
        source_relative: String,
        output_relative: String,
    },
    VerifyBlender {
        source_relative: String,
        scene_relative: String,
        output_relative: String,
        #[serde(default)]
        detail_tolerance: Option<f64>,
    },
    Blender {
        source_relative: String,
        output_relative: String,
        category: String,
        subcategory: String,
        slug: String,
        /// Optional generated appearance raster. Geometry remains sourced from GeometrySpec.
        #[serde(default)]
        appearance_relative: Option<String>,
        #[serde(default)]
        appearance_brief: Option<String>,
        /// Optional project-local design reference recorded in the published manifest.
        #[serde(default)]
        design_reference_relative: Option<String>,
        /// Optional editable black ink canvas to bundle with the technical revision.
        #[serde(default)]
        drawing_reference_relative: Option<String>,
        /// Publish the exact reviewed scene and drawing instead of rebuilding the proxy.
        #[serde(default)]
        reviewed_proxy: Option<Box<ReviewedProxy>>,
        #[serde(default)]
        publish: bool,
        /// Compare-and-swap bootstrap for an existing unpinned managed scene.
        #[serde(default)]
        expected_previous_sha256: Option<String>,
    },
    Recover,
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn geometry_error(error: GeometryError) -> String {
    let code = match &error {
        GeometryError::Draft(_) => "UNDERDETERMINED",
        GeometryError::Unsupported(_) => "UNSUPPORTED_OPERATION",
        GeometryError::Conflict(_) => "CONSTRAINT_CONFLICT",
        GeometryError::NodeNotFound(_) => "INVALID_REFERENCE",
        _ => "INVALID_GEOMETRY",
    };
    format!("{code}: {error}")
}
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn load_spec(workspace: &ManagedWorkspace, relative: &str) -> Result<GeometrySpec> {
    let path = workspace.existing(Path::new(relative)).map_err(err)?;
    if path.metadata().map_err(err)?.len() > 8 * 1024 * 1024 {
        return Err("INVALID_REFERENCE: spec exceeds 8 MiB".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
}

fn stage_directory(workspace: &ManagedWorkspace, relative: &str) -> Result<PathBuf> {
    workspace
        .new_staging_directory(Path::new(relative))
        .map_err(err)
}

fn checked_design_reference(workspace: &ManagedWorkspace, relative: &str) -> Result<PathBuf> {
    let path = workspace.existing(Path::new(relative)).map_err(|error| {
        format!("INVALID_REFERENCE: design reference must be an existing project file: {error}")
    })?;
    let metadata = path.metadata().map_err(err)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(
            "INVALID_REFERENCE: design reference must be a file no larger than 64 MiB".into(),
        );
    }
    Ok(path)
}

fn read_appearance(
    workspace: &ManagedWorkspace,
    relative: &str,
) -> Result<(PathBuf, RgbaImage, String, Vec<u8>)> {
    let path = workspace.existing(Path::new(relative)).map_err(|error| {
        format!("INVALID_REFERENCE: appearance must be an existing project file: {error}")
    })?;
    let metadata = path.metadata().map_err(err)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("INVALID_REFERENCE: appearance must be a file no larger than 64 MiB".into());
    }
    let bytes = fs::read(&path).map_err(err)?;
    let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(err)?
        .into_dimensions()
        .map_err(err)?;
    let pixels = u64::from(dimensions.0) * u64::from(dimensions.1);
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > 4096
        || dimensions.1 > 4096
        || pixels > 16_777_216
    {
        return Err("INVALID_REFERENCE: appearance dimensions exceed the 4096px/16MP limit".into());
    }
    let image = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(err)?
        .decode()
        .map_err(err)?
        .into_rgba8();
    Ok((path, image, hash(&bytes), bytes))
}

fn alpha_bounds(image: &RgbaImage, threshold: u8) -> Option<(u32, u32, u32, u32)> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (image.width(), image.height(), 0_u32, 0_u32);
    let mut found = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] < threshold {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + 1);
        max_y = max_y.max(y + 1);
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

fn register_appearance(appearance: &RgbaImage, contour: &RgbaImage) -> Result<RgbaImage> {
    let (source_x0, source_y0, source_x1, source_y1) = alpha_bounds(appearance, 16)
        .ok_or("INVALID_REFERENCE: appearance has no visible pixels")?;
    let (target_x0, target_y0, target_x1, target_y1) =
        alpha_bounds(contour, 1).ok_or("INVALID_REFERENCE: projected contour is empty")?;
    let crop = image::imageops::crop_imm(
        appearance,
        source_x0,
        source_y0,
        source_x1 - source_x0,
        source_y1 - source_y0,
    )
    .to_image();
    let fitted = image::imageops::resize(
        &crop,
        target_x1 - target_x0,
        target_y1 - target_y0,
        image::imageops::FilterType::Nearest,
    );
    let mut canvas = RgbaImage::new(contour.width(), contour.height());
    image::imageops::replace(
        &mut canvas,
        &fitted,
        i64::from(target_x0),
        i64::from(target_y0),
    );
    Ok(canvas)
}

struct SuppliedAppearance(RgbaImage);

impl GraphicProvider for SuppliedAppearance {
    fn create_appearance(
        &self,
        _: &GraphicReferenceInput<'_>,
    ) -> std::result::Result<RgbaImage, String> {
        Ok(self.0.clone())
    }
}

struct AppearanceSource<'a> {
    image: &'a RgbaImage,
    sha256: &'a str,
    relative: &'a str,
    original_bytes: &'a [u8],
    brief: &'a str,
    design_reference_relative: Option<&'a str>,
    design_reference_sha256: Option<&'a str>,
}

fn stage_appearance_reference(
    workspace: &ManagedWorkspace,
    spec: &GeometrySpec,
    appearance: AppearanceSource<'_>,
    directory: &Path,
) -> Result<Value> {
    let projection = spec.project(View::Side).map_err(geometry_error)?;
    let raster = rasterize_projection(&projection, 480, 160)?;
    let request = GraphicReferenceRequest::from_inputs(
        spec,
        &projection,
        &raster.contour,
        &raster.component_masks,
        None,
        appearance.brief,
    )?;
    let fitted = register_appearance(appearance.image, &raster.contour)?;
    let provider = SuppliedAppearance(fitted);
    let input = GraphicReferenceInput {
        request: &request,
        geometry: spec,
        projection: &projection,
        contour: &raster.contour,
        component_masks: &raster.component_masks,
        design_reference: None,
    };
    let clipped = render_with_provider(&provider, &input)?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(clipped.clone())
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(err)?;
    workspace
        .write_new(&directory.join("graphic.png"), encoded.get_ref())
        .map_err(err)?;
    workspace
        .write_new(
            &directory.join("appearance_input.bin"),
            appearance.original_bytes,
        )
        .map_err(err)?;
    write_json(workspace, &directory.join("graphic_request.json"), &request)?;
    let clipped_hash = hash(encoded.get_ref());
    let report = json!({
        "status":"PASS_WITH_WARNINGS",
        "geometry_revision":spec.revision,
        "view":"side",
        "appearance_input_sha256":appearance.sha256,
        "appearance_input_relative":appearance.relative,
        "appearance_input_bundle_path":"appearance_input.bin",
        "design_reference_relative":appearance.design_reference_relative,
        "design_reference_sha256":appearance.design_reference_sha256,
        "graphic_sha256":clipped_hash,
        "contour_clipping":"PASS",
        "component_mask_count":raster.component_masks.legend.len(),
        "semantic_appearance":"NOT_VERIFIABLE",
        "warning":"Appearance detail is not a geometry or semantic correctness claim."
    });
    write_json(workspace, &directory.join("graphic_report.json"), &report)?;
    Ok(report)
}

fn write_json(
    workspace: &ManagedWorkspace,
    path: &Path,
    value: &impl Serialize,
) -> Result<PathBuf> {
    workspace
        .write_new(path, &serde_json::to_vec_pretty(value).map_err(err)?)
        .map_err(err)
}

pub fn validate_spec(spec: &GeometrySpec) -> Result<Value> {
    let validation = spec.validate();
    if !validation.is_ready() {
        let code = match validation.status {
            dd_core::geometry::ValidationStatus::Draft => "UNDERDETERMINED",
            dd_core::geometry::ValidationStatus::Unsupported => "UNSUPPORTED_OPERATION",
            dd_core::geometry::ValidationStatus::Invalid
                if validation
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "constraint_conflict") =>
            {
                "CONSTRAINT_CONFLICT"
            }
            dd_core::geometry::ValidationStatus::Invalid
                if validation.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code.contains("dimension") && diagnostic.code.contains("node")
                }) =>
            {
                "INVALID_DIMENSION_ANCHOR"
            }
            dd_core::geometry::ValidationStatus::Invalid => "INVALID_GEOMETRY",
            dd_core::geometry::ValidationStatus::Valid => unreachable!("ready validation"),
        };
        return Err(format!(
            "{code}: reference geometry gate returned {:?}: {:?}",
            validation.status, validation.diagnostics
        ));
    }
    let report = serde_json::to_value(validation).map_err(err)?;
    // proxy is also a strict geometry/constraint gate, never infer success from a log.
    spec.proxy().map_err(geometry_error)?;
    Ok(report)
}

pub fn render_bundle(
    workspace: &ManagedWorkspace,
    spec: &GeometrySpec,
    relative: &str,
) -> Result<Value> {
    export_document(workspace, &ReferenceDocument::new(spec.clone()), relative)
}

fn save_revision(
    workspace: &ManagedWorkspace,
    document: &ReferenceDocument,
    relative: &str,
) -> Result<Value> {
    let validation = document.spec.validate();
    if validation.is_ready() {
        return export_document(workspace, document, relative);
    }
    if !validation.is_valid() {
        return Err(format!("INVALID_GEOMETRY: {:?}", validation.diagnostics));
    }
    stage_directory(workspace, relative)?;
    let base = Path::new(relative);
    write_json(workspace, &base.join("geometry.json"), &document.spec)?;
    write_json(workspace, &base.join("document.json"), document)?;
    write_json(workspace, &base.join("validation.json"), &validation)?;
    Ok(json!({
        "directory": relative, "revision": document.spec.revision, "views": [],
        "validation": validation, "projection_gate": "NOT_VERIFIABLE"
    }))
}

pub fn export_document(
    workspace: &ManagedWorkspace,
    document: &ReferenceDocument,
    relative: &str,
) -> Result<Value> {
    let spec = &document.spec;
    let validation = validate_spec(spec)?;
    let view_data: Vec<_> = spec
        .technical_sheet_layout
        .views()
        .iter()
        .copied()
        .map(|view| spec.project(view).map_err(geometry_error))
        .collect::<Result<_>>()?;
    let projection_gate = dd_core::reference_validation::validate_projections(spec, &view_data);
    if projection_gate.status != dd_core::reference_validation::GateStatus::Pass {
        return Err(format!(
            "PROJECTION_FAILED: {:?}",
            projection_gate.diagnostics
        ));
    }
    stage_directory(workspace, relative)?;
    let base = Path::new(relative);
    write_json(workspace, &base.join("geometry.json"), spec)?;
    write_json(workspace, &base.join("document.json"), document)?;
    write_json(workspace, &base.join("validation.json"), &validation)?;
    write_json(
        workspace,
        &base.join("projection_validation.json"),
        &projection_gate,
    )?;
    write_json(
        workspace,
        &base.join("proxy.json"),
        &spec.proxy().map_err(geometry_error)?,
    )?;
    let svg = render_technical_sheet_svg(spec, 1440, 1000).map_err(geometry_error)?;
    workspace
        .write_new(&base.join("technical.svg"), svg.as_bytes())
        .map_err(err)?;
    let mut projections = Vec::new();
    let mut silhouette_views = Vec::new();
    for &view in spec.technical_sheet_layout.views() {
        let name = view.key();
        let projection = spec.project(view).map_err(geometry_error)?;
        write_json(
            workspace,
            &base.join(format!("{name}.projection.json")),
            &projection,
        )?;
        let pixels = render_pixel_reference(spec, view, 480, 160, PixelRenderOptions::default())
            .map_err(geometry_error)?;
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(err)?;
        workspace
            .write_new(&base.join(format!("{name}.png")), encoded.get_ref())
            .map_err(err)?;
        let silhouette = render_silhouette_reference(
            &projection,
            SILHOUETTE_WIDTH,
            SILHOUETTE_HEIGHT,
            SILHOUETTE_INK,
            SILHOUETTE_PAPER,
        )?;
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(silhouette)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .map_err(err)?;
        let silhouette_path = format!("{name}_silhouette.png");
        workspace
            .write_new(&base.join(&silhouette_path), encoded.get_ref())
            .map_err(err)?;
        projections.push(name);
        silhouette_views.push(silhouette_path);
    }
    Ok(
        json!({"directory":relative,"revision":spec.revision,"views":projections,"validation":validation,
        "projection_validation":projection_gate,"status":"PASS",
        "silhouette_views":silhouette_views,
        "silhouette_resolution":[SILHOUETTE_WIDTH,SILHOUETTE_HEIGHT],
        "silhouette_source":"GeometrySpec projection mask; no image generation",
        "quality_note":"Programmatic material views and a separate single-color silhouette reference. Artist acceptance is separate from geometry validation."}),
    )
}

pub fn load_document(workspace: &ManagedWorkspace, source: &str) -> Result<ReferenceDocument> {
    let spec = load_spec(workspace, source)?;
    let relative = Path::new(source)
        .parent()
        .ok_or("INVALID_REFERENCE: spec needs a parent directory")?
        .join("document.json");
    if !workspace.root().join(&relative).try_exists().map_err(err)? {
        return Ok(ReferenceDocument::new(spec));
    }
    let path = workspace.existing(&relative).map_err(err)?;
    if path.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
        return Err("INVALID_REFERENCE: history exceeds 16 MiB".into());
    }
    let document: ReferenceDocument =
        serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)?;
    if serde_json::to_value(&document.spec).map_err(err)?
        != serde_json::to_value(&spec).map_err(err)?
    {
        return Err("INVALID_REFERENCE: persisted history does not match geometry source".into());
    }
    Ok(document)
}

pub fn execute(root: &Path, request: ReferenceRequest, cancel: &AtomicBool) -> Result<Value> {
    let workspace = ManagedWorkspace::new(root).map_err(err)?;
    match request {
        ReferenceRequest::Create {
            output_relative,
            spec,
        } => {
            let spec = match spec {
                Some(value) => serde_json::from_value(value).map_err(err)?,
                None => GeometrySpec::shotgun(),
            };
            save_revision(&workspace, &ReferenceDocument::new(spec), &output_relative)
        }
        ReferenceRequest::Inspect { source_relative } => {
            let spec = load_spec(&workspace, &source_relative)?;
            Ok(json!({"spec":spec,"validation":spec.validate()}))
        }
        ReferenceRequest::Apply {
            source_relative,
            output_relative,
            command,
        } => {
            let mut document = load_document(&workspace, &source_relative)?;
            let command: ReferenceCommand = serde_json::from_value(command).map_err(err)?;
            let change = document.execute(command).map_err(geometry_error)?;
            let mut result = save_revision(&workspace, &document, &output_relative)?;
            result["change"] = json!(change);
            Ok(result)
        }
        ReferenceRequest::Undo {
            source_relative,
            output_relative,
        } => {
            let mut document = load_document(&workspace, &source_relative)?;
            let change = document.undo().map_err(geometry_error)?;
            let mut result = save_revision(&workspace, &document, &output_relative)?;
            result["change"] = json!(change);
            Ok(result)
        }
        ReferenceRequest::Redo {
            source_relative,
            output_relative,
        } => {
            let mut document = load_document(&workspace, &source_relative)?;
            let change = document.redo().map_err(geometry_error)?;
            let mut result = save_revision(&workspace, &document, &output_relative)?;
            result["change"] = json!(change);
            Ok(result)
        }
        ReferenceRequest::Render {
            source_relative,
            output_relative,
        } => render_bundle(
            &workspace,
            &load_spec(&workspace, &source_relative)?,
            &output_relative,
        ),
        ReferenceRequest::Blender {
            source_relative,
            output_relative,
            category,
            subcategory,
            slug,
            appearance_relative,
            appearance_brief,
            design_reference_relative,
            drawing_reference_relative,
            reviewed_proxy,
            publish,
            expected_previous_sha256,
        } => {
            let spec = load_spec(&workspace, &source_relative)?;
            blender_round_trip(
                &workspace,
                &spec,
                &source_relative,
                &output_relative,
                (&category, &subcategory, &slug),
                publish,
                expected_previous_sha256.as_deref(),
                appearance_relative.as_deref(),
                appearance_brief.as_deref(),
                design_reference_relative.as_deref(),
                drawing_reference_relative.as_deref(),
                reviewed_proxy.as_deref(),
                cancel,
            )
        }
        ReferenceRequest::VerifyBlender {
            source_relative,
            scene_relative,
            output_relative,
            detail_tolerance,
        } => {
            let spec = load_spec(&workspace, &source_relative)?;
            validate_spec(&spec)?;
            let scene = workspace
                .existing(Path::new(&scene_relative))
                .map_err(err)?;
            if scene.extension().is_none_or(|e| e != "blend") {
                return Err("INVALID_REFERENCE: expected .blend scene".into());
            }
            let before = hash(&fs::read(&scene).map_err(err)?);
            let directory = stage_directory(&workspace, &output_relative)?;
            let job = proxy_job(&spec)?;
            write_json(
                &workspace,
                &Path::new(&output_relative).join("job.json"),
                &job,
            )?;
            run_blender(
                &workspace,
                &directory,
                "inspect",
                Some(&scene),
                true,
                cancel,
            )?;
            let actual: Value = serde_json::from_slice(
                &fs::read(directory.join("inspect_report.json")).map_err(err)?,
            )
            .map_err(err)?;
            if before != hash(&fs::read(&scene).map_err(err)?) {
                return Err("SAVE_VALIDATION_FAILED: inspection modified source".into());
            }
            let comparison = compare_round_trip(&workspace, &job, &actual, detail_tolerance)?;
            write_json(
                &workspace,
                &Path::new(&output_relative).join("comparison.json"),
                &comparison,
            )?;
            Ok(comparison)
        }
        ReferenceRequest::Recover => {
            serde_json::to_value(workspace.recover_publications().map_err(err)?).map_err(err)
        }
    }
}

fn run_blender(
    workspace: &ManagedWorkspace,
    directory: &Path,
    mode: &str,
    scene: Option<&Path>,
    renders: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    let executable = std::env::var_os("DD_MAGIC_BLENDER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/Programs/Blender/blender.exe"));
    if !executable.is_file() {
        return Err("BLENDER_OFFLINE: configure DD_MAGIC_BLENDER".into());
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/blender/reference_bridge.py")
        .canonicalize()
        .map_err(err)?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("{mode}.log")))
        .map_err(err)?;
    let mut process = Command::new(executable);
    process.args([
        "--background",
        "--factory-startup",
        "--disable-autoexec",
        "--python-exit-code",
        "1",
    ]);
    if let Some(scene) = scene {
        process.arg(scene);
    }
    process
        .arg("--python")
        .arg(script)
        .arg("--")
        .arg(mode)
        .arg(directory.join("job.json"));
    if renders {
        process.arg("--renders");
    }
    process
        .current_dir(directory)
        .envs(workspace.child_environment().map_err(err)?);
    process.env("PYTHONDONTWRITEBYTECODE", "1");
    process
        .stdin(Stdio::null())
        .stderr(Stdio::from(log.try_clone().map_err(err)?))
        .stdout(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    let mut child = process.spawn().map_err(err)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(err)? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "SAVE_VALIDATION_FAILED: Blender {mode} exited {status}; see {}",
                    directory.display()
                ))
            };
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "CANCELLED_OR_TIMEOUT: staged diagnostics retained; final scene unchanged".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[allow(clippy::too_many_arguments)]
fn blender_round_trip(
    workspace: &ManagedWorkspace,
    spec: &GeometrySpec,
    source_relative: &str,
    output_relative: &str,
    naming: (&str, &str, &str),
    publish: bool,
    previous_hash: Option<&str>,
    appearance_relative: Option<&str>,
    appearance_brief: Option<&str>,
    design_reference_relative: Option<&str>,
    drawing_reference_relative: Option<&str>,
    reviewed_proxy: Option<&ReviewedProxy>,
    cancel: &AtomicBool,
) -> Result<Value> {
    validate_spec(spec)?;
    if drawing_reference_relative.is_some() && design_reference_relative.is_some() {
        return Err(
            "INVALID_REFERENCE: choose either the editable drawing or an external design reference"
                .into(),
        );
    }
    let drawing_reference = drawing_reference_relative
        .map(|relative| drawing::read_reference(workspace, relative))
        .transpose()?;
    let source_path = workspace
        .existing(Path::new(source_relative))
        .map_err(err)?;
    let source_hash = hash(&fs::read(&source_path).map_err(err)?);
    let source_spec: GeometrySpec =
        serde_json::from_slice(&fs::read(&source_path).map_err(err)?).map_err(err)?;
    if serde_json::to_value(source_spec).map_err(err)? != serde_json::to_value(spec).map_err(err)? {
        return Err(
            "SAVE_VALIDATION_FAILED: geometry source changed before the external job".into(),
        );
    }
    let reviewed_scene = reviewed_proxy
        .map(|reviewed| -> Result<Vec<u8>> {
            if !publish || appearance_relative.is_some() {
                return Err("INVALID_REFERENCE: a reviewed proxy requires publication and uses its bundled drawing".into());
            }
            let (strokes, _) = drawing_reference.as_ref().ok_or(
                "INVALID_REFERENCE: a reviewed proxy requires drawing_reference_relative",
            )?;
            if reviewed.geometry_sha256 != source_hash || reviewed.drawing_sha256 != hash(strokes) {
                return Err("SAVE_VALIDATION_FAILED: geometry or drawing differs from the reviewed revision".into());
            }
            let relative = Path::new(&reviewed.scene_relative);
            if !relative.starts_with(".assetlab/staging")
                || relative.extension().is_none_or(|extension| extension != "blend")
            {
                return Err("INVALID_REFERENCE: reviewed scene must be a .blend under .assetlab/staging".into());
            }
            let path = workspace.existing(relative).map_err(err)?;
            let metadata = path.metadata().map_err(err)?;
            if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
                return Err("INVALID_REFERENCE: reviewed proxy must be a file no larger than 64 MiB".into());
            }
            let bytes = fs::read(path).map_err(err)?;
            if reviewed.scene_sha256 != hash(&bytes) {
                return Err("SAVE_VALIDATION_FAILED: scene differs from the reviewed revision".into());
            }
            Ok(bytes)
        })
        .transpose()?;
    let baseline = if publish {
        Some(
            workspace
                .publication_baseline(naming.0, naming.1, naming.2)
                .map_err(err)?,
        )
    } else {
        None
    };
    if publish {
        check_previous_scene(workspace, spec, naming, previous_hash)?;
    }
    let design_reference = design_reference_relative
        .map(|relative| {
            let path = checked_design_reference(workspace, relative)?;
            let bytes = fs::read(&path).map_err(err)?;
            Ok::<_, String>((path, hash(&bytes)))
        })
        .transpose()?;
    let supplied_appearance = appearance_relative
        .map(|relative| {
            let (path, image, digest, bytes) = read_appearance(workspace, relative)?;
            Ok::<_, String>((path, image, digest, bytes))
        })
        .transpose()?;
    let directory = stage_directory(workspace, output_relative)?;
    workspace.ensure_free_space(64 * 1024 * 1024).map_err(err)?;
    let job = proxy_job(spec)?;
    write_json(
        workspace,
        &Path::new(output_relative).join("job.json"),
        &job,
    )?;
    if let Some(bytes) = reviewed_scene {
        workspace
            .write_new(&Path::new(output_relative).join("scene.blend"), &bytes)
            .map_err(err)?;
    } else {
        run_blender(workspace, &directory, "build", None, false, cancel)?;
    }
    let scene = directory.join("scene.blend");
    let verified_scene_hash = hash(&fs::read(&scene).map_err(err)?);
    run_blender(workspace, &directory, "inspect", Some(&scene), true, cancel)?;
    let actual: Value =
        serde_json::from_slice(&fs::read(directory.join("inspect_report.json")).map_err(err)?)
            .map_err(err)?;
    let comparison = compare_round_trip(workspace, &job, &actual, None)?;
    if verified_scene_hash != hash(&fs::read(&scene).map_err(err)?) {
        return Err("SAVE_VALIDATION_FAILED: staged scene changed during inspection".into());
    }
    if source_hash != hash(&fs::read(&source_path).map_err(err)?) {
        return Err(
            "SAVE_VALIDATION_FAILED: geometry source changed during the external job".into(),
        );
    }
    if let Some((path, expected_hash)) = &design_reference
        && *expected_hash != hash(&fs::read(path).map_err(err)?)
    {
        return Err(
            "SAVE_VALIDATION_FAILED: design reference changed during the external job".into(),
        );
    }
    if let Some((path, _, expected_hash, _)) = &supplied_appearance
        && *expected_hash != hash(&fs::read(path).map_err(err)?)
    {
        return Err(
            "SAVE_VALIDATION_FAILED: appearance input changed during the external job".into(),
        );
    }
    write_json(
        workspace,
        &Path::new(output_relative).join("comparison.json"),
        &comparison,
    )?;
    let mut result = json!({"staged_scene":format!("{output_relative}/scene.blend"),"comparison":comparison,"published":false});
    if publish || supplied_appearance.is_some() || drawing_reference.is_some() {
        if cancel.load(Ordering::Relaxed) {
            return Err("CANCELLED_OR_TIMEOUT: publication cancelled".into());
        }
        let staged_reference = format!("{output_relative}/reference");
        let document = load_document(workspace, source_relative)?;
        if document.spec != *spec {
            return Err(
                "SAVE_VALIDATION_FAILED: source history changed during Blender verification".into(),
            );
        }
        export_document(workspace, &document, &staged_reference)?;
        if let Some((strokes, png)) = &drawing_reference {
            let bundle = Path::new(&staged_reference);
            workspace
                .write_new(&bundle.join("design_strokes.json"), strokes)
                .map_err(err)?;
            workspace
                .write_new(&bundle.join("design_drawing.png"), png)
                .map_err(err)?;
            write_json(
                workspace,
                &bundle.join("design_drawing_provenance.json"),
                &json!({
                    "schema_version": 1,
                    "source_relative": drawing_reference_relative,
                    "source_sha256": hash(strokes),
                    "preview_sha256": hash(png),
                    "geometry_sha256": source_hash
                }),
            )?;
            result["design_drawing"] = json!(format!("{staged_reference}/design_drawing.png"));
        }
        if let Some((_, appearance, appearance_hash, appearance_bytes)) = &supplied_appearance {
            let appearance_source = AppearanceSource {
                image: appearance,
                sha256: appearance_hash,
                relative: appearance_relative
                    .expect("appearance path accompanies decoded appearance"),
                original_bytes: appearance_bytes,
                brief: appearance_brief.unwrap_or(
                    "Artist-supplied appearance candidate; verify materials and semantic details manually.",
                ),
                design_reference_relative,
                design_reference_sha256: design_reference
                    .as_ref()
                    .map(|(_, digest)| digest.as_str()),
            };
            let report = stage_appearance_reference(
                workspace,
                spec,
                appearance_source,
                Path::new(&staged_reference),
            )?;
            result["graphic_reference"] = json!(format!("{staged_reference}/graphic.png"));
            result["graphic_report"] = report;
        }
        if !publish {
            return Ok(result);
        }
        let revision_key = if let Some((strokes, _)) = &drawing_reference {
            format!(
                "r{}_{}_d{}",
                spec.revision,
                &source_hash[..12],
                &hash(strokes)[..12]
            )
        } else {
            format!("r{}_{}", spec.revision, &source_hash[..12])
        };
        let relative = workspace
            .promote_reference_revision(Path::new(&staged_reference), naming.2, &revision_key)
            .map_err(err)?;
        let geometry = relative
            .join("geometry.json")
            .to_string_lossy()
            .replace('\\', "/");
        let bundled_design = drawing_reference.as_ref().map(|_| {
            relative
                .join("design_drawing.png")
                .to_string_lossy()
                .replace('\\', "/")
        });
        if let Some(path) = &bundled_design {
            result["design_drawing"] = json!(path);
            result["design_strokes"] = json!(
                relative
                    .join("design_strokes.json")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
        }
        let mut manifest = asset_manifest(
            workspace,
            spec,
            &geometry,
            naming,
            bundled_design.as_deref().or(design_reference_relative),
        )?;
        let mut sources = manifest["source_artifacts"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        sources.retain(|source| source["artifact_id"] != "managed_proxy_scene");
        sources.push(
            json!({"artifact_id":"managed_proxy_scene","role":"proxy_model",
            "path":format!("blender/models/{}/{}/{}/{}.blend",naming.0,naming.1,naming.2,naming.2),
            "sha256":verified_scene_hash}),
        );
        if let Some((strokes, _)) = &drawing_reference {
            sources.retain(|source| source["artifact_id"] != "design_strokes");
            sources.push(json!({
                "artifact_id":"design_strokes",
                "role":"design_source",
                "path":result["design_strokes"],
                "sha256":hash(strokes)
            }));
        }
        if let Some((_, _, appearance_hash, _)) = &supplied_appearance {
            sources.retain(|source| source["artifact_id"] != "supplied_appearance_input");
            sources.push(json!({
                "artifact_id":"supplied_appearance_input",
                "role":"appearance_source",
                "path":format!("{}/appearance_input.bin", relative.to_string_lossy().replace('\\', "/")),
                "sha256":appearance_hash
            }));
        }
        manifest["source_artifacts"] = json!(sources);
        let manifest_relative = Path::new(output_relative).join("asset.toml");
        let manifest_text = toml::to_string_pretty(&manifest).map_err(err)?;
        workspace
            .write_new(&manifest_relative, manifest_text.as_bytes())
            .map_err(err)?;
        let published = workspace
            .publish_scene_checked(
                naming.0,
                naming.1,
                naming.2,
                &Path::new(output_relative).join("scene.blend"),
                &manifest_relative,
                baseline
                    .as_ref()
                    .expect("publication baseline captured before Blender"),
            )
            .map_err(err)?;
        result["publication"] = serde_json::to_value(published).map_err(err)?;
        result["geometry_path"] = json!(geometry);
        result["published"] = Value::Bool(true);
        result
            .as_object_mut()
            .expect("publication result")
            .remove("staged_scene");
    }
    Ok(result)
}

fn check_previous_scene(
    workspace: &ManagedWorkspace,
    spec: &GeometrySpec,
    naming: (&str, &str, &str),
    bootstrap: Option<&str>,
) -> Result<()> {
    let relative = PathBuf::from(format!(
        "blender/models/{}/{}/{}/{}.blend",
        naming.0, naming.1, naming.2, naming.2
    ));
    let manifest_relative = PathBuf::from(format!("assets/{}/asset.toml", naming.2));
    let mut pinned = None;
    if workspace
        .root()
        .join(&manifest_relative)
        .try_exists()
        .map_err(err)?
    {
        let path = workspace.existing(&manifest_relative).map_err(err)?;
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(path).map_err(err)?).map_err(err)?;
        if manifest.get("asset_uid").and_then(toml::Value::as_str) != Some(&spec.asset_uid) {
            return Err("INVALID_REFERENCE: existing asset UID mismatch".into());
        }
        if manifest
            .get("protected_components")
            .and_then(toml::Value::as_array)
            .is_some_and(|v| !v.is_empty())
        {
            return Err("INVALID_REFERENCE: protected geometry cannot be replaced".into());
        }
        pinned = manifest
            .get("source_artifacts")
            .and_then(toml::Value::as_array)
            .and_then(|sources| {
                sources.iter().find(|s| {
                    s.get("artifact_id").and_then(toml::Value::as_str)
                        == Some("managed_proxy_scene")
                })
            })
            .and_then(|s| s.get("sha256").and_then(toml::Value::as_str))
            .map(str::to_owned);
    }
    if workspace.root().join(&relative).try_exists().map_err(err)? {
        let actual = hash(&fs::read(workspace.existing(&relative).map_err(err)?).map_err(err)?);
        let expected=pinned.as_deref().or(bootstrap).ok_or("SAVE_VALIDATION_FAILED: existing scene is unpinned; provide an explicit expected_previous_sha256 after inspection")?;
        if expected.trim_start_matches("sha256:") != actual {
            return Err("SAVE_VALIDATION_FAILED: existing scene changed since the recorded baseline; preserve artist edits and inspect drift".into());
        }
    } else if pinned.is_some() {
        return Err("SAVE_VALIDATION_FAILED: pinned final scene is missing; recover its previous state before publication".into());
    }
    Ok(())
}

fn proxy_job(spec: &GeometrySpec) -> Result<Value> {
    let proxy = spec.proxy().map_err(geometry_error)?;
    if proxy
        .components
        .iter()
        .any(|component| component.triangles.is_empty())
    {
        return Err("UNSUPPORTED_OPERATION: Blender proxy build requires solid mesh components; curves remain available in technical references".into());
    }
    let components: Vec<Value> = proxy
        .components
        .iter()
        .map(|c| {
            json!({"id":c.component_id,"vertices":c.vertices,"faces":c.faces,"triangles":c.triangles,
            "color":if c.material == "wood" { [0.27,0.12,0.05,1.0] } else { [0.19,0.25,0.29,1.0] }})
        })
        .collect();
    let mut cameras = serde_json::Map::new();
    for &view in spec.technical_sheet_layout.views() {
        let projection = spec.project(view).map_err(geometry_error)?;
        cameras.insert(view.key().into(), json!(projection.matrix));
    }
    Ok(
        json!({"schema_version":1,"asset_uid":spec.asset_uid,"revision":spec.revision,
        "units":spec.units,"components":components,"cameras":cameras}),
    )
}

fn compare_round_trip(
    workspace: &ManagedWorkspace,
    expected: &Value,
    actual: &Value,
    detail_tolerance: Option<f64>,
) -> Result<Value> {
    let mut comparison = match detail_tolerance {
        Some(tolerance) => compare_detailed_envelope(expected, actual, tolerance)?,
        None => compare_actual(expected, actual)?,
    };
    let renders = actual["renders"]
        .as_array()
        .ok_or("SAVE_VALIDATION_FAILED: missing render evidence")?;
    let cameras = expected["cameras"]
        .as_object()
        .ok_or("SAVE_VALIDATION_FAILED: missing expected cameras")?;
    if renders.len() != cameras.len() {
        return Err("SAVE_VALIDATION_FAILED: incorrect number of render views".into());
    }
    for view in cameras.keys() {
        let matching: Vec<_> = renders
            .iter()
            .filter(|render| render["view"].as_str() == Some(view.as_str()))
            .collect();
        if matching.len() != 1
            || matching[0]["silhouette"]["status"] != "PASS"
            || matching[0]["silhouette"]["mismatch_beyond_tolerance"].as_u64() != Some(0)
        {
            return Err(format!(
                "SAVE_VALIDATION_FAILED: missing or failed silhouette for {view}"
            ));
        }
    }
    let dependencies = actual["dependencies"]
        .as_array()
        .ok_or("SAVE_VALIDATION_FAILED: missing dependency evidence")?;
    for dependency in dependencies {
        let absolute = Path::new(
            dependency
                .as_str()
                .ok_or("SAVE_VALIDATION_FAILED: invalid dependency path")?,
        );
        let canonical = absolute.canonicalize().map_err(err)?;
        let relative = canonical
            .strip_prefix(workspace.root())
            .map_err(|_| "SAVE_VALIDATION_FAILED: dependency outside managed workspace")?;
        workspace.existing(relative).map_err(err)?;
    }
    comparison["renders"] = json!(renders);
    comparison["blender_version"] = actual["blender_version"].clone();
    comparison["revision"] = actual["revision"].clone();
    Ok(comparison)
}

fn component_bounds(component: &Value) -> Result<[[f64; 3]; 2]> {
    let vertices = component["vertices"]
        .as_array()
        .filter(|vertices| !vertices.is_empty())
        .ok_or("SAVE_VALIDATION_FAILED: component has no evaluated vertices")?;
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for vertex in vertices {
        for axis in 0..3 {
            let coordinate = vertex[axis]
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or("SAVE_VALIDATION_FAILED: invalid evaluated coordinate")?;
            bounds[0][axis] = bounds[0][axis].min(coordinate);
            bounds[1][axis] = bounds[1][axis].max(coordinate);
        }
    }
    Ok(bounds)
}

fn compare_detailed_envelope(expected: &Value, actual: &Value, tolerance: f64) -> Result<Value> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(
            "INVALID_REFERENCE: detail_tolerance must be finite and positive in source units"
                .into(),
        );
    }
    for key in ["asset_uid", "revision", "units"] {
        if expected[key].is_null() || expected[key] != actual[key] {
            return Err(format!("SAVE_VALIDATION_FAILED: {key} mismatch"));
        }
    }
    let wanted = expected["components"]
        .as_array()
        .ok_or("INVALID_GEOMETRY: components")?;
    let found = actual["components"]
        .as_array()
        .ok_or("SAVE_VALIDATION_FAILED: no components")?;
    let mut outer = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut maximum_delta = 0.0_f64;
    for component in wanted {
        let mut matches = found
            .iter()
            .filter(|candidate| candidate["id"] == component["id"]);
        let observed = matches
            .next()
            .filter(|_| matches.next().is_none())
            .ok_or_else(|| {
                format!(
                    "SAVE_VALIDATION_FAILED: required component {} missing or duplicate",
                    component["id"]
                )
            })?;
        let expected_bounds = component_bounds(component)?;
        let observed_bounds = component_bounds(observed)?;
        for axis in 0..3 {
            outer[0][axis] = outer[0][axis].min(expected_bounds[0][axis]);
            outer[1][axis] = outer[1][axis].max(expected_bounds[1][axis]);
            for boundary in 0..2 {
                let delta =
                    (expected_bounds[boundary][axis] - observed_bounds[boundary][axis]).abs();
                maximum_delta = maximum_delta.max(delta);
                if delta > tolerance {
                    return Err(format!(
                        "SAVE_VALIDATION_FAILED: {} boundary {boundary} axis {axis} delta {delta} > {tolerance}",
                        component["id"]
                    ));
                }
            }
        }
    }
    for component in found {
        let bounds = component_bounds(component)?;
        for axis in 0..3 {
            if bounds[0][axis] < outer[0][axis] - tolerance
                || bounds[1][axis] > outer[1][axis] + tolerance
            {
                return Err(format!(
                    "SAVE_VALIDATION_FAILED: {} exceeds the overall envelope on axis {axis}",
                    component["id"]
                ));
            }
        }
    }
    Ok(json!({
        "gate": "blender_detailed_envelope", "status": "PASS_WITH_WARNINGS",
        "scope": "Required component bounds and four external silhouettes. Internal topology and visual quality require separate review.",
        "components": wanted.len(), "additional_components": found.len() - wanted.len(),
        "maximum_bound_delta": maximum_delta, "bound_tolerance": tolerance
    }))
}

/// Independent reopened evaluated mesh comparison. Bounds alone cannot detect swapped or
/// deformed vertices: compare coordinates and topology as well as identity and revision.
pub fn compare_actual(expected: &Value, actual: &Value) -> Result<Value> {
    for key in ["asset_uid", "revision", "units"] {
        if expected[key] != actual[key] {
            return Err(format!("SAVE_VALIDATION_FAILED: {key} mismatch"));
        }
    }
    let wanted = expected["components"]
        .as_array()
        .ok_or("INVALID_GEOMETRY: components")?;
    let found = actual["components"]
        .as_array()
        .ok_or("SAVE_VALIDATION_FAILED: no components")?;
    if wanted.len() != found.len() {
        return Err("SAVE_VALIDATION_FAILED: missing/additional proxy components".into());
    }
    let mut maximum_error = 0.0_f64;
    for component in wanted {
        let matching: Vec<_> = found
            .iter()
            .filter(|v| v["id"] == component["id"])
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "SAVE_VALIDATION_FAILED: component {} missing/duplicate",
                component["id"]
            ));
        }
        let observed = matching[0];
        let vertices = component["vertices"]
            .as_array()
            .ok_or("INVALID_GEOMETRY: vertices")?;
        let actual_vertices = observed["vertices"]
            .as_array()
            .ok_or("SAVE_VALIDATION_FAILED: vertices")?;
        let expected_faces = component["faces"].as_array();
        let topology_mismatch = match expected_faces {
            Some(faces) => {
                component["faces"] != observed["faces"]
                    || (faces
                        .iter()
                        .all(|face| face.as_array().is_some_and(|v| v.len() == 3))
                        && component["triangles"] != observed["triangles"])
            }
            None => component["triangles"] != observed["triangles"],
        };
        if vertices.len() != actual_vertices.len() || topology_mismatch {
            return Err(format!(
                "SAVE_VALIDATION_FAILED: topology mismatch for {}",
                component["id"]
            ));
        }
        let extent = vertices
            .iter()
            .flat_map(|v| v.as_array().into_iter().flatten())
            .filter_map(Value::as_f64)
            .fold(1.0_f64, |a, b| a.max(b.abs()));
        let tolerance = extent * 2.0e-6; // Blender float32 storage; component-relative scale.
        for (a, b) in vertices.iter().zip(actual_vertices) {
            for axis in 0..3 {
                let distance = (a[axis].as_f64().ok_or("INVALID_GEOMETRY")?
                    - b[axis].as_f64().ok_or("SAVE_VALIDATION_FAILED")?)
                .abs();
                if !distance.is_finite() || distance > tolerance {
                    return Err(format!(
                        "SAVE_VALIDATION_FAILED: {} vertex axis {axis} delta {distance} > {tolerance}",
                        component["id"]
                    ));
                }
                maximum_error = maximum_error.max(distance);
            }
        }
    }
    Ok(
        json!({"gate":"blender_round_trip","status":"PASS","components":wanted.len(),"maximum_vertex_error":maximum_error,
        "scope":"Exact proxy mesh/component/revision verification; artistic acceptance and detailed-mesh correspondence are separate."}),
    )
}

fn record_versioned_reference(
    references: &mut Vec<Value>,
    role: &str,
    path: &Path,
    bytes: &[u8],
) -> String {
    let normalized_path = path.to_string_lossy().replace('\\', "/");
    let digest = hash(bytes);
    if let Some(existing) = references.iter().find(|reference| {
        reference["role"] == role
            && reference["path"] == normalized_path
            && reference["sha256"] == digest
    }) {
        return existing["reference_id"]
            .as_str()
            .unwrap_or("reference")
            .to_owned();
    }
    let version = references
        .iter()
        .filter(|reference| reference["role"] == role)
        .filter_map(|reference| reference["reference_version"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let id = format!("{role}_r{version}");
    references.push(json!({
        "reference_id":id,
        "role":role,
        "path":normalized_path,
        "reference_version":version,
        "sha256":digest
    }));
    id
}

fn asset_manifest(
    workspace: &ManagedWorkspace,
    spec: &GeometrySpec,
    geometry: &str,
    naming: (&str, &str, &str),
    design_reference_relative: Option<&str>,
) -> Result<Value> {
    let (category, subcategory, slug) = naming;
    let geometry_path = workspace.existing(Path::new(geometry)).map_err(err)?;
    let old_path = workspace.root().join(format!("assets/{slug}/asset.toml"));
    let mut manifest = if old_path.exists() {
        let checked = workspace
            .existing(Path::new(&format!("assets/{slug}/asset.toml")))
            .map_err(err)?;
        let value: toml::Value =
            toml::from_str(&fs::read_to_string(checked).map_err(err)?).map_err(err)?;
        serde_json::to_value(value).map_err(err)?
    } else {
        json!({"schema_version":2,"asset_id":slug,"asset_uid":spec.asset_uid,"asset_version":spec.revision,
            "category":category,"protected_components":[],"references":[],"source_artifacts":[]})
    };
    if manifest["asset_uid"] != spec.asset_uid {
        return Err("INVALID_REFERENCE: existing asset UID mismatch".into());
    }
    if manifest["protected_components"]
        .as_array()
        .is_some_and(|v| !v.is_empty())
    {
        return Err("INVALID_REFERENCE: protected components prevent proxy publication".into());
    }
    manifest["slug"] = json!(slug);
    manifest["category"] = json!(category);
    manifest["subcategory"] = json!(subcategory);
    manifest["blend_path"] = json!(format!(
        "blender/models/{category}/{subcategory}/{slug}/{slug}.blend"
    ));
    manifest["display_name"] = json!(spec.name);
    manifest["asset_version"] = json!(spec.revision);
    let mut history = manifest["technical"]["revision_history"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !history.iter().any(|entry| entry.as_str() == Some(geometry)) {
        history.push(json!(geometry));
    }
    let mut references = manifest["references"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut active_references = manifest["active_references"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(parent) = Path::new(geometry).parent() {
        let silhouette_filename = if workspace
            .root()
            .join(parent)
            .join("side_silhouette.png")
            .is_file()
        {
            "side_silhouette.png"
        } else {
            "technical.svg"
        };
        let silhouette_relative = parent.join(silhouette_filename);
        if workspace.root().join(&silhouette_relative).is_file() {
            let actual = workspace.existing(&silhouette_relative).map_err(err)?;
            let role = "silhouette_reference";
            let id = record_versioned_reference(
                &mut references,
                role,
                &silhouette_relative,
                &fs::read(actual).map_err(err)?,
            );
            active_references.insert(role.to_owned(), json!(id));
        }

        let graphic_relative = parent.join("graphic.png");
        if workspace.root().join(&graphic_relative).is_file() {
            let actual = workspace.existing(&graphic_relative).map_err(err)?;
            let role = "graphic_reference";
            let id = record_versioned_reference(
                &mut references,
                role,
                &graphic_relative,
                &fs::read(actual).map_err(err)?,
            );
            active_references.insert(role.to_owned(), json!(id));
        } else {
            active_references.remove("graphic_reference");
        }
    }
    if let Some(design_relative) = design_reference_relative {
        let relative = Path::new(design_relative);
        let actual = checked_design_reference(workspace, design_relative)?;
        let stored_path = relative.to_string_lossy().replace('\\', "/");
        let design_hash = hash(&fs::read(actual).map_err(err)?);
        let existing = references.iter().find(|reference| {
            reference["role"] == "design_reference"
                && reference["path"] == stored_path
                && reference["sha256"] == design_hash
        });
        let reference_id = if let Some(existing) = existing {
            existing["reference_id"]
                .as_str()
                .unwrap_or("design_reference")
                .to_owned()
        } else {
            let version = references
                .iter()
                .filter(|reference| reference["role"] == "design_reference")
                .filter_map(|reference| reference["reference_version"].as_u64())
                .max()
                .unwrap_or(0)
                + 1;
            let id = format!("design_reference_v{version}");
            references.push(json!({"reference_id":id,"role":"design_reference",
                "path":stored_path,"reference_version":version,"sha256":design_hash}));
            id
        };
        active_references.insert("design_reference".to_owned(), json!(reference_id));
    }
    manifest["references"] = json!(references);
    manifest["active_references"] = json!(active_references);
    manifest["technical"] = json!({"schema_version":1,"geometry_path":geometry,"geometry_sha256":hash(&fs::read(geometry_path).map_err(err)?),
        "revision":spec.revision,"subcategory":subcategory,"validation_status":"proxy_verified","revision_history":history});
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reference_export_writes_a_deterministic_single_color_silhouette_sheet() {
        let root = std::env::temp_dir().join(format!(
            "dd-silhouette-export-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".assetlab/staging")).unwrap();
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let document = ReferenceDocument::new(GeometrySpec::shotgun());
        let result =
            export_document(&workspace, &document, ".assetlab/staging/silhouette_export").unwrap();
        assert_eq!(
            result["silhouette_source"],
            "GeometrySpec projection mask; no image generation"
        );
        assert_eq!(result["silhouette_resolution"], json!([1440, 480]));
        for view in ["side", "top", "front", "three_quarter"] {
            assert!(
                root.join(format!(
                    ".assetlab/staging/silhouette_export/{view}_silhouette.png"
                ))
                .is_file()
            );
        }
        let silhouette =
            image::open(root.join(".assetlab/staging/silhouette_export/side_silhouette.png"))
                .unwrap()
                .into_rgba8();
        assert_eq!(silhouette.dimensions(), (1440, 480));
        let unique = silhouette
            .pixels()
            .map(|pixel| pixel.0)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique,
            [SILHOUETTE_INK, SILHOUETTE_PAPER].into_iter().collect()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn independent_geometry_change_is_detected() {
        let expected = json!({"asset_uid":"a","revision":1,"units":"relative","components":[{"id":"barrel","vertices":[[0,0,0],[10,0,0],[0,1,0]],"triangles":[[0,1,2]]}]});
        assert!(compare_actual(&expected, &expected).is_ok());
        let mut changed = expected.clone();
        changed["components"][0]["vertices"][1][0] = json!(9);
        assert!(
            compare_actual(&expected, &changed)
                .unwrap_err()
                .contains("barrel")
        );
        changed = expected.clone();
        changed["revision"] = json!(2);
        assert!(
            compare_actual(&expected, &changed)
                .unwrap_err()
                .contains("revision")
        );
    }

    #[test]
    fn unsupported_or_underdetermined_geometry_cannot_be_exported() {
        let root = std::env::temp_dir().join(format!(
            "dd-validation-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let workspace = ManagedWorkspace::new(&root).unwrap();

        let mut unsupported = GeometrySpec::shotgun();
        unsupported
            .constraints
            .push(dd_core::geometry::GeometryConstraint::Unsupported {
                id: "future-relation".into(),
                description: "not implemented by the bounded validator".into(),
            });
        assert!(
            validate_spec(&unsupported)
                .unwrap_err()
                .starts_with("UNSUPPORTED_OPERATION:")
        );
        assert!(
            export_document(
                &workspace,
                &ReferenceDocument::new(unsupported),
                ".assetlab/staging/unsupported-reference"
            )
            .unwrap_err()
            .starts_with("UNSUPPORTED_OPERATION:")
        );
        assert!(
            !root
                .join(".assetlab/staging/unsupported-reference")
                .exists()
        );

        let mut draft = GeometrySpec::shotgun();
        draft.root.children.clear();
        draft.dimensions.clear();
        draft.driving_dimensions.clear();
        draft.constraints.clear();
        assert!(
            validate_spec(&draft)
                .unwrap_err()
                .starts_with("UNDERDETERMINED:")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_pins_optional_design_reference_and_keeps_legacy_request_shape() {
        let root = std::env::temp_dir().join(format!(
            "dd-graphic-manifest-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("assets/long_shotgun/references/r1")).unwrap();
        fs::create_dir_all(root.join("requests")).unwrap();
        let geometry = GeometrySpec::shotgun();
        let geometry_path = "assets/long_shotgun/references/r1/geometry.json";
        let geometry_bytes = serde_json::to_vec(&geometry).unwrap();
        fs::write(root.join(geometry_path), geometry_bytes).unwrap();
        let silhouette_path = "assets/long_shotgun/references/r1/side_silhouette.png";
        fs::write(root.join(silhouette_path), b"coded silhouette").unwrap();
        let design_relative = "requests/long_shotgun_design_brief.md";
        let design_bytes = b"Fictional stylized prop design brief.";
        fs::write(root.join(design_relative), design_bytes).unwrap();
        let workspace = ManagedWorkspace::new(&root).unwrap();

        let manifest = asset_manifest(
            &workspace,
            &geometry,
            geometry_path,
            ("props", "weapons", "long_shotgun"),
            Some(design_relative),
        )
        .unwrap();
        let reference = manifest["references"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["role"] == "design_reference")
            .unwrap();
        assert_eq!(reference["path"], design_relative);
        assert_eq!(reference["sha256"], hash(design_bytes));
        let silhouette_reference = manifest["references"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["role"] == "silhouette_reference")
            .unwrap();
        assert_eq!(silhouette_reference["path"], silhouette_path);
        let manifest_path = root.join("assets/long_shotgun/asset.toml");
        let mut previous_manifest = manifest.clone();
        previous_manifest["references"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "reference_id":"graphic_reference_r1",
                "role":"graphic_reference",
                "path":"assets/long_shotgun/references/r1/graphic.png",
                "reference_version":1,
                "sha256":"old-graphic"
            }));
        previous_manifest["active_references"]["graphic_reference"] = json!("graphic_reference_r1");
        fs::write(
            &manifest_path,
            toml::to_string_pretty(&previous_manifest).unwrap(),
        )
        .unwrap();
        let next_design_relative = "requests/long_shotgun_design_brief_v2.md";
        let next_design_bytes = b"Pixel-art image candidate generated from this design brief.";
        fs::write(root.join(next_design_relative), next_design_bytes).unwrap();
        let next_manifest = asset_manifest(
            &workspace,
            &geometry,
            geometry_path,
            ("props", "weapons", "long_shotgun"),
            Some(next_design_relative),
        )
        .unwrap();
        let design_references = next_manifest["references"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["role"] == "design_reference")
            .collect::<Vec<_>>();
        assert_eq!(design_references.len(), 2);
        assert_eq!(design_references[0]["reference_version"], 1);
        assert_eq!(design_references[1]["reference_version"], 2);
        assert_eq!(
            next_manifest["active_references"]["design_reference"],
            "design_reference_v2"
        );
        assert!(
            next_manifest["active_references"]
                .get("graphic_reference")
                .is_none()
        );
        assert!(
            next_manifest["references"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["reference_id"] == "graphic_reference_r1")
        );
        fs::write(
            &manifest_path,
            toml::to_string_pretty(&next_manifest).unwrap(),
        )
        .unwrap();
        let same_manifest = asset_manifest(
            &workspace,
            &geometry,
            geometry_path,
            ("props", "weapons", "long_shotgun"),
            Some(next_design_relative),
        )
        .unwrap();
        assert_eq!(
            same_manifest["references"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["role"] == "design_reference")
                .count(),
            2
        );

        let legacy_request = serde_json::from_value::<ReferenceRequest>(json!({
            "operation":"blender","source_relative":geometry_path,
            "output_relative":".assetlab/staging/legacy_request",
            "category":"props","subcategory":"weapons","slug":"long_shotgun"
        }))
        .unwrap();
        assert!(matches!(
            legacy_request,
            ReferenceRequest::Blender {
                design_reference_relative: None,
                ..
            }
        ));

        assert!(
            asset_manifest(
                &workspace,
                &geometry,
                geometry_path,
                ("props", "weapons", "long_shotgun"),
                Some("../outside.md"),
            )
            .unwrap_err()
            .contains("INVALID_REFERENCE")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn silhouette_manifest_versions_advance_without_replacing_prior_references() {
        let root = std::env::temp_dir().join(format!(
            "dd-silhouette-version-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bundle = "assets/long_shotgun/references/technical/r2_new";
        fs::create_dir_all(root.join(bundle)).unwrap();
        let geometry_path = format!("{bundle}/geometry.json");
        let mut geometry = GeometrySpec::shotgun();
        geometry.revision = 2;
        fs::write(
            root.join(&geometry_path),
            serde_json::to_vec(&geometry).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(bundle).join("side_silhouette.png"),
            b"new silhouette",
        )
        .unwrap();

        let old_silhouette_path = "assets/long_shotgun/references/technical/r2/technical.svg";
        let old_graphic_path = "assets/long_shotgun/references/technical/r2/graphic.png";
        let manifest = json!({
            "schema_version":2,
            "asset_id":"long_shotgun",
            "asset_uid":geometry.asset_uid,
            "asset_version":2,
            "category":"props",
            "subcategory":"weapons",
            "slug":"long_shotgun",
            "protected_components":[],
            "source_artifacts":[],
            "references":[
                {"reference_id":"silhouette_reference_r1","role":"silhouette_reference","path":"assets/long_shotgun/references/technical/r1/technical.svg","reference_version":1,"sha256":"old-r1"},
                {"reference_id":"silhouette_reference_r2","role":"silhouette_reference","path":old_silhouette_path,"reference_version":2,"sha256":"old-r2"},
                {"reference_id":"graphic_reference_r2","role":"graphic_reference","path":old_graphic_path,"reference_version":2,"sha256":"old-graphic"}
            ],
            "active_references":{
                "silhouette_reference":"silhouette_reference_r2",
                "graphic_reference":"graphic_reference_r2"
            },
            "technical":{"revision_history":[]}
        });
        fs::create_dir_all(root.join("assets/long_shotgun")).unwrap();
        fs::write(
            root.join("assets/long_shotgun/asset.toml"),
            toml::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let updated = asset_manifest(
            &workspace,
            &geometry,
            &geometry_path,
            ("props", "weapons", "long_shotgun"),
            None,
        )
        .unwrap();
        let references = updated["references"].as_array().unwrap();
        assert!(
            references
                .iter()
                .any(|reference| reference["reference_id"] == "silhouette_reference_r2")
        );
        assert!(
            references
                .iter()
                .any(|reference| reference["reference_id"] == "silhouette_reference_r3")
        );
        assert_eq!(
            updated["active_references"]["silhouette_reference"],
            "silhouette_reference_r3"
        );
        assert!(
            updated["active_references"]
                .get("graphic_reference")
                .is_none()
        );
        assert!(
            references
                .iter()
                .any(|reference| reference["reference_id"] == "graphic_reference_r2")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn supplied_appearance_is_persisted_and_clipped_to_the_technical_contour() {
        let root = std::env::temp_dir().join(format!(
            "dd-graphic-stage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let relative = Path::new(".assetlab/staging/graphic-bundle");
        fs::create_dir_all(root.join(relative)).unwrap();
        let workspace = ManagedWorkspace::new(&root).unwrap();
        let spec = GeometrySpec::shotgun();
        let original_spec = spec.clone();
        let appearance =
            RgbaImage::from_fn(64, 24, |x, y| image::Rgba([x as u8, y as u8, 127, 255]));
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(appearance.clone())
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();
        let source_bytes = source.into_inner();
        let source_hash = hash(&source_bytes);
        let report = stage_appearance_reference(
            &workspace,
            &spec,
            AppearanceSource {
                image: &appearance,
                sha256: &source_hash,
                relative: "fixture/appearance.png",
                original_bytes: &source_bytes,
                brief: "Pixel-art test candidate",
                design_reference_relative: Some("requests/design.md"),
                design_reference_sha256: Some("design-hash"),
            },
            relative,
        )
        .unwrap();
        let graphic: RgbaImage = image::open(root.join(relative).join("graphic.png"))
            .unwrap()
            .into_rgba8();
        let projection = spec.project(View::Side).unwrap();
        let expected = rasterize_projection(&projection, 480, 160).unwrap();
        assert!(
            graphic
                .pixels()
                .zip(expected.contour.pixels())
                .all(|(actual, contour)| (actual[3] > 0) == (contour[3] > 0))
        );
        assert_eq!(
            fs::read(root.join(relative).join("appearance_input.bin")).unwrap(),
            source_bytes
        );
        assert_eq!(report["status"], "PASS_WITH_WARNINGS");
        assert_eq!(report["semantic_appearance"], "NOT_VERIFIABLE");
        assert_eq!(report["design_reference_sha256"], "design-hash");
        assert_eq!(spec, original_spec);
        fs::remove_dir_all(root).unwrap();
    }
}
