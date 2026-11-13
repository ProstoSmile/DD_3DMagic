//! Optional appearance-provider contract.
//!
//! Geometry and projection are supplied as immutable, authoritative inputs. A provider
//! can return appearance pixels only; callers should use [`render_with_provider`] so the
//! result is dimension-checked and clipped to the technical contour before it is used.
use dd_core::geometry::{GeometrySpec, ProjectedComponent, Projection};
use image::{GrayImage, ImageBuffer, Luma, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

const MAX_REFERENCE_SIDE: u32 = 4096;
const MAX_REFERENCE_PIXELS: u64 = 16_777_216;
const MAX_COMPONENTS: usize = 4096;
const MAX_STRUCTURED_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_APPEARANCE_BRIEF_BYTES: usize = 16 * 1024;

pub type ComponentMaskId = u16;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicReferenceRequest {
    pub schema_version: u32,
    pub asset_uid: String,
    pub geometry_revision: u64,
    /// SHA-256 of compact `serde_json` serialization of the authoritative `GeometrySpec`.
    pub geometry_sha256: String,
    /// SHA-256 of compact `serde_json` serialization of the authoritative `Projection`.
    pub projection_sha256: String,
    /// SHA-256 of the contour image, including dimensions and RGBA pixels.
    pub contour_sha256: String,
    /// SHA-256 of the indexed component-mask image and its legend.
    #[serde(default)]
    pub component_masks_sha256: String,
    /// Optional SHA-256 of the design-reference image, including dimensions and pixels.
    #[serde(default)]
    pub design_reference_sha256: Option<String>,
    pub appearance_brief: String,
}

/// Maps categorical pixels to stable geometry component IDs. Mask ID zero is reserved for
/// background; every nonzero ID must have exactly one legend entry. The indexed image keeps
/// the component masks compact while `mask_for_component` exposes a conventional binary mask.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentMasks {
    pub ids: ImageBuffer<Luma<ComponentMaskId>, Vec<ComponentMaskId>>,
    pub legend: Vec<ComponentMaskLegendEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentMaskLegendEntry {
    pub mask_id: ComponentMaskId,
    pub component_id: String,
    pub label: String,
    pub material: String,
}

impl ComponentMasks {
    pub fn new(
        ids: ImageBuffer<Luma<ComponentMaskId>, Vec<ComponentMaskId>>,
        legend: Vec<ComponentMaskLegendEntry>,
    ) -> Self {
        Self { ids, legend }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        self.ids.dimensions()
    }

    /// Return a 0/255 mask for the requested component ID, if it is present in the legend.
    pub fn mask_for_component(&self, component_id: &str) -> Option<GrayImage> {
        let entry = self
            .legend
            .iter()
            .find(|entry| entry.component_id == component_id)?;
        Some(GrayImage::from_fn(
            self.ids.width(),
            self.ids.height(),
            |x, y| {
                if self.ids.get_pixel(x, y)[0] == entry.mask_id {
                    Luma([255])
                } else {
                    Luma([0])
                }
            },
        ))
    }

    fn validate_for_projection(&self, projection: &Projection) -> Result<(), String> {
        check_dimensions(self.dimensions(), "component masks")?;
        if projection.components.len() > MAX_COMPONENTS || self.legend.len() > MAX_COMPONENTS {
            return Err(invalid("component count exceeds the provider limit"));
        }
        if self.legend.len() != projection.components.len() {
            return Err(invalid(
                "component-mask legend does not cover every projected component",
            ));
        }

        let mut projected = HashMap::with_capacity(projection.components.len());
        for component in &projection.components {
            if projected
                .insert(component.component_id.as_str(), component)
                .is_some()
            {
                return Err(invalid("projection contains duplicate component IDs"));
            }
        }

        let mut mask_ids = HashSet::with_capacity(self.legend.len());
        let mut component_ids = HashSet::with_capacity(self.legend.len());
        for entry in &self.legend {
            if entry.mask_id == 0 || !mask_ids.insert(entry.mask_id) {
                return Err(invalid("component-mask IDs must be unique and nonzero"));
            }
            if entry.component_id.is_empty() || !component_ids.insert(entry.component_id.as_str()) {
                return Err(invalid(
                    "component-mask legend has an empty or duplicate component ID",
                ));
            }
            let component = projected
                .get(entry.component_id.as_str())
                .ok_or_else(|| invalid("component-mask legend names an unknown component"))?;
            if entry.label != component.label || entry.material != component.material {
                return Err(invalid(
                    "component-mask legend differs from projected component metadata",
                ));
            }
        }

        if projected
            .keys()
            .any(|component_id| !component_ids.contains(component_id))
        {
            return Err(invalid("component-mask legend omits a projected component"));
        }
        if self
            .ids
            .pixels()
            .any(|pixel| pixel[0] != 0 && !mask_ids.contains(&pixel[0]))
        {
            return Err(invalid(
                "component-mask image contains an ID missing from its legend",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionRaster {
    pub contour: RgbaImage,
    pub component_masks: ComponentMasks,
}

/// Rasterize projected triangles with a single orthographic fit. The contour is the union of
/// covered pixels; each pixel in the indexed mask belongs to its nearest projected component.
/// Mask IDs are assigned by sorted component ID, while ID zero remains transparent background.
pub fn rasterize_projection(
    projection: &Projection,
    width: u32,
    height: u32,
) -> Result<ProjectionRaster, String> {
    check_dimensions((width, height), "projection raster")?;
    if projection.components.len() > MAX_COMPONENTS {
        return Err(invalid("component count exceeds the provider limit"));
    }
    for axis in 0..2 {
        let min = projection.bounds.min[axis];
        let max = projection.bounds.max[axis];
        if !min.is_finite() || !max.is_finite() || min > max || !(max - min).is_finite() {
            return Err(invalid("projection bounds are invalid"));
        }
    }

    let mut sorted: Vec<&ProjectedComponent> = projection.components.iter().collect();
    sorted.sort_by(|left, right| left.component_id.cmp(&right.component_id));
    let mut legend = Vec::with_capacity(sorted.len());
    let mut mask_ids = HashMap::with_capacity(sorted.len());
    for (index, component) in sorted.iter().enumerate() {
        if component.component_id.is_empty()
            || component.component_id.len() > 256
            || component.label.len() > 1024
            || component.material.len() > 128
        {
            return Err(invalid(
                "projected component metadata exceeds the provider limit",
            ));
        }
        let mask_id = (index + 1) as ComponentMaskId;
        if mask_ids
            .insert(component.component_id.as_str(), mask_id)
            .is_some()
        {
            return Err(invalid("projection contains duplicate component IDs"));
        }
        legend.push(ComponentMaskLegendEntry {
            mask_id,
            component_id: component.component_id.clone(),
            label: component.label.clone(),
            material: component.material.clone(),
        });
    }

    let fit = RasterFit::new(projection, width, height)?;
    let pixel_count = (width as usize) * (height as usize);
    let mut depths = vec![f64::NEG_INFINITY; pixel_count];
    let mut owners = vec![0_u16; pixel_count];
    for component in &projection.components {
        let mask_id = *mask_ids
            .get(component.component_id.as_str())
            .ok_or_else(|| invalid("projection component is missing its mask ID"))?;
        for triangle in &component.triangles {
            for vertex in triangle.vertices {
                if vertex.iter().any(|coordinate| !coordinate.is_finite())
                    || (0..2).any(|axis| {
                        vertex[axis] < projection.bounds.min[axis] - 1.0e-8
                            || vertex[axis] > projection.bounds.max[axis] + 1.0e-8
                    })
                {
                    return Err(invalid(
                        "projected triangle vertex is non-finite or outside bounds",
                    ));
                }
            }
            if triangle.depth.iter().any(|depth| !depth.is_finite()) {
                return Err(invalid("projected triangle has a non-finite depth"));
            }
            let points = triangle.vertices.map(|point| fit.map(point));
            rasterize_triangle_ids(
                width,
                height,
                &mut depths,
                &mut owners,
                points,
                triangle.depth,
                mask_id,
            )?;
        }
    }

    let ids: ImageBuffer<Luma<ComponentMaskId>, Vec<ComponentMaskId>> =
        ImageBuffer::from_raw(width, height, owners)
            .ok_or_else(|| invalid("could not allocate indexed component-mask image"))?;
    let contour = RgbaImage::from_fn(width, height, |x, y| {
        if ids.get_pixel(x, y)[0] == 0 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    Ok(ProjectionRaster {
        contour,
        component_masks: ComponentMasks::new(ids, legend),
    })
}

/// Renders the geometry contour as one flat silhouette. Both colors and the mask are
/// deterministic; no appearance provider participates in this path.
pub fn render_silhouette_reference(
    projection: &Projection,
    width: u32,
    height: u32,
    ink: [u8; 4],
    paper: [u8; 4],
) -> Result<RgbaImage, String> {
    let raster = rasterize_projection(projection, width, height)?;
    Ok(RgbaImage::from_fn(width, height, |x, y| {
        if raster.contour.get_pixel(x, y)[3] == 0 {
            image::Rgba(paper)
        } else {
            image::Rgba(ink)
        }
    }))
}

/// Follows dd_core's `ScreenMap` fit and 8-pixel default padding so the contour and every
/// component mask share identical pixel coordinates and orthographic scale.
#[derive(Clone, Copy)]
struct RasterFit {
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    height: u32,
}

impl RasterFit {
    fn new(projection: &Projection, width: u32, height: u32) -> Result<Self, String> {
        let padding = 8_u32
            .min(width.saturating_sub(1) / 2)
            .min(height.saturating_sub(1) / 2) as f64;
        let available_width = (width as f64 - 2.0 * padding).max(1.0);
        let available_height = (height as f64 - 2.0 * padding).max(1.0);
        let span = [
            projection.bounds.max[0] - projection.bounds.min[0],
            projection.bounds.max[1] - projection.bounds.min[1],
        ];
        let scale =
            (available_width / span[0].max(1.0e-6)).min(available_height / span[1].max(1.0e-6));
        if !scale.is_finite() || scale <= 0.0 {
            return Err(invalid("projection cannot be fit to the requested raster"));
        }
        let used_width = span[0] * scale;
        let used_height = span[1] * scale;
        let offset_x =
            padding + (available_width - used_width) * 0.5 - projection.bounds.min[0] * scale;
        let offset_y =
            padding + (available_height - used_height) * 0.5 - projection.bounds.min[1] * scale;
        if !offset_x.is_finite() || !offset_y.is_finite() {
            return Err(invalid("projection fit has non-finite offsets"));
        }
        Ok(Self {
            scale,
            offset_x,
            offset_y,
            height,
        })
    }

    fn map(self, point: [f64; 2]) -> [f64; 2] {
        [
            self.offset_x + point[0] * self.scale,
            (self.height as f64 - 1.0) - (self.offset_y + point[1] * self.scale),
        ]
    }
}

fn rasterize_triangle_ids(
    width: u32,
    height: u32,
    depth_buffer: &mut [f64],
    owners: &mut [ComponentMaskId],
    points: [[f64; 2]; 3],
    vertex_depths: [f64; 3],
    mask_id: ComponentMaskId,
) -> Result<(), String> {
    let min_x = points
        .iter()
        .map(|point| point[0].floor())
        .fold(f64::INFINITY, f64::min)
        .clamp(0.0, width as f64 - 1.0) as u32;
    let max_x = points
        .iter()
        .map(|point| point[0].ceil())
        .fold(f64::NEG_INFINITY, f64::max)
        .clamp(0.0, width as f64 - 1.0) as u32;
    let min_y = points
        .iter()
        .map(|point| point[1].floor())
        .fold(f64::INFINITY, f64::min)
        .clamp(0.0, height as f64 - 1.0) as u32;
    let max_y = points
        .iter()
        .map(|point| point[1].ceil())
        .fold(f64::NEG_INFINITY, f64::max)
        .clamp(0.0, height as f64 - 1.0) as u32;
    let denominator = (points[1][1] - points[2][1]) * (points[0][0] - points[2][0])
        + (points[2][0] - points[1][0]) * (points[0][1] - points[2][1]);
    if !denominator.is_finite() {
        return Err(invalid("projected triangle raster coordinates overflow"));
    }
    if denominator.abs() < 1.0e-9 {
        return Ok(());
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let a = ((points[1][1] - points[2][1]) * (px - points[2][0])
                + (points[2][0] - points[1][0]) * (py - points[2][1]))
                / denominator;
            let b = ((points[2][1] - points[0][1]) * (px - points[2][0])
                + (points[0][0] - points[2][0]) * (py - points[2][1]))
                / denominator;
            let c = 1.0 - a - b;
            if a < -1.0e-9 || b < -1.0e-9 || c < -1.0e-9 {
                continue;
            }
            let depth = a * vertex_depths[0] + b * vertex_depths[1] + c * vertex_depths[2];
            if !depth.is_finite() {
                return Err(invalid("projected triangle depth interpolation overflowed"));
            }
            let index = y as usize * width as usize + x as usize;
            // Projection depth is dot(position, toward); like dd_core's renderer, larger
            // values are nearer. Equal-depth ties go to the lower stable mask ID.
            if depth > depth_buffer[index]
                || (depth == depth_buffer[index] && (owners[index] == 0 || mask_id < owners[index]))
            {
                depth_buffer[index] = depth;
                owners[index] = mask_id;
            }
        }
    }
    Ok(())
}

/// All inputs are borrowed read-only. The same full `Projection` that was validated against
/// `geometry` is passed through with the contour, component masks, and optional design image.
#[derive(Debug, Clone, Copy)]
pub struct GraphicReferenceInput<'a> {
    pub request: &'a GraphicReferenceRequest,
    pub geometry: &'a GeometrySpec,
    pub projection: &'a Projection,
    pub contour: &'a RgbaImage,
    pub component_masks: &'a ComponentMasks,
    pub design_reference: Option<&'a RgbaImage>,
}

impl GraphicReferenceInput<'_> {
    /// Verify identity, revision, projection provenance, dimensions, bounds, and input hashes
    /// before any provider is called.
    pub fn validate(&self) -> Result<(), String> {
        let request = self.request;
        if request.schema_version != 1 {
            return Err(invalid("unsupported graphic-reference schema version"));
        }
        if request.asset_uid.is_empty()
            || request.asset_uid.len() > 256
            || request.asset_uid != self.geometry.asset_uid
        {
            return Err(invalid("asset UID mismatch"));
        }
        if request.geometry_revision != self.geometry.revision
            || self.projection.revision != self.geometry.revision
        {
            return Err(invalid("geometry revision mismatch"));
        }
        if self.projection.asset_uid != self.geometry.asset_uid {
            return Err(invalid("projection asset UID mismatch"));
        }
        if request.appearance_brief.len() > MAX_APPEARANCE_BRIEF_BYTES {
            return Err(invalid("appearance brief exceeds the provider limit"));
        }

        let dimensions = self.contour.dimensions();
        check_dimensions(dimensions, "contour")?;
        if self.component_masks.dimensions() != dimensions {
            return Err(invalid("component-mask dimensions differ from contour"));
        }
        if self
            .design_reference
            .is_some_and(|image| image.dimensions() != dimensions)
        {
            return Err(invalid("design-reference dimensions differ from contour"));
        }

        self.component_masks
            .validate_for_projection(self.projection)?;
        let expected_projection = self
            .geometry
            .project(self.projection.view)
            .map_err(|error| {
                invalid(&format!("cannot derive authoritative projection: {error}"))
            })?;
        if expected_projection != *self.projection {
            return Err(invalid(
                "projection does not match the authoritative geometry",
            ));
        }

        check_hash(
            "geometry",
            &serialized_sha256(self.geometry)?,
            &request.geometry_sha256,
        )?;
        check_hash(
            "projection",
            &serialized_sha256(self.projection)?,
            &request.projection_sha256,
        )?;
        check_hash(
            "contour",
            &rgba_sha256(self.contour),
            &request.contour_sha256,
        )?;
        check_hash(
            "component masks",
            &component_masks_sha256(self.component_masks)?,
            &request.component_masks_sha256,
        )?;
        match (
            self.design_reference,
            request.design_reference_sha256.as_deref(),
        ) {
            (Some(image), Some(expected)) => {
                check_hash("design reference", &rgba_sha256(image), expected)?;
            }
            (None, None) => {}
            _ => return Err(invalid("design-reference image/hash presence mismatch")),
        }
        Ok(())
    }
}

impl GraphicReferenceRequest {
    /// Build a request whose fingerprints match the exact immutable inputs supplied to a
    /// provider. Validation still runs before dispatch through `render_with_provider`.
    pub fn from_inputs(
        geometry: &GeometrySpec,
        projection: &Projection,
        contour: &RgbaImage,
        component_masks: &ComponentMasks,
        design_reference: Option<&RgbaImage>,
        appearance_brief: impl Into<String>,
    ) -> Result<Self, String> {
        let appearance_brief = appearance_brief.into();
        validate_input_bounds(
            contour,
            component_masks,
            design_reference,
            &appearance_brief,
        )?;
        let request = Self {
            schema_version: 1,
            asset_uid: geometry.asset_uid.clone(),
            geometry_revision: geometry.revision,
            geometry_sha256: serialized_sha256(geometry)?,
            projection_sha256: serialized_sha256(projection)?,
            contour_sha256: rgba_sha256(contour),
            component_masks_sha256: component_masks_sha256(component_masks)?,
            design_reference_sha256: design_reference.map(rgba_sha256),
            appearance_brief,
        };
        let input = GraphicReferenceInput {
            request: &request,
            geometry,
            projection,
            contour,
            component_masks,
            design_reference,
        };
        input.validate()?;
        Ok(request)
    }
}

pub trait GraphicProvider {
    /// Return appearance-only pixels. Geometry and source images are immutable borrowed inputs.
    fn create_appearance(&self, input: &GraphicReferenceInput<'_>) -> Result<RgbaImage, String>;
}

pub struct OfflineGraphicProvider;
impl GraphicProvider for OfflineGraphicProvider {
    fn create_appearance(&self, _: &GraphicReferenceInput<'_>) -> Result<RgbaImage, String> {
        Err("UNSUPPORTED_PROVIDER: no external appearance provider is configured; technical references remain available".into())
    }
}

/// Validate all input provenance, call the optional provider, then clip its output to the
/// authoritative contour. This is the bounded path callers should use for provider output.
pub fn render_with_provider(
    provider: &impl GraphicProvider,
    input: &GraphicReferenceInput<'_>,
) -> Result<RgbaImage, String> {
    input.validate()?;
    let appearance = provider.create_appearance(input)?;
    clip_to_contour(&appearance, input.contour)
}

/// A provider cannot add pixels outside the authoritative technical silhouette. This checks
/// image-space containment only, not semantic correctness of details.
pub fn clip_to_contour(appearance: &RgbaImage, contour: &RgbaImage) -> Result<RgbaImage, String> {
    if appearance.dimensions() != contour.dimensions() {
        return Err(invalid("provider dimensions differ from contour"));
    }
    let mut result = appearance.clone();
    for (pixel, mask) in result.pixels_mut().zip(contour.pixels()) {
        pixel[3] = ((u16::from(pixel[3]) * u16::from(mask[3])) / 255) as u8;
        if pixel[3] == 0 {
            pixel.0 = [0; 4];
        }
    }
    Ok(result)
}

fn check_dimensions((width, height): (u32, u32), name: &str) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_REFERENCE_SIDE
        || height > MAX_REFERENCE_SIDE
        || pixels > MAX_REFERENCE_PIXELS
    {
        return Err(invalid(&format!(
            "{name} dimensions are empty or exceed the provider limit"
        )));
    }
    Ok(())
}

fn validate_input_bounds(
    contour: &RgbaImage,
    component_masks: &ComponentMasks,
    design_reference: Option<&RgbaImage>,
    appearance_brief: &str,
) -> Result<(), String> {
    let dimensions = contour.dimensions();
    check_dimensions(dimensions, "contour")?;
    check_dimensions(component_masks.dimensions(), "component masks")?;
    if component_masks.dimensions() != dimensions {
        return Err(invalid("component-mask dimensions differ from contour"));
    }
    if design_reference.is_some_and(|image| image.dimensions() != dimensions) {
        return Err(invalid("design-reference dimensions differ from contour"));
    }
    if component_masks.legend.len() > MAX_COMPONENTS {
        return Err(invalid("component count exceeds the provider limit"));
    }
    if appearance_brief.len() > MAX_APPEARANCE_BRIEF_BYTES {
        return Err(invalid("appearance brief exceeds the provider limit"));
    }
    Ok(())
}

fn invalid(reason: &str) -> String {
    format!("INVALID_REFERENCE: {reason}")
}

fn check_hash(name: &str, actual: &str, expected: &str) -> Result<(), String> {
    if expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        || actual != expected
    {
        return Err(invalid(&format!(
            "{name} SHA-256 mismatch or malformed hash"
        )));
    }
    Ok(())
}

fn serialized_sha256(value: &impl Serialize) -> Result<String, String> {
    let bytes = bounded_json(value, MAX_STRUCTURED_INPUT_BYTES, "serialized input")?;
    Ok(hex_sha256(&bytes))
}

fn rgba_sha256(image: &RgbaImage) -> String {
    let mut digest = Sha256::new();
    digest.update(image.width().to_le_bytes());
    digest.update(image.height().to_le_bytes());
    digest.update(image.as_raw());
    finish_sha256(digest)
}

fn component_masks_sha256(masks: &ComponentMasks) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(masks.ids.width().to_le_bytes());
    digest.update(masks.ids.height().to_le_bytes());
    for id in masks.ids.as_raw() {
        digest.update(id.to_le_bytes());
    }
    let legend = bounded_json(
        &masks.legend,
        MAX_STRUCTURED_INPUT_BYTES,
        "component-mask legend",
    )?;
    digest.update(legend);
    Ok(finish_sha256(digest))
}

fn bounded_json(value: &impl Serialize, limit: usize, name: &str) -> Result<Vec<u8>, String> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| invalid(&format!("cannot fingerprint {name}: {error}")))?;
    Ok(writer.bytes)
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for LimitedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(buffer.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "serialized input exceeds the provider limit",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

fn finish_sha256(digest: Sha256) -> String {
    hex_digest(&digest.finalize())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dd_core::geometry::{
        Axis, Bounds2, ComponentKind, GeometryUnits, PixelRenderOptions, ProjectedAxis,
        ProjectedTriangle, ProjectionConvention, ProjectionScale, View, render_pixel_reference,
    };

    fn projected_triangle_component(
        component_id: &str,
        label: &str,
        vertices: [[f64; 2]; 3],
        depth: f64,
    ) -> ProjectedComponent {
        ProjectedComponent {
            component_id: component_id.into(),
            label: label.into(),
            material: "steel".into(),
            kind: ComponentKind::TriangleMesh,
            vertices: vertices.to_vec(),
            depths: vec![depth; 3],
            edges: Vec::new(),
            triangles: vec![ProjectedTriangle {
                vertices,
                depth: [depth; 3],
                normal: [0.0, 0.0, 1.0],
            }],
        }
    }

    fn overlapping_projection() -> Projection {
        Projection {
            asset_uid: "raster-test".into(),
            revision: 7,
            units: GeometryUnits::ArtisticRelative,
            view: View::Side,
            convention: ProjectionConvention::ThirdAngle,
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            projected_axes: [
                ProjectedAxis {
                    axis: Axis::X,
                    direction: [1.0, 0.0],
                },
                ProjectedAxis {
                    axis: Axis::Y,
                    direction: [0.0, 1.0],
                },
                ProjectedAxis {
                    axis: Axis::Z,
                    direction: [0.0, 0.0],
                },
            ],
            scale: ProjectionScale {
                world_units_per_projection_unit: 1.0,
            },
            bounds: Bounds2 {
                min: [0.0, 0.0],
                max: [1.0, 1.0],
            },
            // Reverse ID order to check that categorical IDs do not depend on component order.
            components: vec![
                projected_triangle_component(
                    "rear",
                    "Rear plate",
                    [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                    0.0,
                ),
                projected_triangle_component(
                    "front",
                    "Front plate",
                    [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]],
                    1.0,
                ),
            ],
        }
    }

    #[test]
    fn projection_rasterizer_assigns_nearest_component_and_contour_union_deterministically() {
        let projection = overlapping_projection();
        let first = rasterize_projection(&projection, 32, 32).unwrap();
        let second = rasterize_projection(&projection, 32, 32).unwrap();
        assert_eq!(first, second);

        let front_id = first
            .component_masks
            .legend
            .iter()
            .find(|entry| entry.component_id == "front")
            .unwrap()
            .mask_id;
        let rear_id = first
            .component_masks
            .legend
            .iter()
            .find(|entry| entry.component_id == "rear")
            .unwrap()
            .mask_id;
        assert!(front_id < rear_id);
        assert_eq!(first.component_masks.ids.get_pixel(14, 17)[0], front_id);
        assert_eq!(first.component_masks.ids.get_pixel(21, 22)[0], rear_id);

        let front_mask = first.component_masks.mask_for_component("front").unwrap();
        let rear_mask = first.component_masks.mask_for_component("rear").unwrap();
        assert_eq!(front_mask.get_pixel(14, 17)[0], 255);
        assert_eq!(rear_mask.get_pixel(14, 17)[0], 0);
        assert_eq!(rear_mask.get_pixel(21, 22)[0], 255);
        for y in 0..32 {
            for x in 0..32 {
                let covered = first.component_masks.ids.get_pixel(x, y)[0] != 0;
                assert_eq!(first.contour.get_pixel(x, y)[3] == 255, covered);
                assert_eq!(
                    front_mask.get_pixel(x, y)[0] == 255 || rear_mask.get_pixel(x, y)[0] == 255,
                    covered
                );
            }
        }
    }

    #[test]
    fn projection_rasterizer_rejects_unbounded_image_dimensions() {
        let projection = overlapping_projection();
        assert!(rasterize_projection(&projection, 0, 32).is_err());
        assert!(rasterize_projection(&projection, 4097, 1).is_err());
    }

    #[test]
    fn side_contour_uses_the_same_fit_as_the_core_pixel_renderer() {
        let geometry = GeometrySpec::shotgun();
        let projection = geometry.project(View::Side).unwrap();
        let raster = rasterize_projection(&projection, 480, 160).unwrap();
        let rendered = render_pixel_reference(
            &geometry,
            View::Side,
            480,
            160,
            PixelRenderOptions {
                padding_pixels: 8,
                background: [0, 0, 0, 0],
                outline: false,
                component_edges: false,
            },
        )
        .unwrap();
        let mismatches = raster
            .contour
            .pixels()
            .zip(rendered.pixels())
            .filter(|(contour, rendered)| (contour[3] > 0) != (rendered[3] > 0))
            .count();
        assert!(
            mismatches <= 32,
            "mask/render silhouette differs by {mismatches} pixels"
        );
    }

    #[test]
    fn silhouette_reference_is_a_single_flat_ink_shape_from_the_projection_mask() {
        let geometry = GeometrySpec::shotgun();
        let projection = geometry.project(View::Side).unwrap();
        let ink = [20, 24, 28, 255];
        let paper = [246, 244, 236, 255];
        let rendered = render_silhouette_reference(&projection, 720, 240, ink, paper).unwrap();
        let expected = rasterize_projection(&projection, 720, 240).unwrap();
        let mut silhouette_pixels = 0;
        for (actual, contour) in rendered.pixels().zip(expected.contour.pixels()) {
            if contour[3] == 0 {
                assert_eq!(actual.0, paper);
            } else {
                assert_eq!(actual.0, ink);
                silhouette_pixels += 1;
            }
        }
        assert!(silhouette_pixels > 500);
        assert!(silhouette_pixels < (720 * 240) as usize);
    }

    fn inputs<'a>(
        geometry: &'a GeometrySpec,
        projection: &'a Projection,
        contour: &'a RgbaImage,
        masks: &'a ComponentMasks,
        design_reference: Option<&'a RgbaImage>,
    ) -> GraphicReferenceRequest {
        GraphicReferenceRequest::from_inputs(
            geometry,
            projection,
            contour,
            masks,
            design_reference,
            "walnut and blued steel, preserve all part boundaries",
        )
        .unwrap()
    }

    fn masks_for(projection: &Projection, width: u32, height: u32) -> ComponentMasks {
        let legend: Vec<_> = projection
            .components
            .iter()
            .enumerate()
            .map(|(index, component)| ComponentMaskLegendEntry {
                mask_id: (index + 1) as ComponentMaskId,
                component_id: component.component_id.clone(),
                label: component.label.clone(),
                material: component.material.clone(),
            })
            .collect();
        let first_id = legend.first().map_or(0, |entry| entry.mask_id);
        ComponentMasks::new(
            ImageBuffer::from_pixel(width, height, Luma([first_id])),
            legend,
        )
    }

    fn valid_fixture() -> (
        GeometrySpec,
        Projection,
        RgbaImage,
        ComponentMasks,
        RgbaImage,
    ) {
        let geometry = GeometrySpec::shotgun();
        let projection = geometry.project(View::Side).unwrap();
        let contour = RgbaImage::from_pixel(2, 1, image::Rgba([255, 255, 255, 255]));
        let masks = masks_for(&projection, 2, 1);
        let design = RgbaImage::from_pixel(2, 1, image::Rgba([50, 60, 70, 255]));
        (geometry, projection, contour, masks, design)
    }

    #[test]
    fn provider_receives_authoritative_projection_and_component_legend_then_is_clipped() {
        struct PaintedProvider;
        impl GraphicProvider for PaintedProvider {
            fn create_appearance(
                &self,
                input: &GraphicReferenceInput<'_>,
            ) -> Result<RgbaImage, String> {
                assert_eq!(input.projection.view, View::Side);
                assert_eq!(input.projection.asset_uid, input.geometry.asset_uid);
                assert!(input.design_reference.is_some());
                assert_eq!(
                    input.component_masks.legend.len(),
                    input.projection.components.len()
                );
                Ok(RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255])))
            }
        }

        let (geometry, projection, mut contour, masks, design) = valid_fixture();
        contour.put_pixel(1, 0, image::Rgba([255, 255, 255, 0]));
        let request = inputs(&geometry, &projection, &contour, &masks, Some(&design));
        let original_geometry = geometry.clone();
        let input = GraphicReferenceInput {
            request: &request,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&design),
        };
        let result = render_with_provider(&PaintedProvider, &input).unwrap();
        assert_eq!(result.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(1, 0).0, [0; 4]);
        assert_eq!(geometry, original_geometry);
        assert_eq!(projection, geometry.project(View::Side).unwrap());
    }

    #[test]
    fn input_validation_rejects_revision_hash_and_dimension_mismatches() {
        let (geometry, projection, contour, masks, design) = valid_fixture();
        let request = inputs(&geometry, &projection, &contour, &masks, Some(&design));
        let mut stale = request.clone();
        stale.geometry_revision += 1;
        let stale_input = GraphicReferenceInput {
            request: &stale,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&design),
        };
        assert!(
            stale_input
                .validate()
                .unwrap_err()
                .contains("revision mismatch")
        );

        let mut corrupt_hash = request.clone();
        corrupt_hash.contour_sha256 = "0".repeat(64);
        let corrupt_input = GraphicReferenceInput {
            request: &corrupt_hash,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&design),
        };
        assert!(
            corrupt_input
                .validate()
                .unwrap_err()
                .contains("contour SHA-256 mismatch")
        );

        let different_size = RgbaImage::new(1, 1);
        let size_input = GraphicReferenceInput {
            request: &request,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&different_size),
        };
        assert!(
            size_input
                .validate()
                .unwrap_err()
                .contains("design-reference dimensions")
        );

        let wrong_size_masks = masks_for(&projection, 1, 1);
        let mask_input = GraphicReferenceInput {
            request: &request,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &wrong_size_masks,
            design_reference: Some(&design),
        };
        assert!(
            mask_input
                .validate()
                .unwrap_err()
                .contains("component-mask dimensions")
        );
    }

    #[test]
    fn component_mask_ids_must_match_the_projection_legend() {
        let (geometry, projection, contour, mut masks, design) = valid_fixture();
        masks.ids.put_pixel(0, 0, Luma([u16::MAX]));
        let request = GraphicReferenceRequest {
            schema_version: 1,
            asset_uid: geometry.asset_uid.clone(),
            geometry_revision: geometry.revision,
            geometry_sha256: serialized_sha256(&geometry).unwrap(),
            projection_sha256: serialized_sha256(&projection).unwrap(),
            contour_sha256: rgba_sha256(&contour),
            component_masks_sha256: component_masks_sha256(&masks).unwrap(),
            design_reference_sha256: Some(rgba_sha256(&design)),
            appearance_brief: "test".into(),
        };
        let input = GraphicReferenceInput {
            request: &request,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&design),
        };
        assert!(
            input
                .validate()
                .unwrap_err()
                .contains("ID missing from its legend")
        );
    }

    #[test]
    fn offline_provider_keeps_the_existing_unsupported_result() {
        let (geometry, projection, contour, masks, design) = valid_fixture();
        let request = inputs(&geometry, &projection, &contour, &masks, Some(&design));
        let input = GraphicReferenceInput {
            request: &request,
            geometry: &geometry,
            projection: &projection,
            contour: &contour,
            component_masks: &masks,
            design_reference: Some(&design),
        };
        assert!(
            render_with_provider(&OfflineGraphicProvider, &input)
                .unwrap_err()
                .contains("UNSUPPORTED_PROVIDER")
        );
    }

    #[test]
    fn clip_to_contour_rejects_dimensions_and_does_not_mutate_provider_pixels() {
        let appearance = RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        let mut mask = RgbaImage::new(2, 1);
        mask.put_pixel(0, 0, image::Rgba([255; 4]));
        let result = clip_to_contour(&appearance, &mask).unwrap();
        assert_eq!(result.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(1, 0).0, [0; 4]);
        assert_eq!(appearance.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert!(clip_to_contour(&appearance, &RgbaImage::new(3, 1)).is_err());
    }
}
