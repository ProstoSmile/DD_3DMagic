use dd_core::geometry::{
    DrivingDimension, GeometryNode, GeometrySpec, ReferenceCommand, ReferenceDocument, View,
};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub struct ReferenceEditor {
    document: Option<ReferenceDocument>,
    view: usize,
    preview: Option<egui::TextureHandle>,
    drawing_preview: Option<egui::TextureHandle>,
    show_drawing: bool,
    source: String,
    output: String,
    appearance_source: String,
    appearance_brief: String,
    design_reference: String,
    status: String,
    receiver: Option<mpsc::Receiver<Result<serde_json::Value, String>>>,
    cancel: Arc<AtomicBool>,
    selected_component: String,
    driving_dimension_values: Vec<DrivingDimension>,
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

impl Default for ReferenceEditor {
    fn default() -> Self {
        Self {
            document: None,
            view: 0,
            preview: None,
            drawing_preview: None,
            show_drawing: false,
            source: String::new(),
            output: String::new(),
            appearance_source: String::new(),
            appearance_brief: String::new(),
            design_reference: String::new(),
            status: "Open a reference or create the demonstration asset.".into(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            selected_component: String::new(),
            driving_dimension_values: Vec::new(),
        }
    }
}
impl ReferenceEditor {
    fn load_bundled_drawing(&mut self, root: &Path, ctx: &egui::Context) {
        self.show_drawing = false;
        self.drawing_preview = None;
        let Some(parent) = Path::new(&self.source).parent() else {
            return;
        };
        let relative = parent.join("design_strokes.json");
        if !root.join(&relative).exists() {
            return;
        }
        let result = dd_bridge::drawing::execute(
            root,
            dd_bridge::drawing::DrawingRequest::Capture {
                source_relative: relative.to_string_lossy().into_owned(),
            },
        )
        .and_then(|frame| {
            image::load_from_memory_with_format(&frame.png, image::ImageFormat::Png)
                .map(|image| image.into_rgba8())
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(image) => {
                self.drawing_preview = Some(ctx.load_texture(
                    "design drawing",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::NEAREST,
                ));
            }
            Err(error) => self.status = format!("Geometry opened. Drawing unavailable: {error}"),
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        if let Some(doc) = &self.document {
            let view = [View::Side, View::Top, View::Front, View::ThreeQuarter][self.view];
            let rendered = doc
                .spec
                .project(view)
                .map_err(|error| error.to_string())
                .and_then(|projection| {
                    dd_bridge::graphic::render_silhouette_reference(
                        &projection,
                        720,
                        240,
                        [226, 232, 213, 255],
                        [0, 0, 0, 0],
                    )
                });
            match rendered {
                Ok(image) => {
                    self.preview = Some(ctx.load_texture(
                        "reference",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            image.as_raw(),
                        ),
                        egui::TextureOptions::NEAREST,
                    ));
                }
                Err(error) => {
                    self.preview = None;
                    self.status = error.to_string();
                }
            }
        }
    }
    fn next_output(&mut self) {
        self.output = format!(
            ".assetlab/staging/reference_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, root: &Path, asset: Option<&dd_core::AssetManifest>) {
        let ctx = ui.ctx().clone();
        if let Some(receiver) = &self.receiver {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            match receiver.try_recv() {
                Ok(result) => {
                    self.status = match result {
                        Ok(value) => value.to_string(),
                        Err(error) => error,
                    };
                    self.receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Blender job ended before returning a report.".into();
                    self.receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        ui.heading("Technical reference");
        ui.label("Coded silhouette · Side X/Z · Top X/Y · Front Y/Z · artistic relative units");
        let busy = self.receiver.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("New long shotgun").clicked() {
                    let document = ReferenceDocument::new(GeometrySpec::shotgun());
                    self.driving_dimension_values = document.spec.driving_dimensions.clone();
                    self.document = Some(document);
                    self.source.clear();
                    self.show_drawing = false;
                    self.drawing_preview = None;
                    self.next_output();
                    self.refresh(&ctx);
                    self.status =
                        "Fictional design candidate with a walnut stock and a long steel barrel."
                            .into();
                }
                ui.label("Reference path");
                ui.text_edit_singleline(&mut self.source);
                let selected = asset.and_then(|value| value.technical.as_ref());
                let open_selected = ui
                    .add_enabled(selected.is_some(), egui::Button::new("Open selected asset"))
                    .clicked();
                if open_selected && let Some(technical) = selected {
                    self.source = technical.geometry_path.clone();
                }
                if open_selected || ui.button("Open").clicked() {
                    let result = dd_core::storage::ManagedWorkspace::new(root)
                        .map_err(|e| e.to_string())
                        .and_then(|workspace| dd_bridge::load_document(&workspace, &self.source));
                    match result {
                        Ok(document) => {
                            self.status = format!(
                                "Opened {} · revision {}",
                                document.spec.name, document.spec.revision
                            );
                            self.driving_dimension_values =
                                document.spec.driving_dimensions.clone();
                            self.document = Some(document);
                            self.selected_component.clear();
                            self.next_output();
                            self.refresh(&ctx);
                            self.load_bundled_drawing(root, &ctx);
                        }
                        Err(error) => self.status = error,
                    }
                }
            });
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                for (index, name) in ["Side", "Top", "Front", "3/4"].iter().enumerate() {
                    if ui
                        .selectable_label(!self.show_drawing && self.view == index, *name)
                        .clicked()
                    {
                        changed |= self.view != index;
                        self.view = index;
                        self.show_drawing = false;
                    }
                }
                if self.drawing_preview.is_some()
                    && ui.selectable_label(self.show_drawing, "Drawing").clicked()
                {
                    self.show_drawing = true;
                }
                if ui.button("Validate").clicked()
                    && let Some(doc) = &self.document
                {
                    let geometry = dd_bridge::validate_spec(&doc.spec);
                    let projections = doc.spec.technical_sheet_layout.views()
                        .iter().copied()
                        .map(|view| doc.spec.project(view).map_err(|error| error.to_string()))
                        .collect::<Result<Vec<_>, _>>();
                    self.status = match (geometry, projections) {
                        (Ok(geometry), Ok(views)) => {
                            let projection = dd_core::reference_validation::validate_projections(
                                &doc.spec, &views,
                            );
                            serde_json::to_string_pretty(
                                &serde_json::json!({"geometry":geometry,"projection":projection}),
                            )
                            .unwrap_or_else(|error| error.to_string())
                        }
                        (Err(error), _) | (_, Err(error)) => error,
                    };
                }
            });
            if let Some(asset) = asset
                && let (Some(technical), Some(scene)) = (&asset.technical, &asset.blend_path)
            {
                ui.horizontal_wrapped(|ui| {
                    ui.label(scene);
                    if ui.button("Verify saved scene").clicked() {
                        self.next_output();
                        self.start_job(
                            root.to_path_buf(),
                            dd_bridge::ReferenceRequest::VerifyBlender {
                                source_relative: technical.geometry_path.clone(),
                                scene_relative: scene.clone(),
                                output_relative: self.output.clone(),
                                detail_tolerance: None,
                            },
                        );
                        self.next_output();
                    }
                });
            }
            if changed {
                self.refresh(&ctx);
            }
            self.component_controls(ui, &ctx);
            ui.collapsing("Optional visual references", |ui| {
                ui.label("Project-relative paths. Imported appearance is clipped to the technical side silhouette and still needs artist review.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("Appearance raster");
                    ui.text_edit_singleline(&mut self.appearance_source);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Appearance notes");
                    ui.text_edit_singleline(&mut self.appearance_brief);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Design brief");
                    ui.text_edit_singleline(&mut self.design_reference);
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("New output folder");
                ui.text_edit_singleline(&mut self.output);
                if ui.button("Export reference").clicked()
                    && let Some(doc) = &self.document
                {
                    let result = dd_core::storage::ManagedWorkspace::new(root)
                        .map_err(|e| e.to_string())
                        .and_then(|workspace| {
                            dd_bridge::export_document(&workspace, doc, &self.output)
                        });
                    match result {
                        Ok(value) => {
                            self.source = format!("{}/geometry.json", self.output);
                            self.status = value.to_string();
                            self.next_output();
                        }
                        Err(e) => self.status = e,
                    }
                }
                if ui.button("Stage Blender proxy").clicked() && !self.source.is_empty() {
                    let request = dd_bridge::ReferenceRequest::Blender {
                        source_relative: self.source.clone(),
                        output_relative: self.output.clone(),
                        category: "props".into(),
                        subcategory: "weapons".into(),
                        slug: "long_shotgun".into(),
                        appearance_relative: optional_text(&self.appearance_source),
                        appearance_brief: optional_text(&self.appearance_brief),
                        design_reference_relative: optional_text(&self.design_reference),
                        drawing_reference_relative: None,
                        reviewed_proxy: None,
                        publish: false,
                        expected_previous_sha256: None,
                    };
                    self.start_job(root.to_path_buf(), request);
                    self.next_output();
                }
            });
        });
        if busy && ui.button("Cancel Blender job").clicked() {
            self.cancel.store(true, Ordering::Relaxed);
        }
        ui.separator();
        let preview = if self.show_drawing {
            self.drawing_preview.as_ref()
        } else {
            self.preview.as_ref()
        };
        if let Some(preview) = preview {
            let scale = (ui.available_width() / preview.size_vec2().x).min(2.0);
            ui.image((preview.id(), preview.size_vec2() * scale));
        }
        let mut geometry_edited = false;
        let mut geometry_status = None;
        let mut driving_dimension_values = self.driving_dimension_values.clone();
        if let Some(doc) = self.document.as_mut() {
            ui.collapsing("Geometry and dimensions",|ui| {
                ui.label(format!("{} · revision {}",doc.spec.asset_uid,doc.spec.revision));
                ui.label("Dimensions are recomputed from source geometry. Pixel illustration needs artist review.");
                if let Ok(dimensions)=doc.spec.measure_dimensions() {
                    for dimension in dimensions {
                        if let Some(value)=dimension.value {ui.label(format!("{}: {value:.3} {}",dimension.label,dimension.unit_label));}
                        else {ui.label(format!("{}: unsupported",dimension.label));}
                    }
                }
                if !doc.spec.driving_dimensions.is_empty() {
                    ui.separator();
                    ui.label("Driving dimensions · edit values, then apply as one history step.");
                    for dimension in &mut driving_dimension_values {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} · {}",dimension.label,dimension.assignment.node_id));
                            ui.add(egui::DragValue::new(&mut dimension.assignment.value).speed(0.05));
                            ui.label(&dimension.unit_label);
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply driving dimensions").clicked() {
                            match doc.execute(ReferenceCommand::ReplaceDrivingDimensions {dimensions:driving_dimension_values.clone()}) {
                                Ok(_)=>{geometry_edited=true;geometry_status=Some("Driving dimensions applied; all projections updated.".to_owned());}
                                Err(error)=>geometry_status=Some(error.to_string()),
                            }
                        }
                        if ui.button("Solve saved dimensions").clicked() {
                            match doc.execute(ReferenceCommand::Solve) {
                                Ok(_)=>{geometry_edited=true;geometry_status=Some("Saved driving dimensions solved.".to_owned());}
                                Err(error)=>geometry_status=Some(error.to_string()),
                            }
                        }
                    });
                }
            });
        }
        self.driving_dimension_values = if geometry_edited {
            self.document
                .as_ref()
                .map(|document| document.spec.driving_dimensions.clone())
                .unwrap_or_default()
        } else {
            driving_dimension_values
        };
        if geometry_edited {
            self.source.clear();
            self.refresh(&ctx);
        }
        if let Some(status) = geometry_status {
            self.status = status;
        }
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                ui.label(&self.status);
            });
    }
    fn component_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        fn collect(node: &GeometryNode, nodes: &mut Vec<GeometryNode>) {
            if node.primitive.is_some() {
                nodes.push(node.clone());
            }
            for child in &node.children {
                collect(child, nodes);
            }
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let mut nodes = Vec::new();
        collect(&document.spec.root, &mut nodes);
        if self.selected_component.is_empty()
            && let Some(node) = nodes.first()
        {
            self.selected_component = node.id.clone();
        }
        let mut edited = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("reference_component").selected_text(&self.selected_component).show_ui(ui,|ui| {
                for node in &nodes {ui.selectable_value(&mut self.selected_component,node.id.clone(),&node.label);}
            });
            if let Some(node)=nodes.iter().find(|node| node.id==self.selected_component) {
                let mut transform=node.transform;
                ui.label("Position");
                let mut changed=false;
                changed |= ui.add(egui::DragValue::new(&mut transform.translation.x).speed(0.01).prefix("X ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut transform.translation.y).speed(0.01).prefix("Y ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut transform.translation.z).speed(0.01).prefix("Z ")).changed();
                ui.label("Scale");
                changed |= ui.add(egui::DragValue::new(&mut transform.scale.x).speed(0.01).range(0.01..=100.0).prefix("X ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut transform.scale.y).speed(0.01).range(0.01..=100.0).prefix("Y ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut transform.scale.z).speed(0.01).range(0.01..=100.0).prefix("Z ")).changed();
                if changed {
                    match document.execute(ReferenceCommand::SetNodeTransform {node_id:node.id.clone(),transform}) {
                        Ok(_)=>{edited=true;self.status="Component moved; all views updated. Export a new revision before building a proxy.".into();},
                        Err(error)=>self.status=error.to_string(),
                    }
                }
            }
            if ui.add_enabled(document.can_undo(),egui::Button::new("Undo geometry")).clicked() {
                match document.undo(){Ok(_)=>edited=true,Err(error)=>self.status=error.to_string()}
            }
            if ui.add_enabled(document.can_redo(),egui::Button::new("Redo geometry")).clicked() {
                match document.redo(){Ok(_)=>edited=true,Err(error)=>self.status=error.to_string()}
            }
        });
        if edited {
            if let Some(document) = &self.document {
                self.driving_dimension_values = document.spec.driving_dimensions.clone();
            }
            self.source.clear();
            self.refresh(ctx);
        }
    }
    fn start_job(&mut self, root: PathBuf, request: dd_bridge::ReferenceRequest) {
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.status = "Blender proxy running; output remains staged.".into();
        std::thread::spawn(move || {
            let _ = sender.send(dd_bridge::execute(&root, request, &cancel));
        });
    }
}
impl Drop for ReferenceEditor {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
