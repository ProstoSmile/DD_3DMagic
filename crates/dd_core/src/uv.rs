//! World-space UV density analysis for exported Blender loop triangles.
//! Validates per-triangle density and anisotropy. Does NOT claim to verify UV overlap,
//! per-island padding, camera projection, or generated art consistency.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvTriangle {
    pub object: String,
    pub positions_m: [[f64; 3]; 3],
    pub uv: [[f64; 2]; 3],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvExport {
    pub schema_version: u32,
    pub triangles: Vec<UvTriangle>,
}
#[derive(Debug, Clone, Serialize)]
pub struct TriangleMetrics {
    pub object: String,
    pub surface_area_m2: f64,
    pub uv_area: f64,
    pub td_geomean_px_per_m: f64,
    pub td_min_px_per_m: f64,
    pub td_max_px_per_m: f64,
    pub anisotropy_ratio: f64,
}
#[derive(Debug, Clone, Serialize)]
pub struct UvReport {
    pub triangles_total: usize,
    pub triangles_valid: usize,
    pub triangles_degenerate: usize,
    pub surface_area_m2: f64,
    pub average_td_px_per_m: f64,
    pub min_td_px_per_m: f64,
    pub max_td_px_per_m: f64,
    pub worst_anisotropy: f64,
    pub target_td_px_per_m: f64,
    pub target_outlier_count: usize,
    pub metrics: Vec<TriangleMetrics>,
    pub notes: Vec<String>,
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

pub fn measure_triangle(tri: &UvTriangle, atlas: [u32; 2]) -> Option<TriangleMetrics> {
    if atlas[0] == 0 || atlas[1] == 0 {
        return None;
    }
    if !tri
        .positions_m
        .iter()
        .flatten()
        .chain(tri.uv.iter().flatten())
        .all(|x| x.is_finite())
    {
        return None;
    }
    let a = sub(tri.positions_m[1], tri.positions_m[0]);
    let b = sub(tri.positions_m[2], tri.positions_m[0]);
    let x_len = length(a);
    if x_len < 1e-10 {
        return None;
    }
    let x = dot(a, b) / x_len;
    let y = length(cross(a, b)) / x_len;
    if y < 1e-10 {
        return None;
    }
    let au = (tri.uv[1][0] - tri.uv[0][0]) * atlas[0] as f64;
    let av = (tri.uv[1][1] - tri.uv[0][1]) * atlas[1] as f64;
    let bu = (tri.uv[2][0] - tri.uv[0][0]) * atlas[0] as f64;
    let bv = (tri.uv[2][1] - tri.uv[0][1]) * atlas[1] as f64;
    // Jacobian: world-space local tangent basis -> atlas-pixel coordinates.
    let j00 = au / x_len;
    let j10 = av / x_len;
    let j01 = (bu - x * j00) / y;
    let j11 = (bv - x * j10) / y;
    let trace = j00 * j00 + j01 * j01 + j10 * j10 + j11 * j11;
    let det = j00 * j11 - j01 * j10;
    let spread = (trace * trace - 4.0 * det * det).max(0.0).sqrt();
    let max_td = ((trace + spread) / 2.0).max(0.0).sqrt();
    let min_td = ((trace - spread) / 2.0).max(0.0).sqrt();
    if min_td < 1e-10 {
        return None;
    }
    let uv_cross = (tri.uv[1][0] - tri.uv[0][0]) * (tri.uv[2][1] - tri.uv[0][1])
        - (tri.uv[1][1] - tri.uv[0][1]) * (tri.uv[2][0] - tri.uv[0][0]);
    Some(TriangleMetrics {
        object: tri.object.clone(),
        surface_area_m2: x_len * y * 0.5,
        uv_area: uv_cross.abs() * 0.5,
        td_geomean_px_per_m: det.abs().sqrt(),
        td_min_px_per_m: min_td,
        td_max_px_per_m: max_td,
        anisotropy_ratio: max_td / min_td,
    })
}

pub fn analyze(
    data: &UvExport,
    atlas: [u32; 2],
    target_td: f64,
    tolerance_pct: f64,
) -> Result<UvReport, String> {
    if data.schema_version != 1 {
        return Err(format!(
            "unsupported UV export schema {}",
            data.schema_version
        ));
    }
    if atlas[0] == 0
        || atlas[1] == 0
        || !target_td.is_finite()
        || target_td <= 0.0
        || !tolerance_pct.is_finite()
        || tolerance_pct < 0.0
    {
        return Err("invalid analysis parameters".into());
    }
    if data.triangles.len() > 2_000_000 {
        return Err("UV export exceeds triangle safety limit".into());
    }
    let metrics: Vec<_> = data
        .triangles
        .iter()
        .filter_map(|t| measure_triangle(t, atlas))
        .collect();
    let area: f64 = metrics.iter().map(|m| m.surface_area_m2).sum();
    let average = if area > 0.0 {
        metrics
            .iter()
            .map(|m| m.td_geomean_px_per_m * m.surface_area_m2)
            .sum::<f64>()
            / area
    } else {
        0.0
    };
    let min_td = metrics
        .iter()
        .map(|m| m.td_min_px_per_m)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let max_td = metrics
        .iter()
        .map(|m| m.td_max_px_per_m)
        .reduce(f64::max)
        .unwrap_or(0.0);
    let anisotropy = metrics
        .iter()
        .map(|m| m.anisotropy_ratio)
        .reduce(f64::max)
        .unwrap_or(0.0);
    let outliers = metrics
        .iter()
        .filter(|m| ((m.td_geomean_px_per_m / target_td) - 1.0).abs() * 100.0 > tolerance_pct)
        .count();
    Ok(UvReport {
        triangles_total: data.triangles.len(), triangles_valid: metrics.len(),
        triangles_degenerate: data.triangles.len()-metrics.len(), surface_area_m2: area,
        average_td_px_per_m: average, min_td_px_per_m: min_td,
        max_td_px_per_m: max_td, worst_anisotropy: anisotropy,
        target_td_px_per_m: target_td, target_outlier_count: outliers,
        metrics,
        notes: vec![
            "TD assumes world coordinates in meters and pixel atlas dimensions.".into(),
            "Overlapping UVs, island boundaries, padding, and seam continuity are NOT checked by this release.".into(),
            "Mirrored UVs require intentional overlap policy; UV face area alone does not detect collisions.".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_square_triangle_at_128_pixels_has_128_density() {
        let t = UvTriangle {
            object: "Quad".into(),
            positions_m: [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            uv: [[0., 0.], [1., 0.], [0., 1.]],
        };
        let m = measure_triangle(&t, [128, 128]).unwrap();
        assert!((m.td_geomean_px_per_m - 128.0).abs() < 1e-6);
        assert!((m.anisotropy_ratio - 1.0).abs() < 1e-6);
    }
    #[test]
    fn degenerate_triangle_is_not_reported_as_valid() {
        let t = UvTriangle {
            object: "Line".into(),
            positions_m: [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
            uv: [[0., 0.], [1., 0.], [0., 1.]],
        };
        assert!(measure_triangle(&t, [256, 256]).is_none());
    }
}
