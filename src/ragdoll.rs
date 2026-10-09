//! Collision cleanup of a captured clip.
//!
//! This is the part that knows the program: it turns a clip and its skin
//! into the bodies the solver in `crates/ragdoll` works with, runs the
//! solver on a thread of its own, keeps what it solved in a file, and puts
//! the result back on the skeleton.
//!
//! Three steps, each with a node to look at it:
//!
//! - **Calamari**: the skin is cut into one rigid piece per main bone, by
//!   the bone each vertex follows most. Each piece gets a convex hull.
//! - **Ragdoll**: the hulls follow the capture and are kept out of a
//!   collider mesh and out of each other.
//! - The result is a clip like any other: rotations on the same skeleton.
//!
//! A long take is solved a chunk of frames at a time, and each chunk is
//! written to a file as soon as it is done. What comes back from the file
//! is small: a rotation per body per frame.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use bevy::math::{Mat3, Mat4, Quat, Vec3};
use bevy::prelude::Transform;
use xms_ragdoll::{BakeInput, BakeReport, BodyDef, Bvh, Hinge, Hull, Params, Pose, Role, Side, Solver};

use crate::core::human::{Human, Slot};
use crate::core::human_ik::Limits;

use crate::core::anim::{AnimData, SkinMesh, Track};
use crate::types::MeshData;

/// Bumped when solved files of an earlier build must not be reused.
const VERSION: u32 = 4;

/// What the Ragdoll node sets.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Distance from a surface at which bodies start to be pushed away, centimetres.
    pub margin:     f32,
    pub friction:   f32,
    pub self_collision: bool,
    /// Depth by which parts of the character may sink into each other, centimetres.
    #[serde(default = "default_self_slack")]
    pub self_slack: f32,
    /// How strongly the bodies hold to the capture: 1 is the tuned value.
    pub stiffness:  f32,
    /// Speed at which a body is pushed out of something, centimetres per second.
    pub release:    f32,
    /// Furthest the trunk is moved out of the collider, centimetres. Deeper
    /// than this in its capture, it is moved this far and left in by the rest.
    pub ghost_depth: f32,
    /// Depth the trunk may rest in a surface, centimetres: a seat gives under a sitter.
    #[serde(default)]
    pub sink:       f32,
    pub fade_out:   u32,
    pub fade_in:    u32,
    /// Distance from the capture at which a limb lets go, centimetres.
    pub limb_limit: f32,
    /// Frames either side over which the solver's changes are evened out.
    #[serde(default = "default_smooth")]
    pub smooth:     u32,
    /// Fineness of the hulls: 0, 1 or 2.
    pub detail:     u32,
}

fn default_self_slack() -> f32 { 3.0 }
fn default_smooth() -> u32 { 2 }

impl Default for Settings {
    fn default() -> Self {
        Settings { margin: 1.2, friction: 0.5, self_collision: true, self_slack: 3.0, stiffness: 1.0, release: 60.0, ghost_depth: 12.0, sink: 0.0, fade_out: 6, fade_in: 10, limb_limit: 45.0, smooth: 2, detail: 1 }
    }
}

impl Settings {
    fn params(&self, fps: f32) -> Params {
        Params {
            fps,
            margin: (self.margin * 0.01).max(0.0),
            friction: self.friction.max(0.0),
            self_collision: self.self_collision,
            self_slack: (self.self_slack * 0.01).max(0.0),
            release_speed: (self.release * 0.01).max(0.01),
            ghost_depth: (self.ghost_depth * 0.01).max(0.005),
            fade_out: self.fade_out.max(1) as usize,
            fade_in: self.fade_in.max(1) as usize,
            limb_limit: (self.limb_limit * 0.01).max(0.02),
            smooth: self.smooth.min(8) as usize,
            threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16),
            ..Params::default()
        }
    }
}

// ============================================================================
// RIG: FROM SKELETON AND SKIN TO BODIES
// ============================================================================

pub struct Rig {
    /// Joint of each body.
    pub joints: Vec<usize>,
    pub roles:  Vec<(Role, Side)>,
    pub defs:   Vec<BodyDef>,
    /// The bodies where the skin was bound.
    pub bind:   Vec<Pose>,
    /// For each joint, the body its skin belongs to.
    pub body_of_joint: Vec<Option<usize>>,
    /// Bodies that had too little skin and got a stand-in shape.
    pub stand_ins: Vec<String>,
}

fn pose_of(m: &Mat4) -> Pose {
    let (_, q, p) = m.to_scale_rotation_translation();
    Pose { p, q: q.normalize() }
}

/// Where the bodies are at one sample of the clip.
pub fn body_poses(clip: &AnimData, joints: &[usize], index: usize) -> Vec<Pose> {
    let world = clip.world_pose(index);
    joints.iter().map(|j| pose_of(&world[*j])).collect()
}

/// The joints that get a body, from what the skeleton is: hips, spine,
/// neck, head, and three per limb. Toes are part of the foot, and collar
/// bones part of the chest: too small to push anything around, and thrown
/// about if they try. Twist joints are part of their limb.
fn bodies_of(h: &Human) -> Vec<(usize, (Role, Side))> {
    let mut out: Vec<(usize, (Role, Side))> = vec![];
    let mut add = |j: Option<usize>, r: Role, s: Side| { if let Some(j) = j { if !out.iter().any(|o| o.0 == j) { out.push((j, (r, s))); } } };
    add(h.get(Slot::Hips), Role::Pelvis, Side::Centre);
    for j in &h.spine { add(Some(*j), Role::Spine, Side::Centre); }
    for j in &h.neck { add(Some(*j), Role::Neck, Side::Centre); }
    add(h.get(Slot::Head), Role::Head, Side::Centre);
    for l in &h.limbs {
        let s = if l.left { Side::Left } else { Side::Right };
        let (a, b, c) = if l.arm { (Role::UpperArm, Role::LowerArm, Role::Hand) } else { (Role::Thigh, Role::Calf, Role::Foot) };
        add(l.upper, a, s); add(l.mid, b, s); add(l.end, c, s);
    }
    // Parents before children.
    out.sort_by_key(|o| o.0);
    out
}

/// The bodies of a clip: one per main joint of a human, shaped by the
/// skin that follows it.
pub fn build_rig(clip: &AnimData, settings: &Settings) -> Result<Rig, String> {
    let picked = bodies_of(&Human::of(clip));
    // A sliver of a body between two heavy ones is thrown about by them. A
    // part that comes out far lighter than its neighbours is not a body:
    // its skin goes to the part above and its joint keeps its capture.
    let first = rig_of(clip, settings, &picked)?;
    let heaviest = first.defs.iter().map(|d| d.mass).fold(0.0, f32::max);
    let kept: Vec<(usize, (Role, Side))> = picked.iter().enumerate()
        .filter(|(b, (_, r))| matches!(r.0, Role::Pelvis | Role::Head | Role::Hand | Role::Foot) || first.defs[*b].mass > heaviest * 0.04)
        .map(|(_, p)| *p).collect();
    if kept.len() == picked.len() { return Ok(first); }
    rig_of(clip, settings, &kept)
}

