//! Math operations on attributes

use crate::core::Attribute;
use crate::ice::ops::{ExecutionContext, IceNode};
use bevy::prelude::*;

// ============================================================================
// ADD VEC3
// ============================================================================

/// Add two Vec3 attributes element-wise
/// 
/// This is the ICE equivalent of your current AddVec3 subnet node,
/// but operates on entire arrays at once instead of per-point.
#[derive(Clone, Debug)]
pub struct AddVec3 {
    pub input_a: String,
    pub input_b: String,
    pub output: String,
}

impl AddVec3 {
    pub fn new(input_a: impl Into<String>, input_b: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            input_a: input_a.into(),
            input_b: input_b.into(),
            output: output.into(),
        }
    }
}

impl IceNode for AddVec3 {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        // Get input attributes
        let a = ctx.get_vec3(&self.input_a)?;
        let b = ctx.get_vec3(&self.input_b)?;

        // Check lengths match
        if a.len() != b.len() {
            return Err(format!(
                "AddVec3: input length mismatch ({} vs {})",
                a.len(), b.len()
            ));
        }

        // ✅ KEY DIFFERENCE: Process entire array at once!
        // Old way: called N times, once per point
        // New way: called once, processes all points
        let result_data: Vec<Vec3> = a.data.iter()
            .zip(&b.data)
            .map(|(&va, &vb)| va + vb)
            .collect();

        // Write result
        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "AddVec3"
    }
}

// ============================================================================
// SUBTRACT VEC3
// ============================================================================

#[derive(Clone, Debug)]
pub struct SubtractVec3 {
    pub input_a: String,
    pub input_b: String,
    pub output: String,
}

impl SubtractVec3 {
    pub fn new(input_a: impl Into<String>, input_b: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            input_a: input_a.into(),
            input_b: input_b.into(),
            output: output.into(),
        }
    }
}

impl IceNode for SubtractVec3 {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let a = ctx.get_vec3(&self.input_a)?;
        let b = ctx.get_vec3(&self.input_b)?;

        if a.len() != b.len() {
            return Err(format!("SubtractVec3: input length mismatch"));
        }

        let result_data: Vec<Vec3> = a.data.iter()
            .zip(&b.data)
            .map(|(&va, &vb)| va - vb)
            .collect();

        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "SubtractVec3"
    }
}

// ============================================================================
// MULTIPLY VEC3 (by scalar)
// ============================================================================

#[derive(Clone, Debug)]
pub struct MultiplyVec3 {
    pub input: String,
    pub scalar: f32,
    pub output: String,
}

impl MultiplyVec3 {
    pub fn new(input: impl Into<String>, scalar: f32, output: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            scalar,
            output: output.into(),
        }
    }
}

impl IceNode for MultiplyVec3 {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let input = ctx.get_vec3(&self.input)?;

        let result_data: Vec<Vec3> = input.data.iter()
            .map(|&v| v * self.scalar)
            .collect();

        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "MultiplyVec3"
    }
}

// ============================================================================
// NORMALIZE VEC3
// ============================================================================

#[derive(Clone, Debug)]
pub struct NormalizeVec3 {
    pub input: String,
    pub output: String,
}

impl NormalizeVec3 {
    pub fn new(input: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
        }
    }
}

impl IceNode for NormalizeVec3 {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let input = ctx.get_vec3(&self.input)?;

        let result_data: Vec<Vec3> = input.data.iter()
            .map(|&v| {
                if v.length_squared() > 1e-10 {
                    v.normalize()
                } else {
                    Vec3::ZERO
                }
            })
            .collect();

        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "NormalizeVec3"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_vec3() {
        let mut ctx = ExecutionContext::new();
        
        // Set up input attributes
        ctx.set_vec3(Attribute::new("A", vec![
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
        ]));
        ctx.set_vec3(Attribute::new("B", vec![
            Vec3::new(10.0, 20.0, 30.0),
            Vec3::new(40.0, 50.0, 60.0),
        ]));

        // Execute AddVec3 node
        let add = AddVec3::new("A", "B", "result");
        add.execute(&mut ctx).unwrap();

        // Check result
        let result = ctx.get_vec3("result").unwrap();
        assert_eq!(result.data[0], Vec3::new(11.0, 22.0, 33.0));
        assert_eq!(result.data[1], Vec3::new(44.0, 55.0, 66.0));
    }

