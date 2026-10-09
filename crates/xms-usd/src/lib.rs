//! `xms-usd`: OpenUSD stage → [`xms_scene::Scene`] translation for XMS | Imago.
//!
//! Forked from rray-usd @ <rray commit hash>, 2026-10-09. Only the translator side
//! is kept: composition (sublayers, references, payloads, variants, inherits…) is
//! resolved by `openusd`, and `to_scene` captures the composed stage as raw,
//! USD-shaped data. rray's renderer side (from_scene, emit_* builders, BLAS,
//! materials → OpenPBR, lights, session) is deliberately not here.
//!
//! | module | role |
//! |---|---|
//! | `to_scene` | USD stage → scene layer: the only code reading the stage |
//! | `values` | `sdf::Value` conversions |
//! | `xform` | xformOps, quaternions |
//! | `curves` | curve UV-set names (shared constant) |
//! | `materialx` | `.mtlx` references authored in the root `.usda` layer |
//!
//! `openusd`'s `Stage` is `!Send` (built on `Rc`): translate on one thread and
//! send only the resulting [`xms_scene::Scene`] across threads.

// Fast non-cryptographic hashing for internal maps
use rustc_hash::FxHashMap as HashMap;

use nalgebra::{Matrix4, Rotation3, Vector3};
use openusd::gf;
use openusd::sdf::{Value, ValueKind};
use openusd::usd::Prim;

mod curves;
mod materialx;
mod to_scene;
mod values;
mod xform;
// mod export;   // later: .usda override-layer export (needs adapting to xms-scene)

pub use to_scene::{translate, translate_stage, SceneOptions, Translated, CURVE_PRIMVAR_PREFIX};
/// rray's name for [`translate`], kept so code ported from rray reads the same.
pub use to_scene::translate as translate_to_scene;

// Crate-internal helpers, visible to every module through `use crate::*`
use values::*;

type M4 = Matrix4<f64>;