fn rig_of(clip: &AnimData, settings: &Settings, picked: &[(usize, (Role, Side))]) -> Result<Rig, String> {
    let n = clip.joints.len();
    let joints: Vec<usize> = picked.iter().map(|p| p.0).collect();
    let roles: Vec<(Role, Side)> = picked.iter().map(|p| p.1).collect();
    if !roles.iter().any(|r| r.0 == Role::Pelvis) || joints.len() < 4 {
        return Err("No human skeleton found: the hips and the limbs are not known from the joint names. Put a Characterize node before this one and set which joint is which.".into());
    }
    // Nearest body at or above each joint.
    let mut body_of_joint: Vec<Option<usize>> = vec![None; n];
    for (b, j) in joints.iter().enumerate() { body_of_joint[*j] = Some(b); }
    for j in 0..n {
        if body_of_joint[j].is_none() {
            if let Some(p) = clip.joints[j].parent { if p < j { body_of_joint[j] = body_of_joint[p]; } }
        }
    }
    let parent_body = |b: usize| -> Option<usize> {
        let mut cur = clip.joints[joints[b]].parent;
        while let Some(p) = cur {
            if let Some(pb) = body_of_joint[p].filter(|pb| joints[*pb] == p) { return Some(pb); }
            cur = clip.joints[p].parent;
        }
        None
    };

    // Bind pose: where the skin was bound, or the first frame without a skin.
    let first = clip.world_pose(0);
    let bind_world: Vec<Mat4> = match &clip.skin {
        Some(s) if s.bind.len() == n => s.bind.clone(),
        _ => first.clone(),
    };
    let bind: Vec<Pose> = joints.iter().map(|j| pose_of(&bind_world[*j])).collect();

    // Skin of each body, in the frame of its bone.
    let mut points: Vec<Vec<Vec3>> = vec![vec![]; joints.len()];
    if let Some(skin) = &clip.skin {
        for (v, p) in skin.positions.iter().enumerate() {
            let Some(b) = body_of_joint.get(skin.joint[v] as usize).copied().flatten() else { continue };
            points[b].push(bind[b].q.inverse() * (*p - bind[b].p));
        }
    }
    // Size of the character, for stand-in shapes.
    let height = bind.iter().map(|p| p.p.y).fold(f32::MIN, f32::max) - bind.iter().map(|p| p.p.y).fold(f32::MAX, f32::min);
    let scale = (height / 1.55).clamp(0.05, 20.0);
    let child_of = |b: usize| -> Option<usize> { (0..joints.len()).find(|c| parent_body(*c) == Some(b)) };

    let spacing = [0.035f32, 0.025, 0.018][settings.detail.min(2) as usize] * scale;
    let mut stand_ins = vec![];
    let mut defs = vec![];
    for b in 0..joints.len() {
        let role = roles[b].0;
        let t = role.tuning();
        // Direction of the bone in its own frame: to the body below, else away from the one above.
        let toward = child_of(b).map(|c| bind[b].q.inverse() * (bind[c].p - bind[b].p))
            .or_else(|| parent_body(b).map(|p| bind[b].q.inverse() * (bind[b].p - bind[p].p)))
            .unwrap_or(Vec3::X);
        let length = toward.length().max(0.02 * scale);
        let axis = toward.normalize_or(Vec3::X);
        let pts = &points[b];
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in pts { lo = lo.min(*p); hi = hi.max(*p); }
        let extent = if pts.is_empty() { Vec3::ZERO } else { hi - lo };
        let too_little = pts.len() < 8 || (role == Role::Head && extent.max_element() < 0.14 * scale);
        let hull = if too_little {
            stand_ins.push(clip.joints[joints[b]].name.clone());
            if role == Role::Head {
                Hull::capsule(axis * 0.05 * scale, axis * 0.13 * scale, 0.095 * scale, 2)
            } else {
                let r = (t.fallback * length).clamp(0.01 * scale, 0.2 * scale);
                Hull::capsule(axis * r.min(length * 0.5), axis * (length - r).max(length * 0.5), r, 2)
            }
        } else {
            let area = 2.0 * (extent.x * extent.y + extent.y * extent.z + extent.z * extent.x);
            let wanted = area / (spacing * spacing);
            Hull::from_points(pts, if wanted < 70.0 { 1 } else if wanted < 320.0 { 2 } else { 3 })
        };
        defs.push(BodyDef {
            name: clip.joints[joints[b]].name.clone(),
            parent: parent_body(b),
            mass: (hull.volume * 1000.0).max(0.05),
            hull,
            follow: (1.0 - (1.0 - t.follow).powf(settings.stiffness.clamp(0.1, 4.0))).clamp(0.02, 0.98),
            follow_turn: (1.0 - (1.0 - t.follow_turn).powf(settings.stiffness.clamp(0.1, 4.0))).clamp(0.02, 0.98),
            trunk: role.trunk(),
            turn_resist: role.turn_resist(),
            swing: t.swing.to_radians(),
            twist: t.twist.to_radians(),
            twist_axis: axis,
            hinge: None,
            range: None,
            core: t.core,
            sink: if t.core { (settings.sink * 0.01).max(0.0) } else { 0.0 },
        });
    }
    // Elbows and knees: the axis they bend about and how far, from the clip itself.
    for b in 0..joints.len() {
        if !roles[b].0.tuning().hinge { continue; }
        let Some(p) = defs[b].parent else { continue };
        defs[b].hinge = find_hinge(clip, joints[p], joints[b], child_of(b).map(|c| joints[c]));
        if defs[b].hinge.is_none() { defs[b].swing = 25f32.to_radians(); }
    }
    // Wrists and ankles: within what a human wrist or ankle does.
    for (end, mid, range) in crate::core::human_ik::end_ranges(clip, &Human::of(clip)) {
        let Some(b) = joints.iter().position(|j| *j == end) else { continue };
        if defs[b].parent.map(|p| joints[p]) == Some(mid) { defs[b].range = Some(range); }
    }
    Ok(Rig { joints, roles, defs, bind, body_of_joint, stand_ins })
}

/// The axis a joint bends about, measured from the clip: the direction its
/// rotation varies most along. The bend is counted from the straightest
/// pose the clip holds, and may not go further that way.
fn find_hinge(clip: &AnimData, parent: usize, joint: usize, child: Option<usize>) -> Option<Hinge> {
    let frames = clip.frames.max(1);
    let count = frames.min(240);
    let mut rels = Vec::with_capacity(count);
    let mut straightest = (f32::MAX, Quat::IDENTITY);
    for k in 0..count {
        let world = clip.world_pose(k * (frames - 1).max(1) / (count - 1).max(1));
        let (qp, qj) = (pose_of(&world[parent]).q, pose_of(&world[joint]).q);
        let rel = (qp.inverse() * qj).normalize();
        if let Some(c) = child {
            let upper = (world[joint].w_axis.truncate() - world[parent].w_axis.truncate()).normalize_or_zero();
            let lower = (world[c].w_axis.truncate() - world[joint].w_axis.truncate()).normalize_or_zero();
            let bend = upper.angle_between(lower);
            if bend < straightest.0 { straightest = (bend, rel); }
        }
        rels.push(rel);
    }
    let reference = if child.is_some() { straightest.1 } else { rels[0] };
    // With a bone below, the axis is square to the plane the two bones
    // make where the joint is well bent: the hinge itself, whatever else
    // the joint does. In the joint's frame.
    let mut plane = Vec3::ZERO;
    if let Some(c) = child {
        for k in 0..count {
            let world = clip.world_pose(k * (frames - 1).max(1) / (count - 1).max(1));
            let upper = (world[joint].w_axis.truncate() - world[parent].w_axis.truncate()).normalize_or_zero();
            let lower = (world[c].w_axis.truncate() - world[joint].w_axis.truncate()).normalize_or_zero();
            let n = upper.cross(lower);
            if n.length() > 20f32.to_radians().sin() { plane += pose_of(&world[joint]).q.inverse() * n; }
        }
    }
    // Rotation vectors away from the reference, in the joint's frame.
    let vectors: Vec<Vec3> = rels.iter().map(|r| {
        let d = (reference.inverse() * *r).normalize();
        let d = if d.w < 0.0 { -d } else { d };
        let v = Vec3::new(d.x, d.y, d.z);
        let s = v.length();
        if s < 1e-6 { Vec3::ZERO } else { v / s * 2.0 * s.atan2(d.w) }
    }).collect();
    let mut cov = Mat3::ZERO;
    for v in &vectors { cov += Mat3::from_cols(*v * v.x, *v * v.y, *v * v.z); }
    // Largest direction of the spread, by repeated multiplication.
    let mut axis = Vec3::new(0.577, 0.577, 0.577);
    for _ in 0..40 { axis = (cov * axis).normalize_or_zero(); if axis == Vec3::ZERO { return None; } }
    if plane.length() > 1e-3 { axis = plane.normalize(); }
    let bends: Vec<f32> = rels.iter().map(|r| xms_ragdoll::solver::twist_angle((reference.inverse() * *r).normalize(), axis)).collect();
    let (lo, hi) = bends.iter().fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
    // A joint that hardly bends in the clip tells nothing about its axis.
    if hi - lo < 15f32.to_radians() { return None; }
    // Bending counts up from the straight pose.
    let (axis, lo, hi) = if hi.abs() >= lo.abs() { (axis, lo, hi) } else { (-axis, -hi, -lo) };
    let slack = 3f32.to_radians();
    Some(Hinge { axis, reference, min: lo - slack, max: (hi + 15f32.to_radians()).max(150f32.to_radians()).min(165f32.to_radians()) })
}

