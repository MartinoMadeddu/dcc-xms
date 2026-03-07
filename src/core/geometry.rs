use bevy::prelude::*;
use std::collections::HashMap;
use super::{Attribute, AttributeData, Topology};

/// Core geometry representation
/// 
/// This is the fundamental data structure for all geometry in the system.
/// It stores:
/// - Point positions (P attribute)
/// - Topology (connectivity)
/// - Arbitrary typed attributes
#[derive(Clone, Debug, Default)]
pub struct Geometry {
    /// Point positions (always present)
    pub points: Vec<Vec3>,
    
    /// Topology (defines structure)
    pub topology: Topology,
    
    /// All other attributes (Cd, N, uv, custom, etc.)
    pub attributes: HashMap<String, AttributeData>,
}

impl Geometry {
    /// Create empty geometry
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a point cloud
    pub fn from_points(points: Vec<Vec3>) -> Self {
        Self {
            points,
            topology: Topology::Points,
            attributes: HashMap::new(),
        }
    }

    /// Create a triangle mesh
    pub fn from_triangles(points: Vec<Vec3>, indices: Vec<usize>) -> Self {
        Self {
            points,
            topology: Topology::triangles(indices),
            attributes: HashMap::new(),
        }
    }

    /// Number of points
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// Number of primitives (faces, curves, etc.)
    pub fn primitive_count(&self) -> usize {
        self.topology.primitive_count()
    }

    /// Get point positions as an attribute
    pub fn get_p(&self) -> Attribute<Vec3> {
        Attribute::new("P", self.points.clone())
    }

    /// Set point positions from an attribute
    pub fn set_p(&mut self, attr: &Attribute<Vec3>) {
        self.points = attr.data.clone();
    }

    /// Get a typed attribute
    pub fn get_attribute<T: Clone + 'static>(&self, name: &str) -> Option<Attribute<T>> {
        let attr_data = self.attributes.get(name)?;
        
        // This is a bit hacky but works for our supported types
        // In a more sophisticated system, you'd use proper trait-based dispatch
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>() {
            attr_data.as_float().map(|a| unsafe {
                std::mem::transmute_copy(a)
            })
        } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<Vec3>() {
            attr_data.as_vec3().map(|a| unsafe {
                std::mem::transmute_copy(a)
            })
        } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i32>() {
            attr_data.as_int().map(|a| unsafe {
                std::mem::transmute_copy(a)
            })
        } else {
            None
        }
    }

    /// Set an f32 attribute
    pub fn set_float_attribute(&mut self, attr: Attribute<f32>) {
        self.attributes.insert(attr.name.clone(), AttributeData::Float(attr));
    }

    /// Set a Vec3 attribute
    pub fn set_vec3_attribute(&mut self, attr: Attribute<Vec3>) {
        self.attributes.insert(attr.name.clone(), AttributeData::Vec3(attr));
    }

    /// Set an i32 attribute
    pub fn set_int_attribute(&mut self, attr: Attribute<i32>) {
        self.attributes.insert(attr.name.clone(), AttributeData::Int(attr));
    }

    /// Get an f32 attribute
    pub fn get_float_attribute(&self, name: &str) -> Option<&Attribute<f32>> {
        self.attributes.get(name)?.as_float()
    }

    /// Get a Vec3 attribute
    pub fn get_vec3_attribute(&self, name: &str) -> Option<&Attribute<Vec3>> {
        self.attributes.get(name)?.as_vec3()
    }

    /// Get an i32 attribute
    pub fn get_int_attribute(&self, name: &str) -> Option<&Attribute<i32>> {
        self.attributes.get(name)?.as_int()
    }

    /// Check if an attribute exists
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    /// Remove an attribute
    pub fn remove_attribute(&mut self, name: &str) -> Option<AttributeData> {
        self.attributes.remove(name)
    }

    /// List all attribute names
    pub fn attribute_names(&self) -> Vec<&String> {
        self.attributes.keys().collect()
    }

    /// Clear all attributes (keeps points and topology)
    pub fn clear_attributes(&mut self) {
        self.attributes.clear();
    }
}

impl std::fmt::Display for Geometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Geometry[{} points, {}, {} attributes]",
            self.point_count(),
            self.topology,
            self.attributes.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_cloud_creation() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let geo = Geometry::from_points(points);
        
        assert_eq!(geo.point_count(), 3);
        assert!(geo.topology.is_points());
    }

    #[test]
    fn test_triangle_mesh_creation() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let indices = vec![0, 1, 2];
        let geo = Geometry::from_triangles(points, indices);
        
        assert_eq!(geo.point_count(), 3);
        assert_eq!(geo.primitive_count(), 1);
        assert!(geo.topology.is_polymesh());
    }

    #[test]
    fn test_attribute_storage() {
        let mut geo = Geometry::from_points(vec![Vec3::ZERO; 3]);
        
        // Set a density attribute
        let density = Attribute::new("density", vec![1.0, 2.0, 3.0]);
        geo.set_float_attribute(density);
        
        // Retrieve it
        let retrieved = geo.get_float_attribute("density").unwrap();
        assert_eq!(retrieved.data, vec![1.0, 2.0, 3.0]);
        
        // Check it exists
        assert!(geo.has_attribute("density"));
        assert!(!geo.has_attribute("nonexistent"));
    }

    #[test]
    fn test_p_attribute() {
        let points = vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)];
        let mut geo = Geometry::from_points(points.clone());
        
        // Get P
        let p = geo.get_p();
        assert_eq!(p.data, points);
        
        // Modify P
        let new_points = vec![Vec3::new(10.0, 20.0, 30.0), Vec3::new(40.0, 50.0, 60.0)];
        let new_p = Attribute::new("P", new_points.clone());
        geo.set_p(&new_p);
        
        assert_eq!(geo.points, new_points);
    }
}