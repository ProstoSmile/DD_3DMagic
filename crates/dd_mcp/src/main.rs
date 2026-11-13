//! Local stdio MCP server exposing only reviewed domain operations.
//! stdout is reserved for MCP JSON-RPC; diagnostics belong on stderr.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use dd_core::context::{
    ContextError, TextureOverrides, discover_assets, discover_projects, ensure_staging_dir,
    read_project, resolve_context, safe_existing, select_project,
};
use dd_core::naming::{check_name_collision, format_material_name, format_texture_name};
use dd_core::raster::{RasterDocument, RasterOp, open_image, write_new_png};
use dd_core::storage::ManagedWorkspace;
use dd_core::uv::{UvExport, analyze};
use rmcp::{
    RoleServer, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock as Content},
    schemars,
    service::RequestContext,
    tool, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
struct AssetLab {
    root: Arc<PathBuf>,
    workspace_mode: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContextArgs {
    project_id: Option<String>,
    asset_id: Option<String>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProjectArgs {
    project_id: Option<String>,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EmptyArgs {}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReferenceArgs {
    project_id: Option<String>,
    request: dd_bridge::ReferenceRequest,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DrawingArgs {
    project_id: Option<String>,
    request: dd_bridge::drawing::DrawingRequest,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UvArgs {
    project_id: Option<String>,
    asset_id: String,
    uv_dump_relative: String,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TextureArgs {
    project_id: Option<String>,
    asset_id: String,
    source_relative: String,
    output_relative: String,
    width: Option<u32>,
    height: Option<u32>,
    use_project_palette: bool,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReferenceFrameArgs {
    project_id: Option<String>,
    stage_relative: String,
    view: ReferenceFrameView,
}
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ReferenceFrameView {
    Side,
    Top,
    Front,
}
impl ReferenceFrameView {
    fn as_str(self) -> &'static str {
        match self {
            Self::Side => "side",
            Self::Top => "top",
            Self::Front => "front",
        }
    }
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NameArgs {
    project_id: Option<String>,
    asset_id: String,
    surface: String,
    channel: String,
    width: Option<u32>,
    height: Option<u32>,
}

fn fail(err: impl std::fmt::Display) -> String {
    serde_json::json!({"ok":false,"error":err.to_string()}).to_string()
}
fn done(value: impl serde::Serialize) -> String {
    serde_json::json!({"ok":true,"result":value}).to_string()
}

const MAX_REFERENCE_FRAME_BYTES: u64 = 8 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn is_staging_directory_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(part)) if part == ".assetlab")
        && matches!(components.next(), Some(Component::Normal(part)) if part == "staging")
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn read_reference_frame(
    root: &Path,
    args: &ReferenceFrameArgs,
) -> Result<(serde_json::Value, Vec<u8>), String> {
    let stage_relative = Path::new(&args.stage_relative);
    if !is_staging_directory_path(stage_relative) {
        return Err("stage_relative must name a directory beneath .assetlab/staging".into());
    }

    let workspace = ManagedWorkspace::new(root).map_err(|error| error.to_string())?;
    let stage = workspace
        .existing(stage_relative)
        .map_err(|error| error.to_string())?;
    let stage_metadata = std::fs::symlink_metadata(&stage).map_err(|error| error.to_string())?;
    if !stage_metadata.is_dir() {
        return Err("stage_relative must name an existing directory".into());
    }

    let view = args.view.as_str();
    let filename = format!("blender_{view}.png");
    let image_relative = stage_relative.join(&filename);
    let image_path = workspace
        .existing(&image_relative)
        .map_err(|error| error.to_string())?;
    let path_metadata =
        std::fs::symlink_metadata(&image_path).map_err(|error| error.to_string())?;
    if !path_metadata.file_type().is_file() {
        return Err(format!("{filename} must be a regular file"));
    }
    if path_metadata.len() > MAX_REFERENCE_FRAME_BYTES {
        return Err(format!("{filename} exceeds the 8 MiB safety limit"));
    }

    let mut file = std::fs::File::open(&image_path).map_err(|error| error.to_string())?;
    let opened_metadata = file.metadata().map_err(|error| error.to_string())?;
    if !opened_metadata.is_file() {
        return Err(format!("{filename} must be a regular file"));
    }
    if opened_metadata.len() > MAX_REFERENCE_FRAME_BYTES {
        return Err(format!("{filename} exceeds the 8 MiB safety limit"));
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(MAX_REFERENCE_FRAME_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_REFERENCE_FRAME_BYTES {
        return Err(format!("{filename} exceeds the 8 MiB safety limit"));
    }
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(format!("{filename} does not have a PNG signature"));
    }

    let metadata = serde_json::json!({
        "view": view,
        "stage_relative": args.stage_relative,
        "filename": filename,
        "mime_type": "image/png",
        "size_bytes": bytes.len(),
    });
    Ok((metadata, bytes))
}

fn reference_response(
    result: Result<Result<serde_json::Value, String>, tokio::task::JoinError>,
) -> String {
    match result {
        Ok(Ok(value)) => done(value),
        Ok(Err(error)) => {
            let code = error
                .split_once(':')
                .map(|(code, _)| code)
                .filter(|code| code.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
                .unwrap_or("INVALID_REFERENCE");
            serde_json::json!({"ok":false,"error":{"code":code,"message":error}}).to_string()
        }
        Err(error) => fail(error),
    }
}

impl AssetLab {
    fn selected_root(&self, project_id: Option<&str>) -> Result<PathBuf, ContextError> {
        if self.workspace_mode {
            let Some(project_id) = project_id else {
                let count = discover_projects(&self.root).map_or(0, |projects| projects.len());
                return Err(ContextError::ProjectSelectionRequired(count));
            };
            return Ok(select_project(&self.root, project_id)?.path);
        }
        let project = read_project(&self.root)?;
        if project_id.is_some_and(|id| id != project.project_id) {
            return Err(ContextError::ProjectNotFound(
                project_id.unwrap_or_default().to_owned(),
            ));
        }
        Ok(self.root.as_ref().clone())
    }
}

#[tool_router(server_handler)]
impl AssetLab {
    #[tool(
        description = "Read one saved Blender modeling checkpoint image from an existing project-relative .assetlab/staging directory. view is side, top, or front; the server reads only the fixed blender_{view}.png filename, validates the PNG signature and 8 MiB limit, and returns metadata plus image/png. No files are written."
    )]
    fn reference_frame(&self, Parameters(args): Parameters<ReferenceFrameArgs>) -> CallToolResult {
        let root = match self.selected_root(args.project_id.as_deref()) {
            Ok(root) => root,
            Err(error) => return CallToolResult::error(vec![Content::text(error.to_string())]),
        };
        match read_reference_frame(&root, &args) {
            Ok((metadata, bytes)) => CallToolResult::success(vec![
                Content::text(done(metadata)),
                Content::image(STANDARD.encode(bytes), "image/png"),
            ]),
            Err(error) => CallToolResult::error(vec![Content::text(error)]),
        }
    }

    #[tool(
        description = "Draw on a white pixel canvas using black strokes. Create a canvas, append ordered pixel paths to a new revision, capture a saved revision, or export PNG. Every successful call returns the current canvas as a PNG image. Coordinates start at the top left. Paths are project-relative, with new files under .assetlab/staging on D:."
    )]
    async fn reference_drawing(&self, Parameters(args): Parameters<DrawingArgs>) -> CallToolResult {
        let root = match self.selected_root(args.project_id.as_deref()) {
            Ok(root) => root,
            Err(error) => return CallToolResult::error(vec![Content::text(error.to_string())]),
        };
        match tokio::task::spawn_blocking(move || dd_bridge::drawing::execute(&root, args.request))
            .await
        {
            Ok(Ok(frame)) => CallToolResult::success(vec![
                Content::text(done(frame.metadata)),
                Content::image(STANDARD.encode(frame.png), "image/png"),
            ]),
            Ok(Err(error)) => CallToolResult::error(vec![Content::text(error)]),
            Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
        }
    }

    #[tool(
        description = "Create, inspect, edit, undo/redo, render or validate a versioned technical reference through a bounded Blender proxy bridge. Select project_id when bound to a workspace. request is a typed operation; paths are project-relative. Outputs must be new .assetlab/staging subdirectories on D:. No arbitrary scripts. Appearance is separate from technical geometry."
    )]
    async fn reference_operation(
        &self,
        Parameters(args): Parameters<ReferenceArgs>,
        context: RequestContext<RoleServer>,
    ) -> String {
        let root = match self.selected_root(args.project_id.as_deref()) {
            Ok(root) => root,
            Err(error) => return fail(error),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_flag = Arc::clone(&cancelled);
        let mut operation = tokio::task::spawn_blocking(move || {
            dd_bridge::execute(&root, args.request, &worker_flag)
        });
        tokio::select! {
            result = &mut operation => reference_response(result),
            _ = context.ct.cancelled() => {
                cancelled.store(true, Ordering::Relaxed);
                reference_response(operation.await)
            }
        }
    }

    #[tool(description = "List configured projects in the bound workspace; read-only.")]
    fn list_projects(&self, Parameters(_args): Parameters<EmptyArgs>) -> String {
        match discover_projects(&self.root) {
            Ok(projects) => done(projects),
            Err(error) => fail(error),
        }
    }

    #[tool(
        description = "List asset manifests for a project selected from the workspace; read-only."
    )]
    fn list_assets(&self, Parameters(args): Parameters<ProjectArgs>) -> String {
        let result = self
            .selected_root(args.project_id.as_deref())
            .and_then(|root| discover_assets(&root));
        match result {
            Ok(assets) => done(assets),
            Err(error) => fail(error),
        }
    }

