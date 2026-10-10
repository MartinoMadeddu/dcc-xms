//! Geometry generation nodes

use std::sync::Arc;

use crate::core::geo::{Geo, Topology};
use crate::ice::ops::{ExecutionContext, IceNode};
use bevy::prelude::*;

/// A geometry's polygons as triangles (each polygon as a fan), by their corners.
fn triangles(geo: &Geo) -> Vec<[Vec3; 3]> {
    let Topology::Mesh { counts, indices, .. } = &geo.topology else { return vec![] };
    let p = geo.points();
    let at = |c: usize| Vec3::from_array(p[indices[c] as usize]);
    let mut out = Vec::with_capacity(indices.len());
    let mut first = 0usize;
    for &n in counts.iter() {
        let n = n as usize;
        for k in 1..n.saturating_sub(1) {
            out.push([at(first), at(first + k), at(first + k + 1)]);
        }
        first += n;
    }
    out
}

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
        if !matches!(ctx.geometry.topology, Topology::Mesh { .. }) {
            return Err("ScatterPoints requires polygons".into());
        }
        let tris = triangles(&ctx.geometry);
        if tris.is_empty() {
            return Err("No valid triangles found for scattering".into());
        }

        // Scatter points on triangles
        let mut rng = LcgRng::new(self.seed);
        let mut scattered = Vec::with_capacity(self.count as usize);
        for _ in 0..self.count {
            let [a, b, c] = tris[(rng.next_u32() as usize) % tris.len()];
            let mut r1 = rng.next_f32();
            let mut r2 = rng.next_f32();
            if r1 + r2 > 1.0 {
                r1 = 1.0 - r1;
                r2 = 1.0 - r2;
            }
            let r3 = 1.0 - r1 - r2;
            scattered.push((a * r3 + b * r1 + c * r2).to_array());
        }

        // A point cloud of its own: the surface's attributes do not carry over.
        ctx.geometry = Geo::from_points(scattered);
        Ok(())
    }

    fn name(&self) -> &str {
        "ScatterPoints"
    }
}

// ============================================================================
// COPY TO POINTS
// ============================================================================

/// Copy template geometry to each point in the current geometry
///
/// Requires:
/// - Current geometry with points (P attribute)
/// - Template geometry in external context (passed via subnet's template input)
#[derive(Clone, Debug, Default)]
pub struct CopyToPoints;

impl CopyToPoints {
    pub fn new() -> Self {
        Self
    }
}

impl IceNode for CopyToPoints {
    fn execute(&self, ctx: &mut ExecutionContext) -> Result<(), String> {
        let targets = ctx.geometry.points().to_vec();
        if targets.is_empty() {
            return Err("CopyToPoints: No points to copy to".into());
        }
        let template = ctx.get_external_geometry("template").ok_or_else(|| {
            "CopyToPoints: No template geometry available. \
             Connect a geometry to the ICE subnet node's template input.".to_string()
        })?;
        let tp = template.points();
        if tp.is_empty() {
            return Err("CopyToPoints: Template has no points".into());
        }

        // One copy of the template per point, moved to it.
        let mut points = Vec::with_capacity(tp.len() * targets.len());
        for t in &targets {
            points.extend(tp.iter().map(|p| [p[0] + t[0], p[1] + t[1], p[2] + t[2]]));
        }
        let topology = match &template.topology {
            Topology::Mesh { counts, indices, left_handed, subdiv } => {
                let n = tp.len() as u32;
                let (mut all_counts, mut all_indices) = (Vec::with_capacity(counts.len() * targets.len()), Vec::with_capacity(indices.len() * targets.len()));
                for copy in 0..targets.len() as u32 {
                    all_counts.extend_from_slice(counts);
                    all_indices.extend(indices.iter().map(|&i| i + copy * n));
                }
                Topology::Mesh { counts: Arc::new(all_counts), indices: Arc::new(all_indices), left_handed: *left_handed, subdiv: *subdiv }
            }
            Topology::Curves { counts, basis, wrap } => {
                let all: Vec<u32> = (0..targets.len()).flat_map(|_| counts.iter().copied()).collect();
                Topology::curves(all, *basis, *wrap)
            }
            Topology::Points => Topology::Points,
        };
        let mut out = Geo::from_points(points);
        out.topology = topology;
        ctx.geometry = out;
        Ok(())
    }

    fn name(&self) -> &str {
        "CopyToPoints"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_to_points() {
        // Where to copy to, and a triangle to copy.
        let targets = Geo::from_points(vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]]);
        let template = Geo::from_polygons(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], vec![3], vec![0, 1, 2]);
        let mut ctx = ExecutionContext::from_geometry(targets);
        ctx.add_external_geometry("template", template);
        CopyToPoints::new().execute(&mut ctx).unwrap();
        // 3 copies × 3 points, 3 triangles, every index valid.
        assert_eq!(ctx.geometry.point_count(), 9);
        assert_eq!(ctx.geometry.topology.primitive_count(), 3);
        assert!(ctx.geometry.validate().is_ok());
        assert_eq!(ctx.geometry.points()[4], [11.0, 0.0, 0.0]);
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
mod scatter_tests {
    use super::*;

    #[test]
    fn test_scatter_points() {
        // A quad: both of its triangles are scattered on, not only true triangles.
        let geo = Geo::from_polygons(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], vec![4], vec![0, 1, 2, 3]);
        let mut ctx = ExecutionContext::from_geometry(geo);
        ScatterPoints::new(100, 42).execute(&mut ctx).unwrap();
        assert_eq!(ctx.geometry.point_count(), 100);
        assert!(matches!(ctx.geometry.topology, Topology::Points));
        assert!(ctx.geometry.points().iter().all(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1])));
    }
}
