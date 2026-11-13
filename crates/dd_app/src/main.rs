//! DD_3DMagic native shell. Real v0.1 features: PNG/JPEG loading, non-destructive
//! raster operation stack, nearest-neighbor preview, project profile inspection,
//! new-file PNG export. Blender viewport/UV paint/docking are explicitly future work.

use dd_core::ProjectContext;
use dd_core::context::{
    AssetDescriptor, ProjectDescriptor, discover_assets, discover_projects, ensure_staging_dir,
    read_project, resolve_context, safe_existing,
};
use dd_core::naming::{check_name_collision, format_material_name, format_texture_name};
use dd_core::raster::{RasterDocument, RasterOp, open_image, write_new_png};
use eframe::egui::{self, Color32, RichText, TextureHandle};
use std::path::{Path, PathBuf};
mod reference_editor;

const BG: Color32 = Color32::from_rgb(10, 18, 18);
const PANEL: Color32 = Color32::from_rgb(16, 29, 28);
const INK: Color32 = Color32::from_rgb(198, 222, 182);
const DIM: Color32 = Color32::from_rgb(115, 153, 132);
const ACCENT: Color32 = Color32::from_rgb(126, 208, 147);
const AMBER: Color32 = Color32::from_rgb(227, 184, 101);

struct Editor {
    reference_editor: reference_editor::ReferenceEditor,
    reference_mode: bool,
    workspace_root: PathBuf,
    root: PathBuf,
    projects: Vec<ProjectDescriptor>,
    selected_project_id: Option<String>,
    assets: Vec<AssetDescriptor>,
    selected_asset_id: Option<String>,
    selected_surface: Option<String>,
    context: Option<ProjectContext>,
    profile_error: Option<String>,
    document: Option<RasterDocument>,
    document_project_id: Option<String>,
    document_asset_id: Option<String>,
    preview: Option<TextureHandle>,
    file_name: String,
    open_path: String,
    zoom: f32,
    resolution: u32,
    logs: Vec<String>,
    export_serial: u32,
}

