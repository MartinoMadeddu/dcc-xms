//! Core data structures for geometry representation.
//! 
//! This module provides the foundation for attribute-based geometry processing.
//! All types here are pure data containers with no procedural logic.

pub mod attribute;
pub mod geometry;
pub mod topology;

// Re-export main types for convenience
pub use attribute::{Attribute, AttributeData, AttributeScope};
pub use geometry::Geometry;
pub use topology::Topology;