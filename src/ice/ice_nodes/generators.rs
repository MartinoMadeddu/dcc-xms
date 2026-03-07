//! Geometry generation nodes

use crate::core::{Attribute, Topology};
use crate::ice::ops::{ExecutionContext, IceNode};
use bevy::prelude::*;

// ============================================================================
// SCATTER POINTS
// ============================================================================

/// Scatter points on input surface geometry
#[derive(Clone, Debug)]
pub struct ScatterPoints {
    pub count: u32,
    pub seed: u32,
}

impl ScatterPoints {
    pub fn new(count: u32, seed: u32) -> Self {
        Self { count, seed }
    }
}

impl IceNode for ScatterPoints {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        // Get input geometry points and topology
        let points = &ctx.geometry.points;
        
        let triangles: Vec<(Vec3, Vec3, Vec3)> = match &ctx.geometry.topology {
            Topology::PolyMesh { face_counts, face_indices } => {
                let mut tris = Vec::new();
                let mut idx = 0;
                for &count in face_counts {
                    if count == 3 && idx + 2 < face_indices.len() {
                        let a = face_indices[idx];
                        let b = face_indices[idx + 1];
                        let c = face_indices[idx + 2];
                        if a < points.len() && b < points.len() && c < points.len() {
                            tris.push((points[a], points[b], points[c]));
                        }
                    }
                    idx += count;
                }
                tris
            }
            _ => return Err("ScatterPoints requires PolyMesh topology".into()),
        };

        if triangles.is_empty() {
            return Err("No valid triangles found for scattering".into());
        }

        // Scatter points on triangles
        let mut rng = LcgRng::new(self.seed);
        let mut scattered = Vec::with_capacity(self.count as usize);

        for _ in 0..self.count {
            let tri_idx = (rng.next_u32() as usize) % triangles.len();
            let (a, b, c) = triangles[tri_idx];
            
            let mut r1 = rng.next_f32();
            let mut r2 = rng.next_f32();
            if r1 + r2 > 1.0 {
                r1 = 1.0 - r1;
                r2 = 1.0 - r2;
            }
            let r3 = 1.0 - r1 - r2;
            
            scattered.push(a * r3 + b * r1 + c * r2);
        }

        // Update geometry: new points, point cloud topology
        ctx.geometry.points = scattered;
        ctx.geometry.topology = Topology::Points;
        ctx.geometry.clear_attributes(); // Scattered points have no attributes yet

        Ok(())
    }

    fn name(&self) -> &str {
        "ScatterPoints"
    }
}

// Simple LCG random number generator (same as your current one)
struct LcgRng(u64);

impl LcgRng {
    fn new(seed: u32) -> Self {
        Self(seed as u64 | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self.0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Geometry;

    #[test]
    fn test_scatter_points() {
        // Create a simple triangle
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let indices = vec![0, 1, 2];
        let geo = Geometry::from_triangles(points, indices);
        
        let mut ctx = ExecutionContext::from_geometry(geo);
        
        let scatter = ScatterPoints::new(100, 42);
        scatter.execute(&mut ctx).unwrap();
        
        // Should have 100 scattered points
        assert_eq!(ctx.geometry.point_count(), 100);
        
        // Should be point cloud topology
        assert!(ctx.geometry.topology.is_points());
    }
}