// ============================================================================
// CALAMARI: THE SKIN IN RIGID PIECES
// ============================================================================

/// The clip with its skin cut into rigid pieces, one per body, or with the
/// convex hull of each piece in place of the skin.
pub fn calamari(clip: &AnimData, hulls: bool, detail: u32) -> AnimData {
    let settings = Settings { detail, ..Default::default() };
    let Ok(rig) = build_rig(clip, &settings) else { return clip.clone() };
    let bind_world: Vec<Mat4> = match &clip.skin {
        Some(s) if s.bind.len() == clip.joints.len() => s.bind.clone(),
        _ => clip.world_pose(0),
    };
    let mut out = SkinMesh { bind: bind_world, ..Default::default() };
    if hulls || clip.skin.is_none() {
        for (b, def) in rig.defs.iter().enumerate() {
            let base = out.positions.len() as u32;
            let pose = &rig.bind[b];
            let middle = pose.p + pose.q * def.hull.com;
            for s in &def.hull.samples {
                let p = pose.p + pose.q * *s;
                out.positions.push(p);
                out.normals.push((p - middle).normalize_or(Vec3::Y));
                out.joint.push(rig.joints[b] as u32);
            }
            for t in def.hull.tris.iter() { out.faces.push([base + t[0], base + t[1], base + t[2], base + t[2]]); }
        }
    } else if let Some(skin) = &clip.skin {
        // Each face goes to the body most of its corners follow. A vertex
        // shared by faces of two bodies is doubled: the pieces come apart.
        let body = |v: u32| rig.body_of_joint.get(skin.joint[v as usize] as usize).copied().flatten();
        let mut copy: HashMap<(u32, usize), u32> = HashMap::new();
        for f in &skin.faces {
            let corners = if SkinMesh::is_tri(f) { 3 } else { 4 };
            let mut votes: Vec<(usize, u32)> = vec![];
            for k in 0..corners {
                if let Some(b) = body(f[k]) {
                    match votes.iter_mut().find(|v| v.0 == b) { Some(v) => v.1 += 1, None => votes.push((b, 1)) }
                }
            }
            let Some((b, _)) = votes.into_iter().max_by_key(|v| v.1) else { continue };
            let mut face = [0u32; 4];
            for k in 0..4 {
                face[k] = *copy.entry((f[k], b)).or_insert_with(|| {
                    out.positions.push(skin.positions[f[k] as usize]);
                    out.normals.push(skin.normals[f[k] as usize]);
                    out.joint.push(rig.joints[b] as u32);
                    out.positions.len() as u32 - 1
                });
            }
            out.faces.push(face);
        }
    }
    AnimData { skin: Some(Arc::new(out)), ..clip.clone() }
}

// ============================================================================
// SOLVED CLIPS
// ============================================================================

/// What the solver made of a clip: where the first body is and how every
/// body is turned, per frame.
pub struct Solved {
    pub frames: usize,
    /// Joint of each body.
    pub joints: Vec<usize>,
    /// Per frame: position of the first body, then a rotation per body.
    data:       Vec<f32>,
    /// Per frame: collision weight, depth left in the collider, distance from the capture, contacts.
    stats:      Vec<[f32; 4]>,
    pub report: BakeReport,
    pub seconds: f32,
}

impl Solved {
    fn stride(&self) -> usize { 3 + 4 * self.joints.len() }
    fn root(&self, f: usize) -> Vec3 { let d = &self.data[f * self.stride()..]; Vec3::new(d[0], d[1], d[2]) }
    fn rot(&self, f: usize, b: usize) -> Quat { let d = &self.data[f * self.stride() + 3 + 4 * b..]; Quat::from_xyzw(d[0], d[1], d[2], d[3]) }
    /// Collision weight of a frame: 0 where the clip was left as captured.
    pub fn weight(&self, f: usize) -> f32 { self.stats.get(f).map(|s| s[0]).unwrap_or(1.0) }
    pub fn deviation(&self, f: usize) -> f32 { self.stats.get(f).map(|s| s[2]).unwrap_or(0.0) }

    fn report_from_stats(&mut self) {
        let mut r = BakeReport { frames: self.frames, ..Default::default() };
        let mut start = None;
        for (f, s) in self.stats.iter().enumerate() {
            r.max_residual = r.max_residual.max(s[1]);
            r.max_deviation = r.max_deviation.max(s[2]);
            if s[3] > 0.0 { r.contact_frames += 1; }
            if s[0] < 1.0 { r.ghost_frames += 1; if start.is_none() { start = Some(f); } }
            else if let Some(a) = start.take() { r.ghost_ranges.push((a, f - 1)); }
        }
        if let Some(a) = start { r.ghost_ranges.push((a, self.frames.saturating_sub(1))); }
        self.report = r;
    }
}

/// Put a solved result on the clip: new rotations for the body joints, a
/// new position for the first. Every other joint keeps its capture, so
/// fingers and helper joints ride along and no bone changes length.
pub fn apply(clip: &AnimData, solved: &Solved) -> AnimData {
    let n = clip.joints.len();
    let frames = clip.frames.max(1).min(solved.frames.max(1));
    let mut body_of = vec![usize::MAX; n];
    for (b, j) in solved.joints.iter().enumerate() { if *j < n { body_of[*j] = b; } }
    let last = solved.joints.iter().copied().filter(|j| *j < n).max().unwrap_or(0);
    let root = solved.joints.first().copied().unwrap_or(0);
    let mut tracks: Vec<Track> = (*clip.tracks).clone();
    for j in solved.joints.iter().filter(|j| **j < n) {
        if tracks[*j].len() < clip.frames.max(1) { tracks[*j] = vec![clip.joints[*j].rest; clip.frames.max(1)]; }
    }
    let mut world = vec![Mat4::IDENTITY; last + 1];
    for f in 0..frames {
        for j in 0..=last {
            let mut local: Transform = clip.local(j, f);
            let parent = clip.joints[j].parent.filter(|p| *p < j).map(|p| world[p]);
            if body_of[j] != usize::MAX {
                let (_, parent_rot, _) = parent.unwrap_or(Mat4::IDENTITY).to_scale_rotation_translation();
                local.rotation = (parent_rot.inverse() * solved.rot(f, body_of[j])).normalize();
                if j == root {
                    local.translation = parent.unwrap_or(Mat4::IDENTITY).inverse().transform_point3(solved.root(f));
                }
                tracks[j][f] = local;
            }
            let m = local.compute_matrix();
            world[j] = match parent { Some(p) => p * m, None => m };
        }
    }
    AnimData { tracks: Arc::new(tracks), ..clip.clone() }
}

