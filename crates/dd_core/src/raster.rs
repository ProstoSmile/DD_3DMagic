//! Non-destructive operations: every preview is recomputed from the source raster.
//! IMPORTANT: nearest-neighbor reduction is not an art-quality pixel-art solver.
//! Artistic clusters, semantic paint masks, and UV correspondence are future modules.

use image::{ImageFormat, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::BufWriter;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RasterOp {
    ResizeNearest { width: u32, height: u32 },
    PaletteNearest { colors: Vec<[u8; 3]> },
    Brightness { delta: i16 },
    Grayscale,
}

#[derive(Debug, Clone)]
pub struct RasterDocument {
    pub original: RgbaImage,
    pub operations: Vec<RasterOp>,
}

#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    #[error("invalid image dimensions (1..=8192 required)")]
    Dimensions,
    #[error("image file is larger than 32 MiB")]
    FileTooLarge,
    #[error("empty palette")]
    EmptyPalette,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),
}

fn valid_dims(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= 8192 && h <= 8192
}

pub fn open_image(path: &Path) -> Result<RgbaImage, RasterError> {
    if fs::metadata(path)?.len() > 32 * 1024 * 1024 {
        return Err(RasterError::FileTooLarge);
    }
    let (width, height) = image::image_dimensions(path)?;
    if !valid_dims(width, height) {
        return Err(RasterError::Dimensions);
    }
    Ok(image::open(path)?.to_rgba8())
}

impl RasterDocument {
    pub fn new(source: RgbaImage) -> Self {
        Self {
            original: source,
            operations: Vec::new(),
        }
    }
    pub fn apply(&mut self, op: RasterOp) -> Result<(), RasterError> {
        match &op {
            RasterOp::ResizeNearest { width, height } if !valid_dims(*width, *height) => {
                return Err(RasterError::Dimensions);
            }
            RasterOp::PaletteNearest { colors } if colors.is_empty() => {
                return Err(RasterError::EmptyPalette);
            }
            _ => {}
        }
        self.operations.push(op);
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        self.operations.pop().is_some()
    }
    pub fn render(&self) -> Result<RgbaImage, RasterError> {
        let mut pixels = self.original.clone();
        for op in &self.operations {
            match op {
                RasterOp::ResizeNearest { width, height } => {
                    if !valid_dims(*width, *height) {
                        return Err(RasterError::Dimensions);
                    }
                    pixels = image::imageops::resize(&pixels, *width, *height, FilterType::Nearest);
                }
                RasterOp::PaletteNearest { colors } => {
                    if colors.is_empty() {
                        return Err(RasterError::EmptyPalette);
                    }
                    for p in pixels.pixels_mut() {
                        if p[3] == 0 {
                            continue;
                        }
                        let best = colors
                            .iter()
                            .min_by_key(|rgb| {
                                (0..3)
                                    .map(|i| (p[i] as i32 - rgb[i] as i32).pow(2))
                                    .sum::<i32>()
                            })
                            .expect("nonempty palette");
                        p[0] = best[0];
                        p[1] = best[1];
                        p[2] = best[2];
                    }
                }
                RasterOp::Brightness { delta } => {
                    for p in pixels.pixels_mut() {
                        for channel in p.0[..3].iter_mut() {
                            *channel = (*channel as i32 + *delta as i32).clamp(0, 255) as u8;
                        }
                    }
                }
                RasterOp::Grayscale => {
                    for p in pixels.pixels_mut() {
                        let luma =
                            (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64)
                                .round() as u8;
                        p[0] = luma;
                        p[1] = luma;
                        p[2] = luma;
                    }
                }
            }
        }
        Ok(pixels)
    }
}

/// Never overwrite: a new staging file must be chosen by the caller.
pub fn write_new_png(image: &RgbaImage, output: &Path) -> Result<(), RasterError> {
    crate::storage::require_d_path(output).map_err(|error| {
        RasterError::Io(std::io::Error::other(format!("PATH_OUTSIDE_D: {error}")))
    })?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut writer = BufWriter::new(file);
    use std::io::Write;
    let result = (|| -> Result<(), RasterError> {
        image::DynamicImage::ImageRgba8(image.clone()).write_to(&mut writer, ImageFormat::Png)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(())
    })();
    drop(writer);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn pipeline_undo_preserves_original() {
        let source = RgbaImage::from_fn(2, 2, |x, _| {
            if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let mut doc = RasterDocument::new(source.clone());
        doc.apply(RasterOp::ResizeNearest {
            width: 1,
            height: 1,
        })
        .unwrap();
        assert_eq!(doc.render().unwrap().width(), 1);
        assert!(doc.undo());
        assert_eq!(doc.render().unwrap(), source);
    }
    #[test]
    fn palette_preserves_alpha() {
        let src = RgbaImage::from_pixel(1, 1, Rgba([220, 80, 60, 37]));
        let mut doc = RasterDocument::new(src);
        doc.apply(RasterOp::PaletteNearest {
            colors: vec![[255, 0, 0], [0, 0, 255]],
        })
        .unwrap();
        let pix = doc.render().unwrap();
        assert_eq!(pix.get_pixel(0, 0).0, [255, 0, 0, 37]);
    }
}
