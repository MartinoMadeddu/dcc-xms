use crate::core::{Attribute, Geometry};
use bevy::prelude::*;
use std::collections::HashMap;

/// Execution context holds geometry and attributes during node evaluation
#[derive(Clone, Debug, Default)]
pub struct ExecutionContext {
    /// Primary geometry being processed
    pub geometry: Geometry,
    
    /// Additional named geometries (for GetGeometry, templates, etc.)
    pub external_geometry: HashMap<String, Geometry>,
}

impl ExecutionContext {
    pub fn new() -> Self {
        Self {
            geometry: Geometry::new(),
            external_geometry: HashMap::new(),
        }
    }

    /// Create from existing geometry
    pub fn from_geometry(geo: Geometry) -> Self {
        Self {
            geometry: geo,
            external_geometry: HashMap::new(),
        }
    }

    // ── Convenient attribute accessors (shortcuts to geometry.attributes) ──

    pub fn get_float(&self, name: &str) -> Result<&Attribute<f32>, String> {
        if name == "P" {
            return Err("Use get_vec3 for P (position) attribute".into());
        }
        self.geometry.get_float_attribute(name)
            .ok_or_else(|| format!("Attribute '{}' not found or wrong type (expected Float)", name))
    }

    pub fn get_vec3(&self, name: &str) -> Result<Attribute<Vec3>, String> {
        if name == "P" {
            return Ok(self.geometry.get_p());
        }
        self.geometry.get_vec3_attribute(name)
            .cloned()
            .ok_or_else(|| format!("Attribute '{}' not found or wrong type (expected Vec3)", name))
    }

    pub fn get_int(&self, name: &str) -> Result<&Attribute<i32>, String> {
        self.geometry.get_int_attribute(name)
            .ok_or_else(|| format!("Attribute '{}' not found or wrong type (expected Int)", name))
    }

    pub fn set_float(&mut self, attr: Attribute<f32>) {
        self.geometry.set_float_attribute(attr);
    }

    pub fn set_vec3(&mut self, attr: Attribute<Vec3>) {
        if attr.name == "P" {
            self.geometry.set_p(&attr);
        } else {
            self.geometry.set_vec3_attribute(attr);
        }
    }

    pub fn set_int(&mut self, attr: Attribute<i32>) {
        self.geometry.set_int_attribute(attr);
    }

    // ── Geometry access ──

    pub fn point_count(&self) -> usize {
        self.geometry.point_count()
    }

    /// Add external geometry (for GetGeometry node, templates, etc.)
    pub fn add_external_geometry(&mut self, name: impl Into<String>, geo: Geometry) {
        self.external_geometry.insert(name.into(), geo);
    }

    /// Get external geometry by name
    pub fn get_external_geometry(&self, name: &str) -> Option<&Geometry> {
        self.external_geometry.get(name)
    }
}

/// All ICE nodes implement this trait
pub trait IceNode: Send + Sync {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String>;

    fn name(&self) -> &str {
        "IceNode"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Attribute;

    #[test]
    fn test_context_float_storage() {
        let mut ctx = ExecutionContext::new();
        
        let density = Attribute::new("density", vec![1.0, 2.0, 3.0]);
        ctx.set_float(density);
        
        let retrieved = ctx.get_float("density").unwrap();
        assert_eq!(retrieved.data, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_context_vec3_storage() {
        let mut ctx = ExecutionContext::new();
        
        let positions = Attribute::new("P", vec![
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
        ]);
        ctx.set_vec3(positions);
        
        let retrieved = ctx.get_vec3("P").unwrap();
        assert_eq!(retrieved.len(), 2);
    }

    #[test]
    fn test_context_missing_attribute() {
        let ctx = ExecutionContext::new();
        
        let result = ctx.get_float("nonexistent");
        assert!(result.is_err());
    }
}