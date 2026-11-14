//! CPU drawing of the on-screen `ui`, without I/O: the scene cut into items and rasterised
//! into tiles ([`render`]), the plan that brings an output's layer from one scene to the next ([`layer`]), and the
//! blend of `ui` pixels over NV12 pictures ([`nv12`]).

pub mod layer;
pub mod nv12;
pub mod render;

/// The rasteriser's pixmaps, for whoever hands the renderer pictures.
pub use tiny_skia;
