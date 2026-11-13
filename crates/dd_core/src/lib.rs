//! Deterministic domain core shared by the native editor and the local MCP server.
//! This crate must never call image generation services or execute Blender scripts.

pub mod context;
pub mod geometry;
pub mod naming;
pub mod raster;
pub mod reference_validation;
pub mod storage;
pub mod uv;

pub use context::{
    AssetManifest, ProjectContext, ProjectSettings, TextureProfile, convert_to_meters,
    discover_assets, discover_projects, resolve_context, resolve_workspace_context, select_project,
};