    #[test]
    fn test_subtract_vec3() {
        let mut ctx = ExecutionContext::new();
        
        ctx.set_vec3(Attribute::new("A", vec![
            Vec3::new(10.0, 20.0, 30.0),
        ]));
        ctx.set_vec3(Attribute::new("B", vec![
            Vec3::new(1.0, 2.0, 3.0),
        ]));

        let sub = SubtractVec3::new("A", "B", "result");
        sub.execute(&mut ctx).unwrap();

        let result = ctx.get_vec3("result").unwrap();
        assert_eq!(result.data[0], Vec3::new(9.0, 18.0, 27.0));
    }

    #[test]
    fn test_multiply_vec3() {
        let mut ctx = ExecutionContext::new();
        
        ctx.set_vec3(Attribute::new("V", vec![
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
        ]));

        let mul = MultiplyVec3::new("V", 2.0, "result");
        mul.execute(&mut ctx).unwrap();

        let result = ctx.get_vec3("result").unwrap();
        assert_eq!(result.data[0], Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(result.data[1], Vec3::new(8.0, 10.0, 12.0));
    }

    #[test]
    fn test_normalize_vec3() {
        let mut ctx = ExecutionContext::new();
        
        ctx.set_vec3(Attribute::new("V", vec![
            Vec3::new(3.0, 0.0, 0.0),  // Length 3
            Vec3::new(0.0, 4.0, 0.0),  // Length 4
        ]));

        let norm = NormalizeVec3::new("V", "result");
        norm.execute(&mut ctx).unwrap();

        let result = ctx.get_vec3("result").unwrap();
        assert!((result.data[0] - Vec3::new(1.0, 0.0, 0.0)).length() < 0.001);
        assert!((result.data[1] - Vec3::new(0.0, 1.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn test_length_mismatch_error() {
        let mut ctx = ExecutionContext::new();
        
        ctx.set_vec3(Attribute::new("A", vec![Vec3::ZERO, Vec3::ZERO]));
        ctx.set_vec3(Attribute::new("B", vec![Vec3::ZERO])); // Different length!

        let add = AddVec3::new("A", "B", "result");
        let result = add.execute(&mut ctx);
        
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("length mismatch"));
    }
}

// ============================================================================
// CROSS PRODUCT
// ============================================================================

#[derive(Clone, Debug)]
pub struct CrossProduct {
    pub input_a: String,
    pub input_b: String,
    pub output: String,
}

impl CrossProduct {
    pub fn new(input_a: impl Into<String>, input_b: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            input_a: input_a.into(),
            input_b: input_b.into(),
            output: output.into(),
        }
    }
}

impl IceNode for CrossProduct {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let a = ctx.get_vec3(&self.input_a)?;
        let b = ctx.get_vec3(&self.input_b)?;

        if a.len() != b.len() {
            return Err(format!("CrossProduct: input length mismatch"));
        }

        let result_data: Vec<Vec3> = a.data.iter()
            .zip(&b.data)
            .map(|(&va, &vb)| va.cross(vb))
            .collect();

        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "CrossProduct"
    }
}

// ============================================================================
// LERP VEC3
// ============================================================================

#[derive(Clone, Debug)]
pub struct LerpVec3 {
    pub input_a: String,
    pub input_b: String,
    pub t: f32,
    pub output: String,
}

impl LerpVec3 {
    pub fn new(input_a: impl Into<String>, input_b: impl Into<String>, t: f32, output: impl Into<String>) -> Self {
        Self {
            input_a: input_a.into(),
            input_b: input_b.into(),
            t,
            output: output.into(),
        }
    }
}

impl IceNode for LerpVec3 {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let a = ctx.get_vec3(&self.input_a)?;
        let b = ctx.get_vec3(&self.input_b)?;

        if a.len() != b.len() {
            return Err(format!("LerpVec3: input length mismatch"));
        }

        let result_data: Vec<Vec3> = a.data.iter()
            .zip(&b.data)
            .map(|(&va, &vb)| va.lerp(vb, self.t))
            .collect();

        ctx.set_vec3(Attribute::new(&self.output, result_data));
        Ok(())
    }

    fn name(&self) -> &str {
        "LerpVec3"
    }
}