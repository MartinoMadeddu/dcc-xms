//! Minimal math for scene data: double-precision 4×4 matrices (USD's `Matrix4d`,
//! row-vector convention: `p' = p · M`, translation in the last row).

pub type Vec2f = [f32; 2];
pub type Vec3f = [f32; 3];
pub type Vec4f = [f32; 4];
/// Quaternion as (i, j, k, real), USD's `quath` / `quatf` component order.
pub type Quatf = [f32; 4];

/// Row-major 4×4 double matrix, USD convention (row vectors, translation in row 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4d(pub [[f64; 4]; 4]);

impl Default for Mat4d {
    fn default() -> Self {
        Mat4d::IDENTITY
    }
}

impl Mat4d {
    pub const IDENTITY: Mat4d = Mat4d([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    pub fn translation(t: [f64; 3]) -> Mat4d {
        let mut m = Mat4d::IDENTITY;
        m.0[3][0] = t[0];
        m.0[3][1] = t[1];
        m.0[3][2] = t[2];
        m
    }

    /// `self · other`: in USD's row-vector convention, apply `self` first, then
    /// `other` (local · parent = world).
    pub fn mul(&self, other: &Mat4d) -> Mat4d {
        let (a, b) = (&self.0, &other.0);
        Mat4d(std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum())))
    }

    /// Transform a point (row vector, w = 1).
    pub fn transform_point(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        std::array::from_fn(|j| p[0] * m[0][j] + p[1] * m[1][j] + p[2] * m[2][j] + m[3][j])
    }

    pub fn is_identity(&self) -> bool {
        *self == Mat4d::IDENTITY
    }
}

impl crate::time::Lerp for Mat4d {
    /// Component-wise, like USD's interpolation of resolved matrices. Fine across
    /// a shutter interval; large rotations between samples shear slightly.
    fn lerp(&self, b: &Mat4d, t: f64) -> Mat4d {
        Mat4d(std::array::from_fn(|i| std::array::from_fn(|j| self.0[i][j] + (b.0[i][j] - self.0[i][j]) * t)))
    }
}

/// Normalized linear interpolation of quaternions (shortest arc), for instancer
/// orientations.
pub fn nlerp(a: &Quatf, b: &Quatf, t: f64) -> Quatf {
    let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let t = t as f32;
    let q: Quatf = std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * s * t);
    let len = q.iter().map(|x| x * x).sum::<f32>().sqrt();
    if len > 0.0 { q.map(|x| x / len) } else { *a }
}