// ── Fingerprints ─────────────────────────────────────────────────────────────
// A solved result belongs to a clip, a collider and settings. Clips and
// meshes are told apart by a hash of a sample of their content, so the
// result is found again in another session.

fn hash_f32(h: &mut impl Hasher, v: f32) { v.to_bits().hash(h); }

fn clip_print(clip: &Arc<AnimData>) -> u64 {
    static SEEN: OnceLock<Mutex<Vec<(Weak<AnimData>, u64)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(vec![]));
    let mut seen = seen.lock().unwrap();
    seen.retain(|(w, _)| w.strong_count() > 0);
    if let Some((_, p)) = seen.iter().find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(clip))) { return *p; }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (clip.frames, clip.start_frame, clip.joints.len(), clip.rate.num, clip.rate.den).hash(&mut h);
    for j in &clip.joints { j.name.hash(&mut h); j.parent.hash(&mut h); }
    let total = clip.joints.len() * clip.frames.max(1);
    for k in 0..512usize {
        let i = k * total / 512;
        let t = clip.local(i % clip.joints.len().max(1), i / clip.joints.len().max(1));
        for v in t.translation.to_array().into_iter().chain(t.rotation.to_array()) { hash_f32(&mut h, v); }
    }
    if let Some(s) = &clip.skin {
        (s.positions.len(), s.faces.len(), s.weights.len()).hash(&mut h);
        for k in 0..256usize { if let Some(p) = s.positions.get(k * s.positions.len() / 256) { for v in p.to_array() { hash_f32(&mut h, v); } } }
    }
    let p = h.finish();
    seen.push((Arc::downgrade(clip), p));
    p
}

fn mesh_print(mesh: &Arc<MeshData>) -> u64 {
    static SEEN: OnceLock<Mutex<Vec<(Weak<MeshData>, u64)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(vec![]));
    let mut seen = seen.lock().unwrap();
    seen.retain(|(w, _)| w.strong_count() > 0);
    if let Some((_, p)) = seen.iter().find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(mesh))) { return *p; }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (mesh.vertices.len(), mesh.indices.len()).hash(&mut h);
    for k in 0..1024usize {
        if let Some(v) = mesh.vertices.get(k * mesh.vertices.len() / 1024) { for x in v { hash_f32(&mut h, *x); } }
        if let Some(i) = mesh.indices.get(k * mesh.indices.len() / 1024) { i.hash(&mut h); }
    }
    let p = h.finish();
    seen.push((Arc::downgrade(mesh), p));
    p
}

/// What identifies one solve.
pub fn key(clip: &Arc<AnimData>, collider: Option<&Arc<MeshData>>, settings: &Settings) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    VERSION.hash(&mut h);
    clip_print(clip).hash(&mut h);
    collider.map(mesh_print).hash(&mut h);
    serde_json::to_string(settings).unwrap_or_default().hash(&mut h);
    // Joints set by hand make another rig. Picks that say what the names say do not.
    if let Some(p) = &clip.human {
        if !Human::of(clip).same_skeleton(&Human::detect(&clip.joints, &vec![])) { serde_json::to_string(&**p).unwrap_or_default().hash(&mut h); }
    }
    h.finish()
}

/// The triangle tree of a collider. The last two are kept, by content, so
/// the same model arriving again does not build its tree again.
fn tree(mesh: &Arc<MeshData>) -> Arc<Bvh> {
    static TREES: OnceLock<Mutex<Vec<(u64, Arc<Bvh>)>>> = OnceLock::new();
    let trees = TREES.get_or_init(|| Mutex::new(vec![]));
    let print = mesh_print(mesh);
    if let Some((_, t)) = trees.lock().unwrap().iter().find(|(p, _)| *p == print) { return t.clone(); }
    let built = Arc::new(Bvh::new(&mesh.vertices, &mesh.indices));
    let mut trees = trees.lock().unwrap();
    if trees.len() >= 2 { trees.remove(0); }
    trees.push((print, built.clone()));
    built
}

// ── Files ────────────────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    // A folder of one's own choosing comes first.
    if let Some(dir) = std::env::var_os("XMS_CACHE_DIR") { return Some(PathBuf::from(dir).join("ragdoll")); }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }?;
    Some(base.join("xms").join("ragdoll"))
}

fn file_of(key: u64) -> Option<PathBuf> { cache_dir().map(|d| d.join(format!("{key:016x}.rag"))) }

const MAGIC: &[u8; 8] = b"XMSRAG03";

fn write_f32s(w: &mut impl Write, v: &[f32]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v { bytes.extend_from_slice(&x.to_le_bytes()); }
    w.write_all(&bytes)
}

fn read_solved(path: &std::path::Path) -> Option<Solved> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut head = [0u8; 20];
    f.read_exact(&mut head).ok()?;
    if &head[..8] != MAGIC { return None; }
    let u = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    let (frames, bodies, seconds) = (u(8) as usize, u(12) as usize, f32::from_le_bytes([head[16], head[17], head[18], head[19]]));
    if bodies == 0 || bodies > 4096 || frames > 50_000_000 { return None; }
    let mut buf = vec![0u8; bodies * 4];
    f.read_exact(&mut buf).ok()?;
    let joints: Vec<usize> = buf.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize).collect();
    let stride = 3 + 4 * bodies + 4;
    let mut buf = vec![0u8; frames * stride * 4];
    f.read_exact(&mut buf).ok()?;
    let all: Vec<f32> = buf.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut solved = Solved { frames, joints, data: Vec::with_capacity(frames * (stride - 4)), stats: Vec::with_capacity(frames), report: BakeReport::default(), seconds };
    for row in all.chunks_exact(stride) {
        solved.data.extend_from_slice(&row[..stride - 4]);
        solved.stats.push([row[stride - 4], row[stride - 3], row[stride - 2], row[stride - 1]]);
    }
    solved.report_from_stats();
    Some(solved)
}

// ============================================================================
// JOBS
// ============================================================================

pub enum State {
    Running,
    Done(Arc<Solved>),
    Failed(String),
    Cancelled,
}

pub struct Job {
    pub done:    AtomicUsize,
    pub total:   usize,
    cancel:      AtomicBool,
    pub state:   Mutex<State>,
    pub started: std::time::Instant,
    /// What the rig was made of, for the panel.
    pub notes:   Mutex<String>,
}

static JOBS: OnceLock<Mutex<HashMap<u64, Arc<Job>>>> = OnceLock::new();
static CHANGED: AtomicBool = AtomicBool::new(false);

fn jobs() -> &'static Mutex<HashMap<u64, Arc<Job>>> { JOBS.get_or_init(|| Mutex::new(HashMap::new())) }

/// True once after a solve finished: the graph shows something new.
pub fn take_changed() -> bool { CHANGED.swap(false, Ordering::Relaxed) }

pub fn job(key: u64) -> Option<Arc<Job>> { jobs().lock().unwrap().get(&key).cloned() }