impl Editor {
    fn new(ctx: &egui::Context) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let explicit_project = std::env::var_os("DD_MAGIC_PROJECT_ROOT").map(PathBuf::from);
        let workspace_root = explicit_project
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .or_else(|| {
                std::env::var_os("DD_MAGIC_WORKSPACE_ROOT")
                    .map(PathBuf::from)
                    .and_then(|path| path.canonicalize().ok())
            })
            .unwrap_or_else(|| {
                if discover_projects(&cwd).is_ok() {
                    return cwd;
                }
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| {
                        exe.ancestors()
                            .skip(1)
                            .find(|path| path.join("demo_project/.assetlab/project.toml").is_file())
                            .map(Path::to_path_buf)
                    })
                    .unwrap_or(cwd)
            });
        let (projects, discovery_error) = match discover_projects(&workspace_root) {
            Ok(projects) => (projects, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        let requested_project = std::env::var("DD_MAGIC_PROJECT_ID").ok();
        let selected_project_id = if let Some(path) = explicit_project {
            read_project(&path).ok().map(|settings| settings.project_id)
        } else if let Some(requested) = requested_project.as_ref() {
            projects
                .iter()
                .find(|project| &project.project_id == requested)
                .map(|project| project.project_id.clone())
        } else if projects.len() == 1 {
            projects.first().map(|project| project.project_id.clone())
        } else {
            None
        };
        let mut app = Self {
            reference_editor: Default::default(),
            reference_mode: false,
            workspace_root: workspace_root.clone(),
            root: workspace_root,
            projects,
            selected_project_id: None,
            assets: Vec::new(),
            selected_asset_id: None,
            selected_surface: None,
            context: None,
            profile_error: None,
            document: None,
            document_project_id: None,
            document_asset_id: None,
            preview: None,
            file_name: "NO DOCUMENT".into(),
            open_path: String::new(),
            zoom: 3.0,
            resolution: 128,
            logs: Vec::new(),
            export_serial: 1,
        };
        app.log("Editor initialized. Drag a PNG/JPEG or use Open path.");
        if let Some(project_id) = selected_project_id {
            app.activate_project(ctx, &project_id);
        } else if app.projects.is_empty() {
            app.profile_error = Some(discovery_error.unwrap_or_else(|| {
                format!(
                    "No configured project found in workspace {}. Choose a workspace containing a project folder with .assetlab/project.toml.",
                    app.workspace_root.display()
                )
            }));
        } else if let Some(requested) = requested_project {
            app.profile_error = Some(format!(
                "Project `{requested}` was not found in workspace {}.",
                app.workspace_root.display()
            ));
        } else {
            app.profile_error =
                Some("Select a project to resolve its style and asset rules.".into());
        }
        app
    }

    fn has_unexported_operations(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| !document.operations.is_empty())
    }

    fn activate_project(&mut self, ctx: &egui::Context, project_id: &str) {
        if self.selected_project_id.as_deref() == Some(project_id) {
            return;
        }
        if self.has_unexported_operations()
            && self.selected_project_id.as_deref() != Some(project_id)
        {
            self.log("Export or reset the current operation stack before changing projects.");
            return;
        }
        let Some(project) = self
            .projects
            .iter()
            .find(|candidate| candidate.project_id == project_id)
            .cloned()
        else {
            self.profile_error = Some(format!("Selected project `{project_id}` is unavailable."));
            return;
        };
        self.root = project.path;
        self.selected_project_id = Some(project.project_id);
        self.document = None;
        self.preview = None;
        self.document_project_id = None;
        self.document_asset_id = None;
        self.file_name = "NO DOCUMENT".into();
        self.assets = match discover_assets(&self.root) {
            Ok(assets) => assets,
            Err(error) => {
                self.assets.clear();
                self.profile_error = Some(error.to_string());
                return;
            }
        };
        let requested_asset = std::env::var("DD_MAGIC_ASSET_ID").ok();
        self.selected_asset_id = requested_asset
            .filter(|id| self.assets.iter().any(|asset| &asset.asset_id == id))
            .or_else(|| (self.assets.len() == 1).then(|| self.assets[0].asset_id.clone()));
        self.selected_surface = None;
        self.refresh_context(ctx, true);
    }

    fn activate_asset(&mut self, ctx: &egui::Context, asset_id: Option<String>) {
        if self.has_unexported_operations() && self.selected_asset_id != asset_id {
            self.log("Export or reset the current operation stack before changing assets.");
            return;
        }
        self.selected_asset_id = asset_id;
        self.selected_surface = None;
        if !self.has_unexported_operations() {
            self.document = None;
            self.preview = None;
            self.document_project_id = None;
            self.document_asset_id = None;
            self.file_name = "NO DOCUMENT".into();
        }
        self.refresh_context(ctx, true);
    }

    fn refresh_context(&mut self, ctx: &egui::Context, load_source_if_empty: bool) {
        if self.selected_project_id.is_none() {
            self.context = None;
            self.profile_error = Some("Select a project to continue.".into());
            return;
        }
        match resolve_context(&self.root, self.selected_asset_id.as_deref(), None) {
            Ok(context) => {
                self.resolution = context.texture.width;
                self.selected_surface = context
                    .asset
                    .as_ref()
                    .and_then(|asset| asset.materials.first())
                    .map(|material| material.material_id.clone());
                self.profile_error = None;
                self.context = Some(context);
                if load_source_if_empty && self.document.is_none() {
                    self.load_selected_source(ctx);
                }
            }
            Err(error) => {
                self.context = None;
                self.profile_error = Some(error.to_string());
            }
        }
    }

    fn load_selected_source(&mut self, ctx: &egui::Context) {
        let relative = self.context.as_ref().and_then(|context| {
            let asset = context.asset.as_ref()?;
            asset
                .source_artifacts
                .iter()
                .find(|source| source.role == "source_texture")
                .map(|source| source.path.as_str())
                .or_else(|| {
                    asset
                        .references
                        .iter()
                        .find(|reference| reference.role == "texture_motif")
                        .map(|reference| reference.path.as_str())
                })
        });
        if let Some(relative) = relative {
            match safe_existing(&self.root, Path::new(relative)) {
                Ok(source) => self.load_image(ctx, &source),
                Err(error) => self.log(format!("SOURCE ERROR: {error}")),
            }
        }
    }
    fn log(&mut self, msg: impl Into<String>) {
        self.logs.push(msg.into());
        if self.logs.len() > 100 {
            self.logs.remove(0);
        }
    }
    fn load_image(&mut self, ctx: &egui::Context, path: &Path) {
        let managed_path = (|| -> Result<PathBuf, String> {
            let actual = path.canonicalize().map_err(|e| e.to_string())?;
            let workspace =
                dd_core::storage::ManagedWorkspace::new(&self.root).map_err(|e| e.to_string())?;
            if actual.starts_with(workspace.root()) {
                return Ok(actual);
            }
            // Decode/size-check before copying an external source. Never change the original.
            open_image(&actual).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(&actual).map_err(|e| e.to_string())?;
            let extension = actual
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            if !["png", "jpg", "jpeg"].contains(&extension.as_str()) {
                return Err("Only PNG/JPEG imports are supported".into());
            }
            workspace
                .directory(Path::new(".assetlab/staging/imports"))
                .map_err(|e| e.to_string())?;
            let relative = PathBuf::from(format!(
                ".assetlab/staging/imports/{}.{}",
                dd_bridge::hash(&bytes),
                extension
            ));
            if workspace.root().join(&relative).exists() {
                let existing = workspace.existing(&relative).map_err(|e| e.to_string())?;
                if std::fs::read(&existing).map_err(|e| e.to_string())? != bytes {
                    return Err("Managed import hash collision or modified source".into());
                }
                return Ok(existing);
            }
            workspace
                .write_new(&relative, &bytes)
                .map_err(|e| e.to_string())
        })();
        let managed_path = match managed_path {
            Ok(path) => path,
            Err(error) => {
                self.log(format!("IMPORT ERROR: {error}"));
                return;
            }
        };
        let path = managed_path.as_path();
        match open_image(path) {
            Ok(img) => {
                self.document = Some(RasterDocument::new(img));
                self.document_project_id = self.selected_project_id.clone();
                self.document_asset_id = self.selected_asset_id.clone();
                self.file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.log(format!("Loaded raster: {}", path.display()));
                self.update_preview(ctx);
            }
            Err(e) => self.log(format!("LOAD ERROR: {e}")),
        }
    }
    fn update_preview(&mut self, ctx: &egui::Context) {
        let rendered = self.document.as_ref().map(|d| d.render());
        match rendered {
            Some(Ok(img)) => {
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                self.preview =
                    Some(ctx.load_texture("ddmagic-preview", color, egui::TextureOptions::NEAREST));
            }
            Some(Err(e)) => self.log(format!("PREVIEW ERROR: {e}")),
            None => self.preview = None,
        }
    }
    fn apply(&mut self, ctx: &egui::Context, op: RasterOp) {
        if let Some(d) = self.document.as_mut() {
            match d.apply(op) {
                Ok(_) => {
                    self.log("Operation appended; original raster preserved.");
                    self.update_preview(ctx);
                }
                Err(e) => self.log(format!("OPERATION ERROR: {e}")),
            }
        } else {
            self.log("No source image loaded.");
        }
    }
    fn export_png(&mut self) {
        if self.context.is_none() {
            self.log("Resolve a project profile before exporting into staging.");
            return;
        }
        let Some(document) = &self.document else {
            self.log("Nothing to export.");
            return;
        };
        if self.document_project_id != self.selected_project_id
            || self.document_asset_id != self.selected_asset_id
        {
            self.log("The open image belongs to a different project or asset selection. Reopen it before export.");
            return;
        }
        let rendered = match document.render() {
            Ok(image) => image,
            Err(error) => {
                self.log(format!("EXPORT ERROR: {error}"));
                return;
            }
        };
        if self.selected_asset_id.is_some()
            && self.selected_surface.is_none()
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.asset.is_some())
        {
            self.log("Select a material surface before exporting an asset texture.");
            return;
        }
        if let Err(error) = ensure_staging_dir(&self.root) {
            self.log(format!("EXPORT ERROR: {error}"));
            return;
        }
        let file_name = match (
            &self.context,
            &self.selected_asset_id,
            &self.selected_surface,
        ) {
            (Some(context), Some(asset_id), Some(surface)) => format_texture_name(
                &context.naming,
                asset_id,
                surface,
                "basecolor",
                rendered.width(),
                rendered.height(),
            ),
            _ => Ok(format!("texture_preview_{:04}.png", self.export_serial)),
        };
        let file_name = match file_name {
            Ok(name) => name,
            Err(error) => {
                self.log(format!("EXPORT ERROR: {error}"));
                return;
            }
        };
        let relative = Path::new(".assetlab/staging").join(file_name);
        let file = match check_name_collision(&self.root, &relative) {
            Ok(file) => file,
            Err(error) => {
                self.log(format!("EXPORT ERROR: {error}"));
                return;
            }
        };
        let result = write_new_png(&rendered, &file);
        match result {
            Ok(()) => {
                self.log(format!("Created new PNG: {}", file.display()));
                self.export_serial += 1;
            }
            Err(e) => self.log(format!(
                "EXPORT ERROR: {e} (existing files are not overwritten)"
            )),
        }
    }
    fn heading(ui: &mut egui::Ui, label: &str) {
        ui.add_space(10.0);
        ui.label(RichText::new(label).monospace().strong().color(ACCENT));
        ui.separator();
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped_path = ctx.input(|i| {
            i.raw.dropped_files.iter().find_map(|file| {
                let path = file.path();
                (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
            })
        });
        if let Some(path) = dropped_path {
            self.load_image(&ctx, &path);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Z) && i.modifiers.command)
            && self.document.as_mut().is_some_and(|doc| doc.undo())
        {
            self.log("Undo last operation.");
            self.update_preview(&ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.command) {
            self.export_png();
        }
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("DD_3DMAGIC")
                            .strong()
                            .monospace()
                            .size(20.0)
                            .color(ACCENT),
                    );
                    ui.label(
                        RichText::new("// ASSET WORKSTATION 0.1")
                            .monospace()
                            .color(DIM),
                    );
                    ui.separator();
                    ui.label(RichText::new(&self.file_name).monospace().color(INK));
                    ui.selectable_value(&mut self.reference_mode, false, "Texture");
                    ui.selectable_value(&mut self.reference_mode, true, "Reference");
                    ui.add_space(12.0);
                    if ui.button("EXPORT PNG  [Ctrl+S]").clicked() {
                        self.export_png();
                    }
                });
            });
        egui::Panel::bottom("console")
            .resizable(true)
            .default_size(118.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("EVENT LOG / LOCAL")
                            .strong()
                            .monospace()
                            .color(AMBER),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new("No telemetry  •  No generative service in this build")
                            .small()
                            .color(DIM),
                    );
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for msg in &self.logs {
                            ui.label(
                                RichText::new(format!("> {msg}"))
                                    .monospace()
                                    .small()
                                    .color(DIM),
                            );
                        }
                    });
            });
        let mut requested_project = None;
        let mut requested_asset: Option<Option<String>> = None;
        egui::Panel::left("workbench")
            .resizable(true)
            .default_size(248.0)
            .min_size(210.0)
            .show(ui, |ui| {
                Self::heading(ui, "01 / WORKSPACE");
                ui.label(
                    RichText::new(self.workspace_root.to_string_lossy().to_string())
                        .small()
                        .monospace()
                        .color(DIM),
                );
                let project_label = self
                    .selected_project_id
                    .as_deref()
                    .and_then(|id| {
                        self.projects
                            .iter()
                            .find(|project| project.project_id == id)
                    })
                    .map(|project| format!("{} ({})", project.display_name, project.project_id))
                    .unwrap_or_else(|| "Select project".into());
                egui::ComboBox::from_id_salt("workspace_project")
                    .selected_text(project_label)
                    .show_ui(ui, |ui| {
                        for project in &self.projects {
                            if ui
                                .selectable_label(
                                    self.selected_project_id.as_deref()
                                        == Some(project.project_id.as_str()),
                                    format!("{} ({})", project.display_name, project.project_id),
                                )
                                .clicked()
                            {
                                requested_project = Some(project.project_id.clone());
                            }
                        }
                    });
                let asset_label = self
                    .selected_asset_id
                    .as_deref()
                    .and_then(|id| self.assets.iter().find(|asset| asset.asset_id == id))
                    .map(|asset| format!("{} / {}", asset.asset_id, asset.category))
                    .unwrap_or_else(|| "Project profile (no asset)".into());
                egui::ComboBox::from_id_salt("project_asset")
                    .selected_text(asset_label)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.selected_asset_id.is_none(), "Project profile")
                            .clicked()
                        {
                            requested_asset = Some(None);
                        }
                        for asset in &self.assets {
                            if ui
                                .selectable_label(
                                    self.selected_asset_id.as_deref()
                                        == Some(asset.asset_id.as_str()),
                                    format!("{} / {}", asset.asset_id, asset.category),
                                )
                                .clicked()
                            {
                                requested_asset = Some(Some(asset.asset_id.clone()));
                            }
                        }
                    });
                if let Some(error) = &self.profile_error {
                    ui.colored_label(AMBER, error);
                }
                Self::heading(ui, "02 / SOURCE");
                ui.label("Drop a PNG/JPEG onto the window, or enter a path:");
                ui.text_edit_singleline(&mut self.open_path);
                if ui.button("OPEN IMAGE").clicked() {
                    let entered = self.open_path.trim();
                    let path = PathBuf::from(entered);
                    let path = if path.is_absolute() {
                        path
                    } else {
                        self.root.join(path)
                    };
                    self.load_image(&ctx, &path);
                }
                if let Some(context) = &self.context
                    && let Some(asset) = &context.asset
                    && !asset.materials.is_empty()
                {
                    let surface_label = self
                        .selected_surface
                        .as_deref()
                        .unwrap_or("Select material");
                    egui::ComboBox::from_id_salt("texture_surface")
                        .selected_text(format!("Texture surface: {surface_label}"))
                        .show_ui(ui, |ui| {
                            for material in &asset.materials {
                                if ui
                                    .selectable_label(
                                        self.selected_surface.as_deref()
                                            == Some(material.material_id.as_str()),
                                        format!("{} ({})", material.display_name, material.role),
                                    )
                                    .clicked()
                                {
                                    self.selected_surface = Some(material.material_id.clone());
                                }
                            }
                        });
                }
                Self::heading(ui, "03 / RASTER OPERATIONS");
                ui.label(RichText::new("Actual pixel dimensions").color(DIM));
                ui.horizontal(|ui| {
                    for n in [64_u32, 128, 256] {
                        if ui
                            .selectable_label(self.resolution == n, format!("{n}"))
                            .clicked()
                        {
                            self.resolution = n;
                        }
                    }
                });
                ui.add(egui::Slider::new(&mut self.resolution, 16..=1024).text("px"));
                if ui.button("RESIZE / NEAREST").clicked() {
                    self.apply(
                        &ctx,
                        RasterOp::ResizeNearest {
                            width: self.resolution,
                            height: self.resolution,
                        },
                    );
                }
                if ui.button("APPLY PROJECT PALETTE").clicked() {
                    let palette = self
                        .context
                        .as_ref()
                        .map(|c| c.texture.palette.clone())
                        .unwrap_or_default();
                    if palette.is_empty() {
                        self.log("Project palette unavailable.");
                    } else {
                        self.apply(&ctx, RasterOp::PaletteNearest { colors: palette });
                    }
                }
                if ui.button("GRAYSCALE").clicked() {
                    self.apply(&ctx, RasterOp::Grayscale);
                }
                ui.horizontal(|ui| {
                    if ui.button("BRIGHTER").clicked() {
                        self.apply(&ctx, RasterOp::Brightness { delta: 12 });
                    }
                    if ui.button("DARKER").clicked() {
                        self.apply(&ctx, RasterOp::Brightness { delta: -12 });
                    }
                });
                if ui.button("UNDO LAST [Ctrl+Z]").clicked()
                    && self.document.as_mut().is_some_and(|doc| doc.undo())
                {
                    self.update_preview(&ctx);
                }
                if ui.button("RESET STACK").clicked()
                    && let Some(doc) = &mut self.document
                {
                    doc.operations.clear();
                    self.update_preview(&ctx);
                }
                ui.add_space(12.0);
                ui.label(
                    RichText::new("UV projection and 3D viewport: planned, not simulated.")
                        .small()
                        .color(AMBER),
                );
            });
        if let Some(project_id) = requested_project {
            self.activate_project(&ctx, &project_id);
        }
        if let Some(asset_id) = requested_asset {
            self.activate_asset(&ctx, asset_id);
        }
        egui::Panel::right("layers")
            .resizable(true)
            .default_size(272.0)
            .min_size(220.0)
            .show(ui, |ui| {
                Self::heading(ui, "04 / DOCUMENT STACK");
                if let Some(doc) = &self.document {
                    ui.label(RichText::new("SOURCE / IMMUTABLE").monospace().color(AMBER));
                    ui.label(format!(
                        "{} × {} RGBA",
                        doc.original.width(),
                        doc.original.height()
                    ));
                    ui.add_space(9.0);
                    ui.label(
                        RichText::new("OPERATIONS (BOTTOM → TOP)")
                            .monospace()
                            .color(ACCENT),
                    );
                    if doc.operations.is_empty() {
                        ui.label(RichText::new("No adjustments").color(DIM));
                    }
                    for (idx, op) in doc.operations.iter().enumerate() {
                        ui.label(
                            RichText::new(format!("{:02}  {op:?}", idx + 1))
                                .monospace()
                                .small()
                                .color(INK),
                        );
                    }
                } else {
                    ui.label(RichText::new("Load an image to start").color(DIM));
                }
                Self::heading(ui, "05 / PROJECT PROFILE");
                if let Some(c) = &self.context {
                    ui.label(format!("Project: {} v{}", c.project_id, c.project_version));
                    ui.label(RichText::new(&c.project_uid).small().monospace().color(DIM));
                    ui.label(format!("Style: {}", c.style));
                    ui.label(format!("Units: {} ({} m/unit)", c.units, c.meters_per_unit));
                    if let Some(asset) = &c.asset {
                        ui.label(format!(
                            "Asset: {} v{} / {}",
                            asset.asset_id, asset.asset_version, asset.category
                        ));
                        ui.label(
                            RichText::new(&asset.asset_uid)
                                .small()
                                .monospace()
                                .color(DIM),
                        );
                        ui.label(format!(
                            "Face IDs: {}  |  Materials: {}",
                            asset.face_ids.len(),
                            asset.materials.len()
                        ));
                        for material in &asset.materials {
                            ui.label(format!(
                                "{} | role: {} | slot: {}",
                                material.display_name, material.role, material.slot_name
                            ));
                        }
                        if let Some(surface) = &self.selected_surface {
                            if let Ok(texture_name) = format_texture_name(
                                &c.naming,
                                &asset.asset_id,
                                surface,
                                "basecolor",
                                c.texture.width,
                                c.texture.height,
                            ) {
                                ui.label(format!("Texture name: {texture_name}"));
                            }
                            if let Ok(material_name) =
                                format_material_name(&c.naming, &asset.asset_id, surface)
                            {
                                ui.label(format!("Material name: {material_name}"));
                            }
                        }
                    } else {
                        ui.label("Asset: none");
                    }
                    ui.label(format!(
                        "Target TD: {:.1} px/m",
                        c.texture.target_td_px_per_m
                    ));
                    ui.label(format!("Atlas: {} × {}", c.texture.width, c.texture.height));
                    ui.label(format!("Padding: {} px", c.texture.padding_px));
                    ui.label(format!("Palette entries: {}", c.texture.palette.len()));
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("RESOLUTION SOURCES")
                            .monospace()
                            .color(ACCENT),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for (property, source) in &c.provenance {
                                ui.label(
                                    RichText::new(format!("{property}  ←  {source}"))
                                        .small()
                                        .monospace()
                                        .color(DIM),
                                );
                            }
                        });
                    for artifact in &c.artifact_integrity {
                        ui.label(format!(
                            "{} / {}: {:?}",
                            artifact.kind, artifact.artifact_id, artifact.state
                        ));
                    }
                }
                Self::heading(ui, "06 / STATUS");
                ui.colored_label(ACCENT, "Raster editor: operational design");
                ui.colored_label(ACCENT, "Blender: bounded staged proxy bridge");
                ui.colored_label(AMBER, "AI generation: not connected");
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                if self.reference_mode {
                    self.reference_editor.ui(
                        ui,
                        &self.root,
                        self.context
                            .as_ref()
                            .and_then(|context| context.asset.as_ref()),
                    );
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("CANVAS // RGBA / NEAREST")
                            .monospace()
                            .color(ACCENT),
                    );
                    ui.separator();
                    ui.add(
                        egui::Slider::new(&mut self.zoom, 0.25..=12.0)
                            .logarithmic(true)
                            .text("Zoom"),
                    );
                    if ui.button("1:1").clicked() {
                        self.zoom = 1.0;
                    }
                    if ui.button("4:1").clicked() {
                        self.zoom = 4.0;
                    }
                });
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    if let Some(texture) = &self.preview {
                        let size = texture.size_vec2() * self.zoom;
                        let (rect, _) = ui.allocate_exact_size(
                            size + egui::vec2(40.0, 40.0),
                            egui::Sense::hover(),
                        );
                        let canvas =
                            egui::Rect::from_min_size(rect.min + egui::vec2(20.0, 20.0), size);
                        ui.painter()
                            .rect_filled(canvas, 0.0, Color32::from_rgb(35, 43, 41));
                        // Max 32×32 checker tiles, independent of image resolution.
                        let tile = (size.x.max(size.y) / 32.0).max(12.0);
                        let cols = ((size.x / tile).ceil() as usize).min(32);
                        let rows = ((size.y / tile).ceil() as usize).min(32);
                        for y in 0..rows {
                            for x in 0..cols {
                                if (x + y) % 2 == 0 {
                                    let xy =
                                        canvas.min + egui::vec2(x as f32 * tile, y as f32 * tile);
                                    let cell = egui::Rect::from_min_max(
                                        xy,
                                        (xy + egui::vec2(tile, tile)).min(canvas.max),
                                    );
                                    ui.painter().rect_filled(
                                        cell,
                                        0.0,
                                        Color32::from_rgb(51, 59, 54),
                                    );
                                }
                            }
                        }
                        ui.painter().image(
                            texture.id(),
                            canvas,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        ui.painter().rect_stroke(
                            canvas,
                            0.0,
                            egui::Stroke::new(1.0, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    } else {
                        ui.add_space(120.0);
                        ui.centered_and_justified(|ui| {
                            ui.heading(
                                RichText::new("AWAITING TEXTURE INPUT")
                                    .monospace()
                                    .color(DIM),
                            );
                        });
                    }
                });
            });
    }
}

fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("reference") {
        match reference_cli() {
            Ok(result) => println!("{result}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DD_3DMagic | Asset Workstation")
            .with_inner_size([1360.0, 850.0])
            .with_min_inner_size([1000.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DD_3DMagic",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = PANEL;
            visuals.window_fill = PANEL;
            visuals.override_text_color = Some(INK);
            visuals.widgets.inactive.bg_fill = PANEL;
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 57, 49);
            visuals.widgets.active.bg_fill = Color32::from_rgb(42, 73, 55);
            visuals.selection.bg_fill = Color32::from_rgb(46, 98, 69);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(Editor::new(&cc.egui_ctx)))
        }),
    )
}

fn reference_cli() -> Result<String, String> {
    let root = std::env::var_os("DD_MAGIC_PROJECT_ROOT")
        .or_else(|| std::env::var_os("DD_MAGIC_WORKSPACE_ROOT"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    let workspace = dd_core::storage::ManagedWorkspace::new(&root).map_err(|e| e.to_string())?;
    let argument = std::env::args()
        .nth(2)
        .ok_or("Usage: dd-3dmagic reference <project-relative request.json>")?;
    let request_path = workspace
        .existing(Path::new(&argument))
        .map_err(|e| e.to_string())?;
    if request_path.metadata().map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("Request exceeds 8 MiB".into());
    }
    let request = serde_json::from_slice(&std::fs::read(request_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let result = dd_bridge::execute(&root, request, &std::sync::atomic::AtomicBool::new(false))?;
    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}