    #[tool(
        description = "Read resolved project/asset context, including style, texture TD and provenance; no mutation."
    )]
    fn resolve_asset_context(&self, Parameters(args): Parameters<ContextArgs>) -> String {
        let result = self
            .selected_root(args.project_id.as_deref())
            .and_then(|root| resolve_context(&root, args.asset_id.as_deref(), None));
        match result {
            Ok(context) => done(context),
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Format project texture/material names and check whether the proposed texture filename is available in staging; read-only."
    )]
    fn format_asset_names(&self, Parameters(args): Parameters<NameArgs>) -> String {
        let result = (|| -> Result<serde_json::Value, String> {
            let root = self
                .selected_root(args.project_id.as_deref())
                .map_err(|e| e.to_string())?;
            let context =
                resolve_context(&root, Some(&args.asset_id), None).map_err(|e| e.to_string())?;
            let asset = context.asset.as_ref().ok_or("asset context is required")?;
            if !asset
                .materials
                .iter()
                .any(|material| material.material_id == args.surface)
            {
                return Err(format!(
                    "surface `{}` is not a material_id in asset `{}`",
                    args.surface, args.asset_id
                ));
            }
            let width = args.width.unwrap_or(context.texture.width);
            let height = args.height.unwrap_or(context.texture.height);
            let texture_name = format_texture_name(
                &context.naming,
                &args.asset_id,
                &args.surface,
                &args.channel,
                width,
                height,
            )
            .map_err(|e| e.to_string())?;
            let material_name =
                format_material_name(&context.naming, &args.asset_id, &args.surface)
                    .map_err(|e| e.to_string())?;
            let output_relative = Path::new(".assetlab/staging").join(&texture_name);
            let available = if root.join(".assetlab/staging").is_dir() {
                match check_name_collision(&root, &output_relative) {
                    Ok(_) => true,
                    Err(dd_core::context::ContextError::NameCollision(_)) => false,
                    Err(error) => return Err(error.to_string()),
                }
            } else {
                true
            };
            Ok(serde_json::json!({
                "texture_name": texture_name,
                "material_name": material_name,
                "texture_name_available": available,
                "output_directory": ".assetlab/staging"
            }))
        })();
        match result {
            Ok(value) => done(value),
            Err(error) => fail(error),
        }
    }

    #[tool(
        description = "Measure world-space texel density and per-triangle UV distortion from a Blender-exported UV JSON; no mutation."
    )]
    fn analyze_uv(&self, Parameters(args): Parameters<UvArgs>) -> String {
        let result = (|| -> Result<serde_json::Value, String> {
            let root = self
                .selected_root(args.project_id.as_deref())
                .map_err(|e| e.to_string())?;
            let context =
                resolve_context(&root, Some(&args.asset_id), None).map_err(|e| e.to_string())?;
            let path = safe_existing(&root, Path::new(&args.uv_dump_relative))
                .map_err(|e| e.to_string())?;
            if path.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
                return Err("UV file exceeds 128 MiB safety limit".into());
            }
            let data: UvExport =
                serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let report = analyze(
                &data,
                [context.texture.width, context.texture.height],
                context.texture.target_td_px_per_m,
                context.texture.td_tolerance_pct,
            )?;
            serde_json::to_value(report).map_err(|e| e.to_string())
        })();
        match result {
            Ok(v) => done(v),
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Create a NEW PNG in .assetlab/staging/ by nearest-neighbor resampling an existing project image, optionally quantizing to the project palette. Never overwrite existing files."
    )]
    fn stage_pixel_texture(&self, Parameters(args): Parameters<TextureArgs>) -> String {
        let result = (|| -> Result<serde_json::Value, String> {
            let root = self
                .selected_root(args.project_id.as_deref())
                .map_err(|e| e.to_string())?;
            let context =
                resolve_context(&root, Some(&args.asset_id), None).map_err(|e| e.to_string())?;
            let override_ = TextureOverrides {
                width: args.width,
                height: args.height,
                ..TextureOverrides::default()
            };
            let mut dimensions = context.texture.clone();
            override_.apply_to(&mut dimensions);
            dimensions.validate().map_err(|e| e.to_string())?;
            let output_relative = Path::new(&args.output_relative);
            if output_relative.parent() != Some(Path::new(".assetlab/staging"))
                || output_relative.extension().and_then(|ext| ext.to_str()) != Some("png")
            {
                return Err("output must be a new PNG directly inside .assetlab/staging/".into());
            }
            let input = safe_existing(&root, Path::new(&args.source_relative))
                .map_err(|e| e.to_string())?;
            ensure_staging_dir(&root).map_err(|e| e.to_string())?;
            let output = check_name_collision(&root, output_relative).map_err(|e| e.to_string())?;
            let img = open_image(&input).map_err(|e| e.to_string())?;
            let mut doc = RasterDocument::new(img);
            doc.apply(RasterOp::ResizeNearest {
                width: dimensions.width,
                height: dimensions.height,
            })
            .map_err(|e| e.to_string())?;
            if args.use_project_palette {
                if dimensions.palette.is_empty() {
                    return Err("palette requested but profile palette is empty".into());
                }
                doc.apply(RasterOp::PaletteNearest {
                    colors: dimensions.palette.clone(),
                })
                .map_err(|e| e.to_string())?;
            }
            write_new_png(&doc.render().map_err(|e| e.to_string())?, &output)
                .map_err(|e| e.to_string())?;
            Ok(
                serde_json::json!({"new_output":args.output_relative,"dimensions":[dimensions.width,dimensions.height],"source":args.source_relative,"quality_note":"Nearest-resize and RGB palette matching are mechanical transforms, not artistic pixel-art authoring."}),
            )
        })();
        match result {
            Ok(v) => done(v),
            Err(e) => fail(e),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (root, workspace_mode) =
        if let Some(project_root) = std::env::var_os("DD_MAGIC_PROJECT_ROOT") {
            (PathBuf::from(project_root).canonicalize()?, false)
        } else {
            let explicit_workspace = std::env::var_os("DD_MAGIC_WORKSPACE_ROOT");
            let workspace = explicit_workspace
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            if let Some(project_id) = std::env::var_os("DD_MAGIC_PROJECT_ID") {
                (
                    select_project(&workspace, &project_id.to_string_lossy())?.path,
                    false,
                )
            } else if explicit_workspace.is_some() {
                (workspace.canonicalize()?, true)
            } else if workspace.join(".assetlab/project.toml").exists() {
                (workspace.canonicalize()?, false)
            } else {
                match discover_projects(&workspace) {
                    Ok(projects) if projects.len() == 1 => {
                        (projects.into_iter().next().unwrap().path, false)
                    }
                    Ok(_) | Err(ContextError::NoProjects(_)) => (workspace.canonicalize()?, true),
                    Err(error) => return Err(error.into()),
                }
            }
        };
    if !workspace_mode {
        read_project(&root)?;
    }
    let app = AssetLab {
        root: Arc::new(root),
        workspace_mode,
    };
    let server = app.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
