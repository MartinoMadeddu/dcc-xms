//! Forked from rray-scene @ <rray commit hash>, 2026-10-09.
//!
//! rray's scene layer: a USD-shaped, path-keyed, time-sampled scene description
//! with Hydra-style change tracking. Scene sources (the USD importer, a DCC such
//! as XMS) fill it; the renderer *syncs* its own structures (BVHs, compiled
//! materials, light sets) from it and, after edits, rebuilds only what changed.
//!
//! * Prims mirror UsdGeom / UsdShade / UsdLux / UsdRender schemas ([`prim`]), so
//!   edits are USD attribute changes and can be written back as override layers.
//! * Values that USD can animate are [`Sampled`]: linear between samples, held
//!   outside them, with [`Sampled::within`] giving what a shutter interval needs.
//! * Transforms are double precision ([`Mat4d`], USD's row-vector convention).
//! * Dependency-free, so any DCC can use it without pulling in the renderer.

pub mod math;
pub mod path;
pub mod prim;
pub mod scene;
pub mod time;

pub use math::{nlerp, Mat4d, Quatf, Vec2f, Vec3f, Vec4f};
pub use path::Path;
pub use prim::*;
pub use scene::{Changes, Dirty, Scene, StageInfo, Summary};
pub use time::{Lerp, Sampled, SceneTime};

#[cfg(test)]
mod tests;
