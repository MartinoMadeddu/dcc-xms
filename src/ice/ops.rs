//! What ICE nodes run on: the geometry being processed, as `Geo`, and the
//! other geometries a tree can read (the template).
//!
//! Values are read and written per point, as whole arrays: `P` is the
//! points; any other name is a per-point attribute of the geometry. A
//! written attribute replaces its column, so everything a tree does not
//! touch stays shared with the input.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use crate::core::geo::{Attr, Column, Context, Geo, Role, POINTS};

/// A named array of values, as nodes pass them: a per-point attribute read
/// from the geometry, or one to write to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute<T> {
    pub name: String,
    pub data: Vec<T>,
}

impl<T> Attribute<T> {
    pub fn new(name: impl Into<String>, data: Vec<T>) -> Self {
        Self { name: name.into(), data }
    }
    pub fn len(&self) -> usize { self.data.len() }
    pub fn is_empty(&self) -> bool { self.data.is_empty() }
}

/// The geometry a tree works on, and the others it can read.
#[derive(Clone, Debug, Default)]
pub struct ExecutionContext {
    /// Primary geometry being processed
    pub geometry: Geo,
    /// Additional named geometries (the template)
    pub external_geometry: HashMap<String, Geo>,
}

/// The column an ICE name stands for: `P` is the points.
fn column_name(name: &str) -> &str {
    if name == "P" { POINTS } else { name }
}

impl ExecutionContext {
    pub fn new() -> Self { Self::default() }

    pub fn from_geometry(geo: Geo) -> Self {
        Self { geometry: geo, external_geometry: HashMap::new() }
    }

    fn attr(&self, name: &str) -> Option<&Attr> {
        self.geometry.attr(Context::Point, column_name(name))
    }

    /// A per-point attribute's values, indices resolved.
    fn values<T: Copy>(&self, name: &str, get: impl Fn(&Column) -> Option<&[T]>, kind: &str) -> Result<Vec<T>, String> {
        let attr = self.attr(name).ok_or_else(|| format!("Attribute '{name}' not found"))?;
        let values = get(&attr.column).ok_or_else(|| format!("Attribute '{name}' is not {kind}"))?;
        (0..attr.len())
            .map(|e| attr.value_index(e).and_then(|i| values.get(i).copied()))
            .collect::<Option<Vec<T>>>()
            .ok_or_else(|| format!("Attribute '{name}' has an index past its values"))
    }

    pub fn get_float(&self, name: &str) -> Result<Attribute<f32>, String> {
        self.values(name, |c| c.floats(), "Float").map(|d| Attribute::new(name, d))
    }

    pub fn get_vec3(&self, name: &str) -> Result<Attribute<Vec3>, String> {
        self.values(name, |c| c.vec3s(), "Vec3").map(|d| Attribute::new(name, d.into_iter().map(Vec3::from_array).collect()))
    }

    pub fn get_int(&self, name: &str) -> Result<Attribute<i32>, String> {
        self.values(name, |c| c.ints(), "Int").map(|d| Attribute::new(name, d))
    }

    fn set(&mut self, name: &str, column: Column, role: Role) {
        self.geometry.set(Context::Point, column_name(name), Attr::new(column, role));
    }

    pub fn set_float(&mut self, attr: Attribute<f32>) {
        self.set(&attr.name, Column::Float(Arc::new(attr.data)), Role::None);
    }

    pub fn set_vec3(&mut self, attr: Attribute<Vec3>) {
        let role = if attr.name == "P" { Role::Point } else { Role::Vector };
        let name = attr.name.clone();
        self.set(&name, Column::Vec3(Arc::new(attr.data.into_iter().map(|v| v.to_array()).collect())), role);
    }

    pub fn set_int(&mut self, attr: Attribute<i32>) {
        self.set(&attr.name, Column::Int(Arc::new(attr.data)), Role::None);
    }

    pub fn point_count(&self) -> usize {
        self.geometry.point_count()
    }

    /// Add external geometry (for GetGeometry node, templates, etc.)
    pub fn add_external_geometry(&mut self, name: impl Into<String>, geo: Geo) {
        self.external_geometry.insert(name.into(), geo);
    }

    /// Get external geometry by name
    pub fn get_external_geometry(&self, name: &str) -> Option<&Geo> {
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

    #[test]
    fn test_context_float_storage() {
        let mut ctx = ExecutionContext::from_geometry(Geo::from_points(vec![[0.0; 3]; 3]));
        ctx.set_float(Attribute::new("density", vec![1.0, 2.0, 3.0]));
        assert_eq!(ctx.get_float("density").unwrap().data, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn p_is_the_points() {
        let mut ctx = ExecutionContext::new();
        ctx.set_vec3(Attribute::new("P", vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)]));
        assert_eq!(ctx.get_vec3("P").unwrap().len(), 2);
        assert_eq!(ctx.geometry.points()[1], [4.0, 5.0, 6.0]);
    }

    #[test]
    fn writing_one_attribute_leaves_the_rest_shared() {
        let mut geo = Geo::from_points(vec![[0.0; 3]; 2]);
        geo.set(Context::Point, "heat", Attr::new(Column::Float(Arc::new(vec![1.0, 2.0])), Role::None));
        let mut ctx = ExecutionContext::from_geometry(geo.clone());
        ctx.set_vec3(Attribute::new("P", vec![Vec3::X, Vec3::Y]));
        assert!(ctx.geometry.attr(Context::Point, "heat").unwrap().same(geo.attr(Context::Point, "heat").unwrap()));
    }

    #[test]
    fn test_context_missing_attribute() {
        assert!(ExecutionContext::new().get_float("nonexistent").is_err());
    }
}
