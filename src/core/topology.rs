use std::fmt;

/// Topology information - defines connectivity and structure
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Topology {
    /// Point cloud (no connectivity)
    #[default]
    Points,
    
    /// Polygon mesh
    PolyMesh {
        /// Number of vertices per face
        face_counts: Vec<usize>,
        /// Vertex indices (flattened, indexed by face_counts)
        face_indices: Vec<usize>,
    },
    
    /// Curves/lines
    Curves {
        /// Number of points per curve
        curve_counts: Vec<usize>,
    },
    
    // Future: Volumes, NURBs, etc.
}

impl Topology {
    /// Create a triangle mesh topology
    pub fn triangles(indices: Vec<usize>) -> Self {
        let num_triangles = indices.len() / 3;
        Self::PolyMesh {
            face_counts: vec![3; num_triangles],
            face_indices: indices,
        }
    }

    /// Create a quad mesh topology
    pub fn quads(indices: Vec<usize>) -> Self {
        let num_quads = indices.len() / 4;
        Self::PolyMesh {
            face_counts: vec![4; num_quads],
            face_indices: indices,
        }
    }

    /// Create arbitrary polygon mesh
    pub fn polymesh(face_counts: Vec<usize>, face_indices: Vec<usize>) -> Self {
        Self::PolyMesh {
            face_counts,
            face_indices,
        }
    }

    /// Create a point cloud
    pub fn points() -> Self {
        Self::Points
    }

    /// Get number of primitives (faces, curves, etc.)
    pub fn primitive_count(&self) -> usize {
        match self {
            Topology::Points => 0,
            Topology::PolyMesh { face_counts, .. } => face_counts.len(),
            Topology::Curves { curve_counts } => curve_counts.len(),
        }
    }

    /// Get total number of face-vertex pairs (for PolyMesh)
    pub fn face_vertex_count(&self) -> usize {
        match self {
            Topology::PolyMesh { face_indices, .. } => face_indices.len(),
            _ => 0,
        }
    }

    /// Check if this is a point cloud
    pub fn is_points(&self) -> bool {
        matches!(self, Topology::Points)
    }

    /// Check if this is a polygon mesh
    pub fn is_polymesh(&self) -> bool {
        matches!(self, Topology::PolyMesh { .. })
    }
}

impl fmt::Display for Topology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Topology::Points => write!(f, "Points"),
            Topology::PolyMesh { face_counts, .. } => {
                write!(f, "PolyMesh({} faces)", face_counts.len())
            }
            Topology::Curves { curve_counts } => {
                write!(f, "Curves({} curves)", curve_counts.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triangle_topology() {
        let topo = Topology::triangles(vec![0, 1, 2, 2, 3, 0]);
        assert_eq!(topo.primitive_count(), 2);
        assert_eq!(topo.face_vertex_count(), 6);
    }

    #[test]
    fn test_point_cloud() {
        let topo = Topology::points();
        assert!(topo.is_points());
        assert_eq!(topo.primitive_count(), 0);
    }
}