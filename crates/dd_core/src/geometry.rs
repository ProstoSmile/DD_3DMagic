//! Deterministic reference geometry, projection, and rendering.
//! Coordinates use the unit system declared by GeometrySpec: X is length, Y width, Z vertical.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as FmtWrite,
};

use image::{Rgba, RgbaImage};
use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

pub const GEOMETRY_SCHEMA_VERSION: u32 = 1;
const MAX_HISTORY: usize = 128;
const MAX_RENDER_DIMENSION: u32 = 4096;
const MAX_RENDER_PIXELS: u64 = 16_777_216;
const EPS: f64 = 1.0e-9;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    fn from_array(p: [f64; 3]) -> Self {
        Self::new(p[0], p[1], p[2])
    }
    fn array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
    fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn length(self) -> f64 {
        self.dot(self).sqrt()
    }
    fn norm(self) -> Self {
        let l = self.length();
        if l <= EPS {
            Self::ZERO
        } else {
            self.mul(1.0 / l)
        }
    }
    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    fn at(self, axis: Axis) -> f64 {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    XY,
    XZ,
    YZ,
}
impl Plane {
    fn horizontal_axis(self) -> Axis {
        match self {
            Self::XY | Self::XZ => Axis::X,
            Self::YZ => Axis::Y,
        }
    }
    fn vertical_axis(self) -> Axis {
        match self {
            Self::XY | Self::YZ => Axis::Y,
            Self::XZ => Axis::Z,
        }
    }
    fn normal_axis(self) -> Axis {
        match self {
            Self::XY => Axis::Z,
            Self::XZ => Axis::Y,
            Self::YZ => Axis::X,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveParameter {
    Radius,
    Diameter,
    Depth,
    BoxSize(Axis),
    Translation(Axis),
    Rotation(Axis),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrivingParameter {
    pub node_id: String,
    pub parameter: PrimitiveParameter,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrivingDimension {
    pub id: String,
    pub label: String,
    pub assignment: DrivingParameter,
    #[serde(default = "default_length_unit_label")]
    pub unit_label: String,
}

fn default_length_unit_label() -> String {
    "au".into()
}
impl Axis {
    fn vector(self) -> Vec3 {
        match self {
            Self::X => Vec3::new(1.0, 0.0, 0.0),
            Self::Y => Vec3::new(0.0, 1.0, 0.0),
            Self::Z => Vec3::new(0.0, 0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryUnits {
    ArtisticRelative,
    Meters,
}

impl GeometryUnits {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArtisticRelative => "artistic_relative",
            Self::Meters => "meters",
        }
    }

    pub const fn length_label(self) -> &'static str {
        match self {
            Self::ArtisticRelative => "au",
            Self::Meters => "m",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::ArtisticRelative => "artistic relative units",
            Self::Meters => "meters",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionConvention {
    FirstAngle,
    #[default]
    ThirdAngle,
}
impl ProjectionConvention {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstAngle => "first_angle",
            Self::ThirdAngle => "third_angle",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TechnicalSheetLayout {
    #[default]
    ThreeViewPlusQuarter,
    OrthographicThree,
}
impl TechnicalSheetLayout {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreeViewPlusQuarter => "three_view_plus_quarter",
            Self::OrthographicThree => "orthographic_three",
        }
    }
    pub const fn views(self) -> &'static [View] {
        match self {
            Self::ThreeViewPlusQuarter => &[View::Side, View::Top, View::Front, View::ThreeQuarter],
            Self::OrthographicThree => &[View::Side, View::Top, View::Front],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterBasis {
    Artistic,
    Estimated,
    Measured,
    UserAuthored,
    Derived,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterProvenance {
    pub path: String,
    pub basis: ParameterBasis,
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }
}
impl Transform {
    fn valid(self) -> bool {
        self.translation.finite()
            && self.rotation.finite()
            && self.scale.finite()
            && self.scale.x.abs() > EPS
            && self.scale.y.abs() > EPS
            && self.scale.z.abs() > EPS
    }
    /// Euler radians, applied X, then Y, then Z.
    fn point(self, p: Vec3) -> Vec3 {
        let mut p = Vec3::new(p.x * self.scale.x, p.y * self.scale.y, p.z * self.scale.z);
        let (s, c) = self.rotation.x.sin_cos();
        p = Vec3::new(p.x, p.y * c - p.z * s, p.y * s + p.z * c);
        let (s, c) = self.rotation.y.sin_cos();
        p = Vec3::new(p.x * c + p.z * s, p.y, -p.x * s + p.z * c);
        let (s, c) = self.rotation.z.sin_cos();
        p = Vec3::new(p.x * c - p.y * s, p.x * s + p.y * c, p.z);
        p.add(self.translation)
    }
    fn vector(self, v: Vec3) -> Vec3 {
        self.point(v).sub(self.translation)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    Wood,
    Steel,
    DarkSteel,
    Brass,
    Rubber,
    Leather,
}
impl Material {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wood => "wood",
            Self::Steel => "steel",
            Self::DarkSteel => "dark_steel",
            Self::Brass => "brass",
            Self::Rubber => "rubber",
            Self::Leather => "leather",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "steel" => Self::Steel,
            "dark_steel" => Self::DarkSteel,
            "brass" => Self::Brass,
            "rubber" => Self::Rubber,
            "leather" => Self::Leather,
            _ => Self::Wood,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "parameters", rename_all = "snake_case")]
pub enum Primitive {
    Box {
        size: Vec3,
    },
    Cylinder {
        radius: f64,
        depth: f64,
        axis: Axis,
        segments: u16,
    },
    Sphere {
        radius: f64,
        longitude_segments: u16,
        latitude_segments: u16,
    },
    ExtrudedProfile {
        profile: Vec<[f64; 2]>,
        depth: f64,
        axis: Axis,
    },
    Point {
        position: Vec3,
    },
    Line {
        start: Vec3,
        end: Vec3,
    },
    Circle {
        radius: f64,
        axis: Axis,
        segments: u16,
    },
    Arc {
        radius: f64,
        axis: Axis,
        start_radians: f64,
        end_radians: f64,
        segments: u16,
    },
    Bezier {
        control_points: [Vec3; 4],
        segments: u16,
    },
    Revolve {
        profile: Vec<[f64; 2]>,
        axis: Axis,
        segments: u16,
    },
    TriangleMesh {
        vertices: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
    },
    PolygonMesh {
        vertices: Vec<Vec3>,
        faces: Vec<Vec<u32>>,
    },
    Unsupported {
        kind: String,
        description: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeometryNode {
    pub id: String,
    pub label: String,
    pub transform: Transform,
    pub primitive: Option<Primitive>,
    pub material: Material,
    pub children: Vec<GeometryNode>,
}
impl GeometryNode {
    pub fn group(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            transform: Transform::default(),
            primitive: None,
            material: Material::Steel,
            children: Vec::new(),
        }
    }
    pub fn primitive(
        id: impl Into<String>,
        label: impl Into<String>,
        material: Material,
        primitive: Primitive,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            transform: Transform::default(),
            primitive: Some(primitive),
            material,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeometrySpec {
    pub schema_version: u32,
    pub asset_uid: String,
    pub revision: u64,
    pub units: GeometryUnits,
    pub name: String,
    #[serde(default)]
    pub projection_convention: ProjectionConvention,
    #[serde(default)]
    pub technical_sheet_layout: TechnicalSheetLayout,
    #[serde(default)]
    pub parameter_provenance: Vec<ParameterProvenance>,
    pub root: GeometryNode,
    pub dimensions: Vec<Dimension>,
    #[serde(default)]
    pub driving_dimensions: Vec<DrivingDimension>,
    pub constraints: Vec<GeometryConstraint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub id: String,
    pub label: String,
    pub target: DimensionTarget,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DimensionTarget {
    OverallExtent {
        axis: Axis,
    },
    NodeExtent {
        node_id: String,
        axis: Axis,
    },
    NodeDistance {
        first_node: String,
        second_node: String,
    },
    Radius {
        node_id: String,
    },
    Diameter {
        node_id: String,
    },
    Angle {
        node_id: String,
        axis: Axis,
    },
    CenterCoordinate {
        node_id: String,
        axis: Axis,
    },
    Unsupported {
        description: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredDimension {
    pub id: String,
    pub label: String,
    pub value: Option<f64>,
    pub units: GeometryUnits,
    pub unit_label: String,
    pub status: MeasurementStatus,
    pub detail: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementStatus {
    Measured,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GeometryConstraint {
    ExtentRange {
        id: String,
        node_id: Option<String>,
        axis: Axis,
        minimum: f64,
        maximum: f64,
    },
    NodeDistanceRange {
        id: String,
        first_node: String,
        second_node: String,
        minimum: f64,
        maximum: f64,
    },
    AxesParallel {
        id: String,
        first_node: String,
        first_axis: Axis,
        second_node: String,
        second_axis: Axis,
        tolerance: f64,
    },
    Coincident {
        id: String,
        first_node: String,
        second_node: String,
        tolerance: f64,
    },
    HorizontalInPlane {
        id: String,
        first_node: String,
        second_node: String,
        plane: Plane,
        tolerance: f64,
    },
    VerticalInPlane {
        id: String,
        first_node: String,
        second_node: String,
        plane: Plane,
        tolerance: f64,
    },
    AxesPerpendicular {
        id: String,
        first_node: String,
        first_axis: Axis,
        second_node: String,
        second_axis: Axis,
        tolerance: f64,
    },
    EqualLength {
        id: String,
        first_node: String,
        first_axis: Axis,
        second_node: String,
        second_axis: Axis,
        tolerance: f64,
    },
    EqualRadius {
        id: String,
        first_node: String,
        second_node: String,
        tolerance: f64,
    },
    FixedDistance {
        id: String,
        first_node: String,
        second_node: String,
        distance: f64,
        tolerance: f64,
    },
    FixedAngle {
        id: String,
        first_node: String,
        first_axis: Axis,
        second_node: String,
        second_axis: Axis,
        angle_degrees: f64,
        tolerance_degrees: f64,
    },
    SymmetricAcrossPlane {
        id: String,
        first_node: String,
        second_node: String,
        plane: Plane,
        center_node: Option<String>,
        tolerance: f64,
    },
    Concentric {
        id: String,
        first_node: String,
        first_axis: Axis,
        second_node: String,
        second_axis: Axis,
        tolerance: f64,
    },
    ComponentAlignment {
        id: String,
        first_node: String,
        second_node: String,
        axis: Axis,
        tolerance: f64,
    },
    RelativePosition {
        id: String,
        first_node: String,
        second_node: String,
        axis: Axis,
        relation: PositionRelation,
    },
    Unsupported {
        id: String,
        description: String,
    },
}
impl GeometryConstraint {
    fn id(&self) -> &str {
        match self {
            Self::ExtentRange { id, .. }
            | Self::NodeDistanceRange { id, .. }
            | Self::AxesParallel { id, .. }
            | Self::Coincident { id, .. }
            | Self::HorizontalInPlane { id, .. }
            | Self::VerticalInPlane { id, .. }
            | Self::AxesPerpendicular { id, .. }
            | Self::EqualLength { id, .. }
            | Self::EqualRadius { id, .. }
            | Self::FixedDistance { id, .. }
            | Self::FixedAngle { id, .. }
            | Self::SymmetricAcrossPlane { id, .. }
            | Self::Concentric { id, .. }
            | Self::ComponentAlignment { id, .. }
            | Self::RelativePosition { id, .. }
            | Self::Unsupported { id, .. } => id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionRelation {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
    Draft,
    Unsupported,
    Invalid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Info,
    Warning,
    Unsupported,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub status: DiagnosticStatus,
    pub message: String,
    pub node_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub status: ValidationStatus,
    pub diagnostics: Vec<Diagnostic>,
}
impl ValidationReport {
    /// Drafts are structurally valid but cannot be projected.
    pub fn is_valid(&self) -> bool {
        self.status != ValidationStatus::Invalid
    }
    pub fn is_ready(&self) -> bool {
        self.status == ValidationStatus::Valid
    }
}

#[derive(Debug, Error)]
pub enum GeometryError {
    #[error("geometry is invalid: {0}")]
    Invalid(String),
    #[error("geometry is underdefined: {0}")]
    Draft(String),
    #[error("geometry operation is unsupported: {0}")]
    Unsupported(String),
    #[error("node not found: {0}")]
    NodeNotFound(String),
    #[error("geometry command conflicts with the document: {0}")]
    Conflict(String),
    #[error("render dimensions must be non-zero and at most 4096px per side")]
    RenderSize,
    #[error("geometry contains no renderable points")]
    EmptyGeometry,
    #[error("revision counter overflow")]
    RevisionOverflow,
}

impl GeometrySpec {
    /// Deterministic fictional long-shotgun fixture; lengths are not real-world measurements.
    pub fn shotgun() -> Self {
        let mut root = GeometryNode::group("shotgun", "Stylized long shotgun");
        let mut parts = Vec::new();
        let mut node = GeometryNode::primitive(
            "stock",
            "Carved walnut stock",
            Material::Wood,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-1.46, -0.40],
                    [-1.50, -0.37],
                    [-1.53, -0.31],
                    [-1.55, -0.21],
                    [-1.55, -0.08],
                    [-1.54, 0.05],
                    [-1.51, 0.17],
                    [-1.47, 0.25],
                    [-1.40, 0.30],
                    [-1.30, 0.33],
                    [-1.18, 0.35],
                    [-1.06, 0.36],
                    [-0.95, 0.35],
                    [-0.84, 0.32],
                    [-0.75, 0.29],
                    [-0.67, 0.28],
                    [-0.60, 0.30],
                    [-0.53, 0.33],
                    [-0.45, 0.35],
                    [-0.37, 0.35],
                    [-0.29, 0.32],
                    [-0.23, 0.29],
                    [-0.18, 0.26],
                    [-0.10, 0.25],
                    [0.08, 0.25],
                    [0.25, 0.25],
                    [0.37, 0.23],
                    [0.42, 0.19],
                    [0.42, -0.16],
                    [0.39, -0.23],
                    [0.31, -0.27],
                    [0.21, -0.29],
                    [0.10, -0.28],
                    [-0.02, -0.27],
                    [-0.12, -0.29],
                    [-0.20, -0.34],
                    [-0.25, -0.41],
                    [-0.29, -0.48],
                    [-0.35, -0.52],
                    [-0.44, -0.53],
                    [-0.54, -0.51],
                    [-0.65, -0.47],
                    [-0.78, -0.43],
                    [-0.92, -0.41],
                    [-1.07, -0.40],
                    [-1.21, -0.41],
                    [-1.33, -0.42],
                    [-1.42, -0.41],
                ],
                depth: 0.38,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(-1.35, 0.0, 0.0);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "receiver",
            "Steel receiver",
            Material::Steel,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-0.61, -0.10],
                    [-0.60, -0.17],
                    [-0.56, -0.21],
                    [-0.49, -0.24],
                    [-0.39, -0.24],
                    [-0.30, -0.23],
                    [-0.23, -0.25],
                    [-0.17, -0.30],
                    [-0.10, -0.32],
                    [0.12, -0.32],
                    [0.20, -0.29],
                    [0.27, -0.25],
                    [0.40, -0.24],
                    [0.50, -0.22],
                    [0.57, -0.18],
                    [0.61, -0.11],
                    [0.61, 0.12],
                    [0.59, 0.18],
                    [0.54, 0.22],
                    [0.47, 0.25],
                    [0.35, 0.28],
                    [0.23, 0.28],
                    [0.14, 0.25],
                    [-0.10, 0.25],
                    [-0.21, 0.28],
                    [-0.42, 0.30],
                    [-0.52, 0.28],
                    [-0.58, 0.23],
                    [-0.61, 0.15],
                ],
                depth: 0.54,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(-0.35, 0.0, 0.07);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "receiver-side-plate",
            "Receiver side plate",
            Material::DarkSteel,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-0.42, -0.06],
                    [-0.40, -0.12],
                    [-0.35, -0.15],
                    [-0.18, -0.16],
                    [0.20, -0.16],
                    [0.38, -0.14],
                    [0.43, -0.10],
                    [0.43, 0.07],
                    [0.40, 0.12],
                    [0.35, 0.15],
                    [0.18, 0.16],
                    [-0.19, 0.16],
                    [-0.37, 0.14],
                    [-0.42, 0.09],
                ],
                depth: 0.045,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(-0.37, -0.292, 0.085);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "barrel",
            "Long steel barrel",
            Material::DarkSteel,
            Primitive::Cylinder {
                radius: 0.095,
                depth: 4.92,
                axis: Axis::X,
                segments: 48,
            },
        );
        node.transform.translation = Vec3::new(2.65, 0.0, 0.18);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "muzzle",
            "Muzzle collar",
            Material::Steel,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-0.09, -0.07],
                    [-0.075, -0.12],
                    [-0.04, -0.14],
                    [0.04, -0.14],
                    [0.075, -0.12],
                    [0.09, -0.07],
                    [0.09, 0.07],
                    [0.075, 0.12],
                    [0.04, 0.14],
                    [-0.04, 0.14],
                    [-0.075, 0.12],
                    [-0.09, 0.07],
                ],
                depth: 0.18,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(5.20, 0.0, 0.18);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "forearm",
            "Walnut forearm",
            Material::Wood,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-1.08, -0.035],
                    [-1.06, -0.09],
                    [-1.02, -0.13],
                    [-0.95, -0.16],
                    [-0.85, -0.18],
                    [-0.73, -0.18],
                    [-0.64, -0.17],
                    [-0.58, -0.19],
                    [-0.54, -0.22],
                    [-0.50, -0.19],
                    [-0.45, -0.17],
                    [-0.31, -0.16],
                    [-0.12, -0.15],
                    [0.08, -0.16],
                    [0.27, -0.18],
                    [0.45, -0.19],
                    [0.63, -0.19],
                    [0.78, -0.17],
                    [0.90, -0.14],
                    [1.00, -0.10],
                    [1.05, -0.05],
                    [1.06, 0.02],
                    [1.03, 0.08],
                    [0.98, 0.13],
                    [0.90, 0.16],
                    [0.80, 0.18],
                    [0.66, 0.19],
                    [0.48, 0.18],
                    [0.28, 0.16],
                    [0.08, 0.15],
                    [-0.14, 0.15],
                    [-0.36, 0.17],
                    [-0.57, 0.18],
                    [-0.76, 0.17],
                    [-0.91, 0.15],
                    [-1.00, 0.12],
                    [-1.05, 0.08],
                ],
                depth: 0.48,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(1.25, 0.0, -0.15);
        parts.push(node);
        for (id, label, x) in [
            ("barrel-band-rear", "Rear barrel band", 0.54),
            ("barrel-band-front", "Front barrel band", 2.18),
        ] {
            let mut node = GeometryNode::primitive(
                id,
                label,
                Material::Brass,
                Primitive::Cylinder {
                    radius: 0.106,
                    depth: 0.075,
                    axis: Axis::X,
                    segments: 32,
                },
            );
            node.transform.translation = Vec3::new(x, 0.0, 0.18);
            parts.push(node);
        }
        for (id, label, x, z) in [
            ("trigger-guard-front", "Trigger guard front", -0.02, -0.34),
            ("trigger-guard-back", "Trigger guard rear", -0.72, -0.34),
        ] {
            let mut node = GeometryNode::primitive(
                id,
                label,
                Material::DarkSteel,
                Primitive::ExtrudedProfile {
                    profile: vec![
                        [-0.045, -0.15],
                        [-0.042, -0.18],
                        [-0.025, -0.20],
                        [0.025, -0.20],
                        [0.042, -0.18],
                        [0.045, 0.14],
                        [0.032, 0.17],
                        [0.0, 0.18],
                        [-0.032, 0.17],
                        [-0.045, 0.14],
                    ],
                    depth: 0.09,
                    axis: Axis::Y,
                },
            );
            node.transform.translation = Vec3::new(x, 0.0, z);
            node.transform.rotation.y = if id == "trigger-guard-front" {
                0.14
            } else {
                -0.14
            };
            parts.push(node);
        }
        let mut node = GeometryNode::primitive(
            "trigger-guard-bottom",
            "Trigger guard lower bar",
            Material::DarkSteel,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-0.34, -0.02],
                    [-0.32, -0.05],
                    [-0.22, -0.07],
                    [-0.08, -0.075],
                    [0.08, -0.065],
                    [0.22, -0.04],
                    [0.32, -0.02],
                    [0.34, 0.01],
                    [0.33, 0.04],
                    [0.20, 0.055],
                    [0.0, 0.06],
                    [-0.18, 0.05],
                    [-0.32, 0.03],
                    [-0.34, 0.0],
                ],
                depth: 0.09,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(-0.37, 0.0, -0.49);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "trigger",
            "Trigger",
            Material::Steel,
            Primitive::ExtrudedProfile {
                profile: vec![
                    [-0.025, 0.09],
                    [-0.018, 0.04],
                    [-0.008, 0.00],
                    [0.008, -0.04],
                    [0.03, -0.08],
                    [0.055, -0.09],
                    [0.042, -0.05],
                    [0.023, -0.01],
                    [0.008, 0.04],
                    [0.015, 0.09],
                ],
                depth: 0.065,
                axis: Axis::Y,
            },
        );
        node.transform.translation = Vec3::new(-0.37, 0.0, -0.34);
        node.transform.rotation.y = -0.18;
        parts.push(node);
        for (id, label, x, z) in [
            ("rear-sight", "Rear sight", 0.05, 0.30),
            ("front-sight", "Front sight", 4.78, 0.31),
        ] {
            let mut node = GeometryNode::primitive(
                id,
                label,
                Material::DarkSteel,
                Primitive::ExtrudedProfile {
                    profile: vec![
                        [-0.07, -0.06],
                        [-0.065, -0.09],
                        [-0.035, -0.10],
                        [0.035, -0.10],
                        [0.065, -0.08],
                        [0.07, -0.03],
                        [0.045, 0.015],
                        [0.02, 0.045],
                        [-0.02, 0.045],
                        [-0.045, 0.015],
                    ],
                    depth: 0.11,
                    axis: Axis::Y,
                },
            );
            node.transform.translation = Vec3::new(x, 0.0, z);
            parts.push(node);
        }
        let mut node = GeometryNode::primitive(
            "sight-bead",
            "Brass sight bead",
            Material::Brass,
            Primitive::Sphere {
                radius: 0.055,
                longitude_segments: 20,
                latitude_segments: 10,
            },
        );
        node.transform.translation = Vec3::new(4.82, 0.0, 0.40);
        parts.push(node);
        let mut node = GeometryNode::primitive(
            "lever",
            "Receiver lever",
            Material::Steel,
            Primitive::Cylinder {
                radius: 0.045,
                depth: 0.28,
                axis: Axis::Z,
                segments: 24,
            },
        );
        node.transform.translation = Vec3::new(-0.45, -0.34, -0.22);
        parts.push(node);
        for (id, x, z) in [
            ("stock-screw", -1.05, 0.04),
            ("receiver-screw-front", -0.04, 0.13),
            ("receiver-screw-back", -0.68, 0.13),
        ] {
            let mut node = GeometryNode::primitive(
                id,
                "Brass fastener",
                Material::Brass,
                Primitive::Cylinder {
                    radius: 0.055,
                    depth: 0.025,
                    axis: Axis::Y,
                    segments: 20,
                },
            );
            node.transform.translation = Vec3::new(x, -0.305, z);
            parts.push(node);
        }
        root.children = parts;
        Self {
            schema_version:GEOMETRY_SCHEMA_VERSION, asset_uid:"fictional-long-shotgun".into(), revision:1,
            units:GeometryUnits::ArtisticRelative, name:"Stylized Long Shotgun".into(),
            projection_convention:ProjectionConvention::ThirdAngle,
            technical_sheet_layout:TechnicalSheetLayout::ThreeViewPlusQuarter,
            parameter_provenance:vec![
                ParameterProvenance{path:"*".into(),basis:ParameterBasis::Artistic,explanation:"Fictional concept proportions in artistic relative units; no real-world measurements are claimed.".into()},
                ParameterProvenance{path:"nodes.barrel.primitive.depth".into(),basis:ParameterBasis::Artistic,explanation:"Fictional shotgun concept proportion in artistic relative units; not a real-world measurement.".into()},
            ],
            root,
            dimensions:vec![
                Dimension{id:"overall-length".into(),label:"Overall length".into(),target:DimensionTarget::OverallExtent{axis:Axis::X}},
                Dimension{id:"barrel-diameter".into(),label:"Barrel diameter".into(),target:DimensionTarget::NodeExtent{node_id:"barrel".into(),axis:Axis::Y}},
                Dimension{id:"stock-to-muzzle".into(),label:"Stock to muzzle distance".into(),target:DimensionTarget::NodeDistance{first_node:"stock".into(),second_node:"muzzle".into()}},
            ],
            driving_dimensions: vec![DrivingDimension {
                id: "barrel-depth".into(),
                label: "Barrel depth".into(),
                assignment: DrivingParameter {
                    node_id: "barrel".into(),
                    parameter: PrimitiveParameter::Depth,
                    value: 4.92,
                },
                unit_label: "au".into(),
            }],
            constraints:vec![
                GeometryConstraint::ExtentRange{id:"long-form-factor".into(),node_id:None,axis:Axis::X,minimum:8.0,maximum:9.5},
                GeometryConstraint::AxesParallel{id:"barrel-aligned-to-receiver".into(),first_node:"barrel".into(),first_axis:Axis::X,second_node:"receiver".into(),second_axis:Axis::X,tolerance:0.01},
            ],
        }
    }

    pub fn validate(&self) -> ValidationReport {
        let mut d = Vec::new();
        if self.schema_version != GEOMETRY_SCHEMA_VERSION {
            d.push(Diagnostic {
                code: "unsupported_schema_version".into(),
                status: DiagnosticStatus::Unsupported,
                message: format!(
                    "Schema {} is unsupported; current schema is {GEOMETRY_SCHEMA_VERSION}.",
                    self.schema_version
                ),
                node_id: None,
            });
        }
        if !valid_id(&self.asset_uid) {
            d.push(diag(
                "invalid_asset_uid",
                "Asset UID must use letters, numbers, dot, dash, or underscore.",
                None,
            ));
        }
        if self.revision == 0 {
            d.push(diag("invalid_revision", "Revision must start at 1.", None));
        }
        if self.name.trim().is_empty() {
            d.push(diag("empty_name", "Geometry name cannot be empty.", None));
        }
        for source in &self.parameter_provenance {
            if source.path.trim().is_empty() || source.explanation.trim().is_empty() {
                d.push(diag(
                    "invalid_parameter_provenance",
                    "Parameter provenance needs a path and explanation.",
                    None,
                ));
            }
        }
        let mut nodes = BTreeMap::new();
        validate_node(&self.root, &mut nodes, &mut d);
        let mut dimension_ids = BTreeSet::new();
        for item in &self.dimensions {
            if !valid_id(&item.id) || !dimension_ids.insert(item.id.as_str()) {
                d.push(diag(
                    "invalid_dimension_id",
                    &format!("Dimension ID '{}' is invalid or duplicated.", item.id),
                    None,
                ));
            }
            if item.label.trim().is_empty() {
                d.push(diag(
                    "empty_dimension_label",
                    &format!("Dimension '{}' needs a label.", item.id),
                    None,
                ));
            }
            match &item.target {
                DimensionTarget::NodeExtent { node_id, .. } if !nodes.contains_key(node_id) => d
                    .push(diag(
                        "missing_dimension_node",
                        &format!(
                            "Dimension '{}' refers to missing node '{node_id}'.",
                            item.id
                        ),
                        Some(node_id),
                    )),
                DimensionTarget::NodeDistance {
                    first_node,
                    second_node,
                } => {
                    for id in [first_node, second_node] {
                        if !nodes.contains_key(id) {
                            d.push(diag(
                                "missing_dimension_node",
                                &format!("Dimension '{}' refers to missing node '{id}'.", item.id),
                                Some(id),
                            ));
                        }
                    }
                }
                DimensionTarget::Radius { node_id }
                | DimensionTarget::Diameter { node_id }
                | DimensionTarget::Angle { node_id, .. }
                | DimensionTarget::CenterCoordinate { node_id, .. }
                    if !nodes.contains_key(node_id) =>
                {
                    d.push(diag(
                        "missing_dimension_node",
                        &format!(
                            "Dimension '{}' refers to missing node '{node_id}'.",
                            item.id
                        ),
                        Some(node_id),
                    ));
                }
                DimensionTarget::Radius { node_id } => {
                    if let Some(node) = nodes.get(node_id)
                        && let Err(GeometryError::Unsupported(message)) =
                            read_primitive_parameter(node, PrimitiveParameter::Radius)
                    {
                        d.push(Diagnostic {
                            code: "unsupported_dimension_target".into(),
                            status: DiagnosticStatus::Unsupported,
                            message: format!("Dimension '{}': {message}", item.id),
                            node_id: Some(node_id.clone()),
                        });
                    }
                }
                DimensionTarget::Diameter { node_id } => {
                    if let Some(node) = nodes.get(node_id)
                        && let Err(GeometryError::Unsupported(message)) =
                            read_primitive_parameter(node, PrimitiveParameter::Diameter)
                    {
                        d.push(Diagnostic {
                            code: "unsupported_dimension_target".into(),
                            status: DiagnosticStatus::Unsupported,
                            message: format!("Dimension '{}': {message}", item.id),
                            node_id: Some(node_id.clone()),
                        });
                    }
                }
                DimensionTarget::Unsupported { description } => d.push(Diagnostic {
                    code: "unsupported_dimension".into(),
                    status: DiagnosticStatus::Unsupported,
                    message: format!("Dimension '{}': {description}", item.id),
                    node_id: None,
                }),
                _ => {}
            }
        }
        let mut driving_targets = BTreeMap::<(String, ParameterSlot), f64>::new();
        for dimension in &self.driving_dimensions {
            if !valid_id(&dimension.id) || !dimension_ids.insert(dimension.id.as_str()) {
                d.push(diag(
                    "invalid_dimension_id",
                    &format!(
                        "Driving dimension ID '{}' is invalid or duplicated.",
                        dimension.id
                    ),
                    None,
                ));
            }
            if dimension.label.trim().is_empty() {
                d.push(diag(
                    "empty_dimension_label",
                    &format!("Driving dimension '{}' needs a label.", dimension.id),
                    None,
                ));
            }
            if dimension.unit_label.trim().is_empty() {
                d.push(diag(
                    "empty_dimension_unit_label",
                    &format!("Driving dimension '{}' needs a unit label.", dimension.id),
                    None,
                ));
            }
            let assignment = &dimension.assignment;
            if !nodes.contains_key(&assignment.node_id) {
                d.push(diag(
                    "missing_driving_dimension_node",
                    &format!(
                        "Driving dimension '{}' refers to missing node '{}'.",
                        dimension.id, assignment.node_id
                    ),
                    Some(&assignment.node_id),
                ));
                continue;
            }
            match normalized_assignment(assignment) {
                Ok((slot, value)) => {
                    let key = (assignment.node_id.clone(), slot);
                    if let Some(previous) = driving_targets.insert(key, value)
                        && (previous - value).abs() > EPS
                    {
                        d.push(diag(
                            "conflicting_driving_dimensions",
                            &format!(
                                "Driving dimensions assign conflicting values to '{}'.",
                                assignment.node_id
                            ),
                            Some(&assignment.node_id),
                        ));
                    }
                }
                Err(error) => d.push(diag(
                    "invalid_driving_dimension_value",
                    &format!("Driving dimension '{}': {error}", dimension.id),
                    Some(&assignment.node_id),
                )),
            }
            if let Some(node) = find_node(&self.root, &assignment.node_id)
                && let Err(error) = read_primitive_parameter(node, assignment.parameter)
            {
                match error {
                    GeometryError::Unsupported(message) => d.push(Diagnostic {
                        code: "unsupported_driving_parameter".into(),
                        status: DiagnosticStatus::Unsupported,
                        message,
                        node_id: Some(assignment.node_id.clone()),
                    }),
                    error => d.push(diag(
                        "invalid_driving_parameter",
                        &error.to_string(),
                        Some(&assignment.node_id),
                    )),
                }
            }
        }
        let mut constraint_ids = BTreeSet::new();
        for c in &self.constraints {
            let id = c.id();
            if !valid_id(id) || !constraint_ids.insert(id) {
                d.push(diag(
                    "invalid_constraint_id",
                    &format!("Constraint ID '{id}' is invalid or duplicated."),
                    None,
                ));
            }
            match c {
                GeometryConstraint::ExtentRange {
                    node_id,
                    minimum,
                    maximum,
                    ..
                } => {
                    range_check(*minimum, *maximum, id, &mut d);
                    if let Some(id) = node_id
                        && !nodes.contains_key(id)
                    {
                        d.push(diag(
                            "missing_constraint_node",
                            &format!("Constraint '{id}' refers to a missing node."),
                            Some(id),
                        ));
                    }
                }
                GeometryConstraint::NodeDistanceRange {
                    first_node,
                    second_node,
                    minimum,
                    maximum,
                    ..
                } => {
                    range_check(*minimum, *maximum, id, &mut d);
                    for node in [first_node, second_node] {
                        if !nodes.contains_key(node) {
                            d.push(diag(
                                "missing_constraint_node",
                                &format!("Constraint '{id}' refers to missing node '{node}'."),
                                Some(node),
                            ));
                        }
                    }
                    if first_node == second_node {
                        d.push(diag(
                            "constraint_conflict",
                            &format!("Constraint '{id}' needs two distinct nodes."),
                            Some(first_node),
                        ));
                    }
                }
                GeometryConstraint::AxesParallel {
                    first_node,
                    second_node,
                    tolerance,
                    ..
                } => {
                    if !tolerance.is_finite() || !(0.0..=1.0).contains(tolerance) {
                        d.push(diag(
                            "invalid_tolerance",
                            &format!("Constraint '{id}' tolerance must be in [0,1]."),
                            None,
                        ));
                    }
                    for node in [first_node, second_node] {
                        if !nodes.contains_key(node) {
                            d.push(diag(
                                "missing_constraint_node",
                                &format!("Constraint '{id}' refers to missing node '{node}'."),
                                Some(node),
                            ));
                        }
                    }
                    if first_node == second_node {
                        d.push(diag(
                            "constraint_conflict",
                            &format!("Constraint '{id}' needs two distinct nodes."),
                            Some(first_node),
                        ));
                    }
                }
                GeometryConstraint::Unsupported { description, .. } => d.push(Diagnostic {
                    code: "unsupported_constraint".into(),
                    status: DiagnosticStatus::Unsupported,
                    message: format!("Constraint '{id}': {description}"),
                    node_id: None,
                }),
                relation => validate_relation_shape(relation, &nodes, &mut d),
            }
        }
        if d.iter().any(|x| x.status == DiagnosticStatus::Error) {
            return ValidationReport {
                status: ValidationStatus::Invalid,
                diagnostics: d,
            };
        }
        if d.iter().any(|x| x.status == DiagnosticStatus::Unsupported) {
            return ValidationReport {
                status: ValidationStatus::Unsupported,
                diagnostics: d,
            };
        }
        if d.iter().any(|x| x.status == DiagnosticStatus::Warning) {
            return ValidationReport {
                status: ValidationStatus::Draft,
                diagnostics: d,
            };
        }
        match self.build_proxy_unchecked() {
            Ok(proxy) => match self.check_constraints(&proxy) {
                Err(GeometryError::Conflict(message)) => {
                    d.push(diag("constraint_conflict", &message, None));
                    ValidationReport {
                        status: ValidationStatus::Invalid,
                        diagnostics: d,
                    }
                }
                Err(GeometryError::Unsupported(message)) => {
                    d.push(Diagnostic {
                        code: "unsupported_constraint".into(),
                        status: DiagnosticStatus::Unsupported,
                        message,
                        node_id: None,
                    });
                    ValidationReport {
                        status: ValidationStatus::Unsupported,
                        diagnostics: d,
                    }
                }
                Err(error) => {
                    d.push(diag("geometry_build_failed", &error.to_string(), None));
                    ValidationReport {
                        status: ValidationStatus::Invalid,
                        diagnostics: d,
                    }
                }
                Ok(()) if proxy.components.is_empty() => {
                    d.push(Diagnostic {
                        code: "empty_geometry_draft".into(),
                        status: DiagnosticStatus::Warning,
                        message: "The document has no geometry.".into(),
                        node_id: Some(self.root.id.clone()),
                    });
                    ValidationReport {
                        status: ValidationStatus::Draft,
                        diagnostics: d,
                    }
                }
                Ok(()) => ValidationReport {
                    status: ValidationStatus::Valid,
                    diagnostics: d,
                },
            },
            Err(GeometryError::Unsupported(message)) => {
                d.push(Diagnostic {
                    code: "unsupported_geometry".into(),
                    status: DiagnosticStatus::Unsupported,
                    message,
                    node_id: None,
                });
                ValidationReport {
                    status: ValidationStatus::Unsupported,
                    diagnostics: d,
                }
            }
            Err(GeometryError::Draft(message)) => {
                d.push(Diagnostic {
                    code: "underdefined_geometry".into(),
                    status: DiagnosticStatus::Warning,
                    message,
                    node_id: None,
                });
                ValidationReport {
                    status: ValidationStatus::Draft,
                    diagnostics: d,
                }
            }
            Err(error) => {
                d.push(diag("geometry_build_failed", &error.to_string(), None));
                ValidationReport {
                    status: ValidationStatus::Invalid,
                    diagnostics: d,
                }
            }
        }
    }

    pub fn proxy(&self) -> Result<Proxy, GeometryError> {
        let r = self.validate();
        match r.status {
            ValidationStatus::Valid => self.build_proxy_unchecked(),
            ValidationStatus::Draft => Err(GeometryError::Draft(report_messages(&r))),
            ValidationStatus::Unsupported => Err(GeometryError::Unsupported(report_messages(&r))),
            ValidationStatus::Invalid => Err(GeometryError::Invalid(report_messages(&r))),
        }
    }
    pub fn project(&self, view: View) -> Result<Projection, GeometryError> {
        project_proxy(&self.proxy()?, view, self.projection_convention)
    }
    pub fn measure_dimensions(&self) -> Result<Vec<MeasuredDimension>, GeometryError> {
        let proxy = self.proxy()?;
        let mut measurements = self
            .dimensions
            .iter()
            .map(|d| match dimension_value(&d.target, self, &proxy) {
                Ok(v) => Ok(MeasuredDimension {
                    id: d.id.clone(),
                    label: d.label.clone(),
                    value: Some(v),
                    units: self.units,
                    unit_label: dimension_unit_label(&d.target, self.units).into(),
                    status: MeasurementStatus::Measured,
                    detail: None,
                }),
                Err(GeometryError::Unsupported(msg)) => Ok(MeasuredDimension {
                    id: d.id.clone(),
                    label: d.label.clone(),
                    value: None,
                    units: self.units,
                    unit_label: dimension_unit_label(&d.target, self.units).into(),
                    status: MeasurementStatus::Unsupported,
                    detail: Some(msg),
                }),
                Err(e) => Err(e),
            })
            .collect::<Result<Vec<_>, _>>()?;
        measurements.extend(
            self.driving_dimensions
                .iter()
                .map(|dimension| MeasuredDimension {
                    id: dimension.id.clone(),
                    label: dimension.label.clone(),
                    value: Some(dimension.assignment.value),
                    units: self.units,
                    unit_label: primitive_parameter_unit_label(
                        dimension.assignment.parameter,
                        self.units,
                    )
                    .into(),
                    status: MeasurementStatus::Measured,
                    detail: None,
                }),
        );
        Ok(measurements)
    }
    fn build_proxy_unchecked(&self) -> Result<Proxy, GeometryError> {
        let (mut components, mut node_bounds) = (Vec::new(), BTreeMap::new());
        collect_proxy(
            &self.root,
            &mut Vec::new(),
            &mut components,
            &mut node_bounds,
        )?;
        let points = components
            .iter()
            .flat_map(|c| c.vertices.iter().copied())
            .map(Vec3::from_array)
            .collect::<Vec<_>>();
        let bounds = Bounds3::points(&points).ok_or(GeometryError::EmptyGeometry)?;
        Ok(Proxy {
            schema_version: self.schema_version,
            asset_uid: self.asset_uid.clone(),
            revision: self.revision,
            units: self.units,
            components,
            bounds,
            node_bounds,
        })
    }
    fn check_constraints(&self, proxy: &Proxy) -> Result<(), GeometryError> {
        for c in &self.constraints {
            match c {
                GeometryConstraint::ExtentRange {
                    node_id,
                    axis,
                    minimum,
                    maximum,
                    ..
                } => {
                    let b = match node_id {
                        Some(id) => proxy
                            .node_bounds
                            .get(id)
                            .ok_or_else(|| GeometryError::NodeNotFound(id.clone()))?,
                        None => &proxy.bounds,
                    };
                    let n = b.extent().at(*axis);
                    if n + EPS < *minimum || n - EPS > *maximum {
                        return Err(GeometryError::Conflict(format!(
                            "constraint '{}' extent {n:.4} is outside [{minimum:.4},{maximum:.4}]",
                            c.id()
                        )));
                    }
                }
                GeometryConstraint::NodeDistanceRange {
                    first_node,
                    second_node,
                    minimum,
                    maximum,
                    ..
                } => {
                    let a = proxy
                        .node_bounds
                        .get(first_node)
                        .ok_or_else(|| GeometryError::NodeNotFound(first_node.clone()))?
                        .center();
                    let b = proxy
                        .node_bounds
                        .get(second_node)
                        .ok_or_else(|| GeometryError::NodeNotFound(second_node.clone()))?
                        .center();
                    let n = a.sub(b).length();
                    if n + EPS < *minimum || n - EPS > *maximum {
                        return Err(GeometryError::Conflict(format!(
                            "constraint '{}' distance {n:.4} is outside [{minimum:.4},{maximum:.4}]",
                            c.id()
                        )));
                    }
                }
                GeometryConstraint::AxesParallel {
                    first_node,
                    first_axis,
                    second_node,
                    second_axis,
                    tolerance,
                    ..
                } => {
                    let a = world_axis(&self.root, first_node, *first_axis)?;
                    let b = world_axis(&self.root, second_node, *second_axis)?;
                    let dot = a.dot(b).abs();
                    if 1.0 - dot > *tolerance {
                        return Err(GeometryError::Conflict(format!(
                            "constraint '{}' axes differ (dot {dot:.4}, tolerance {tolerance:.4})",
                            c.id()
                        )));
                    }
                }
                GeometryConstraint::Unsupported { id, description } => {
                    return Err(GeometryError::Unsupported(format!(
                        "constraint '{id}': {description}"
                    )));
                }
                relation => check_relation(relation, self, proxy)?,
            }
        }
        check_driving_dimensions(&self.root, &self.driving_dimensions)?;
        Ok(())
    }
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
fn diag(code: &str, msg: &str, node: Option<&str>) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        status: DiagnosticStatus::Error,
        message: msg.into(),
        node_id: node.map(str::to_owned),
    }
}
fn range_check(min: f64, max: f64, id: &str, d: &mut Vec<Diagnostic>) {
    if !min.is_finite() || !max.is_finite() || min < 0.0 || max < min {
        d.push(diag(
            "invalid_constraint_range",
            &format!("Constraint '{id}' needs a finite non-negative range."),
            None,
        ));
    }
}
fn report_messages(r: &ValidationReport) -> String {
    r.diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_relation_shape(
    relation: &GeometryConstraint,
    nodes: &BTreeMap<String, GeometryNode>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let id = relation.id();
    let (pair, tolerance) = match relation {
        GeometryConstraint::Coincident {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::HorizontalInPlane {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::VerticalInPlane {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::EqualRadius {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::FixedDistance {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::SymmetricAcrossPlane {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::Concentric {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::ComponentAlignment {
            first_node,
            second_node,
            tolerance: value,
            ..
        } => (
            Some((first_node.as_str(), second_node.as_str())),
            Some(*value),
        ),
        GeometryConstraint::AxesPerpendicular {
            first_node,
            second_node,
            tolerance: value,
            ..
        }
        | GeometryConstraint::EqualLength {
            first_node,
            second_node,
            tolerance: value,
            ..
        } => (
            Some((first_node.as_str(), second_node.as_str())),
            Some(*value),
        ),
        GeometryConstraint::FixedAngle {
            first_node,
            second_node,
            angle_degrees,
            tolerance_degrees,
            ..
        } => {
            if !angle_degrees.is_finite() || !(0.0..=180.0).contains(angle_degrees) {
                diagnostics.push(diag(
                    "invalid_constraint_angle",
                    &format!("Constraint '{id}' angle must be between 0 and 180 degrees."),
                    None,
                ));
            }
            (
                Some((first_node.as_str(), second_node.as_str())),
                Some(*tolerance_degrees),
            )
        }
        GeometryConstraint::RelativePosition {
            first_node,
            second_node,
            ..
        } => (Some((first_node.as_str(), second_node.as_str())), None),
        _ => return,
    };
    if let Some((first, second)) = pair {
        for node_id in [first, second] {
            if !nodes.contains_key(node_id) {
                diagnostics.push(diag(
                    "missing_constraint_node",
                    &format!("Constraint '{id}' refers to missing node '{node_id}'."),
                    Some(node_id),
                ));
            }
        }
        if first == second {
            diagnostics.push(diag(
                "constraint_conflict",
                &format!("Constraint '{id}' needs two distinct nodes."),
                Some(first),
            ));
        }
    }
    if let Some(tolerance) = tolerance
        && (!tolerance.is_finite() || tolerance < 0.0)
    {
        diagnostics.push(diag(
            "invalid_tolerance",
            &format!("Constraint '{id}' tolerance must be finite and non-negative."),
            None,
        ));
    }
    if let GeometryConstraint::AxesPerpendicular { tolerance, .. } = relation
        && *tolerance > 1.0
    {
        diagnostics.push(diag(
            "invalid_tolerance",
            &format!("Constraint '{id}' perpendicular tolerance must be in [0,1]."),
            None,
        ));
    }
    if let GeometryConstraint::FixedDistance { distance, .. } = relation
        && (!distance.is_finite() || *distance < 0.0)
    {
        diagnostics.push(diag(
            "invalid_constraint_distance",
            &format!("Constraint '{id}' distance must be finite and non-negative."),
            None,
        ));
    }
    if let GeometryConstraint::FixedAngle {
        tolerance_degrees, ..
    } = relation
        && *tolerance_degrees > 180.0
    {
        diagnostics.push(diag(
            "invalid_tolerance",
            &format!("Constraint '{id}' angle tolerance must be at most 180 degrees."),
            None,
        ));
    }
    if let GeometryConstraint::SymmetricAcrossPlane {
        center_node: Some(center_node),
        ..
    } = relation
        && !nodes.contains_key(center_node)
    {
        diagnostics.push(diag(
            "missing_constraint_node",
            &format!("Constraint '{id}' refers to missing center node '{center_node}'."),
            Some(center_node),
        ));
    }
}

fn validate_node(
    n: &GeometryNode,
    map: &mut BTreeMap<String, GeometryNode>,
    d: &mut Vec<Diagnostic>,
) {
    if !valid_id(&n.id) || map.contains_key(&n.id) {
        d.push(diag(
            "duplicate_or_invalid_node_id",
            &format!("Node ID '{}' is invalid or duplicated.", n.id),
            Some(&n.id),
        ));
    } else {
        map.insert(n.id.clone(), n.clone());
    }
    if n.label.trim().is_empty() {
        d.push(diag(
            "empty_node_label",
            "Node label cannot be empty.",
            Some(&n.id),
        ));
    }
    if !n.transform.valid() {
        d.push(diag(
            "invalid_transform",
            "Transform values must be finite and scale must be non-zero.",
            Some(&n.id),
        ));
    }
    if let Some(p) = &n.primitive {
        validate_primitive(p, &n.id, d);
    } else if n.children.is_empty() {
        d.push(Diagnostic {
            code: "missing_primitive".into(),
            status: DiagnosticStatus::Warning,
            message: "Node has no primitive or children.".into(),
            node_id: Some(n.id.clone()),
        });
    }
    for child in &n.children {
        validate_node(child, map, d);
    }
}

fn validate_primitive(p: &Primitive, id: &str, d: &mut Vec<Diagnostic>) {
    let issue = match p {
        Primitive::Box { size }
            if !size.finite() || size.x <= 0.0 || size.y <= 0.0 || size.z <= 0.0 =>
        {
            Some("Box dimensions must be finite and positive.")
        }
        Primitive::Cylinder {
            radius,
            depth,
            segments,
            ..
        } if !radius.is_finite() || *radius <= 0.0 || !depth.is_finite() || *depth <= 0.0 => {
            Some("Cylinder radius/depth must be finite and positive.")
        }
        Primitive::Cylinder { segments, .. } if !(3..=256).contains(segments) => {
            Some("Cylinder segments must be from 3 to 256.")
        }
        Primitive::Sphere {
            radius,
            longitude_segments,
            latitude_segments,
        } if !radius.is_finite() || *radius <= 0.0 => Some("Sphere radius must be positive."),
        Primitive::Sphere {
            longitude_segments,
            latitude_segments,
            ..
        } if !(3..=256).contains(longitude_segments) || !(2..=128).contains(latitude_segments) => {
            Some("Sphere resolution is outside supported bounds.")
        }
        Primitive::ExtrudedProfile { profile, depth, .. }
            if !depth.is_finite() || *depth <= 0.0 || validate_polygon(profile).is_err() =>
        {
            Some("Extruded profile needs positive depth and a simple finite polygon.")
        }
        Primitive::Point { position } if !position.finite() => {
            Some("Point coordinates must be finite.")
        }
        Primitive::Line { start, end }
            if !start.finite() || !end.finite() || start.sub(*end).length() <= EPS =>
        {
            Some("Line endpoints must be finite and distinct.")
        }
        Primitive::Circle {
            radius, segments, ..
        } if !radius.is_finite() || *radius <= 0.0 => {
            Some("Circle radius must be finite and positive.")
        }
        Primitive::Circle { segments, .. } if !(3..=256).contains(segments) => {
            Some("Circle segments must be from 3 to 256.")
        }
        Primitive::Arc {
            radius,
            start_radians,
            end_radians,
            segments,
            ..
        } if !radius.is_finite()
            || *radius <= 0.0
            || !start_radians.is_finite()
            || !end_radians.is_finite()
            || (*end_radians - *start_radians).abs() <= EPS =>
        {
            Some("Arc radius and angle span must be finite and non-zero.")
        }
        Primitive::Arc { segments, .. } if !(1..=256).contains(segments) => {
            Some("Arc segments must be from 1 to 256.")
        }
        Primitive::Bezier {
            control_points,
            segments,
        } if control_points.iter().any(|v| !v.finite()) => {
            Some("Bezier control points must be finite.")
        }
        Primitive::Bezier { segments, .. } if !(2..=256).contains(segments) => {
            Some("Bezier segments must be from 2 to 256.")
        }
        Primitive::Revolve {
            profile, segments, ..
        } if validate_polygon(profile).is_err() || profile.iter().any(|v| v[1] < 0.0) => {
            Some("Revolve profile must be simple, finite, and have non-negative radii.")
        }
        Primitive::Revolve { segments, .. } if !(3..=256).contains(segments) => {
            Some("Revolve segments must be from 3 to 256.")
        }
        Primitive::TriangleMesh {
            vertices,
            triangles,
        } => validate_authored_mesh(vertices, triangles).err(),
        Primitive::PolygonMesh { vertices, faces } => {
            validate_authored_polygon_mesh(vertices, faces).err()
        }
        Primitive::Unsupported { kind, description } => {
            d.push(Diagnostic {
                code: "unsupported_primitive".into(),
                status: DiagnosticStatus::Unsupported,
                message: format!("Primitive '{kind}' is unsupported: {description}"),
                node_id: Some(id.into()),
            });
            None
        }
        _ => None,
    };
    if let Some(message) = issue {
        d.push(diag("invalid_primitive", message, Some(id)));
    }
}

fn validate_authored_mesh(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Result<(), &'static str> {
    if !(4..=8192).contains(&vertices.len()) || !(4..=16384).contains(&triangles.len()) {
        return Err("Mesh needs 4–8192 vertices and 4–16384 triangles.");
    }
    if vertices.iter().any(|vertex| !vertex.finite()) {
        return Err("Mesh coordinates must be finite.");
    }
    let mut edges = BTreeMap::<(u32, u32), (u32, i32)>::new();
    let mut links = vec![Vec::new(); vertices.len()];
    for &[a, b, c] in triangles {
        let (Some(p), Some(q), Some(r)) = (
            vertices.get(a as usize),
            vertices.get(b as usize),
            vertices.get(c as usize),
        ) else {
            return Err("Mesh face references an out-of-range vertex.");
        };
        if q.sub(*p).cross(r.sub(*p)).length() <= EPS {
            return Err("Mesh contains a degenerate face.");
        }
        for (u, v) in [(a, b), (b, c), (c, a)] {
            let edge = edges.entry((u.min(v), u.max(v))).or_default();
            edge.0 += 1;
            edge.1 += if u < v { 1 } else { -1 };
            links[u as usize].push(v as usize);
            links[v as usize].push(u as usize);
        }
    }
    if edges.values().any(|edge| *edge != (2, 0)) {
        return Err("Mesh must have closed edges with consistent winding.");
    }
    let mut reached = vec![false; vertices.len()];
    reached[0] = true;
    let mut pending = vec![0];
    while let Some(vertex) = pending.pop() {
        for &neighbor in &links[vertex] {
            if !reached[neighbor] {
                reached[neighbor] = true;
                pending.push(neighbor);
            }
        }
    }
    if reached.iter().any(|connected| !connected) {
        return Err(
            "Use separate components for disconnected shells; mesh contains disconnected or unused vertices.",
        );
    }
    Ok(())
}

fn validate_authored_polygon_mesh(
    vertices: &[Vec3],
    faces: &[Vec<u32>],
) -> Result<(), &'static str> {
    if !(4..=8192).contains(&vertices.len()) || !(4..=16384).contains(&faces.len()) {
        return Err("Polygon mesh needs 4–8192 vertices and 4–16384 faces.");
    }
    if vertices.iter().any(|vertex| !vertex.finite()) {
        return Err("Mesh coordinates must be finite.");
    }
    let mut edges = BTreeMap::<(u32, u32), (u32, i32)>::new();
    let mut links = vec![Vec::new(); vertices.len()];
    for face in faces {
        if !(4..=512).contains(&face.len()) {
            return Err(
                "Polygon mesh faces must be quads or n-gons; triangle faces are not accepted.",
            );
        }
        let mut unique = BTreeSet::new();
        for &index in face {
            if index as usize >= vertices.len() {
                return Err("Mesh face references an out-of-range vertex.");
            }
            if !unique.insert(index) {
                return Err("Mesh face repeats a vertex.");
            }
        }
        triangulate_face_indices(vertices, face)
            .map_err(|_| "Mesh contains a degenerate or self-intersecting polygon face.")?;
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            if a == b {
                return Err("Mesh face has a zero-length edge.");
            }
            let edge = edges.entry((a.min(b), a.max(b))).or_default();
            edge.0 += 1;
            edge.1 += if a < b { 1 } else { -1 };
            links[a as usize].push(b as usize);
            links[b as usize].push(a as usize);
        }
    }
    if edges.values().any(|edge| *edge != (2, 0)) {
        return Err("Polygon mesh must have closed edges with consistent winding.");
    }
    let mut reached = vec![false; vertices.len()];
    reached[0] = true;
    let mut pending = vec![0];
    while let Some(vertex) = pending.pop() {
        for &neighbor in &links[vertex] {
            if !reached[neighbor] {
                reached[neighbor] = true;
                pending.push(neighbor);
            }
        }
    }
    if reached.iter().any(|connected| !connected) {
        return Err(
            "Use separate components for disconnected shells; polygon mesh contains disconnected or unused vertices.",
        );
    }
    Ok(())
}

fn triangulate_face_indices(
    vertices: &[Vec3],
    face: &[u32],
) -> Result<Vec<[u32; 3]>, GeometryError> {
    if face.len() < 3 {
        return Err(GeometryError::Invalid(
            "polygon face needs at least three vertices".into(),
        ));
    }
    let points =
        face.iter()
            .map(|&index| {
                vertices.get(index as usize).copied().ok_or_else(|| {
                    GeometryError::Invalid("polygon face index is out of range".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
    let origin = points[0];
    let mut normal = Vec3::ZERO;
    for i in 1..points.len() - 1 {
        normal = normal.add(points[i].sub(origin).cross(points[i + 1].sub(origin)));
    }
    let normal = normal.norm();
    if normal.length() <= EPS {
        return Err(GeometryError::Invalid("polygon face has zero area".into()));
    }
    let drop_axis = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
        0
    } else if normal.y.abs() >= normal.z.abs() {
        1
    } else {
        2
    };
    let projected = points
        .iter()
        .map(|point| match drop_axis {
            0 => [point.y, point.z],
            1 => [point.x, point.z],
            _ => [point.x, point.y],
        })
        .collect::<Vec<_>>();
    triangulate(&projected).map(|triangles| {
        triangles
            .into_iter()
            .map(|[a, b, c]| [face[a], face[b], face[c]])
            .collect()
    })
}

fn validate_polygon(points: &[[f64; 2]]) -> Result<(), &'static str> {
    if points.len() < 3 || points.len() > 512 || points.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid count or coordinate");
    }
    for i in 0..points.len() {
        if distance2(points[i], points[(i + 1) % points.len()]) <= EPS {
            return Err("duplicate neighbours");
        }
    }
    if area2(points).abs() <= EPS {
        return Err("zero area");
    }
    let n = points.len();
    for i in 0..n {
        let i2 = (i + 1) % n;
        for j in i + 1..n {
            let j2 = (j + 1) % n;
            if i == j || i2 == j || j2 == i {
                continue;
            }
            if intersects(points[i], points[i2], points[j], points[j2]) {
                return Err("self intersection");
            }
        }
    }
    Ok(())
}
fn area2(p: &[[f64; 2]]) -> f64 {
    (0..p.len())
        .map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1])
        .sum::<f64>()
        * 0.5
}
fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0]) - EPS
        && p[0] <= a[0].max(b[0]) + EPS
        && p[1] >= a[1].min(b[1]) - EPS
        && p[1] <= a[1].max(b[1]) + EPS
}
fn intersects(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (x, y, z, w) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    ((x > EPS && y < -EPS) || (x < -EPS && y > EPS))
        && ((z > EPS && w < -EPS) || (z < -EPS && w > EPS))
        || (x.abs() <= EPS && on_segment(a, b, c))
        || (y.abs() <= EPS && on_segment(a, b, d))
        || (z.abs() <= EPS && on_segment(c, d, a))
        || (w.abs() <= EPS && on_segment(c, d, b))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}
impl Bounds3 {
    fn points(points: &[Vec3]) -> Option<Self> {
        let first = *points.first()?;
        let mut min = first.array();
        let mut max = min;
        for p in &points[1..] {
            let a = p.array();
            for i in 0..3 {
                min[i] = min[i].min(a[i]);
                max[i] = max[i].max(a[i]);
            }
        }
        Some(Self { min, max })
    }
    fn join(&mut self, other: Self) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }
    fn extent(self) -> Vec3 {
        Vec3::new(
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        )
    }
    fn center(self) -> Vec3 {
        Vec3::new(
            (self.max[0] + self.min[0]) * 0.5,
            (self.max[1] + self.min[1]) * 0.5,
            (self.max[2] + self.min[2]) * 0.5,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds2 {
    pub min: [f64; 2],
    pub max: [f64; 2],
}
impl Bounds2 {
    fn points(points: &[[f64; 2]]) -> Option<Self> {
        let first = *points.first()?;
        let mut min = first;
        let mut max = first;
        for p in &points[1..] {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        Some(Self { min, max })
    }
    fn extent(self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    TriangleMesh,
    PolygonMesh,
    Wire,
    Point,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyComponent {
    pub component_id: String,
    pub label: String,
    pub material: String,
    pub kind: ComponentKind,
    pub vertices: Vec<[f64; 3]>,
    pub edges: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<Vec<u32>>,
    pub triangles: Vec<[u32; 3]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proxy {
    pub schema_version: u32,
    pub asset_uid: String,
    pub revision: u64,
    pub units: GeometryUnits,
    pub components: Vec<ProxyComponent>,
    pub bounds: Bounds3,
    pub node_bounds: BTreeMap<String, Bounds3>,
}

#[derive(Default)]
struct MeshBuilder {
    vertices: Vec<Vec3>,
    edges: Vec<[u32; 2]>,
    faces: Vec<Vec<u32>>,
    triangles: Vec<[u32; 3]>,
    edge_set: BTreeSet<(u32, u32)>,
}
impl MeshBuilder {
    fn vertex(&mut self, p: Vec3) -> u32 {
        let id = self.vertices.len() as u32;
        self.vertices.push(p);
        id
    }
    fn edge(&mut self, a: u32, b: u32) {
        if a == b {
            return;
        }
        let pair = (a.min(b), a.max(b));
        if self.edge_set.insert(pair) {
            self.edges.push([pair.0, pair.1]);
        }
    }
    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.faces.push(vec![a, b, c]);
        self.triangles.push([a, b, c]);
    }
    fn polygon_face(&mut self, face: Vec<u32>) -> Result<(), GeometryError> {
        let triangles = triangulate_face_indices(&self.vertices, &face)?;
        for i in 0..face.len() {
            self.edge(face[i], face[(i + 1) % face.len()]);
        }
        self.faces.push(face);
        self.triangles.extend(triangles);
        Ok(())
    }
    fn kind(&self) -> ComponentKind {
        if self.faces.iter().any(|face| face.len() > 3) {
            ComponentKind::PolygonMesh
        } else if !self.triangles.is_empty() {
            ComponentKind::TriangleMesh
        } else if !self.edges.is_empty() {
            ComponentKind::Wire
        } else {
            ComponentKind::Point
        }
    }
}

fn add_axis_point(axis: Axis, axial: f64, a: f64, b: f64) -> Vec3 {
    match axis {
        Axis::X => Vec3::new(axial, a, b),
        Axis::Y => Vec3::new(a, axial, b),
        Axis::Z => Vec3::new(a, b, axial),
    }
}

fn mesh_for(primitive: &Primitive) -> Result<MeshBuilder, GeometryError> {
    let mut m = MeshBuilder::default();
    match primitive {
        Primitive::Box { size } => {
            let h = [size.x * 0.5, size.y * 0.5, size.z * 0.5];
            for z in [-1.0, 1.0] {
                for y in [-1.0, 1.0] {
                    for x in [-1.0, 1.0] {
                        m.vertex(Vec3::new(x * h[0], y * h[1], z * h[2]));
                    }
                }
            }
            let faces = [
                [0, 1, 3, 2],
                [4, 6, 7, 5],
                [0, 4, 5, 1],
                [2, 3, 7, 6],
                [0, 2, 6, 4],
                [1, 5, 7, 3],
            ];
            for face in faces {
                m.polygon_face(face.into())?;
            }
        }
        Primitive::Cylinder {
            radius,
            depth,
            axis,
            segments,
        } => {
            let n = *segments as usize;
            let mut lo = Vec::with_capacity(n);
            let mut hi = Vec::with_capacity(n);
            for i in 0..n {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                let c = radius * a.cos();
                let s = radius * a.sin();
                lo.push(m.vertex(add_axis_point(*axis, -depth * 0.5, c, s)));
                hi.push(m.vertex(add_axis_point(*axis, depth * 0.5, c, s)));
            }
            for i in 0..n {
                let j = (i + 1) % n;
                m.polygon_face(vec![lo[i], lo[j], hi[j], hi[i]])?;
            }
            let ring_faces_positive = *axis != Axis::Y;
            let mut low_cap = lo;
            let mut high_cap = hi;
            if ring_faces_positive {
                low_cap.reverse();
            } else {
                high_cap.reverse();
            }
            m.polygon_face(low_cap)?;
            m.polygon_face(high_cap)?;
        }
        Primitive::Sphere {
            radius,
            longitude_segments,
            latitude_segments,
        } => {
            let lon = *longitude_segments as usize;
            let lat = *latitude_segments as usize;
            let top = m.vertex(Vec3::new(0.0, 0.0, *radius));
            let bottom = m.vertex(Vec3::new(0.0, 0.0, -*radius));
            let mut rings = Vec::new();
            for i in 1..lat {
                let theta = std::f64::consts::PI * i as f64 / lat as f64;
                let mut ring = Vec::new();
                for j in 0..lon {
                    let phi = std::f64::consts::TAU * j as f64 / lon as f64;
                    ring.push(m.vertex(Vec3::new(
                        radius * theta.sin() * phi.cos(),
                        radius * theta.sin() * phi.sin(),
                        radius * theta.cos(),
                    )));
                }
                rings.push(ring);
            }
            for j in 0..lon {
                let k = (j + 1) % lon;
                m.triangle(top, rings[0][j], rings[0][k]);
                m.edge(top, rings[0][j]);
                m.edge(rings[0][j], rings[0][k]);
                m.triangle(
                    bottom,
                    *rings.last().unwrap().get(k).unwrap(),
                    *rings.last().unwrap().get(j).unwrap(),
                );
                m.edge(bottom, rings.last().unwrap()[j]);
            }
            for r in 0..rings.len().saturating_sub(1) {
                for j in 0..lon {
                    let k = (j + 1) % lon;
                    let (a, b, c, d) = (rings[r][j], rings[r][k], rings[r + 1][k], rings[r + 1][j]);
                    m.triangle(a, b, c);
                    m.triangle(a, c, d);
                    m.edge(a, b);
                    m.edge(a, d);
                }
            }
            for j in 0..lon {
                m.edge(
                    rings.last().unwrap()[j],
                    rings.last().unwrap()[(j + 1) % lon],
                );
            }
        }
        Primitive::ExtrudedProfile {
            profile,
            depth,
            axis,
        } => {
            let n = profile.len();
            for side in [-1.0, 1.0] {
                for p in profile {
                    m.vertex(add_axis_point(*axis, side * depth * 0.5, p[0], p[1]));
                }
            }
            for i in 0..n {
                let j = (i + 1) % n;
                m.polygon_face(vec![i as u32, j as u32, (n + j) as u32, (n + i) as u32])?;
            }
            m.polygon_face((0..n as u32).rev().collect())?;
            m.polygon_face((0..n as u32).map(|index| index + n as u32).collect())?;
        }
        Primitive::Point { position } => {
            m.vertex(*position);
        }
        Primitive::Line { start, end } => {
            let a = m.vertex(*start);
            let b = m.vertex(*end);
            m.edge(a, b);
        }
        Primitive::Circle {
            radius,
            axis,
            segments,
        } => {
            let n = *segments as usize;
            let mut ring = Vec::with_capacity(n);
            for i in 0..n {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                ring.push(m.vertex(add_axis_point(
                    *axis,
                    0.0,
                    radius * a.cos(),
                    radius * a.sin(),
                )));
            }
            for i in 0..n {
                m.edge(ring[i], ring[(i + 1) % n]);
            }
        }
        Primitive::Arc {
            radius,
            axis,
            start_radians,
            end_radians,
            segments,
        } => {
            let n = *segments as usize;
            let mut last = None;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let a = start_radians + (end_radians - start_radians) * t;
                let v = m.vertex(add_axis_point(
                    *axis,
                    0.0,
                    radius * a.cos(),
                    radius * a.sin(),
                ));
                if let Some(prev) = last {
                    m.edge(prev, v);
                }
                last = Some(v);
            }
        }
        Primitive::Bezier {
            control_points,
            segments,
        } => {
            let n = *segments as usize;
            let mut last = None;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let u = 1.0 - t;
                let p = control_points[0]
                    .mul(u * u * u)
                    .add(control_points[1].mul(3.0 * u * u * t))
                    .add(control_points[2].mul(3.0 * u * t * t))
                    .add(control_points[3].mul(t * t * t));
                let v = m.vertex(p);
                if let Some(prev) = last {
                    m.edge(prev, v);
                }
                last = Some(v);
            }
        }
        Primitive::Revolve {
            profile,
            axis,
            segments,
        } => {
            let n = *segments as usize;
            let mut rings = Vec::new();
            for [axial, radial] in profile {
                let mut ring = Vec::new();
                for i in 0..n {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    ring.push(m.vertex(add_axis_point(
                        *axis,
                        *axial,
                        *radial * a.cos(),
                        *radial * a.sin(),
                    )));
                }
                rings.push(ring);
            }
            for r in 0..rings.len() {
                let next_r = (r + 1) % rings.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    let (a, b, c, d) =
                        (rings[r][i], rings[r][j], rings[next_r][j], rings[next_r][i]);
                    m.polygon_face(vec![a, b, c, d])?;
                }
            }
        }
        Primitive::Unsupported { kind, description } => {
            return Err(GeometryError::Unsupported(format!(
                "primitive '{kind}': {description}"
            )));
        }
        Primitive::TriangleMesh {
            vertices,
            triangles,
        } => {
            validate_authored_mesh(vertices, triangles)
                .map_err(|message| GeometryError::Invalid(message.into()))?;
            m.vertices.clone_from(vertices);
            for &[a, b, c] in triangles {
                m.triangle(a, b, c);
                m.edge(a, b);
                m.edge(b, c);
                m.edge(c, a);
            }
        }
        Primitive::PolygonMesh { vertices, faces } => {
            validate_authored_polygon_mesh(vertices, faces)
                .map_err(|message| GeometryError::Invalid(message.into()))?;
            m.vertices.clone_from(vertices);
            for face in faces {
                m.polygon_face(face.clone())?;
            }
        }
    }
    Ok(m)
}

fn triangulate(points: &[[f64; 2]]) -> Result<Vec<[usize; 3]>, GeometryError> {
    validate_polygon(points)
        .map_err(|e| GeometryError::Invalid(format!("invalid polygon: {e}")))?;
    let ccw = area2(points) > 0.0;
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    let mut out = Vec::new();
    while remaining.len() > 3 {
        let mut found = None;
        for k in 0..remaining.len() {
            let a = remaining[(k + remaining.len() - 1) % remaining.len()];
            let b = remaining[k];
            let c = remaining[(k + 1) % remaining.len()];
            let turn = orient(points[a], points[b], points[c]);
            if (ccw && turn <= EPS) || (!ccw && turn >= -EPS) {
                continue;
            }
            let contains = remaining
                .iter()
                .copied()
                .filter(|p| *p != a && *p != b && *p != c)
                .any(|p| point_in_triangle(points[p], points[a], points[b], points[c]));
            if !contains {
                found = Some((k, [a, b, c]));
                break;
            }
        }
        let Some((k, tri)) = found else {
            return Err(GeometryError::Invalid(
                "polygon triangulation stalled".into(),
            ));
        };
        out.push(tri);
        remaining.remove(k);
    }
    out.push([remaining[0], remaining[1], remaining[2]]);
    Ok(out)
}
fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let (u, v, w) = (orient(a, b, p), orient(b, c, p), orient(c, a, p));
    (u >= -EPS && v >= -EPS && w >= -EPS) || (u <= EPS && v <= EPS && w <= EPS)
}

fn collect_proxy(
    node: &GeometryNode,
    path: &mut Vec<Transform>,
    components: &mut Vec<ProxyComponent>,
    node_bounds: &mut BTreeMap<String, Bounds3>,
) -> Result<Option<Bounds3>, GeometryError> {
    path.push(node.transform);
    let mut own = None;
    if let Some(primitive) = &node.primitive {
        let mut mesh = mesh_for(primitive)?;
        for p in &mut mesh.vertices {
            for t in path.iter().rev() {
                *p = t.point(*p);
            }
        }
        let bounds = Bounds3::points(&mesh.vertices).ok_or(GeometryError::EmptyGeometry)?;
        own = Some(bounds);
        components.push(ProxyComponent {
            component_id: node.id.clone(),
            label: node.label.clone(),
            material: node.material.as_str().into(),
            kind: mesh.kind(),
            vertices: mesh.vertices.iter().map(|p| p.array()).collect(),
            edges: mesh.edges,
            faces: mesh.faces,
            triangles: mesh.triangles,
        });
    }
    for child in &node.children {
        if let Some(bounds) = collect_proxy(child, path, components, node_bounds)? {
            if let Some(current) = &mut own {
                current.join(bounds)
            } else {
                own = Some(bounds);
            }
        }
    }
    path.pop();
    if let Some(bounds) = own {
        node_bounds.insert(node.id.clone(), bounds);
    }
    Ok(own)
}

fn find_transforms(node: &GeometryNode, target: &str, path: &mut Vec<Transform>) -> bool {
    path.push(node.transform);
    if node.id == target {
        return true;
    }
    for child in &node.children {
        if find_transforms(child, target, path) {
            return true;
        }
    }
    path.pop();
    false
}
fn world_axis(root: &GeometryNode, target: &str, axis: Axis) -> Result<Vec3, GeometryError> {
    let mut path = Vec::new();
    if !find_transforms(root, target, &mut path) {
        return Err(GeometryError::NodeNotFound(target.into()));
    }
    let mut v = axis.vector();
    for t in path.iter().rev() {
        v = t.vector(v);
    }
    Ok(v.norm())
}

fn dimension_value(
    target: &DimensionTarget,
    spec: &GeometrySpec,
    p: &Proxy,
) -> Result<f64, GeometryError> {
    match target {
        DimensionTarget::OverallExtent { axis } => Ok(p.bounds.extent().at(*axis)),
        DimensionTarget::NodeExtent { node_id, axis } => Ok(p
            .node_bounds
            .get(node_id)
            .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?
            .extent()
            .at(*axis)),
        DimensionTarget::NodeDistance {
            first_node,
            second_node,
        } => {
            let a = p
                .node_bounds
                .get(first_node)
                .ok_or_else(|| GeometryError::NodeNotFound(first_node.clone()))?
                .center();
            let b = p
                .node_bounds
                .get(second_node)
                .ok_or_else(|| GeometryError::NodeNotFound(second_node.clone()))?
                .center();
            Ok(a.sub(b).length())
        }
        DimensionTarget::Radius { node_id } => read_primitive_parameter(
            find_node(&spec.root, node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?,
            PrimitiveParameter::Radius,
        ),
        DimensionTarget::Diameter { node_id } => read_primitive_parameter(
            find_node(&spec.root, node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?,
            PrimitiveParameter::Diameter,
        ),
        DimensionTarget::Angle { node_id, axis } => {
            let node = find_node(&spec.root, node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?;
            Ok(node.transform.rotation.at(*axis).to_degrees())
        }
        DimensionTarget::CenterCoordinate { node_id, axis } => Ok(p
            .node_bounds
            .get(node_id)
            .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?
            .center()
            .at(*axis)),
        DimensionTarget::Unsupported { description } => {
            Err(GeometryError::Unsupported(description.clone()))
        }
    }
}

fn dimension_unit_label(target: &DimensionTarget, units: GeometryUnits) -> &'static str {
    match target {
        DimensionTarget::Angle { .. } => "°",
        _ => units.length_label(),
    }
}

fn primitive_parameter_unit_label(
    parameter: PrimitiveParameter,
    units: GeometryUnits,
) -> &'static str {
    match parameter {
        PrimitiveParameter::Rotation(_) => "°",
        _ => units.length_label(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Front,
    Side,
    Top,
    ThreeQuarter,
}
impl View {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Side => "side",
            Self::Top => "top",
            Self::Front => "front",
            Self::ThreeQuarter => "three_quarter",
        }
    }
    pub const fn all() -> [Self; 4] {
        [Self::Front, Self::Side, Self::Top, Self::ThreeQuarter]
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Front => "Front",
            Self::Side => "Side",
            Self::Top => "Top",
            Self::ThreeQuarter => "Three-quarter",
        }
    }
}
#[derive(Clone, Copy)]
struct ViewBasis {
    right: Vec3,
    up: Vec3,
    toward: Vec3,
}
impl ViewBasis {
    fn for_view(view: View) -> Self {
        match view {
            View::Front => Self {
                right: Axis::Y.vector(),
                up: Axis::Z.vector(),
                toward: Axis::X.vector(),
            },
            View::Side => Self {
                right: Axis::X.vector(),
                up: Axis::Z.vector(),
                toward: Vec3::new(0.0, -1.0, 0.0),
            },
            View::Top => Self {
                right: Axis::X.vector(),
                up: Axis::Y.vector(),
                toward: Axis::Z.vector(),
            },
            View::ThreeQuarter => {
                let toward = Vec3::new(1.0, -1.0, 0.68).norm();
                let right = Vec3::new(0.0, 0.0, 1.0).cross(toward).norm();
                let up = toward.cross(right).norm();
                Self { right, up, toward }
            }
        }
    }
    fn transform(self, p: Vec3) -> ([f64; 2], f64) {
        ([p.dot(self.right), p.dot(self.up)], p.dot(self.toward))
    }
    fn matrix(self) -> [[f64; 4]; 4] {
        [
            [self.right.x, self.right.y, self.right.z, 0.0],
            [self.up.x, self.up.y, self.up.z, 0.0],
            [self.toward.x, self.toward.y, self.toward.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectedAxis {
    pub axis: Axis,
    pub direction: [f64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectionScale {
    pub world_units_per_projection_unit: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectedTriangle {
    pub vertices: [[f64; 2]; 3],
    pub depth: [f64; 3],
    pub normal: [f64; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectedComponent {
    pub component_id: String,
    pub label: String,
    pub material: String,
    pub kind: ComponentKind,
    pub vertices: Vec<[f64; 2]>,
    pub depths: Vec<f64>,
    pub edges: Vec<[u32; 2]>,
    pub triangles: Vec<ProjectedTriangle>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    pub asset_uid: String,
    pub revision: u64,
    pub units: GeometryUnits,
    pub view: View,
    pub convention: ProjectionConvention,
    pub matrix: [[f64; 4]; 4],
    pub projected_axes: [ProjectedAxis; 3],
    pub scale: ProjectionScale,
    pub bounds: Bounds2,
    pub components: Vec<ProjectedComponent>,
}

fn project_proxy(
    proxy: &Proxy,
    view: View,
    convention: ProjectionConvention,
) -> Result<Projection, GeometryError> {
    let basis = ViewBasis::for_view(view);
    let mut all = Vec::new();
    let mut components = Vec::new();
    for component in &proxy.components {
        let mut vertices = Vec::with_capacity(component.vertices.len());
        let mut depths = Vec::with_capacity(component.vertices.len());
        for p in &component.vertices {
            let (point, depth) = basis.transform(Vec3::from_array(*p));
            vertices.push(point);
            depths.push(depth);
            all.push(point);
        }
        let mut triangles = Vec::with_capacity(component.triangles.len());
        for tri in &component.triangles {
            let a = component.vertices[tri[0] as usize];
            let b = component.vertices[tri[1] as usize];
            let c = component.vertices[tri[2] as usize];
            let normal = Vec3::from_array(b)
                .sub(Vec3::from_array(a))
                .cross(Vec3::from_array(c).sub(Vec3::from_array(a)))
                .norm();
            let inds = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            triangles.push(ProjectedTriangle {
                vertices: [vertices[inds[0]], vertices[inds[1]], vertices[inds[2]]],
                depth: [depths[inds[0]], depths[inds[1]], depths[inds[2]]],
                normal: normal.array(),
            });
        }
        components.push(ProjectedComponent {
            component_id: component.component_id.clone(),
            label: component.label.clone(),
            material: component.material.clone(),
            kind: component.kind,
            vertices,
            depths,
            edges: component.edges.clone(),
            triangles,
        });
    }
    let bounds = Bounds2::points(&all).ok_or(GeometryError::EmptyGeometry)?;
    let axes = [Axis::X, Axis::Y, Axis::Z].map(|axis| {
        let v = axis.vector();
        ProjectedAxis {
            axis,
            direction: [v.dot(basis.right), v.dot(basis.up)],
        }
    });
    Ok(Projection {
        asset_uid: proxy.asset_uid.clone(),
        revision: proxy.revision,
        units: proxy.units,
        view,
        convention,
        matrix: basis.matrix(),
        projected_axes: axes,
        scale: ProjectionScale {
            world_units_per_projection_unit: 1.0,
        },
        bounds,
        components,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelRenderOptions {
    pub padding_pixels: u32,
    pub background: [u8; 4],
    pub outline: bool,
    pub component_edges: bool,
}
impl Default for PixelRenderOptions {
    fn default() -> Self {
        Self {
            padding_pixels: 8,
            background: [0, 0, 0, 0],
            outline: true,
            component_edges: true,
        }
    }
}

#[derive(Clone, Copy)]
struct ScreenMap {
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    height: u32,
}
impl ScreenMap {
    fn new(bounds: Bounds2, width: u32, height: u32, padding: u32) -> Result<Self, GeometryError> {
        if width == 0
            || height == 0
            || width > MAX_RENDER_DIMENSION
            || height > MAX_RENDER_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_RENDER_PIXELS
        {
            return Err(GeometryError::RenderSize);
        }
        let pad = padding
            .min(width.saturating_sub(1) / 2)
            .min(height.saturating_sub(1) / 2) as f64;
        let available_w = (width as f64 - 2.0 * pad).max(1.0);
        let available_h = (height as f64 - 2.0 * pad).max(1.0);
        let span = bounds.extent();
        let sx = available_w / span[0].max(1.0e-6);
        let sy = available_h / span[1].max(1.0e-6);
        let scale = sx.min(sy);
        let used_w = span[0] * scale;
        let used_h = span[1] * scale;
        let offset_x = pad + (available_w - used_w) * 0.5 - bounds.min[0] * scale;
        let offset_y = pad + (available_h - used_h) * 0.5 - bounds.min[1] * scale;
        Ok(Self {
            scale,
            offset_x,
            offset_y,
            height,
        })
    }
    fn map(self, p: [f64; 2]) -> [f64; 2] {
        [
            self.offset_x + p[0] * self.scale,
            (self.height as f64 - 1.0) - (self.offset_y + p[1] * self.scale),
        ]
    }
}

/// Rasterizes the projected mesh directly at the requested target grid; no resampling or CRT pass.
pub fn render_pixel_reference(
    spec: &GeometrySpec,
    view: View,
    width: u32,
    height: u32,
    options: PixelRenderOptions,
) -> Result<RgbaImage, GeometryError> {
    let projection = spec.project(view)?;
    render_projection_pixel(&projection, width, height, options)
}
fn render_projection_pixel(
    projection: &Projection,
    width: u32,
    height: u32,
    options: PixelRenderOptions,
) -> Result<RgbaImage, GeometryError> {
    let map = ScreenMap::new(projection.bounds, width, height, options.padding_pixels)?;
    let mut image = RgbaImage::from_pixel(width, height, Rgba(options.background));
    let mut zbuffer = vec![f64::NEG_INFINITY; (width as usize) * (height as usize)];
    let mut covered = vec![false; zbuffer.len()];
    for component in &projection.components {
        let material = Material::parse(&component.material);
        for tri in &component.triangles {
            let points = tri.vertices.map(|p| map.map(p));
            let face = shade(
                material,
                Vec3::from_array(tri.normal),
                ViewBasis::for_view(projection.view).toward,
            );
            raster_triangle(
                &mut image,
                &mut zbuffer,
                &mut covered,
                points,
                tri.depth,
                face,
            );
        }
    }
    if options.component_edges {
        for component in &projection.components {
            if component.kind == ComponentKind::Point {
                if let Some(point) = component.vertices.first() {
                    draw_point(&mut image, map.map(*point), [220, 210, 182, 255]);
                }
                continue;
            }
            let color = edge_color(Material::parse(&component.material));
            for edge in &component.edges {
                let a = edge[0] as usize;
                let b = edge[1] as usize;
                draw_line_depth(
                    &mut image,
                    &mut zbuffer,
                    map.map(component.vertices[a]),
                    map.map(component.vertices[b]),
                    component.depths[a],
                    component.depths[b],
                    color,
                );
            }
        }
    }
    if options.outline {
        apply_outline(&mut image, &covered, [25, 23, 22, 255]);
    }
    Ok(image)
}

fn shade(material: Material, normal: Vec3, toward: Vec3) -> Rgba<u8> {
    let diffuse = normal.dot(toward).abs().clamp(0.0, 1.0);
    let palette: [[u8; 3]; 3] = match material {
        Material::Wood => [[67, 35, 22], [127, 69, 35], [184, 117, 61]],
        Material::Steel => [[49, 56, 62], [102, 114, 121], [174, 184, 187]],
        Material::DarkSteel => [[26, 30, 34], [59, 66, 71], [111, 119, 122]],
        Material::Brass => [[93, 61, 25], [175, 125, 48], [232, 196, 107]],
        Material::Rubber => [[26, 27, 26], [53, 53, 49], [88, 85, 75]],
        Material::Leather => [[57, 31, 22], [105, 58, 35], [151, 91, 53]],
    };
    let level = if diffuse < 0.27 {
        0
    } else if diffuse < 0.68 {
        1
    } else {
        2
    };
    Rgba([palette[level][0], palette[level][1], palette[level][2], 255])
}
fn edge_color(material: Material) -> [u8; 4] {
    match material {
        Material::Wood => [55, 28, 19, 255],
        Material::Steel => [34, 39, 44, 255],
        Material::DarkSteel => [19, 22, 25, 255],
        Material::Brass => [82, 51, 18, 255],
        Material::Rubber => [20, 21, 20, 255],
        Material::Leather => [47, 25, 18, 255],
    }
}
fn raster_triangle(
    image: &mut RgbaImage,
    zbuf: &mut [f64],
    covered: &mut [bool],
    p: [[f64; 2]; 3],
    d: [f64; 3],
    color: Rgba<u8>,
) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    let min_x = p
        .iter()
        .map(|v| v[0].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, width - 1);
    let max_x = p
        .iter()
        .map(|v| v[0].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, width - 1);
    let min_y = p
        .iter()
        .map(|v| v[1].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, height - 1);
    let max_y = p
        .iter()
        .map(|v| v[1].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, height - 1);
    let denom =
        (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
    if denom.abs() < EPS {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let a = ((p[1][1] - p[2][1]) * (px - p[2][0]) + (p[2][0] - p[1][0]) * (py - p[2][1]))
                / denom;
            let b = ((p[2][1] - p[0][1]) * (px - p[2][0]) + (p[0][0] - p[2][0]) * (py - p[2][1]))
                / denom;
            let c = 1.0 - a - b;
            if a < -EPS || b < -EPS || c < -EPS {
                continue;
            }
            let index = y as usize * image.width() as usize + x as usize;
            let depth = a * d[0] + b * d[1] + c * d[2];
            if depth >= zbuf[index] {
                zbuf[index] = depth;
                covered[index] = true;
                image.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}
fn draw_line_depth(
    image: &mut RgbaImage,
    zbuf: &mut [f64],
    a: [f64; 2],
    b: [f64; 2],
    da: f64,
    db: f64,
    color: [u8; 4],
) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let steps = dx.abs().max(dy.abs()).ceil() as usize;
    if steps == 0 {
        draw_point(image, a, color);
        return;
    }
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let x = (a[0] + dx * t).round() as i32;
        let y = (a[1] + dy * t).round() as i32;
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            continue;
        }
        let index = y as usize * image.width() as usize + x as usize;
        let depth = da + (db - da) * t;
        if depth + 1.0e-5 >= zbuf[index] {
            zbuf[index] = depth;
            image.put_pixel(x as u32, y as u32, Rgba(color));
        }
    }
}
fn draw_point(image: &mut RgbaImage, p: [f64; 2], color: [u8; 4]) {
    let (x, y) = (p[0].round() as i32, p[1].round() as i32);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let a = x + dx;
            let b = y + dy;
            if a >= 0 && b >= 0 && a < image.width() as i32 && b < image.height() as i32 {
                image.put_pixel(a as u32, b as u32, Rgba(color));
            }
        }
    }
}
fn apply_outline(image: &mut RgbaImage, covered: &[bool], color: [u8; 4]) {
    let (w, h) = (image.width() as i32, image.height() as i32);
    let mut mask = covered.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y as usize * w as usize + x as usize;
            if covered[i] {
                continue;
            }
            let near = [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ]
            .iter()
            .any(|(dx, dy)| {
                let a = x + dx;
                let b = y + dy;
                a >= 0 && b >= 0 && a < w && b < h && covered[b as usize * w as usize + a as usize]
            });
            if near {
                mask[i] = true;
                image.put_pixel(x as u32, y as u32, Rgba(color));
            }
        }
    }
}

/// Builds a deterministic SVG technical sheet from the same world-space proxy used by projections.
pub fn render_technical_sheet_svg(
    spec: &GeometrySpec,
    width: u32,
    height: u32,
) -> Result<String, GeometryError> {
    if width == 0
        || height == 0
        || width > MAX_RENDER_DIMENSION
        || height > MAX_RENDER_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_RENDER_PIXELS
    {
        return Err(GeometryError::RenderSize);
    }
    let mut views = BTreeMap::new();
    for &view in spec.technical_sheet_layout.views() {
        views.insert(view, spec.project(view)?);
    }
    let dims = spec.measure_dimensions()?;
    let margin = 16.0;
    let header = 50.0;
    let footer = 42.0;
    let cell_w = (width as f64 - 4.0 * margin) / 3.0;
    let cell_h = (height as f64 - header - footer - 4.0 * margin) / 3.0;
    if cell_w < 36.0 || cell_h < 36.0 {
        return Err(GeometryError::RenderSize);
    }
    let mut svg = String::new();
    writeln!(&mut svg,"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">").unwrap();
    svg.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>");
    writeln!(
        &mut svg,
        "<metadata data-projection-convention=\"{}\" data-layout=\"{}\" data-scale-units=\"{}\"/>",
        spec.projection_convention.as_str(),
        spec.technical_sheet_layout.as_str(),
        spec.units.as_str()
    )
    .unwrap();
    writeln!(&mut svg,"<text x=\"{margin:.1}\" y=\"25\" font-family=\"sans-serif\" font-size=\"17\" font-weight=\"700\">{}</text>",xml_escape(&spec.name)).unwrap();
    writeln!(&mut svg,"<text x=\"{margin:.1}\" y=\"42\" font-family=\"sans-serif\" font-size=\"10\" fill=\"#555\">{} · revision {} · {:?} projection · {}</text>",xml_escape(&spec.asset_uid),spec.revision,spec.projection_convention,spec.units.description()).unwrap();
    let panel_views = match spec.projection_convention {
        ProjectionConvention::ThirdAngle => [
            (View::Top, 1, 0),
            (View::ThreeQuarter, 2, 2),
            (View::Front, 1, 1),
            (View::Side, 2, 1),
        ],
        ProjectionConvention::FirstAngle => [
            (View::Side, 0, 1),
            (View::ThreeQuarter, 2, 2),
            (View::Front, 1, 1),
            (View::Top, 1, 2),
        ],
    };
    let charts: Vec<_> = panel_views
        .into_iter()
        .filter(|(view, ..)| views.contains_key(view))
        .map(|(view, col, row)| {
            let x = margin + col as f64 * (cell_w + margin);
            let y = header + margin + row as f64 * (cell_h + margin);
            (view, x, y, x + 8.0, y + 24.0, cell_w - 16.0, cell_h - 32.0)
        })
        .collect();
    // Keep the principal orthographic views at one common scale.
    let common_scale = charts
        .iter()
        .filter(|(v, ..)| *v != View::ThreeQuarter)
        .map(|(view, _, _, _, _, cw, ch)| fit_scale(views[view].bounds, *cw, *ch))
        .fold(f64::INFINITY, f64::min);
    let quarter_scale = charts
        .iter()
        .find(|(v, ..)| *v == View::ThreeQuarter)
        .map(|(_, _, _, _, _, cw, ch)| fit_scale(views[&View::ThreeQuarter].bounds, *cw, *ch))
        .unwrap_or(common_scale);
    for (view, x, y, cx, cy, cw, ch) in charts {
        let projection = &views[&view];
        let scale = if view == View::ThreeQuarter {
            quarter_scale
        } else {
            common_scale
        };
        let map = SvgMap::centered(projection.bounds, cx, cy, cw, ch, scale);
        writeln!(&mut svg,"<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{cell_w:.2}\" height=\"{cell_h:.2}\" fill=\"#fff\" stroke=\"#888\" stroke-width=\"1\"/>").unwrap();
        writeln!(&mut svg,"<text x=\"{:.2}\" y=\"{:.2}\" font-family=\"sans-serif\" font-size=\"11\" font-weight=\"700\">{}</text>",x+8.0,y+16.0,view.label()).unwrap();
        if matches!(view, View::Front | View::Side | View::Top) {
            let path = silhouette_path(projection, map, cx, cy, cw, ch);
            if !path.is_empty() {
                writeln!(&mut svg,"<path d=\"{path}\" fill=\"none\" stroke=\"#202629\" stroke-width=\"1.25\" stroke-linejoin=\"round\"/>").unwrap();
            }
        } else {
            // The three-quarter view is a line drawing of the same mesh, with hidden-edge removal.
            render_visible_three_quarter(&mut svg, projection, map);
        }
        for component in &projection.components {
            if component.kind == ComponentKind::Wire {
                let color = svg_color(Material::parse(&component.material));
                for edge in &component.edges {
                    let a = map.point(component.vertices[edge[0] as usize]);
                    let b = map.point(component.vertices[edge[1] as usize]);
                    writeln!(&mut svg,"<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\" stroke-width=\"1\"/>",a[0],a[1],b[0],b[1]).unwrap();
                }
            } else if component.kind == ComponentKind::Point
                && let Some(p) = component.vertices.first()
            {
                let q = map.point(*p);
                writeln!(
                    &mut svg,
                    "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"2\" fill=\"#202629\"/>",
                    q[0], q[1]
                )
                .unwrap();
            }
        }
        if view == View::Side
            && let Some(value) = dims
                .iter()
                .find(|d| d.id == "overall-length")
                .and_then(|d| d.value)
        {
            draw_overall_dimension(
                &mut svg,
                projection,
                map,
                cy,
                ch,
                value,
                spec.units.length_label(),
            );
        }
    }
    writeln!(&mut svg,"<text x=\"{margin:.1}\" y=\"{:.2}\" font-family=\"sans-serif\" font-size=\"9\" fill=\"#333\">Lengths use {}; drawing values are derived from the current geometry.</text>",height as f64-25.0,spec.units.description()).unwrap();
    let mut cursor = margin;
    for dimension in &dims {
        let text = if let Some(value) = dimension.value {
            format!("{}: {:.3} {}", dimension.label, value, dimension.unit_label)
        } else {
            format!("{}: unsupported", dimension.label)
        };
        writeln!(&mut svg,"<text x=\"{cursor:.1}\" y=\"{:.2}\" font-family=\"sans-serif\" font-size=\"9\" fill=\"#333\">{}</text>",height as f64-10.0,xml_escape(&text)).unwrap();
        cursor += text.chars().count() as f64 * 5.1 + 16.0;
    }
    svg.push_str("</svg>");
    Ok(svg)
}

#[derive(Clone, Copy)]
struct SvgMap {
    scale: f64,
    ox: f64,
    oy: f64,
}
impl SvgMap {
    fn centered(bounds: Bounds2, x: f64, y: f64, width: f64, height: f64, scale: f64) -> Self {
        let span = bounds.extent();
        let used = [span[0] * scale, span[1] * scale];
        Self {
            scale,
            ox: x + (width - used[0]) * 0.5 - bounds.min[0] * scale,
            oy: y + (height - used[1]) * 0.5 + bounds.min[1] * scale + used[1],
        }
    }
    fn point(self, p: [f64; 2]) -> [f64; 2] {
        [self.ox + p[0] * self.scale, self.oy - p[1] * self.scale]
    }
}
fn fit_scale(bounds: Bounds2, width: f64, height: f64) -> f64 {
    let span = bounds.extent();
    (width / span[0].max(1.0e-6)).min(height / span[1].max(1.0e-6))
}

fn silhouette_path(
    projection: &Projection,
    map: SvgMap,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> String {
    const DENSITY: f64 = 2.0;
    let mw = (width * DENSITY).ceil().max(1.0) as usize;
    let mh = (height * DENSITY).ceil().max(1.0) as usize;
    let mut mask = vec![false; mw * mh];
    for component in &projection.components {
        for tri in &component.triangles {
            let p = tri.vertices.map(|v| {
                let q = map.point(v);
                [(q[0] - x) * DENSITY, (q[1] - y) * DENSITY]
            });
            mask_triangle(&mut mask, mw, mh, p);
        }
    }
    let mut path = String::new();
    for py in 0..mh {
        let mut px = 0;
        while px < mw {
            let i = py * mw + px;
            if mask[i] && (py == 0 || !mask[(py - 1) * mw + px]) {
                let start = px;
                while px + 1 < mw
                    && mask[py * mw + px + 1]
                    && (py == 0 || !mask[(py - 1) * mw + px + 1])
                {
                    px += 1;
                }
                let a = [x + start as f64 / DENSITY, y + py as f64 / DENSITY];
                let b = [x + (px + 1) as f64 / DENSITY, a[1]];
                append_segment(&mut path, a, b);
            }
            px += 1;
        }
    }
    for py in 0..mh {
        let mut px = 0;
        while px < mw {
            let i = py * mw + px;
            if mask[i] && (py + 1 == mh || !mask[(py + 1) * mw + px]) {
                let start = px;
                while px + 1 < mw
                    && mask[py * mw + px + 1]
                    && (py + 1 == mh || !mask[(py + 1) * mw + px + 1])
                {
                    px += 1;
                }
                let a = [x + start as f64 / DENSITY, y + (py + 1) as f64 / DENSITY];
                let b = [x + (px + 1) as f64 / DENSITY, a[1]];
                append_segment(&mut path, a, b);
            }
            px += 1;
        }
    }
    for px in 0..mw {
        let mut py = 0;
        while py < mh {
            let i = py * mw + px;
            if mask[i] && (px == 0 || !mask[py * mw + px - 1]) {
                let start = py;
                while py + 1 < mh
                    && mask[(py + 1) * mw + px]
                    && (px == 0 || !mask[(py + 1) * mw + px - 1])
                {
                    py += 1;
                }
                let a = [x + px as f64 / DENSITY, y + start as f64 / DENSITY];
                let b = [a[0], y + (py + 1) as f64 / DENSITY];
                append_segment(&mut path, a, b);
            }
            py += 1;
        }
    }
    for px in 0..mw {
        let mut py = 0;
        while py < mh {
            let i = py * mw + px;
            if mask[i] && (px + 1 == mw || !mask[py * mw + px + 1]) {
                let start = py;
                while py + 1 < mh
                    && mask[(py + 1) * mw + px]
                    && (px + 1 == mw || !mask[(py + 1) * mw + px + 1])
                {
                    py += 1;
                }
                let a = [x + (px + 1) as f64 / DENSITY, y + start as f64 / DENSITY];
                let b = [a[0], y + (py + 1) as f64 / DENSITY];
                append_segment(&mut path, a, b);
            }
            py += 1;
        }
    }
    path
}
fn append_segment(path: &mut String, a: [f64; 2], b: [f64; 2]) {
    write!(path, "M{:.2},{:.2}L{:.2},{:.2}", a[0], a[1], b[0], b[1]).unwrap();
}
fn mask_triangle(mask: &mut [bool], width: usize, height: usize, p: [[f64; 2]; 3]) {
    let min_x = p
        .iter()
        .map(|q| q[0].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, width as i32 - 1);
    let max_x = p
        .iter()
        .map(|q| q[0].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, width as i32 - 1);
    let min_y = p
        .iter()
        .map(|q| q[1].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, height as i32 - 1);
    let max_y = p
        .iter()
        .map(|q| q[1].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, height as i32 - 1);
    let den = (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
    if den.abs() < EPS {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let a =
                ((p[1][1] - p[2][1]) * (px - p[2][0]) + (p[2][0] - p[1][0]) * (py - p[2][1])) / den;
            let b =
                ((p[2][1] - p[0][1]) * (px - p[2][0]) + (p[0][0] - p[2][0]) * (py - p[2][1])) / den;
            let c = 1.0 - a - b;
            if a >= -EPS && b >= -EPS && c >= -EPS {
                mask[y as usize * width + x as usize] = true;
            }
        }
    }
}

fn render_visible_three_quarter(svg: &mut String, projection: &Projection, map: SvgMap) {
    // Pixel depth buffer removes occluded proxy edges in the perspective-like fixed view.
    const D: f64 = 2.0;
    let points = projection
        .components
        .iter()
        .flat_map(|c| c.vertices.iter().map(|p| map.point(*p)))
        .collect::<Vec<_>>();
    let (Some(min_x), Some(max_x), Some(min_y), Some(max_y)) = (
        points.iter().map(|p| p[0]).reduce(f64::min),
        points.iter().map(|p| p[0]).reduce(f64::max),
        points.iter().map(|p| p[1]).reduce(f64::min),
        points.iter().map(|p| p[1]).reduce(f64::max),
    ) else {
        return;
    };
    let ox = min_x.floor();
    let oy = min_y.floor();
    let w = ((max_x - ox) * D).ceil().max(1.0) as usize;
    let h = ((max_y - oy) * D).ceil().max(1.0) as usize;
    if w * h > 4_000_000 {
        return;
    }
    let mut mask = vec![f64::NEG_INFINITY; w * h];
    for c in &projection.components {
        for t in &c.triangles {
            let p = t.vertices.map(|v| {
                let q = map.point(v);
                [(q[0] - ox) * D, (q[1] - oy) * D]
            });
            raster_depth_triangle(&mut mask, w, h, p, t.depth);
        }
    }
    for c in &projection.components {
        if !matches!(
            c.kind,
            ComponentKind::TriangleMesh | ComponentKind::PolygonMesh
        ) {
            continue;
        }
        let color = svg_color(Material::parse(&c.material));
        for edge in &c.edges {
            let ia = edge[0] as usize;
            let ib = edge[1] as usize;
            let a = map.point(c.vertices[ia]);
            let b = map.point(c.vertices[ib]);
            let steps = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) * D)
                .ceil()
                .max(1.0) as usize;
            let mut runs = Vec::new();
            let mut active = None;
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let x = ((q[0] - ox) * D).round() as i32;
                let y = ((q[1] - oy) * D).round() as i32;
                let visible = if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    false
                } else {
                    let z = mask[y as usize * w + x as usize];
                    let depth = c.depths[ia] + (c.depths[ib] - c.depths[ia]) * t;
                    z.is_finite() && depth + 1.0e-4 >= z
                };
                if visible && active.is_none() {
                    active = Some(q);
                }
                if !visible && let Some(start) = active.take() {
                    runs.push((start, q));
                }
            }
            if let Some(start) = active {
                runs.push((start, b));
            }
            for (start, end) in runs {
                writeln!(svg,"<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\" stroke-width=\"0.8\"/>",start[0],start[1],end[0],end[1]).unwrap();
            }
        }
    }
}
fn raster_depth_triangle(
    buffer: &mut [f64],
    width: usize,
    height: usize,
    p: [[f64; 2]; 3],
    d: [f64; 3],
) {
    let min_x = p
        .iter()
        .map(|q| q[0].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, width as i32 - 1);
    let max_x = p
        .iter()
        .map(|q| q[0].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, width as i32 - 1);
    let min_y = p
        .iter()
        .map(|q| q[1].floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, height as i32 - 1);
    let max_y = p
        .iter()
        .map(|q| q[1].ceil() as i32)
        .max()
        .unwrap_or(-1)
        .clamp(0, height as i32 - 1);
    let den = (p[1][1] - p[2][1]) * (p[0][0] - p[2][0]) + (p[2][0] - p[1][0]) * (p[0][1] - p[2][1]);
    if den.abs() < EPS {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let a =
                ((p[1][1] - p[2][1]) * (px - p[2][0]) + (p[2][0] - p[1][0]) * (py - p[2][1])) / den;
            let b =
                ((p[2][1] - p[0][1]) * (px - p[2][0]) + (p[0][0] - p[2][0]) * (py - p[2][1])) / den;
            let c = 1.0 - a - b;
            if a >= -EPS && b >= -EPS && c >= -EPS {
                let i = y as usize * width + x as usize;
                buffer[i] = buffer[i].max(a * d[0] + b * d[1] + c * d[2]);
            }
        }
    }
}

fn draw_overall_dimension(
    svg: &mut String,
    p: &Projection,
    map: SvgMap,
    y: f64,
    height: f64,
    value: f64,
    unit_label: &str,
) {
    let bounds = p.bounds;
    let a = map.point([bounds.min[0], bounds.min[1]]);
    let b = map.point([bounds.max[0], bounds.min[1]]);
    let bottom = y + height - 2.0;
    let line_y = if a[1].max(b[1]) + 14.0 < bottom {
        a[1].max(b[1]) + 12.0
    } else {
        a[1].min(b[1]) - 12.0
    };
    let color = "#333";
    for point in [a, b] {
        writeln!(svg,"<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{line_y:.2}\" stroke=\"{color}\" stroke-width=\"0.7\"/>",point[0],point[1],point[0]).unwrap();
    }
    writeln!(svg,"<line x1=\"{:.2}\" y1=\"{line_y:.2}\" x2=\"{:.2}\" y2=\"{line_y:.2}\" stroke=\"{color}\" stroke-width=\"0.8\"/>",a[0],b[0]).unwrap();
    for (xx, dir) in [(a[0], 1.0), (b[0], -1.0)] {
        writeln!(svg,"<path d=\"M{xx:.2},{line_y:.2}l{:.2},-3m{:.2},3l{:.2},3\" fill=\"none\" stroke=\"{color}\" stroke-width=\"0.8\"/>",dir*4.0,-dir*4.0,dir*4.0).unwrap();
    }
    writeln!(svg,"<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"9\" fill=\"{color}\">{value:.3} {unit_label}</text>",(a[0]+b[0])*0.5,line_y-3.0).unwrap();
}
fn svg_color(material: Material) -> &'static str {
    match material {
        Material::Wood => "#674126",
        Material::Steel => "#3f474c",
        Material::DarkSteel => "#252a2e",
        Material::Brass => "#9a6b27",
        Material::Rubber => "#30312f",
        Material::Leather => "#563324",
    }
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "payload", rename_all = "snake_case")]
pub enum ReferenceCommand {
    SetNodeTransform {
        node_id: String,
        transform: Transform,
    },
    SetNodePrimitive {
        node_id: String,
        primitive: Option<Primitive>,
    },
    SetNodeLabel {
        node_id: String,
        label: String,
    },
    AddNode {
        parent_id: String,
        node: GeometryNode,
    },
    RemoveNode {
        node_id: String,
    },
    ReplaceDimensions {
        dimensions: Vec<Dimension>,
    },
    ReplaceDrivingDimensions {
        dimensions: Vec<DrivingDimension>,
    },
    ReplaceConstraints {
        constraints: Vec<GeometryConstraint>,
    },
    Solve,
    SetName {
        name: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditOutcome {
    pub changed_ids: Vec<String>,
    pub revision: u64,
    pub validation: ValidationReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReferenceDocument {
    pub spec: GeometrySpec,
    #[serde(default)]
    undo: Vec<GeometrySpec>,
    #[serde(default)]
    redo: Vec<GeometrySpec>,
}
#[derive(Deserialize)]
struct ReferenceDocumentWire {
    spec: GeometrySpec,
    #[serde(default)]
    undo: Vec<GeometrySpec>,
    #[serde(default)]
    redo: Vec<GeometrySpec>,
}
impl<'de> Deserialize<'de> for ReferenceDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ReferenceDocumentWire::deserialize(deserializer)?;
        if wire.undo.len() > MAX_HISTORY || wire.redo.len() > MAX_HISTORY {
            return Err(de::Error::custom(
                "reference history exceeds the 128 snapshot limit",
            ));
        }
        let current = wire.spec.revision;
        let uid = wire.spec.asset_uid.as_str();
        for snapshot in wire.undo.iter().chain(wire.redo.iter()) {
            if snapshot.asset_uid != uid {
                return Err(de::Error::custom(
                    "history snapshot asset_uid does not match the document",
                ));
            }
            if snapshot.revision > current {
                return Err(de::Error::custom(
                    "history snapshot revision is newer than the current document",
                ));
            }
            if snapshot.validate().status == ValidationStatus::Invalid {
                return Err(de::Error::custom(
                    "history contains an invalid geometry snapshot",
                ));
            }
        }
        if wire.spec.validate().status == ValidationStatus::Invalid {
            return Err(de::Error::custom(
                "reference document contains invalid geometry",
            ));
        }
        Ok(Self {
            spec: wire.spec,
            undo: wire.undo,
            redo: wire.redo,
        })
    }
}
impl ReferenceDocument {
    pub fn new(spec: GeometrySpec) -> Self {
        Self {
            spec,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub fn execute(&mut self, command: ReferenceCommand) -> Result<EditOutcome, GeometryError> {
        let mut candidate = self.spec.clone();
        let changed = apply_command(&mut candidate, command)?;
        candidate.revision = self
            .spec
            .revision
            .checked_add(1)
            .ok_or(GeometryError::RevisionOverflow)?;
        let validation = candidate.validate();
        if validation.status == ValidationStatus::Invalid {
            return Err(GeometryError::Conflict(report_messages(&validation)));
        }
        push_bounded(&mut self.undo, self.spec.clone());
        self.redo.clear();
        self.spec = candidate;
        Ok(EditOutcome {
            changed_ids: changed,
            revision: self.spec.revision,
            validation,
        })
    }
    pub fn undo(&mut self) -> Result<EditOutcome, GeometryError> {
        let previous = self
            .undo
            .pop()
            .ok_or_else(|| GeometryError::Conflict("there is no command to undo".into()))?;
        let revision = self
            .spec
            .revision
            .checked_add(1)
            .ok_or(GeometryError::RevisionOverflow)?;
        let current = std::mem::replace(&mut self.spec, previous);
        push_bounded(&mut self.redo, current);
        self.spec.revision = revision;
        let validation = self.spec.validate();
        Ok(EditOutcome {
            changed_ids: vec!["document".into()],
            revision,
            validation,
        })
    }
    pub fn redo(&mut self) -> Result<EditOutcome, GeometryError> {
        let next = self
            .redo
            .pop()
            .ok_or_else(|| GeometryError::Conflict("there is no command to redo".into()))?;
        let revision = self
            .spec
            .revision
            .checked_add(1)
            .ok_or(GeometryError::RevisionOverflow)?;
        let current = std::mem::replace(&mut self.spec, next);
        push_bounded(&mut self.undo, current);
        self.spec.revision = revision;
        let validation = self.spec.validate();
        Ok(EditOutcome {
            changed_ids: vec!["document".into()],
            revision,
            validation,
        })
    }
}
fn push_bounded(stack: &mut Vec<GeometrySpec>, snapshot: GeometrySpec) {
    if stack.len() == MAX_HISTORY {
        stack.remove(0);
    }
    stack.push(snapshot);
}

fn apply_command(
    spec: &mut GeometrySpec,
    command: ReferenceCommand,
) -> Result<Vec<String>, GeometryError> {
    match command {
        ReferenceCommand::SetNodeTransform { node_id, transform } => {
            find_node_mut(&mut spec.root, &node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?
                .transform = transform;
            Ok(vec![node_id])
        }
        ReferenceCommand::SetNodePrimitive { node_id, primitive } => {
            find_node_mut(&mut spec.root, &node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?
                .primitive = primitive;
            Ok(vec![node_id])
        }
        ReferenceCommand::SetNodeLabel { node_id, label } => {
            find_node_mut(&mut spec.root, &node_id)
                .ok_or_else(|| GeometryError::NodeNotFound(node_id.clone()))?
                .label = label;
            Ok(vec![node_id])
        }
        ReferenceCommand::AddNode { parent_id, node } => {
            let ids = collect_node_ids(&node);
            find_node_mut(&mut spec.root, &parent_id)
                .ok_or_else(|| GeometryError::NodeNotFound(parent_id.clone()))?
                .children
                .push(node);
            Ok(ids)
        }
        ReferenceCommand::RemoveNode { node_id } => {
            if spec.root.id == node_id {
                return Err(GeometryError::Conflict(
                    "the root node cannot be removed".into(),
                ));
            }
            let mut ids = Vec::new();
            if !remove_node(&mut spec.root, &node_id, &mut ids) {
                return Err(GeometryError::NodeNotFound(node_id));
            }
            Ok(ids)
        }
        ReferenceCommand::ReplaceDimensions { dimensions } => {
            let ids = spec
                .dimensions
                .iter()
                .map(|v| v.id.clone())
                .chain(dimensions.iter().map(|v| v.id.clone()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            spec.dimensions = dimensions;
            Ok(ids)
        }
        ReferenceCommand::ReplaceConstraints { constraints } => {
            let ids = spec
                .constraints
                .iter()
                .map(|v| v.id().to_owned())
                .chain(constraints.iter().map(|v| v.id().to_owned()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            spec.constraints = constraints;
            Ok(ids)
        }
        ReferenceCommand::ReplaceDrivingDimensions { dimensions } => {
            let old_ids = spec
                .driving_dimensions
                .iter()
                .map(|dimension| dimension.id.clone())
                .collect::<BTreeSet<_>>();
            let new_ids = dimensions
                .iter()
                .map(|dimension| dimension.id.clone())
                .collect::<BTreeSet<_>>();
            let mut changed_ids = old_ids.union(&new_ids).cloned().collect::<Vec<_>>();
            spec.driving_dimensions = dimensions;
            changed_ids.extend(solve_driving_dimensions(spec)?);
            changed_ids.sort();
            changed_ids.dedup();
            Ok(changed_ids)
        }
        ReferenceCommand::Solve => {
            if spec.driving_dimensions.is_empty() {
                return Err(GeometryError::Conflict(
                    "the document has no driving dimensions to solve".into(),
                ));
            }
            solve_driving_dimensions(spec)
        }
        ReferenceCommand::SetName { name } => {
            spec.name = name;
            Ok(vec!["document".into()])
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ParameterSlot {
    Radius,
    Depth,
    BoxSize(Axis),
    Translation(Axis),
    Rotation(Axis),
}

fn normalized_assignment(
    assignment: &DrivingParameter,
) -> Result<(ParameterSlot, f64), GeometryError> {
    if !assignment.value.is_finite() {
        return Err(GeometryError::Invalid(format!(
            "driving value for '{}' must be finite",
            assignment.node_id
        )));
    }
    let (slot, value) = match assignment.parameter {
        PrimitiveParameter::Radius => (ParameterSlot::Radius, assignment.value),
        PrimitiveParameter::Diameter => (ParameterSlot::Radius, assignment.value * 0.5),
        PrimitiveParameter::Depth => (ParameterSlot::Depth, assignment.value),
        PrimitiveParameter::BoxSize(axis) => (ParameterSlot::BoxSize(axis), assignment.value),
        PrimitiveParameter::Translation(axis) => {
            (ParameterSlot::Translation(axis), assignment.value)
        }
        PrimitiveParameter::Rotation(axis) => (ParameterSlot::Rotation(axis), assignment.value),
    };
    if matches!(
        slot,
        ParameterSlot::Radius | ParameterSlot::Depth | ParameterSlot::BoxSize(_)
    ) && value <= 0.0
    {
        return Err(GeometryError::Invalid(format!(
            "driving value for '{}' must be greater than zero",
            assignment.node_id
        )));
    }
    Ok((slot, value))
}

fn solve_driving_dimensions(spec: &mut GeometrySpec) -> Result<Vec<String>, GeometryError> {
    let assignments = spec
        .driving_dimensions
        .iter()
        .map(|dimension| dimension.assignment.clone())
        .collect::<Vec<_>>();
    apply_assignments(spec, &assignments)
}

fn apply_assignments(
    spec: &mut GeometrySpec,
    assignments: &[DrivingParameter],
) -> Result<Vec<String>, GeometryError> {
    let mut values = BTreeMap::<(String, ParameterSlot), f64>::new();
    for assignment in assignments {
        let (slot, value) = normalized_assignment(assignment)?;
        let key = (assignment.node_id.clone(), slot);
        if let Some(previous) = values.insert(key, value)
            && (previous - value).abs() > EPS
        {
            return Err(GeometryError::Conflict(format!(
                "conflicting driving assignments target '{}' {:?}",
                assignment.node_id, assignment.parameter
            )));
        }
        let node = find_node(&spec.root, &assignment.node_id)
            .ok_or_else(|| GeometryError::NodeNotFound(assignment.node_id.clone()))?;
        read_primitive_parameter(node, assignment.parameter)?;
    }

    let mut root = spec.root.clone();
    let mut changed = BTreeSet::new();
    for assignment in assignments {
        let node = find_node_mut(&mut root, &assignment.node_id)
            .ok_or_else(|| GeometryError::NodeNotFound(assignment.node_id.clone()))?;
        set_primitive_parameter(node, assignment.parameter, assignment.value)?;
        changed.insert(assignment.node_id.clone());
    }
    spec.root = root;
    Ok(changed.into_iter().collect())
}

fn read_primitive_parameter(
    node: &GeometryNode,
    parameter: PrimitiveParameter,
) -> Result<f64, GeometryError> {
    let unsupported = || {
        GeometryError::Unsupported(format!(
            "node '{}' does not have a compatible {:?} parameter",
            node.id, parameter
        ))
    };
    let value = match parameter {
        PrimitiveParameter::Radius => match node.primitive.as_ref() {
            Some(Primitive::Cylinder { radius, .. })
            | Some(Primitive::Sphere { radius, .. })
            | Some(Primitive::Circle { radius, .. })
            | Some(Primitive::Arc { radius, .. }) => *radius,
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::Diameter => match node.primitive.as_ref() {
            Some(Primitive::Cylinder { radius, .. })
            | Some(Primitive::Sphere { radius, .. })
            | Some(Primitive::Circle { radius, .. })
            | Some(Primitive::Arc { radius, .. }) => radius * 2.0,
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::Depth => match node.primitive.as_ref() {
            Some(Primitive::Cylinder { depth, .. })
            | Some(Primitive::ExtrudedProfile { depth, .. }) => *depth,
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::BoxSize(axis) => match node.primitive.as_ref() {
            Some(Primitive::Box { size }) => size.at(axis),
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::Translation(axis) => node.transform.translation.at(axis),
        PrimitiveParameter::Rotation(axis) => node.transform.rotation.at(axis).to_degrees(),
    };
    Ok(value)
}

fn set_primitive_parameter(
    node: &mut GeometryNode,
    parameter: PrimitiveParameter,
    value: f64,
) -> Result<(), GeometryError> {
    let unsupported = || {
        GeometryError::Unsupported(format!(
            "node '{}' does not have a compatible {:?} parameter",
            node.id, parameter
        ))
    };
    match parameter {
        PrimitiveParameter::Radius | PrimitiveParameter::Diameter => {
            let radius = if parameter == PrimitiveParameter::Diameter {
                value * 0.5
            } else {
                value
            };
            match node.primitive.as_mut() {
                Some(Primitive::Cylinder { radius: target, .. })
                | Some(Primitive::Sphere { radius: target, .. })
                | Some(Primitive::Circle { radius: target, .. })
                | Some(Primitive::Arc { radius: target, .. }) => *target = radius,
                _ => return Err(unsupported()),
            }
        }
        PrimitiveParameter::Depth => match node.primitive.as_mut() {
            Some(Primitive::Cylinder { depth, .. })
            | Some(Primitive::ExtrudedProfile { depth, .. }) => *depth = value,
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::BoxSize(axis) => match node.primitive.as_mut() {
            Some(Primitive::Box { size }) => match axis {
                Axis::X => size.x = value,
                Axis::Y => size.y = value,
                Axis::Z => size.z = value,
            },
            _ => return Err(unsupported()),
        },
        PrimitiveParameter::Translation(axis) => match axis {
            Axis::X => node.transform.translation.x = value,
            Axis::Y => node.transform.translation.y = value,
            Axis::Z => node.transform.translation.z = value,
        },
        PrimitiveParameter::Rotation(axis) => match axis {
            Axis::X => node.transform.rotation.x = value.to_radians(),
            Axis::Y => node.transform.rotation.y = value.to_radians(),
            Axis::Z => node.transform.rotation.z = value.to_radians(),
        },
    }
    Ok(())
}

fn center_for_anchor(proxy: &Proxy, node_id: &str) -> Result<Vec3, GeometryError> {
    proxy
        .node_bounds
        .get(node_id)
        .map(|bounds| bounds.center())
        .ok_or_else(|| {
            GeometryError::Unsupported(format!(
                "constraint anchor '{node_id}' has no measurable geometry center"
            ))
        })
}

fn check_driving_dimensions(
    root: &GeometryNode,
    dimensions: &[DrivingDimension],
) -> Result<(), GeometryError> {
    for dimension in dimensions {
        let assignment = &dimension.assignment;
        let node = find_node(root, &assignment.node_id)
            .ok_or_else(|| GeometryError::NodeNotFound(assignment.node_id.clone()))?;
        let current = read_primitive_parameter(node, assignment.parameter)?;
        let tolerance = EPS * current.abs().max(assignment.value.abs()).max(1.0);
        if (current - assignment.value).abs() > tolerance {
            return Err(GeometryError::Conflict(format!(
                "driving dimension '{}' requires {:?} on '{}' to be {}, found {}",
                dimension.id, assignment.parameter, assignment.node_id, assignment.value, current
            )));
        }
    }
    Ok(())
}

fn check_relation(
    relation: &GeometryConstraint,
    spec: &GeometrySpec,
    proxy: &Proxy,
) -> Result<(), GeometryError> {
    let id = relation.id();
    let conflict = |detail: String| GeometryError::Conflict(format!("constraint '{id}': {detail}"));
    match relation {
        GeometryConstraint::Coincident {
            first_node,
            second_node,
            tolerance,
            ..
        } => {
            let distance = center_for_anchor(proxy, first_node)?
                .sub(center_for_anchor(proxy, second_node)?)
                .length();
            if distance > *tolerance {
                return Err(conflict(format!(
                    "anchors are {distance:.6} apart (tolerance {tolerance:.6})"
                )));
            }
        }
        GeometryConstraint::HorizontalInPlane {
            first_node,
            second_node,
            plane,
            tolerance,
            ..
        }
        | GeometryConstraint::VerticalInPlane {
            first_node,
            second_node,
            plane,
            tolerance,
            ..
        } => {
            let first = center_for_anchor(proxy, first_node)?;
            let second = center_for_anchor(proxy, second_node)?;
            let horizontal = matches!(relation, GeometryConstraint::HorizontalInPlane { .. });
            let aligned_axis = if horizontal {
                plane.vertical_axis()
            } else {
                plane.horizontal_axis()
            };
            let normal_axis = plane.normal_axis();
            let aligned_delta = (first.at(aligned_axis) - second.at(aligned_axis)).abs();
            let normal_delta = (first.at(normal_axis) - second.at(normal_axis)).abs();
            if aligned_delta > *tolerance || normal_delta > *tolerance {
                return Err(conflict(format!(
                    "anchors are not {} in plane {:?} (offsets {aligned_delta:.6}, {normal_delta:.6}; tolerance {tolerance:.6})",
                    if horizontal { "horizontal" } else { "vertical" },
                    plane
                )));
            }
        }
        GeometryConstraint::AxesPerpendicular {
            first_node,
            first_axis,
            second_node,
            second_axis,
            tolerance,
            ..
        } => {
            let dot = world_axis(&spec.root, first_node, *first_axis)?
                .dot(world_axis(&spec.root, second_node, *second_axis)?)
                .abs();
            if dot > *tolerance {
                return Err(conflict(format!(
                    "axes are not perpendicular (absolute dot {dot:.6}, tolerance {tolerance:.6})"
                )));
            }
        }
        GeometryConstraint::EqualLength {
            first_node,
            first_axis,
            second_node,
            second_axis,
            tolerance,
            ..
        } => {
            let first = proxy
                .node_bounds
                .get(first_node)
                .ok_or_else(|| GeometryError::NodeNotFound(first_node.clone()))?
                .extent()
                .at(*first_axis);
            let second = proxy
                .node_bounds
                .get(second_node)
                .ok_or_else(|| GeometryError::NodeNotFound(second_node.clone()))?
                .extent()
                .at(*second_axis);
            if (first - second).abs() > *tolerance {
                return Err(conflict(format!(
                    "lengths differ ({first:.6} vs {second:.6}; tolerance {tolerance:.6})"
                )));
            }
        }
        GeometryConstraint::EqualRadius {
            first_node,
            second_node,
            tolerance,
            ..
        } => {
            let first = read_primitive_parameter(
                find_node(&spec.root, first_node)
                    .ok_or_else(|| GeometryError::NodeNotFound(first_node.clone()))?,
                PrimitiveParameter::Radius,
            )?;
            let second = read_primitive_parameter(
                find_node(&spec.root, second_node)
                    .ok_or_else(|| GeometryError::NodeNotFound(second_node.clone()))?,
                PrimitiveParameter::Radius,
            )?;
            if (first - second).abs() > *tolerance {
                return Err(conflict(format!(
                    "radii differ ({first:.6} vs {second:.6}; tolerance {tolerance:.6})"
                )));
            }
        }
        GeometryConstraint::FixedDistance {
            first_node,
            second_node,
            distance: expected,
            tolerance,
            ..
        } => {
            let distance = center_for_anchor(proxy, first_node)?
                .sub(center_for_anchor(proxy, second_node)?)
                .length();
            if (distance - expected).abs() > *tolerance {
                return Err(conflict(format!(
                    "distance is {distance:.6}, expected {expected:.6} (tolerance {tolerance:.6})"
                )));
            }
        }
        GeometryConstraint::FixedAngle {
            first_node,
            first_axis,
            second_node,
            second_axis,
            angle_degrees,
            tolerance_degrees,
            ..
        } => {
            let first = world_axis(&spec.root, first_node, *first_axis)?;
            let second = world_axis(&spec.root, second_node, *second_axis)?;
            let angle = first.dot(second).clamp(-1.0, 1.0).acos().to_degrees();
            if (angle - angle_degrees).abs() > *tolerance_degrees {
                return Err(conflict(format!(
                    "angle is {angle:.4} degrees, expected {angle_degrees:.4} (tolerance {tolerance_degrees:.4})"
                )));
            }
        }
        GeometryConstraint::SymmetricAcrossPlane {
            first_node,
            second_node,
            plane,
            center_node,
            tolerance,
            ..
        } => {
            let first = center_for_anchor(proxy, first_node)?;
            let second = center_for_anchor(proxy, second_node)?;
            let center = center_node
                .as_deref()
                .map(|node_id| center_for_anchor(proxy, node_id))
                .transpose()?
                .unwrap_or(Vec3::ZERO);
            let normal = plane.normal_axis();
            for axis in [Axis::X, Axis::Y, Axis::Z] {
                let error = if axis == normal {
                    (first.at(axis) + second.at(axis) - 2.0 * center.at(axis)).abs()
                } else {
                    (first.at(axis) - second.at(axis)).abs()
                };
                if error > *tolerance {
                    return Err(conflict(format!(
                        "anchors are not symmetric across {:?} (axis {:?} error {error:.6}, tolerance {tolerance:.6})",
                        plane, axis
                    )));
                }
            }
        }
        GeometryConstraint::Concentric {
            first_node,
            first_axis,
            second_node,
            second_axis,
            tolerance,
            ..
        } => {
            let distance = center_for_anchor(proxy, first_node)?
                .sub(center_for_anchor(proxy, second_node)?)
                .length();
            let dot = world_axis(&spec.root, first_node, *first_axis)?
                .dot(world_axis(&spec.root, second_node, *second_axis)?)
                .abs();
            if distance > *tolerance || 1.0 - dot > *tolerance {
                return Err(conflict(format!(
                    "axes are not concentric (center offset {distance:.6}, parallel error {:.6}, tolerance {tolerance:.6})",
                    1.0 - dot
                )));
            }
        }
        GeometryConstraint::ComponentAlignment {
            first_node,
            second_node,
            axis,
            tolerance,
            ..
        } => {
            let first = center_for_anchor(proxy, first_node)?;
            let second = center_for_anchor(proxy, second_node)?;
            for cross_axis in [Axis::X, Axis::Y, Axis::Z] {
                if cross_axis != *axis {
                    let offset = (first.at(cross_axis) - second.at(cross_axis)).abs();
                    if offset > *tolerance {
                        return Err(conflict(format!(
                            "component centers are offset by {offset:.6} along {:?} (tolerance {tolerance:.6})",
                            cross_axis
                        )));
                    }
                }
            }
        }
        GeometryConstraint::RelativePosition {
            first_node,
            second_node,
            axis,
            relation,
            ..
        } => {
            let first = center_for_anchor(proxy, first_node)?.at(*axis);
            let second = center_for_anchor(proxy, second_node)?.at(*axis);
            let satisfied = match relation {
                PositionRelation::Before => first < second - EPS,
                PositionRelation::After => first > second + EPS,
            };
            if !satisfied {
                return Err(conflict(format!(
                    "node '{first_node}' is not {:?} node '{second_node}' along {:?}",
                    relation, axis
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn find_node<'a>(node: &'a GeometryNode, id: &str) -> Option<&'a GeometryNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find_node(child, id))
}

fn find_node_mut<'a>(node: &'a mut GeometryNode, id: &str) -> Option<&'a mut GeometryNode> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_node_mut(child, id))
}
fn remove_node(node: &mut GeometryNode, id: &str, removed: &mut Vec<String>) -> bool {
    if let Some(index) = node.children.iter().position(|child| child.id == id) {
        let subtree = node.children.remove(index);
        collect_node_ids(&subtree)
            .into_iter()
            .for_each(|value| removed.push(value));
        return true;
    }
    for child in &mut node.children {
        if remove_node(child, id, removed) {
            return true;
        }
    }
    false
}
fn collect_node_ids(node: &GeometryNode) -> Vec<String> {
    let mut ids = vec![node.id.clone()];
    for child in &node.children {
        ids.extend(collect_node_ids(child));
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_mesh_preserves_indices_and_rejects_loose_vertices() {
        let vertices = vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let triangles = vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]];
        let authored = Primitive::TriangleMesh {
            vertices: vertices.clone(),
            triangles: triangles.clone(),
        };
        let mesh = mesh_for(&authored).unwrap();
        assert_eq!(mesh.vertices, vertices);
        assert_eq!(mesh.triangles, triangles);
        assert_eq!(mesh.edges.len(), 6);
        let mut loose = vertices.clone();
        loose.push(Vec3::new(2.0, 2.0, 2.0));
        assert!(
            validate_authored_mesh(&loose, &triangles)
                .unwrap_err()
                .contains("unused")
        );
        assert!(
            validate_authored_mesh(&vertices, &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 8]])
                .is_err()
        );
        assert!(
            validate_authored_mesh(&vertices, &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 3, 0]])
                .is_err()
        );
    }

    fn reference_fixture(id: &str) -> GeometrySpec {
        let mut spec = GeometrySpec::shotgun();
        spec.asset_uid = id.into();
        spec.name = id.into();
        spec.root = GeometryNode::group("fixture", id);
        spec.parameter_provenance.clear();
        spec.dimensions.clear();
        spec.driving_dimensions.clear();
        spec.constraints.clear();
        spec
    }

    fn assert_bounds(actual: Bounds2, min: [f64; 2], max: [f64; 2]) {
        for (actual, expected) in actual
            .min
            .into_iter()
            .chain(actual.max)
            .zip(min.into_iter().chain(max))
        {
            assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
        }
    }

    #[test]
    fn double_barrel_reorients_all_principal_views_from_one_document() {
        let mut spec = reference_fixture("double-barrel");
        for (id, center) in [("first", -0.75), ("second", 0.75)] {
            let mut barrel = GeometryNode::primitive(
                id,
                id,
                Material::Steel,
                Primitive::Cylinder {
                    radius: 0.5,
                    depth: 4.0,
                    axis: Axis::X,
                    segments: 32,
                },
            );
            barrel.transform.translation.z = center;
            spec.root.children.push(barrel);
            spec.dimensions.push(Dimension {
                id: format!("{id}-length"),
                label: format!("{id} length"),
                target: DimensionTarget::NodeExtent {
                    node_id: id.into(),
                    axis: Axis::X,
                },
            });
        }
        let verify = |spec: &GeometrySpec, horizontal: bool| {
            let (y, z) = if horizontal { (1.25, 0.5) } else { (0.5, 1.25) };
            for (view, extent) in [
                (View::Front, [y, z]),
                (View::Side, [2.0, z]),
                (View::Top, [2.0, y]),
            ] {
                let projection = spec.project(view).unwrap();
                assert_eq!(projection.revision, spec.revision);
                assert_bounds(projection.bounds, extent.map(|v| -v), extent);
                for (component, offset) in projection.components.iter().zip([-0.75, 0.75]) {
                    let expected = match (view, horizontal) {
                        (View::Front, false) | (View::Side, false) | (View::Top, true) => {
                            [0.0, offset]
                        }
                        (View::Front, true) => [offset, 0.0],
                        _ => [0.0, 0.0],
                    };
                    let bounds = Bounds2::points(&component.vertices).unwrap();
                    for (i, center) in expected.into_iter().enumerate() {
                        assert!(((bounds.min[i] + bounds.max[i]) * 0.5 - center).abs() < 1e-9);
                    }
                }
            }
            assert!(
                spec.measure_dimensions()
                    .unwrap()
                    .iter()
                    .all(|d| d.value == Some(4.0))
            );
        };
        verify(&spec, false);
        let mut document = ReferenceDocument::new(spec);
        let rotation = Transform {
            rotation: Vec3::new(-std::f64::consts::FRAC_PI_2, 0.0, 0.0),
            ..Default::default()
        };
        document
            .execute(ReferenceCommand::SetNodeTransform {
                node_id: "fixture".into(),
                transform: rotation,
            })
            .unwrap();
        verify(&document.spec, true);
    }

    #[test]
    fn inclined_asymmetric_profile_has_expected_ortho_bounds() {
        let mut spec = reference_fixture("asymmetric-profile");
        let mut profile = GeometryNode::primitive(
            "ramp",
            "Rising profile",
            Material::Wood,
            Primitive::ExtrudedProfile {
                profile: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0]],
                depth: 0.4,
                axis: Axis::Y,
            },
        );
        profile.transform.rotation.y = -std::f64::consts::FRAC_PI_6;
        spec.root.children.push(profile);
        let mut cap = GeometryNode::primitive(
            "heel",
            "Offset heel",
            Material::Steel,
            Primitive::ExtrudedProfile {
                profile: vec![[-1.0, -0.5], [0.2, -0.5], [0.2, 0.2], [-0.8, 0.2]],
                depth: 0.8,
                axis: Axis::Y,
            },
        );
        cap.transform.translation.y = 0.3;
        spec.root.children.push(cap);
        let cosine = std::f64::consts::FRAC_PI_6.cos();
        for (view, min, max) in [
            (View::Front, [-0.2, -0.5], [0.7, 1.0 + cosine]),
            (View::Side, [-1.0, -0.5], [2.0 * cosine, 1.0 + cosine]),
            (View::Top, [-1.0, -0.2], [2.0 * cosine, 0.7]),
        ] {
            assert_bounds(spec.project(view).unwrap().bounds, min, max);
        }
    }

    #[test]
    fn shotgun_fixture_projects_and_measures_from_one_proxy() {
        let spec = GeometrySpec::shotgun();
        let report = spec.validate();
        assert_eq!(report.status, ValidationStatus::Valid, "{report:?}");
        let proxy = spec.proxy().unwrap();
        assert!(
            proxy
                .components
                .iter()
                .any(|c| c.component_id == "barrel" && !c.triangles.is_empty())
        );
        assert!(
            proxy
                .components
                .iter()
                .any(|c| c.component_id == "stock" && c.triangles.len() >= 14)
        );
        for (id, minimum_points) in [("stock", 40), ("receiver", 20), ("forearm", 30)] {
            let node = find_node(&spec.root, id).unwrap();
            assert!(
                matches!(
                    node.primitive.as_ref(),
                    Some(Primitive::ExtrudedProfile { profile, .. }) if profile.len() >= minimum_points
                ),
                "{id} should use a detailed hand-shaped profile"
            );
        }
        fn has_box(node: &GeometryNode) -> bool {
            matches!(&node.primitive, Some(Primitive::Box { .. }))
                || node.children.iter().any(has_box)
        }
        assert!(
            !has_box(&spec.root),
            "shotgun silhouette should not be box-built"
        );
        assert!(matches!(
            find_node(&spec.root, "barrel").and_then(|node| node.primitive.as_ref()),
            Some(Primitive::Cylinder { segments, .. }) if *segments >= 48
        ));
        assert!(
            proxy.node_bounds["stock"].max[0] >= proxy.node_bounds["receiver"].min[0],
            "stock must meet the receiver"
        );
        let length = spec
            .measure_dimensions()
            .unwrap()
            .into_iter()
            .find(|d| d.id == "overall-length")
            .unwrap()
            .value
            .unwrap();
        assert!((8.0..9.5).contains(&length));
        for view in View::all() {
            let projection = spec.project(view).unwrap();
            assert_eq!(projection.revision, spec.revision);
            assert_eq!(projection.convention, ProjectionConvention::ThirdAngle);
        }
    }

    #[test]
    fn reference_dimensions_measure_radius_diameter_angle_and_center() {
        let mut spec = GeometrySpec::shotgun();
        spec.dimensions.extend([
            Dimension {
                id: "barrel-radius-ref".into(),
                label: "Barrel radius".into(),
                target: DimensionTarget::Radius {
                    node_id: "barrel".into(),
                },
            },
            Dimension {
                id: "barrel-diameter-ref".into(),
                label: "Barrel diameter".into(),
                target: DimensionTarget::Diameter {
                    node_id: "barrel".into(),
                },
            },
            Dimension {
                id: "trigger-angle".into(),
                label: "Trigger angle".into(),
                target: DimensionTarget::Angle {
                    node_id: "trigger".into(),
                    axis: Axis::Y,
                },
            },
            Dimension {
                id: "barrel-center-x".into(),
                label: "Barrel center X".into(),
                target: DimensionTarget::CenterCoordinate {
                    node_id: "barrel".into(),
                    axis: Axis::X,
                },
            },
        ]);
        let measured = spec.measure_dimensions().unwrap();
        let value = |id: &str| {
            measured
                .iter()
                .find(|dimension| dimension.id == id)
                .unwrap()
        };
        assert_eq!(value("barrel-radius-ref").value, Some(0.095));
        assert_eq!(value("barrel-diameter-ref").value, Some(0.19));
        assert_eq!(value("barrel-radius-ref").unit_label, "au");
        assert_eq!(value("trigger-angle").unit_label, "°");
        assert!((value("trigger-angle").value.unwrap() + 0.18_f64.to_degrees()).abs() < 1.0e-9);
        assert!((value("barrel-center-x").value.unwrap() - 2.65).abs() < 1.0e-9);
    }

    #[test]
    fn concave_profile_uses_complete_ear_clipped_caps() {
        let polygon = [
            [-2.0, -1.0],
            [2.0, -1.0],
            [2.0, 1.0],
            [0.0, 0.0],
            [-2.0, 1.0],
        ];
        let triangles = triangulate(&polygon).unwrap();
        assert_eq!(triangles.len(), polygon.len() - 2);
        let sum = triangles
            .iter()
            .map(|t| orient(polygon[t[0]], polygon[t[1]], polygon[t[2]]).abs() * 0.5)
            .sum::<f64>();
        assert!((sum - area2(&polygon).abs()).abs() < 1.0e-9);
        let self_intersecting = [[-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0], [1.0, -1.0]];
        assert!(validate_polygon(&self_intersecting).is_err());
    }

    #[test]
    fn cylinder_axis_changes_orthographic_projection() {
        let mut spec = GeometrySpec {
            schema_version: GEOMETRY_SCHEMA_VERSION,
            asset_uid: "axis-test".into(),
            revision: 1,
            units: GeometryUnits::ArtisticRelative,
            name: "Axis test".into(),
            projection_convention: ProjectionConvention::ThirdAngle,
            technical_sheet_layout: TechnicalSheetLayout::ThreeViewPlusQuarter,
            parameter_provenance: Vec::new(),
            root: GeometryNode::group("root", "Root"),
            dimensions: Vec::new(),
            driving_dimensions: Vec::new(),
            constraints: Vec::new(),
        };
        spec.root.children.push(GeometryNode::primitive(
            "cylinder",
            "Cylinder",
            Material::Steel,
            Primitive::Cylinder {
                radius: 0.5,
                depth: 4.0,
                axis: Axis::X,
                segments: 12,
            },
        ));
        let side = spec.project(View::Side).unwrap().bounds.extent();
        let front = spec.project(View::Front).unwrap().bounds.extent();
        assert!(side[0] > 3.9);
        assert!(side[1] < 1.1);
        assert!(front[0] < 1.1);

        spec.root.children[0].primitive = Some(Primitive::Cylinder {
            radius: 0.5,
            depth: 4.0,
            axis: Axis::Z,
            segments: 12,
        });
        let vertical_side = spec.project(View::Side).unwrap().bounds.extent();
        assert!(vertical_side[1] > 3.9);
        assert!(vertical_side[0] < 1.1);
    }

    #[test]
    fn paired_longitudinal_components_update_every_orthographic_view() {
        let mut spec = GeometrySpec::shotgun();
        spec.constraints.clear();
        let mut secondary = GeometryNode::primitive(
            "barrel-secondary",
            "Second long barrel",
            Material::DarkSteel,
            Primitive::Cylinder {
                radius: 0.095,
                depth: 4.92,
                axis: Axis::X,
                segments: 20,
            },
        );
        secondary.transform.translation = Vec3::new(2.65, 0.28, 0.18);
        spec.root.children.push(secondary);
        let mut document = ReferenceDocument::new(spec);

        fn component_extents(projection: &Projection, id: &str) -> [f64; 2] {
            let component = projection
                .components
                .iter()
                .find(|component| component.component_id == id)
                .unwrap();
            let mut min = [f64::INFINITY; 2];
            let mut max = [f64::NEG_INFINITY; 2];
            for point in &component.vertices {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            [max[0] - min[0], max[1] - min[1]]
        }

        let horizontal = View::all()
            .into_iter()
            .map(|view| document.spec.project(view).unwrap())
            .collect::<Vec<_>>();
        for projection in &horizontal {
            assert_eq!(projection.revision, document.spec.revision);
            for id in ["barrel", "barrel-secondary"] {
                assert!(
                    projection
                        .components
                        .iter()
                        .any(|component| component.component_id == id)
                );
            }
        }
        let horizontal_side = horizontal
            .iter()
            .find(|projection| projection.view == View::Side)
            .unwrap();
        let horizontal_top = horizontal
            .iter()
            .find(|projection| projection.view == View::Top)
            .unwrap();
        let horizontal_front = horizontal
            .iter()
            .find(|projection| projection.view == View::Front)
            .unwrap();
        for id in ["barrel", "barrel-secondary"] {
            assert!(component_extents(horizontal_side, id)[0] > 4.9);
            assert!(component_extents(horizontal_top, id)[0] > 4.9);
            assert!(component_extents(horizontal_front, id)[0] < 0.3);
            assert!(component_extents(horizontal_front, id)[1] < 0.3);
        }

        for id in ["barrel", "barrel-secondary"] {
            let mut primitive = find_node(&document.spec.root, id)
                .unwrap()
                .primitive
                .clone()
                .unwrap();
            let Primitive::Cylinder { axis, .. } = &mut primitive else {
                unreachable!();
            };
            *axis = Axis::Z;
            document
                .execute(ReferenceCommand::SetNodePrimitive {
                    node_id: id.into(),
                    primitive: Some(primitive),
                })
                .unwrap();
        }

        let vertical = View::all()
            .into_iter()
            .map(|view| document.spec.project(view).unwrap())
            .collect::<Vec<_>>();
        for projection in &vertical {
            assert_eq!(projection.revision, document.spec.revision);
            for id in ["barrel", "barrel-secondary"] {
                assert!(
                    projection
                        .components
                        .iter()
                        .any(|component| component.component_id == id)
                );
            }
        }
        let side = vertical
            .iter()
            .find(|projection| projection.view == View::Side)
            .unwrap();
        let front = vertical
            .iter()
            .find(|projection| projection.view == View::Front)
            .unwrap();
        let top = vertical
            .iter()
            .find(|projection| projection.view == View::Top)
            .unwrap();
        for id in ["barrel", "barrel-secondary"] {
            assert!(component_extents(side, id)[1] > 4.9);
            assert!(component_extents(front, id)[1] > 4.9);
            assert!(
                component_extents(top, id)
                    .into_iter()
                    .all(|extent| extent < 0.3)
            );
        }
    }

    #[test]
    fn driving_dimensions_apply_atomically_and_solve_stored_values() {
        let mut legacy_spec = serde_json::to_value(GeometrySpec::shotgun()).unwrap();
        legacy_spec
            .as_object_mut()
            .unwrap()
            .remove("driving_dimensions");
        let legacy_spec: GeometrySpec = serde_json::from_value(legacy_spec).unwrap();
        assert!(legacy_spec.driving_dimensions.is_empty());

        let mut document = ReferenceDocument::new(GeometrySpec::shotgun());
        let dimensions = vec![
            DrivingDimension {
                id: "barrel-radius".into(),
                label: "Barrel radius".into(),
                assignment: DrivingParameter {
                    node_id: "barrel".into(),
                    parameter: PrimitiveParameter::Radius,
                    value: 0.12,
                },
                unit_label: "au".into(),
            },
            DrivingDimension {
                id: "barrel-depth".into(),
                label: "Barrel depth".into(),
                assignment: DrivingParameter {
                    node_id: "barrel".into(),
                    parameter: PrimitiveParameter::Depth,
                    value: 5.0,
                },
                unit_label: "au".into(),
            },
            DrivingDimension {
                id: "receiver-width".into(),
                label: "Receiver depth".into(),
                assignment: DrivingParameter {
                    node_id: "receiver".into(),
                    parameter: PrimitiveParameter::Depth,
                    value: 0.6,
                },
                unit_label: "au".into(),
            },
            DrivingDimension {
                id: "receiver-height-position".into(),
                label: "Receiver center Z".into(),
                assignment: DrivingParameter {
                    node_id: "receiver".into(),
                    parameter: PrimitiveParameter::Translation(Axis::Z),
                    value: 0.12,
                },
                unit_label: "au".into(),
            },
            DrivingDimension {
                id: "trigger-rotation".into(),
                label: "Trigger rotation".into(),
                assignment: DrivingParameter {
                    node_id: "trigger".into(),
                    parameter: PrimitiveParameter::Rotation(Axis::Y),
                    value: -10.0,
                },
                unit_label: "°".into(),
            },
        ];
        let edit = document
            .execute(ReferenceCommand::ReplaceDrivingDimensions { dimensions })
            .unwrap();
        assert_eq!(edit.validation.status, ValidationStatus::Valid);
        let barrel = find_node(&document.spec.root, "barrel").unwrap();
        assert_eq!(
            read_primitive_parameter(barrel, PrimitiveParameter::Radius).unwrap(),
            0.12
        );
        assert_eq!(
            read_primitive_parameter(barrel, PrimitiveParameter::Depth).unwrap(),
            5.0
        );
        let receiver = find_node(&document.spec.root, "receiver").unwrap();
        assert_eq!(
            read_primitive_parameter(receiver, PrimitiveParameter::Depth).unwrap(),
            0.6
        );
        assert_eq!(
            read_primitive_parameter(receiver, PrimitiveParameter::Translation(Axis::Z)).unwrap(),
            0.12
        );
        let trigger = find_node(&document.spec.root, "trigger").unwrap();
        assert_eq!(
            read_primitive_parameter(trigger, PrimitiveParameter::Rotation(Axis::Y)).unwrap(),
            -10.0
        );
        let measured = document
            .spec
            .measure_dimensions()
            .unwrap()
            .into_iter()
            .find(|dimension| dimension.id == "barrel-radius")
            .unwrap();
        assert_eq!(measured.value, Some(0.12));
        assert_eq!(measured.unit_label, "au");
        let sheet = render_technical_sheet_svg(&document.spec, 900, 700).unwrap();
        assert!(sheet.contains("Barrel depth: 5.000 au"));
        assert!(sheet.contains("Trigger rotation: -10.000 °"));

        let barrel = find_node_mut(&mut document.spec.root, "barrel").unwrap();
        if let Some(Primitive::Cylinder { radius, .. }) = &mut barrel.primitive {
            *radius = 0.15;
        }
        document.execute(ReferenceCommand::Solve).unwrap();
        let barrel = find_node(&document.spec.root, "barrel").unwrap();
        assert_eq!(
            read_primitive_parameter(barrel, PrimitiveParameter::Radius).unwrap(),
            0.12
        );

        let before_conflict = document.spec.clone();
        let error = document
            .execute(ReferenceCommand::SetNodePrimitive {
                node_id: "barrel".into(),
                primitive: Some(Primitive::Cylinder {
                    radius: 0.15,
                    depth: 5.0,
                    axis: Axis::X,
                    segments: 20,
                }),
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("driving dimension 'barrel-radius'")
        );
        assert_eq!(document.spec, before_conflict);
    }

    #[test]
    fn conflicting_driving_dimensions_fail_without_mutating_the_document() {
        let mut document = ReferenceDocument::new(GeometrySpec::shotgun());
        let before = document.spec.clone();
        let error = document
            .execute(ReferenceCommand::ReplaceDrivingDimensions {
                dimensions: vec![
                    DrivingDimension {
                        id: "radius".into(),
                        label: "Radius".into(),
                        assignment: DrivingParameter {
                            node_id: "barrel".into(),
                            parameter: PrimitiveParameter::Radius,
                            value: 0.1,
                        },
                        unit_label: "au".into(),
                    },
                    DrivingDimension {
                        id: "diameter".into(),
                        label: "Diameter".into(),
                        assignment: DrivingParameter {
                            node_id: "barrel".into(),
                            parameter: PrimitiveParameter::Diameter,
                            value: 0.3,
                        },
                        unit_label: "au".into(),
                    },
                ],
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicting driving assignments")
        );
        assert_eq!(document.spec, before);
        assert_eq!(document.undo_depth(), 0);
    }

    #[test]
    fn invalid_driving_and_relation_anchors_have_error_diagnostics() {
        let mut spec = GeometrySpec::shotgun();
        spec.driving_dimensions = vec![DrivingDimension {
            id: "missing-depth".into(),
            label: "Missing depth anchor".into(),
            assignment: DrivingParameter {
                node_id: "missing-node".into(),
                parameter: PrimitiveParameter::Depth,
                value: 1.0,
            },
            unit_label: "au".into(),
        }];
        spec.constraints.push(GeometryConstraint::Coincident {
            id: "missing-relation-anchor".into(),
            first_node: "missing-node".into(),
            second_node: "barrel".into(),
            tolerance: 0.01,
        });
        let report = spec.validate();
        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_driving_dimension_node")
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_constraint_node")
        );
    }

    #[test]
    fn typed_relation_conflicts_and_unsupported_anchors_keep_distinct_statuses() {
        let mut conflicting = GeometrySpec::shotgun();
        conflicting
            .constraints
            .push(GeometryConstraint::FixedDistance {
                id: "wrong-distance".into(),
                first_node: "barrel".into(),
                second_node: "muzzle".into(),
                distance: 0.1,
                tolerance: 0.001,
            });
        let report = conflicting.validate();
        assert_eq!(report.status, ValidationStatus::Invalid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "constraint_conflict")
        );

        let mut unsupported = GeometrySpec::shotgun();
        unsupported
            .constraints
            .push(GeometryConstraint::EqualRadius {
                id: "wood-radius".into(),
                first_node: "stock".into(),
                second_node: "barrel".into(),
                tolerance: 0.001,
            });
        let report = unsupported.validate();
        assert_eq!(report.status, ValidationStatus::Unsupported);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unsupported_constraint")
        );
    }

    #[test]
    fn view_bases_are_orthonormal_and_right_handed() {
        for view in View::all() {
            let basis = ViewBasis::for_view(view);
            assert!((basis.right.length() - 1.0).abs() < 1.0e-9);
            assert!((basis.up.length() - 1.0).abs() < 1.0e-9);
            assert!((basis.toward.length() - 1.0).abs() < 1.0e-9);
            assert!(basis.right.dot(basis.up).abs() < 1.0e-9);
            assert!(basis.right.dot(basis.toward).abs() < 1.0e-9);
            assert!(basis.up.dot(basis.toward).abs() < 1.0e-9);
            assert!(basis.right.cross(basis.up).dot(basis.toward) > 0.999_999);
        }
    }

    #[test]
    fn renderers_are_deterministic_and_sheet_marks_convention_and_dimensions() {
        let spec = GeometrySpec::shotgun();
        let options = PixelRenderOptions::default();
        let a = render_pixel_reference(&spec, View::Side, 160, 96, options).unwrap();
        let b = render_pixel_reference(&spec, View::Side, 160, 96, options).unwrap();
        assert_eq!(a, b);
        assert!(a.pixels().any(|pixel| pixel[3] > 0));
        let svg = render_technical_sheet_svg(&spec, 900, 700).unwrap();
        assert!(svg.contains("data-projection-convention=\"third_angle\""));
        assert!(svg.contains("Overall length"));
        assert!(svg.contains("data-scale-units=\"artistic_relative\""));
        assert!(svg.contains("<path"));
    }

    #[test]
    fn command_history_is_revisioned_serializable_and_atomic() {
        let mut document = ReferenceDocument::new(GeometrySpec::shotgun());
        let before = document.spec.revision;
        let mut transform = document
            .spec
            .root
            .children
            .iter()
            .find(|n| n.id == "barrel")
            .unwrap()
            .transform;
        transform.translation.z += 0.05;
        let edit = document
            .execute(ReferenceCommand::SetNodeTransform {
                node_id: "barrel".into(),
                transform,
            })
            .unwrap();
        assert_eq!(edit.changed_ids, vec!["barrel"]);
        assert_eq!(edit.revision, before + 1);
        assert!(document.can_undo());
        document.undo().unwrap();
        assert!(document.can_redo());
        document.redo().unwrap();
        let encoded = serde_json::to_string(&document).unwrap();
        let restored: ReferenceDocument = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.spec.revision, document.spec.revision);
        assert!(restored.can_undo());
        let revision = document.spec.revision;
        let bad = Transform {
            scale: Vec3::new(0.0, 1.0, 1.0),
            ..Transform::default()
        };
        assert!(
            document
                .execute(ReferenceCommand::SetNodeTransform {
                    node_id: "barrel".into(),
                    transform: bad
                })
                .is_err()
        );
        assert_eq!(document.spec.revision, revision);
    }

    #[test]
    fn drafts_are_editable_but_unsupported_geometry_is_never_claimed_ready() {
        let mut document = ReferenceDocument::new(GeometrySpec::shotgun());
        let draft = GeometryNode::group("future-part", "Unfinished part");
        let result = document
            .execute(ReferenceCommand::AddNode {
                parent_id: "shotgun".into(),
                node: draft,
            })
            .unwrap();
        assert_eq!(result.validation.status, ValidationStatus::Draft);
        let mut spec = GeometrySpec::shotgun();
        spec.root.children[0].primitive = Some(Primitive::Unsupported {
            kind: "spline_surface".into(),
            description: "No bounded tessellator is implemented.".into(),
        });
        assert_eq!(spec.validate().status, ValidationStatus::Unsupported);
        assert!(matches!(spec.proxy(), Err(GeometryError::Unsupported(_))));
    }
}