/// The solved result for a key, from memory or from its file.
pub fn solved(key: u64) -> Option<Arc<Solved>> {
    if let Some(j) = job(key) {
        return match &*j.state.lock().unwrap() { State::Done(s) => Some(s.clone()), _ => None };
    }
    // The panel asks on every frame: a key with no file is not looked up again.
    static MISSING: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    if MISSING.lock().unwrap().contains(&key) { return None; }
    // Results that ship with the examples are looked for after one's own.
    let shipped = || crate::examples::dir().map(|d| d.join(crate::examples::RAGDOLL_SOLVED).join(format!("{key:016x}.rag")));
    let Some(read) = file_of(key).and_then(|p| read_solved(&p)).or_else(|| shipped().and_then(|p| read_solved(&p))) else {
        let mut missing = MISSING.lock().unwrap();
        if missing.len() > 64 { missing.clear(); }
        missing.push(key);
        return None;
    };
    let s = Arc::new(read);
    let j = Arc::new(Job {
        done: AtomicUsize::new(s.frames), total: s.frames, cancel: AtomicBool::new(false),
        state: Mutex::new(State::Done(s.clone())), started: std::time::Instant::now(), notes: Mutex::new(String::new()),
    });
    jobs().lock().unwrap().insert(key, j);
    Some(s)
}

pub fn cancel(key: u64) { if let Some(j) = job(key) { j.cancel.store(true, Ordering::Relaxed); } }

/// Forget a result, in memory and on disk.
pub fn clear(key: u64) {
    cancel(key);
    jobs().lock().unwrap().remove(&key);
    if let Some(p) = file_of(key) { let _ = std::fs::remove_file(p); }
    CHANGED.store(true, Ordering::Relaxed);
}

/// Solve on a thread of its own. Progress is read from the job.
pub fn start(clip: Arc<AnimData>, collider: Option<Arc<MeshData>>, settings: Settings) -> u64 {
    let key = key(&clip, collider.as_ref(), &settings);
    if let Some(j) = job(key) { if matches!(&*j.state.lock().unwrap(), State::Running) { return key; } }
    let job = Arc::new(Job {
        done: AtomicUsize::new(0), total: clip.frames.max(1), cancel: AtomicBool::new(false),
        state: Mutex::new(State::Running), started: std::time::Instant::now(), notes: Mutex::new(String::new()),
    });
    jobs().lock().unwrap().insert(key, job.clone());
    std::thread::spawn(move || {
        let result = solve(&clip, collider.as_ref(), &settings, key, &job);
        *job.state.lock().unwrap() = match result {
            Ok(Some(s)) => State::Done(Arc::new(s)),
            Ok(None) => State::Cancelled,
            Err(e) => State::Failed(e),
        };
        CHANGED.store(true, Ordering::Relaxed);
    });
    key
}

/// The whole solve: rig, tree, chunks to a file, and the file read back.
fn solve(clip: &Arc<AnimData>, collider: Option<&Arc<MeshData>>, settings: &Settings, key: u64, job: &Job) -> Result<Option<Solved>, String> {
    let rig = build_rig(clip, settings)?;
    let bvh = collider.map(tree);
    *job.notes.lock().unwrap() = format!(
        "{} bodies, {} hull points{}{}",
        rig.defs.len(),
        rig.defs.iter().map(|d| d.hull.samples.len()).sum::<usize>(),
        match &bvh { Some(b) => format!(", collider of {} triangles", b.len()), None => ", no collider".into() },
        if rig.stand_ins.is_empty() { String::new() } else { format!(". No skin on {}: capsules stand in.", rig.stand_ins.join(", ")) });
    let frames = clip.frames.max(1);
    let fps = clip.rate.fps() as f32;
    let mut solver = Solver::new(rig.defs.clone(), settings.params(fps), &rig.bind, bvh);

    let path = file_of(key).ok_or("no folder to keep the result in")?;
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?; }
    let part = path.with_extension("part");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?);
    let mut head = Vec::with_capacity(20 + rig.joints.len() * 4);
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&(frames as u32).to_le_bytes());
    head.extend_from_slice(&(rig.joints.len() as u32).to_le_bytes());
    head.extend_from_slice(&0f32.to_le_bytes());
    for j in &rig.joints { head.extend_from_slice(&(*j as u32).to_le_bytes()); }
    file.write_all(&head).map_err(|e| e.to_string())?;

    let joints = rig.joints.clone();
    let target = |f: usize| body_poses(clip, &joints, f);
    let mut failed: Option<String> = None;
    let report = xms_ragdoll::bake(&mut solver, &BakeInput { frames, chunk: 128, target: &target }, &mut |chunk| {
        // A chunk is done: to the file, and out of memory.
        let mut row = Vec::with_capacity(chunk.len() * (7 + 4 * joints.len()));
        for f in chunk {
            row.extend(f.poses[0].p.to_array());
            for p in &f.poses { row.extend(p.q.to_array()); }
            row.extend([f.stats.weight, f.stats.residual, f.stats.deviation, f.stats.contacts as f32]);
        }
        if let Err(e) = write_f32s(&mut file, &row) { failed = Some(e.to_string()); return false; }
        job.done.fetch_add(chunk.len(), Ordering::Relaxed);
        !job.cancel.load(Ordering::Relaxed)
    });
    if let Some(e) = failed { let _ = std::fs::remove_file(&part); return Err(e); }
    if report.cancelled { drop(file); let _ = std::fs::remove_file(&part); return Ok(None); }
    let seconds = job.started.elapsed().as_secs_f32();
    let mut file = file.into_inner().map_err(|e| e.to_string())?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(16)).and_then(|_| file.write_all(&seconds.to_le_bytes())).map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    let mut solved = read_solved(&path).ok_or("could not read the result back")?;
    solved.report.released = report.released;
    solved.report.resets = report.resets;
    solved.report.substeps = report.substeps;
    Ok(Some(solved))
}

/// The clip a Ragdoll node puts out: the solved one when there is a result
/// for exactly this clip, collider and settings, the clip as it came otherwise.
///
/// The solved clip then goes through the human pass: elbows and knees bend
/// the way they bend, wrists and ankles stay within their range.
pub fn output(clip: &Arc<AnimData>, collider: Option<&Arc<MeshData>>, settings: &Settings, limits: &Limits) -> Arc<AnimData> {
    let key = key(clip, collider, settings);
    let Some(s) = solved(key) else { return clip.clone() };
    let params = format!("ragdoll:{key:x}:{}", serde_json::to_string(limits).unwrap_or_default());
    crate::core::anim::memo(&params, Some(clip), || {
        let moved = apply(clip, &s);
        let bvh = collider.map(tree);
        let crosses = |a: Vec3, b: Vec3| bvh.as_ref().map(|t| t.segment(a, b).is_some()).unwrap_or(false);
        let (out, report) = crate::core::human_ik::humanize(clip, &moved, &Human::of(clip), limits, bvh.as_ref().map(|_| &crosses as &(dyn Fn(Vec3, Vec3) -> bool + Sync)));
        *last_report().lock().unwrap() = Some((key, report));
        Some(out)
    }).unwrap_or_else(|| clip.clone())
}

