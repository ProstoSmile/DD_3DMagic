//! Independent checks of exported projections against world-space source vertices.
//! This module does not call the projection renderer or trust its saved bounds.
use crate::geometry::{GeometrySpec, Projection, View};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GateStatus {
    Pass,
    PassWithWarnings,
    Fail,
    NotVerifiable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateReport {
    pub gate: String,
    pub status: GateStatus,
    pub checks: Vec<String>,
    pub diagnostics: Vec<String>,
    pub revision: u64,
}

pub fn validate_projections(spec: &GeometrySpec, views: &[Projection]) -> GateReport {
    let mut report = GateReport {
        gate: "projection".into(),
        status: GateStatus::Pass,
        checks: vec![
            "revision, units and component identity".into(),
            "orthonormal right-handed camera matrices".into(),
            "fixed principal-view orientation and unit scale".into(),
            "every projected vertex and depth".into(),
            "recomputed projection extrema".into(),
        ],
        diagnostics: Vec::new(),
        revision: spec.revision,
    };
    let proxy = match spec.proxy() {
        Ok(proxy) => proxy,
        Err(error) => {
            report.status = GateStatus::NotVerifiable;
            report.diagnostics.push(error.to_string());
            return report;
        }
    };
    let required = spec.technical_sheet_layout.views();
    if views.len() != required.len() {
        report
            .diagnostics
            .push(format!("{} reference views are required", required.len()));
    }
    for &view in required {
        let matching: Vec<_> = views.iter().filter(|p| p.view == view).collect();
        if matching.len() != 1 {
            report
                .diagnostics
                .push(format!("{view:?}: missing/duplicate view"));
            continue;
        }
        let projection = matching[0];
        if projection.asset_uid != spec.asset_uid
            || projection.revision != spec.revision
            || projection.units != spec.units
            || projection.convention != spec.projection_convention
        {
            report
                .diagnostics
                .push(format!("{view:?}: source provenance mismatch"));
        }
        let matrix = projection.matrix;
        if matrix.iter().flatten().any(|n| !n.is_finite())
            || matrix[3] != [0., 0., 0., 1.]
            || matrix[..3].iter().any(|r| r[3] != 0.)
        {
            report
                .diagnostics
                .push(format!("{view:?}: invalid orthographic matrix"));
            continue;
        }
        let dot = |a: [f64; 4], b: [f64; 4]| (0..3).map(|i| a[i] * b[i]).sum::<f64>();
        let tolerance = 256. * f64::EPSILON;
        for a in 0..3 {
            for b in 0..3 {
                let expected = if a == b { 1. } else { 0. };
                if (dot(matrix[a], matrix[b]) - expected).abs() > tolerance {
                    report
                        .diagnostics
                        .push(format!("{view:?}: non-orthonormal camera rows {a}/{b}"));
                }
            }
        }
        let cross = [
            matrix[0][1] * matrix[1][2] - matrix[0][2] * matrix[1][1],
            matrix[0][2] * matrix[1][0] - matrix[0][0] * matrix[1][2],
            matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0],
        ];
        if (0..3).any(|i| (cross[i] - matrix[2][i]).abs() > tolerance) {
            report
                .diagnostics
                .push(format!("{view:?}: camera basis is reflected"));
        }
        let orientation = match view {
            View::Side => Some(([1., 0., 0., 0.], [0., 0., 1., 0.])),
            View::Top => Some(([1., 0., 0., 0.], [0., 1., 0., 0.])),
            View::Front => Some(([0., 1., 0., 0.], [0., 0., 1., 0.])),
            View::ThreeQuarter => None,
        };
        if orientation.is_some_and(|(right, up)| matrix[0] != right || matrix[1] != up)
            || projection.scale.world_units_per_projection_unit != 1.
        {
            report.diagnostics.push(format!(
                "{view:?}: principal orientation or source scale changed"
            ));
        }
        if projection.components.len() != proxy.components.len() {
            report
                .diagnostics
                .push(format!("{view:?}: component count mismatch"));
        }
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for component in &proxy.components {
            let matching: Vec<_> = projection
                .components
                .iter()
                .filter(|c| c.component_id == component.component_id)
                .collect();
            if matching.len() != 1 {
                report.diagnostics.push(format!(
                    "{view:?}: component {} missing/duplicate",
                    component.component_id
                ));
                continue;
            }
            let projected = matching[0];
            if projected.vertices.len() != component.vertices.len()
                || projected.depths.len() != component.vertices.len()
            {
                report.diagnostics.push(format!(
                    "{view:?}: {} vertex count mismatch",
                    component.component_id
                ));
                continue;
            }
            for (index, point) in component.vertices.iter().enumerate() {
                let expected: Vec<f64> = matrix[..3]
                    .iter()
                    .map(|row| (0..3).map(|i| row[i] * point[i]).sum())
                    .collect();
                let observed = [
                    projected.vertices[index][0],
                    projected.vertices[index][1],
                    projected.depths[index],
                ];
                let epsilon =
                    256. * f64::EPSILON * point.iter().fold(1.0_f64, |a, b| a.max(b.abs()));
                if (0..3).any(|axis| {
                    !observed[axis].is_finite() || (expected[axis] - observed[axis]).abs() > epsilon
                }) {
                    report.diagnostics.push(format!(
                        "{view:?}: {} vertex {index} disagrees with source",
                        component.component_id
                    ));
                    break;
                }
                for axis in 0..2 {
                    low[axis] = low[axis].min(expected[axis]);
                    high[axis] = high[axis].max(expected[axis]);
                }
            }
        }
        for axis in 0..2 {
            let epsilon = 256. * f64::EPSILON * low[axis].abs().max(high[axis].abs()).max(1.);
            if (projection.bounds.min[axis] - low[axis]).abs() > epsilon
                || (projection.bounds.max[axis] - high[axis]).abs() > epsilon
            {
                report.diagnostics.push(format!(
                    "{view:?}: bounds axis {axis} disagree with source extrema"
                ));
            }
        }
    }
    if !report.diagnostics.is_empty() {
        report.status = GateStatus::Fail;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_reflection_stale_revision_and_fabricated_projection_coordinates() {
        let spec = GeometrySpec::shotgun();
        let views: Vec<_> = View::all()
            .into_iter()
            .map(|v| spec.project(v).unwrap())
            .collect();
        assert_eq!(validate_projections(&spec, &views).status, GateStatus::Pass);
        let mut altered = views.clone();
        altered[0].revision += 1;
        assert_eq!(
            validate_projections(&spec, &altered).status,
            GateStatus::Fail
        );
        altered = views.clone();
        altered[0].components[0].vertices[0][0] += 0.1;
        assert_eq!(
            validate_projections(&spec, &altered).status,
            GateStatus::Fail
        );
        altered = views;
        altered[0].matrix[2][0] *= -1.;
        assert_eq!(
            validate_projections(&spec, &altered).status,
            GateStatus::Fail
        );
    }
}
