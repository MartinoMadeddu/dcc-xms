//! Collision cleanup for motion capture.
//!
//! A captured performance is played onto a set of rigid bodies, one per
//! main bone, each with a convex hull cut from the skin. The bodies follow
//! the capture exactly until something is in the way: a mesh given as a
//! collider, or another part of the same character. Then the joints give,
//! each as much as that joint may, and the bodies return to the capture
//! when the way is clear.
//!
//! Nothing here is simulated in the usual sense. There is no gravity, no
//! momentum and no stored energy, so there is nothing that can blow up:
//! each frame starts from the capture and is only ever pulled back toward
//! it. Bones cannot stretch, because the result is rotations.
//!
//! This is its own crate so that it is compiled fully optimised even in a
//! development build of the program, and so that it knows nothing of the
//! program: bodies, poses and triangles go in, poses come out.

pub mod bvh;
pub mod hull;
pub mod roles;
pub mod solver;
pub mod bake;

pub use bvh::Bvh;
pub use hull::Hull;
pub use roles::{Role, Side};
pub use solver::{BodyDef, Hinge, Params, Pose, Solver, FrameStats};
pub use bake::{bake, BakeInput, BakeReport, FrameOut};
