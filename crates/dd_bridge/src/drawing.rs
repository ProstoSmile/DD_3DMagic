//! Native pixel canvas for agent-authored black strokes.

use dd_core::storage::ManagedWorkspace;
use image::{DynamicImage, GrayImage, Luma};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

const MAX_DOCUMENT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEGMENT_STEPS: u64 = 1_000_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawingRequest {
    Create {
        width: u16,
        height: u16,
        output_relative: String,
    },
    Stroke {
        source_relative: String,
        strokes: Vec<Stroke>,
        output_relative: String,
    },
    Capture {
        source_relative: String,
    },
    Export {
        source_relative: String,
        output_relative: String,
    },
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// Brush radius in pixels. Zero draws a single pixel.
    radius: u8,
    points: Vec<[u16; 2]>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Drawing {
    schema_version: u8,
    width: u16,
    height: u16,
    strokes: Vec<Stroke>,
}

pub struct DrawingFrame {
    pub metadata: serde_json::Value,
    pub png: Vec<u8>,
}

fn load(workspace: &ManagedWorkspace, source: &str) -> Result<(Drawing, Vec<u8>), String> {
    let path = workspace
        .existing(Path::new(source))
        .map_err(|e| e.to_string())?;
    if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return Err("Expected a drawing JSON file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_DOCUMENT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err("Drawing JSON exceeds 2 MiB".into());
    }
    let drawing = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok((drawing, bytes))
}

/// Freeze the editable source and preview from the same read.
pub(crate) fn read_reference(
    workspace: &ManagedWorkspace,
    source: &str,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (drawing, bytes) = load(workspace, source)?;
    drawing.validate()?;
    if drawing.strokes.is_empty() {
        return Err("A design drawing must contain at least one stroke".into());
    }
    Ok((bytes, drawing.capture()?))
}

pub fn execute(root: &Path, request: DrawingRequest) -> Result<DrawingFrame, String> {
    let workspace = ManagedWorkspace::new(root).map_err(|e| e.to_string())?;
    let (drawing, source, output, export) = match request {
        DrawingRequest::Create {
            width,
            height,
            output_relative,
        } => (
            Drawing {
                schema_version: 1,
                width,
                height,
                strokes: Vec::new(),
            },
            None,
            Some(output_relative),
            false,
        ),
        DrawingRequest::Stroke {
            source_relative,
            strokes,
            output_relative,
        } => {
            if strokes.is_empty() {
                return Err("Provide at least one stroke".into());
            }
            let (mut drawing, _) = load(&workspace, &source_relative)?;
            drawing.strokes.extend(strokes);
            (drawing, Some(source_relative), Some(output_relative), false)
        }
        DrawingRequest::Capture { source_relative } => (
            load(&workspace, &source_relative)?.0,
            Some(source_relative),
            None,
            false,
        ),
        DrawingRequest::Export {
            source_relative,
            output_relative,
        } => (
            load(&workspace, &source_relative)?.0,
            Some(source_relative),
            Some(output_relative),
            true,
        ),
    };
    drawing.validate()?;
    let png = drawing.capture()?;
    let mut canvas = source.as_deref();
    if let Some(output) = output.as_deref() {
        let path = Path::new(output);
        let extension = if export { "png" } else { "json" };
        if path.extension().and_then(|ext| ext.to_str()) != Some(extension) {
            return Err(format!("Expected a .{extension} output"));
        }
        let document;
        let bytes = if export {
            &png
        } else {
            document = serde_json::to_vec(&drawing).map_err(|e| e.to_string())?;
            if document.len() as u64 > MAX_DOCUMENT_BYTES {
                return Err("Drawing JSON exceeds 2 MiB".into());
            }
            canvas = Some(output);
            &document
        };
        workspace
            .write_new(path, bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(DrawingFrame {
        metadata: serde_json::json!({
            "canvas_relative": canvas, "width": drawing.width, "height": drawing.height,
            "stroke_count": drawing.strokes.len(), "png_relative": output.filter(|_| export)
        }),
        png,
    })
}

impl Drawing {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !(1..=2048).contains(&self.width)
            || !(1..=2048).contains(&self.height)
            || self.strokes.len() > 4096
        {
            return Err("Invalid drawing version, canvas dimensions, or stroke count".into());
        }
        let mut steps = 0;
        for stroke in &self.strokes {
            if stroke.radius > 4
                || stroke.points.is_empty()
                || stroke.points.len() > 4096
                || stroke
                    .points
                    .iter()
                    .any(|[x, y]| *x >= self.width || *y >= self.height)
            {
                return Err("Invalid brush radius or stroke points".into());
            }
            steps += 1 + stroke
                .points
                .windows(2)
                .map(|pair| {
                    u64::from(
                        pair[0][0]
                            .abs_diff(pair[1][0])
                            .max(pair[0][1].abs_diff(pair[1][1]))
                            + 1,
                    )
                })
                .sum::<u64>();
            if steps > MAX_SEGMENT_STEPS {
                return Err("Drawing exceeds the stroke rasterization limit".into());
            }
        }
        Ok(())
    }

    fn capture(&self) -> Result<Vec<u8>, String> {
        let mut image = GrayImage::from_pixel(self.width.into(), self.height.into(), Luma([255]));
        for stroke in &self.strokes {
            let radius = i32::from(stroke.radius);
            let mut stamp = |x: i32, y: i32| {
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx * dx + dy * dy <= radius * radius
                            && let Some(pixel) =
                                image.get_pixel_mut_checked((x + dx) as u32, (y + dy) as u32)
                        {
                            *pixel = Luma([0]);
                        }
                    }
                }
            };
            let [x, y] = stroke.points[0].map(i32::from);
            stamp(x, y);
            for pair in stroke.points.windows(2) {
                let [mut x, mut y] = pair[0].map(i32::from);
                let [end_x, end_y] = pair[1].map(i32::from);
                let dx = (end_x - x).abs();
                let dy = -(end_y - y).abs();
                let sx = (end_x - x).signum();
                let sy = (end_y - y).signum();
                let mut error = dx + dy;
                while (x, y) != (end_x, end_y) {
                    let doubled = 2 * error;
                    if doubled >= dy {
                        error += dy;
                        x += sx;
                    }
                    if doubled <= dx {
                        error += dx;
                        y += sy;
                    }
                    stamp(x, y);
                }
            }
        }
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(png.into_inner())
    }
}