/// What the human pass did on the last result it made, and for which solve.
pub fn last_report() -> &'static Mutex<Option<(u64, crate::core::human_ik::Report)>> {
    static R: OnceLock<Mutex<Option<(u64, crate::core::human_ik::Report)>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::create_test_clip;
    use crate::core::anim::FrameRate;

    fn walk() -> Arc<AnimData> {
        // Results of the tests go to a folder of their own.
        std::env::set_var("XMS_CACHE_DIR", std::env::temp_dir().join("xms_ragdoll_test"));
        Arc::new(create_test_clip(2.0, FrameRate::new(30, 1)).with_proxy_skin(3.0))
    }

    #[test]
    fn the_test_skeleton_becomes_bodies_with_limits() {
        let clip = walk();
        let rig = build_rig(&clip, &Settings::default()).unwrap();
        // Hips, spine, chest, neck, head, two legs of three and two arms of three.
        // Toes go with the feet. The test skeleton has no collar bones.
        assert_eq!(rig.defs.len(), 17);
        assert_eq!(rig.roles[0], (Role::Pelvis, Side::Centre));
        assert!(rig.defs[0].parent.is_none() && rig.defs.iter().skip(1).all(|d| d.parent.is_some()));
        assert!(rig.defs.iter().all(|d| d.mass > 0.0 && d.hull.volume > 0.0 && d.twist_axis.is_normalized()));
        // Knees bend in the walk, so their axis is found: it is the X axis the clip turns them about.
        let knee = rig.defs.iter().find(|d| d.name.ends_with("LeftLeg")).unwrap();
        let h = knee.hinge.expect("knee hinge");
        assert!(h.axis.x.abs() > 0.99, "axis {:?}", h.axis);
        assert!(h.min < 0.0 && h.min > -0.1 && h.max > 2.5);
        // The body of a joint with no part of its own is the one above it.
        assert!(rig.body_of_joint.iter().all(|b| b.is_some()));
    }

    #[test]
    fn a_skeleton_that_is_not_a_human_is_refused_with_the_names_it_wants() {
        let mut clip = (*walk()).clone();
        for (i, j) in clip.joints.iter_mut().enumerate() { j.name = format!("bone_{i}"); }
        let err = build_rig(&clip, &Settings::default()).err().unwrap();
        assert!(err.contains("Characterize"), "{err}");
        // Set by hand, the same skeleton is a human again.
        let names: Vec<String> = clip.joints.iter().map(|j| j.name.clone()).collect();
        let walk = walk();
        let auto = Human::of(&walk);
        let picks: crate::core::human::Picks = Slot::ALL.iter().filter_map(|s| auto.get(*s).map(|j| (*s, names[j].clone()))).collect();
        let set = crate::core::human::with_picks(&clip, &picks);
        assert_eq!(build_rig(&set, &Settings::default()).unwrap().defs.len(), 17);
    }

    #[test]
    fn calamari_cuts_the_skin_into_one_piece_per_body() {
        let clip = walk();
        let cut = calamari(&clip, false, 1);
        let skin = cut.skin.as_ref().unwrap();
        // Every face is carried by one joint, and that joint is a body.
        let rig = build_rig(&clip, &Settings::default()).unwrap();
        for f in &skin.faces {
            let j = skin.joint[f[0] as usize];
            assert!(f.iter().all(|v| skin.joint[*v as usize] == j));
            assert!(rig.joints.contains(&(j as usize)));
        }
        assert_eq!(skin.faces.len(), clip.skin.as_ref().unwrap().faces.len());
        // As hulls: closed, one per body.
        let hulls = calamari(&clip, true, 1);
        let h = hulls.skin.as_ref().unwrap();
        assert_eq!(h.positions.len(), rig.defs.iter().map(|d| d.hull.samples.len()).sum::<usize>());
        // It still follows the skeleton.
        let (p, _) = h.deformed(&hulls.world_pose(0));
        assert!(p.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn solving_with_nothing_to_hit_gives_the_clip_back() {
        let clip = walk();
        let mut settings = Settings::default();
        settings.self_collision = false;
        let rig = build_rig(&clip, &settings).unwrap();
        let mut solver = Solver::new(rig.defs.clone(), settings.params(30.0), &rig.bind, None);
        let joints = rig.joints.clone();
        let target = |f: usize| body_poses(&clip, &joints, f);
        let mut solved = Solved { frames: clip.frames, joints: joints.clone(), data: vec![], stats: vec![], report: BakeReport::default(), seconds: 0.0 };
        xms_ragdoll::bake(&mut solver, &BakeInput { frames: clip.frames, chunk: 16, target: &target }, &mut |chunk| {
            for f in chunk {
                solved.data.extend(f.poses[0].p.to_array());
                for p in &f.poses { solved.data.extend(p.q.to_array()); }
                solved.stats.push([f.stats.weight, f.stats.residual, f.stats.deviation, f.stats.contacts as f32]);
            }
            true
        });
        let out = apply(&clip, &solved);
        for f in [0, 17, clip.frames - 1] {
            let (a, b) = (clip.world_pose(f), out.world_pose(f));
            for j in 0..clip.joints.len() {
                assert!((a[j].w_axis - b[j].w_axis).length() < 1e-3, "frame {f} joint {j}: {}", (a[j].w_axis - b[j].w_axis).length());
            }
        }
    }

    #[test]
    fn a_floor_under_the_walk_lifts_the_feet_and_keeps_every_bone_its_length() {
        let clip = walk();
        // The ankles of the walk come down to 7 cm. A floor at 13 cm is in their way.
        let (v, idx) = {
            let s = 5.0f32;
            (vec![[-s, 0.13, -s], [s, 0.13, -s], [s, 0.13, s], [-s, 0.13, s]], vec![0u32, 2, 1, 0, 3, 2])
        };
        let floor = Arc::new(MeshData::from_triangles(v, idx));
        let settings = Settings::default();
        let key = start(clip.clone(), Some(floor.clone()), settings);
        let job = job(key).unwrap();
        for _ in 0..600 {
            if !matches!(&*job.state.lock().unwrap(), State::Running) { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let solved = solved(key).expect("solved");
        assert_eq!(solved.frames, clip.frames);
        assert!(solved.report.contact_frames > 10 && solved.report.resets == 0);
        let out = output(&clip, Some(&floor), &settings, &Limits::default());
        assert!(!Arc::ptr_eq(&out, &clip));
        let ankles: Vec<usize> = clip.joints.iter().enumerate().filter(|(_, j)| j.name.ends_with("Foot")).map(|(i, _)| i).collect();
        let lowest = |c: &AnimData| (0..c.frames).map(|f| { let w = c.world_pose(f); ankles.iter().map(|j| w[*j].w_axis.y).fold(f32::MAX, f32::min) }).fold(f32::MAX, f32::min);
        assert!(lowest(&clip) < 0.08, "the walk reaches {}", lowest(&clip));
        // Nothing held is left in the floor. The ankles come up from 7 cm to
        // about 12: with the ankle kept within a human range the foot cannot
        // lie flat through the whole stride, and in the frames a leg held too
        // far from the walk lets go, its foot dips back toward the capture.
        assert!(solved.report.max_residual < 0.003, "{:?}", solved.report);
        assert!(lowest(&out) > 0.115, "after: {}", lowest(&out));
        // Bones keep their length: local translations are untouched below the hips.
        for j in 1..clip.joints.len() {
            for f in [0, 20, 40] { assert_eq!(out.local(j, f).translation, clip.local(j, f).translation); }
        }
        // The file is found again under the same key, and gone after a clear.
        assert!(file_of(key).unwrap().exists());
        clear(key);
        assert!(!file_of(key).unwrap().exists());
        assert!(Arc::ptr_eq(&output(&clip, Some(&floor), &settings, &Limits::default()), &clip));
    }
}

/// A whole take against a set, from files of your own:
///
///     XMS_RAGDOLL_CLIP=take.fbx XMS_RAGDOLL_SET=set.fbx cargo test real_take -- --ignored --nocapture
#[cfg(test)]
mod real {
    use super::*;

    #[test]
    #[ignore]
    fn real_take() {
        let (Ok(clip_path), Ok(set_path)) = (std::env::var("XMS_RAGDOLL_CLIP"), std::env::var("XMS_RAGDOLL_SET")) else {
            println!("set XMS_RAGDOLL_CLIP and XMS_RAGDOLL_SET to an FBX take and an FBX set");
            return;
        };
        let t = std::time::Instant::now();
        let clip = crate::fbx_loader::load_fbx(&clip_path, 0).unwrap().anim;
        let mesh = crate::fbx_loader::load_meshes(&set_path).unwrap()[0].1.clone();
        println!("loaded in {:.1} s: {} joints, {} frames, set of {} triangles", t.elapsed().as_secs_f32(), clip.joints.len(), clip.frames, mesh.indices.len() / 3);
        let settings = Settings::default();
        let rig = build_rig(&clip, &settings).unwrap();
        println!("{} bodies: {}", rig.defs.len(), rig.defs.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(", "));
        let mut solver = Solver::new(rig.defs.clone(), settings.params(clip.rate.fps() as f32), &rig.bind, Some(tree(&mesh)));
        let joints = rig.joints.clone();
        let target = |f: usize| body_poses(&clip, &joints, f);
        let t = std::time::Instant::now();
        let mut worst = 0.0f32;
        let report = xms_ragdoll::bake(&mut solver, &BakeInput { frames: clip.frames, chunk: 128, target: &target }, &mut |chunk| {
            for f in chunk { for p in &f.poses { assert!(p.p.is_finite() && p.q.is_finite()); } worst = worst.max(f.stats.deviation); }
            true
        });
        let secs = t.elapsed().as_secs_f32();
        println!("{} frames in {:.1} s, {:.0} a second", report.frames, secs, report.frames as f32 / secs);
        println!("in or near contact: {} frames. Collisions off: {} frames, {:?}", report.contact_frames, report.ghost_frames, report.ghost_ranges);
        println!("furthest from the capture {:.1} cm, limbs let go {} times, bodies put back {}", worst * 100.0, report.released, report.resets);
        assert_eq!(report.frames, clip.frames);
        assert_eq!(report.resets, 0);
    }
}

#[cfg(test)]
mod shipped {
    /// Solve the template and put the result with the examples:
    ///
    ///     cargo test solve_the_template -- --ignored --nocapture
    #[test]
    #[ignore]
    fn solve_the_template() {
        let dir = std::env::temp_dir().join("xms_ragdoll_solve");
        std::env::set_var("XMS_CACHE_DIR", &dir);
        let mut g = crate::node_graph::NodeGraphState::default();
        let mut subnets = crate::ice::SubnetStore::default();
        let t = crate::templates::TEMPLATES.iter().find(|t| t.name.starts_with("Body Collide")).unwrap();
        (t.build)(&mut g, &mut subnets);
        let id = g.nodes.iter().find(|n| matches!(n.node_type, crate::types::NodeType::Ragdoll { .. })).unwrap().id;
        let (clip, car) = g.ragdoll_inputs(id);
        let clip = clip.unwrap();
        let key = super::start(clip.clone(), car, super::Settings::default());
        let job = super::job(key).unwrap();
        loop {
            if !matches!(&*job.state.lock().unwrap(), super::State::Running) { break; }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let solved = super::solved(key).expect("solved");
        println!("{}", job.notes.lock().unwrap());
        println!("{} frames in {:.0} s: {:?}", solved.frames, solved.seconds, solved.report);
        let to = crate::examples::dir().unwrap().join(crate::examples::RAGDOLL_SOLVED);
        for e in std::fs::read_dir(&to).unwrap().flatten() { if e.path().extension().map(|x| x == "rag").unwrap_or(false) { std::fs::remove_file(e.path()).unwrap(); } }
        std::fs::copy(super::file_of(key).unwrap(), to.join(format!("{key:016x}.rag"))).unwrap();
    }

    /// How deep the limbs are in the car, after the solve and after the
    /// human pass, every 4th frame.
    fn depths(clip: &crate::core::anim::AnimData, car: &std::sync::Arc<crate::types::MeshData>, a: &crate::core::anim::AnimData, b: &crate::core::anim::AnimData) {
        let settings = super::Settings::default();
        let rig = super::build_rig(clip, &settings).unwrap();
        let solver = super::Solver::new(rig.defs.clone(), settings.params(30.0), &rig.bind, Some(super::tree(car)));
        for (name, role) in [("upper arms", super::Role::UpperArm), ("forearms", super::Role::LowerArm), ("hands", super::Role::Hand), ("thighs", super::Role::Thigh), ("calves", super::Role::Calf), ("feet", super::Role::Foot)] {
            let bodies: Vec<usize> = (0..rig.defs.len()).filter(|b| rig.roles[*b].0 == role).collect();
            let mut out = [(0usize, 0.0f32); 2];
            let mut deep = [0usize; 2];
            for f in (0..clip.frames).step_by(4) {
                for (k, c) in [a, b].iter().enumerate() {
                    let poses = super::body_poses(c, &rig.joints, f);
                    let deepest = bodies.iter().map(|i| solver.target_depth(*i, &poses[*i], 2)).fold(0.0f32, f32::max);
                    if deepest > 0.01 { out[k].0 += 1; }
                    if deepest > 0.03 { deep[k] += 1; }
                    out[k].1 = out[k].1.max(deepest);
                }
            }
            println!("{name} more than 1 cm in the car: solved {} frames (more than 3 cm {}, deepest {:.1} cm), after {} frames (more than 3 cm {}, deepest {:.1} cm), of {}", out[0].0, deep[0], out[0].1 * 100.0, out[1].0, deep[1], out[1].1 * 100.0, clip.frames.div_ceil(4));
        }
    }

    #[test]
    #[ignore]
    fn hinge_axes_of_the_template() {
        std::env::set_var("XMS_CACHE_DIR", std::env::temp_dir().join("xms_ragdoll_none"));
        let mut g = crate::node_graph::NodeGraphState::default();
        let mut subnets = crate::ice::SubnetStore::default();
        let t = crate::templates::TEMPLATES.iter().find(|t| t.name.starts_with("Body Collide")).unwrap();
        (t.build)(&mut g, &mut subnets);
        let id = g.nodes.iter().find(|n| matches!(n.node_type, crate::types::NodeType::Ragdoll { .. })).unwrap().id;
        let clip = g.ragdoll_inputs(id).0.unwrap();
        let rig = super::build_rig(&clip, &super::Settings::default()).unwrap();
        let rot = |m: &bevy::math::Mat4| m.to_scale_rotation_translation().1;
        for (b, d) in rig.defs.iter().enumerate() {
            let Some(h) = d.hinge else { continue };
            let (j, p) = (rig.joints[b], rig.joints[d.parent.unwrap()]);
            let c = rig.joints[(0..rig.defs.len()).find(|c| rig.defs[*c].parent == Some(b)).unwrap()];
            // Geometric: plane normal of the two bones, in the child's frame, weighted by bend.
            let mut sum = bevy::math::Vec3::ZERO;
            for f in (0..clip.frames).step_by(20) {
                let w = clip.world_pose(f);
                let (s, e, wr) = (w[p].w_axis.truncate(), w[j].w_axis.truncate(), w[c].w_axis.truncate());
                let n = (e - s).normalize().cross((wr - e).normalize());
                if n.length() > 0.34 { sum += rot(&w[j]).inverse() * n; }
            }
            let geo = sum.normalize();
            println!("{}: PCA axis {:?}, plane axis {:?}, apart {:.0} deg, range {:.0}..{:.0}", d.name, h.axis, geo, h.axis.angle_between(geo).to_degrees().min(180.0 - h.axis.angle_between(geo).to_degrees()), h.min.to_degrees(), h.max.to_degrees());
        }
    }

    /// The human pass on the shipped solve of the template, measured:
    ///
    ///     cargo test human_pass_on_the_template -- --ignored --nocapture
    #[test]
    #[ignore]
    fn human_pass_on_the_template() {
        use crate::core::human::{Human, Slot};
        std::env::set_var("XMS_CACHE_DIR", std::env::temp_dir().join("xms_ragdoll_none"));
        let mut g = crate::node_graph::NodeGraphState::default();
        let mut subnets = crate::ice::SubnetStore::default();
        let t = crate::templates::TEMPLATES.iter().find(|t| t.name.starts_with("Body Collide")).unwrap();
        (t.build)(&mut g, &mut subnets);
        let id = g.nodes.iter().find(|n| matches!(n.node_type, crate::types::NodeType::Ragdoll { .. })).unwrap().id;
        let (clip, car) = g.ragdoll_inputs(id);
        let clip = clip.unwrap();
        let solved = super::solved(super::key(&clip, car.as_ref(), &super::Settings::default())).unwrap();
        let h = Human::of(&clip);
        println!("{}: {} of 20 parts", h.convention, h.found());
        println!("solve: {:?}", solved.report);
        if let Ok(p) = std::env::var("XMS_OLD_RAG") {
            let old = super::read_solved(std::path::Path::new(&p)).unwrap();
            println!("earlier solve: {:?}", old.report);
            let before = super::apply(&clip, &old);
            if std::env::var("XMS_DEPTHS").is_ok() { println!("earlier solve, and the human pass on it:"); let bvh = super::tree(car.as_ref().unwrap()); let crosses = |a: bevy::math::Vec3, b: bevy::math::Vec3| bvh.segment(a, b).is_some(); let (o, r) = crate::core::human_ik::humanize(&clip, &before, &h, &Default::default(), Some(&crosses)); println!("{r:?}"); depths(&clip, car.as_ref().unwrap(), &before, &o); println!("this solve:"); }
        }
        let moved = super::apply(&clip, &solved);
        let t0 = std::time::Instant::now();
        let bvh = super::tree(car.as_ref().unwrap());
        let crosses = |a: bevy::math::Vec3, b: bevy::math::Vec3| bvh.segment(a, b).is_some();
        let mut limits = crate::core::human_ik::Limits::default();
        if let Ok(v) = std::env::var("XMS_SWIVEL") { limits.swivel = v.parse().unwrap(); }
        let (out, r) = crate::core::human_ik::humanize(&clip, &moved, &h, &limits, Some(&crosses));
        println!("human pass {:.2} s over {} frames: {r:?}", t0.elapsed().as_secs_f32(), clip.frames);
        // Hand against forearm, and forearm against capture, in degrees.
        let rot = |m: &bevy::math::Mat4| m.to_scale_rotation_translation().1;
        for (hand, fore) in [(Slot::LeftHand, Slot::LeftLowerArm), (Slot::RightHand, Slot::RightLowerArm), (Slot::LeftFoot, Slot::LeftCalf), (Slot::RightFoot, Slot::RightCalf)] {
            let (e, m) = (h.get(hand).unwrap(), h.get(fore).unwrap());
            let rest = clip.skin.as_ref().map(|s| s.bind.clone()).unwrap();
            let neutral = rot(&rest[m]).inverse() * rot(&rest[e]);
            let mut worst = [(0.0f32, 0usize); 3];
            for f in 0..clip.frames {
                for (k, c) in [&*clip, &moved, &out].iter().enumerate() {
                    let w = c.world_pose(f);
                    let a = (neutral.inverse() * rot(&w[m]).inverse() * rot(&w[e])).angle_between(bevy::math::Quat::IDENTITY).to_degrees();
                    if a > worst[k].0 { worst[k] = (a, f); }
                }
            }
            println!("{hand:?} from neutral, most: capture {:.0} (frame {}), solved {:.0} (frame {}), after {:.0} (frame {})", worst[0].0, worst[0].1, worst[1].0, worst[1].1, worst[2].0, worst[2].1);
        }
        for s in [Slot::LeftUpperArm, Slot::RightUpperArm, Slot::LeftLowerArm, Slot::RightLowerArm, Slot::LeftThigh, Slot::RightThigh, Slot::LeftCalf, Slot::RightCalf] {
            let j = h.get(s).unwrap();
            let mut worst = [(0.0f32, 0usize); 2];
            for f in 0..clip.frames {
                let wc = clip.world_pose(f);
                for (k, c) in [&moved, &out].iter().enumerate() {
                    let w = c.world_pose(f);
                    let a = rot(&wc[j]).angle_between(rot(&w[j])).to_degrees();
                    if a > worst[k].0 { worst[k] = (a, f); }
                }
            }
            println!("{s:?} from capture, most: solved {:.0} (frame {}), after {:.0} (frame {})", worst[0].0, worst[0].1, worst[1].0, worst[1].1);
        }
        // Elbows and knees: how far the pass moves them, how many frames it
        // touches, and the largest jump from one frame to the next.
        for s in [Slot::LeftLowerArm, Slot::RightLowerArm, Slot::LeftCalf, Slot::RightCalf] {
            let j = h.get(s).unwrap();
            let (mut most, mut touched, mut jump_s, mut jump_o, mut jump_at) = (0.0f32, 0usize, 0.0f32, 0.0f32, 0usize);
            let (mut prev_s, mut prev_o): (Option<bevy::math::Vec3>, Option<bevy::math::Vec3>) = (None, None);
            for f in 0..clip.frames {
                let (a, b) = (moved.world_pose(f)[j].w_axis.truncate(), out.world_pose(f)[j].w_axis.truncate());
                let d = (a - b).length();
                most = most.max(d);
                if d > 0.005 { touched += 1; }
                if let (Some(ps), Some(po)) = (prev_s, prev_o) { jump_s = jump_s.max((a - ps).length()); if (b - po).length() > jump_o { jump_o = (b - po).length(); jump_at = f; } }
                prev_s = Some(a); prev_o = Some(b);
            }
            println!("{s:?}: moved up to {:.1} cm, more than 5 mm in {touched} frames; largest step between frames {:.1} cm solved, {:.1} cm after (frame {jump_at})", most * 100.0, jump_s * 100.0, jump_o * 100.0);
        }
        if std::env::var("XMS_DEPTHS").is_ok() { depths(&clip, car.as_ref().unwrap(), &moved, &out); }
    }

    /// The template opens solved, from the result that ships with the examples.
    #[test]
    fn the_template_opens_solved_when_its_files_are_there() {
        std::env::set_var("XMS_CACHE_DIR", std::env::temp_dir().join("xms_ragdoll_none"));
        let mut g = crate::node_graph::NodeGraphState::default();
        let mut subnets = crate::ice::SubnetStore::default();
        let t = crate::templates::TEMPLATES.iter().find(|t| t.name.starts_with("Body Collide")).unwrap();
        (t.build)(&mut g, &mut subnets);
        let id = g.nodes.iter().find(|n| matches!(n.node_type, crate::types::NodeType::Ragdoll { .. })).unwrap().id;
        let (clip, car) = g.ragdoll_inputs(id);
        let clip = clip.expect("take");
        let key = super::key(&clip, car.as_ref(), &super::Settings::default());
        let solved = super::solved(key).unwrap_or_else(|| panic!("no shipped result for {key:016x}"));
        assert_eq!(solved.frames, clip.frames);
        assert!(!std::sync::Arc::ptr_eq(&g.eval_anim(id).unwrap(), &clip));
    }
}

