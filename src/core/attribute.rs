use bevy::prelude::*;
use std::fmt;

/// Attribute scope - defines where the attribute lives on the geometry
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttributeScope {
    /// Per point (most common - one value per point)
    Point,
    /// Per vertex (for split attributes at edges)
    Vertex,
    /// Per primitive/face
    Primitive,
    /// Single value for entire geometry
    Detail,
}

impl Default for AttributeScope {
    fn default() -> Self {
        Self::Point
    }
}

/// Generic typed attribute container
/// 
/// Stores an array of values of type T with metadata about scope and name.
/// This is the fundamental data structure for all ICE operations.
#[derive(Clone, Debug)]
pub struct Attribute<T: Clone> {
    pub name: String,
    pub data: Vec<T>,
    pub scope: AttributeScope,
}

impl<T: Clone> Attribute<T> {
    /// Create a new point-scoped attribute
    pub fn new(name: impl Into<String>, data: Vec<T>) -> Self {
        Self {
            name: name.into(),
            data,
            scope: AttributeScope::Point,
        }
    }

    /// Create with explicit scope
    pub fn new_with_scope(name: impl Into<String>, data: Vec<T>, scope: AttributeScope) -> Self {
        Self {
            name: name.into(),
            data,
            scope,
        }
    }

    /// Number of elements
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Builder pattern: set scope
    pub fn with_scope(mut self, scope: AttributeScope) -> Self {
        self.scope = scope;
        self
    }

    /// Get element at index
    pub fn get(&self, index: usize) -> Option<&T> {
        self.data.get(index)
    }

    /// Set element at index
    pub fn set(&mut self, index: usize, value: T) -> Result<(), String> {
        if index < self.data.len() {
            self.data[index] = value;
            Ok(())
        } else {
            Err(format!("Index {} out of bounds (len: {})", index, self.data.len()))
        }
    }

    /// Map over all elements, producing a new attribute
    pub fn map<U: Clone, F>(&self, f: F) -> Attribute<U>
    where
        F: Fn(&T) -> U,
    {
        Attribute {
            name: self.name.clone(),
            data: self.data.iter().map(f).collect(),
            scope: self.scope,
        }
    }

    /// Zip with another attribute, producing a new attribute
    /// Returns error if lengths don't match
    pub fn zip_map<U: Clone, V: Clone, F>(&self, other: &Attribute<U>, f: F) -> Result<Attribute<V>, String>
    where
        F: Fn(&T, &U) -> V,
    {
        if self.len() != other.len() {
            return Err(format!(
                "Attribute length mismatch: {} has {}, {} has {}",
                self.name, self.len(), other.name, other.len()
            ));
        }

        Ok(Attribute {
            name: self.name.clone(),
            data: self.data.iter()
                .zip(&other.data)
                .map(|(a, b)| f(a, b))
                .collect(),
            scope: self.scope,
        })
    }
}

impl<T: Clone + fmt::Display> fmt::Display for Attribute<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Attribute<{}>[\"{}\", {:?}, {} elements]", 
            std::any::type_name::<T>(), 
            self.name, 
            self.scope, 
            self.len()
        )
    }
}

/// Type-erased attribute storage for runtime attribute management
/// 
/// This allows storing different attribute types in a single collection.
/// ICE nodes work with typed Attribute<T>, but the geometry stores AttributeData.
#[derive(Clone, Debug)]
pub enum AttributeData {
    Float(Attribute<f32>),
    Vec3(Attribute<Vec3>),
    Int(Attribute<i32>),
    Vec2(Attribute<Vec2>),
    Vec4(Attribute<Vec4>),
    // Add more types as needed
}

impl AttributeData {
    /// Get the attribute name
    pub fn name(&self) -> &str {
        match self {
            AttributeData::Float(a) => &a.name,
            AttributeData::Vec3(a) => &a.name,
            AttributeData::Int(a) => &a.name,
            AttributeData::Vec2(a) => &a.name,
            AttributeData::Vec4(a) => &a.name,
        }
    }

    /// Get the attribute scope
    pub fn scope(&self) -> AttributeScope {
        match self {
            AttributeData::Float(a) => a.scope,
            AttributeData::Vec3(a) => a.scope,
            AttributeData::Int(a) => a.scope,
            AttributeData::Vec2(a) => a.scope,
            AttributeData::Vec4(a) => a.scope,
        }
    }

    /// Get the number of elements
    pub fn len(&self) -> usize {
        match self {
            AttributeData::Float(a) => a.len(),
            AttributeData::Vec3(a) => a.len(),
            AttributeData::Int(a) => a.len(),
            AttributeData::Vec2(a) => a.len(),
            AttributeData::Vec4(a) => a.len(),
        }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Try to extract as Float attribute
    pub fn as_float(&self) -> Option<&Attribute<f32>> {
        match self {
            AttributeData::Float(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract as Vec3 attribute
    pub fn as_vec3(&self) -> Option<&Attribute<Vec3>> {
        match self {
            AttributeData::Vec3(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract as Int attribute
    pub fn as_int(&self) -> Option<&Attribute<i32>> {
        match self {
            AttributeData::Int(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract as Vec2 attribute
    pub fn as_vec2(&self) -> Option<&Attribute<Vec2>> {
        match self {
            AttributeData::Vec2(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract as Vec4 attribute
    pub fn as_vec4(&self) -> Option<&Attribute<Vec4>> {
        match self {
            AttributeData::Vec4(a) => Some(a),
            _ => None,
        }
    }
}

impl fmt::Display for AttributeData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttributeData::Float(a) => write!(f, "Float({})", a),
            AttributeData::Vec3(a) => write!(f, "Vec3({})", a),
            AttributeData::Int(a) => write!(f, "Int({})", a),
            AttributeData::Vec2(a) => write!(f, "Vec2({})", a),
            AttributeData::Vec4(a) => write!(f, "Vec4({})", a),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribute_creation() {
        let attr = Attribute::new("test", vec![1.0f32, 2.0, 3.0]);
        assert_eq!(attr.name, "test");
        assert_eq!(attr.len(), 3);
        assert_eq!(attr.scope, AttributeScope::Point);
    }

    #[test]
    fn test_attribute_map() {
        let attr = Attribute::new("values", vec![1.0f32, 2.0, 3.0]);
        let doubled = attr.map(|&x| x * 2.0);
        assert_eq!(doubled.data, vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn test_attribute_zip_map() {
        let a = Attribute::new("a", vec![1.0f32, 2.0, 3.0]);
        let b = Attribute::new("b", vec![10.0f32, 20.0, 30.0]);
        let sum = a.zip_map(&b, |&x, &y| x + y).unwrap();
        assert_eq!(sum.data, vec![11.0, 22.0, 33.0]);
    }

    #[test]
    fn test_attribute_data_type_erasure() {
        let attr = Attribute::new("density", vec![1.0f32, 2.0, 3.0]);
        let data = AttributeData::Float(attr);
        
        assert_eq!(data.name(), "density");
        assert_eq!(data.len(), 3);
        assert!(data.as_float().is_some());
        assert!(data.as_vec3().is_none());
    }
}