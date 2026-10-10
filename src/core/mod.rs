//! Core data structures for geometry representation.
//! 
//! This module provides the foundation for attribute-based geometry processing.
//! All types here are pure data containers with no procedural logic.

pub mod anim;
pub mod poly;
pub mod poly_ops;
pub mod anim_tools;
pub mod human;
pub mod human_ik;
pub mod pattern;
pub mod uv;
pub mod manip;
/// The geometry model of packed primitives, the nodes and ICE: typed
/// attribute columns per context.
pub mod geo;
