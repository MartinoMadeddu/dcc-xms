//! The solver: rigid bodies that follow a capture and give way to what is
//! in their way.
//!
//! One frame is taken in small steps. In each step a body is first carried
//! along with the capture, whatever it is doing, so a body that is not held
//! back by anything reproduces the capture exactly. What is left is the
//! body's offset from the capture, and three things act on it:
//!
//! - it shrinks, by a share per frame, so a body that was pushed aside
//!   comes back smoothly when it is let go;
//! - joints hold the bodies together and limit how far each joint may be
//!   bent away from the capture;
//! - contacts push bodies out of the collider and out of each other, no
//!   faster than a set speed, and start pushing softly a little before the
//!   surface.
//!
//! There are no velocities and no forces, only positions that are moved
//! toward satisfying each of these in turn. Nothing can gain energy.

use std::sync::Arc;
use glam::{Quat, Vec3};
use crate::bvh::Bvh;
use crate::hull::Hull;

/// Where a bone is: its joint and its orientation, in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose { pub p: Vec3, pub q: Quat }

impl Pose {
    pub const IDENTITY: Pose = Pose { p: Vec3::ZERO, q: Quat::IDENTITY };
    pub fn lerp(&self, to: &Pose, t: f32) -> Pose {
        Pose { p: self.p.lerp(to.p, t), q: self.q.slerp(to.q, t).normalize() }
    }
}

/// A joint that bends about one axis, with the range it may bend through.
#[derive(Clone, Copy, Debug)]
pub struct Hinge {
    /// The axis, in the frame of the bone below the joint.
    pub axis:      Vec3,
    /// Rotation of that bone in its parent's frame from which the bend is measured.
    pub reference: Quat,
    /// Least and greatest bend, radians.
    pub min:       f32,
    pub max:       f32,
}

/// What a wrist or an ankle allows, counted from where the skin was bound:
/// the swing of the bone below inside an ellipse, wide for flexion and
/// narrow from side to side. Where the capture goes past it, the capture
/// is the limit for that frame.
#[derive(Clone, Copy, Debug)]
pub struct Range {
    /// Rotation of the bone in its parent's frame where the skin was bound.
    pub neutral:   Quat,
    /// Long axis of the bone, and the axis it flexes about, in its own frame.
    pub along:     Vec3,
    pub flex_axis: Vec3,
    /// Most swing about the flexion axis, and from side to side, radians.
    pub flex:      f32,
    pub side:      f32,
    /// Most twist about the long axis either way, radians: turning a hand
    /// over, rolling a foot onto its edge.
    pub twist:     f32,
}

#[derive(Clone)]
pub struct BodyDef {
    pub name:       String,
    /// Body this one is jointed to. Must come before it in the list.
    pub parent:     Option<usize>,
    /// Shape, in the frame of the bone.
    pub hull:       Hull,
    pub mass:       f32,
    /// Share of the way back to the capture covered in one frame, for where
    /// the body is and for how it is turned. Hands, feet and the head hold
    /// their turn more firmly than their place: a foot that is lifted stays level.
    pub follow:     f32,
    pub follow_turn: f32,
    /// Part of the trunk. Two parts of the trunk never collide with each other.
    pub trunk:      bool,
    /// How much harder this body is to turn than its shape alone makes it.
    /// A hand or a foot that is pushed moves the limb, it does not spin.
    pub turn_resist: f32,
    /// How far the joint to the parent may leave the capture, radians.
    pub swing:      f32,
    pub twist:      f32,
    /// Direction of the bone in its own frame: the axis twist is measured about.
    pub twist_axis: Vec3,
    pub hinge:      Option<Hinge>,
    /// The range of a wrist or an ankle, whatever the capture.
    pub range:      Option<Range>,
    /// The character cannot step around something that blocks this body.
    pub core:       bool,
    /// Depth this body may rest in a surface, metres: a seat gives.
    pub sink:       f32,
}

#[derive(Clone, Debug)]
pub struct Params {
    pub fps:            f32,
    /// Distance from a surface at which a body starts to be pushed away, metres.
    pub margin:         f32,
    /// Strength of that push: share of the remaining margin per frame.
    pub soft:           f32,
    /// How strongly a body pressed into a surface resists sliding along it.
    pub friction:       f32,
    pub self_collision: bool,
    /// Depth by which parts of the character may sink into each other, metres.
    pub self_slack:     f32,
    pub iterations:     usize,
    /// A frame is cut into steps so that nothing moves further than this in one, metres.
    pub max_step:       f32,
    pub max_substeps:   usize,
    /// Greatest speed at which a body is pushed out of something, metres per second.
    pub release_speed:  f32,
    /// Furthest a core body is moved out of the collider, metres. Where its
    /// capture lies deeper than this, it is moved this far and left in by
    /// the rest: a trunk sunk a little in a seat is lifted onto it, one sunk
    /// deep is not thrown across the car.
    pub ghost_depth:    f32,
    /// Frames over which collisions fade out before, and in after.
    pub fade_out:       usize,
    pub fade_in:        usize,
    /// A limb held further than this from its capture lets go. Metres.
    pub limb_limit:     f32,
    /// What the solver changed is evened out over this many frames either
    /// side, so a contact is taken up over a few frames and nothing flickers.
    pub smooth:         usize,
    pub threads:        usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            fps: 30.0, margin: 0.012, soft: 0.5, friction: 1.0, self_collision: true, self_slack: 0.03,
            iterations: 4, max_step: 0.012, max_substeps: 16, release_speed: 0.6,
            ghost_depth: 0.12, fade_out: 6, fade_in: 10, limb_limit: 0.30, smooth: 2, threads: 1,
        }
    }
}

/// What happened in one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub substeps:  u32,
    pub contacts:  u32,
    /// Deepest a body is still inside the collider at the end of the frame, metres.
    pub residual:  f32,
    /// Furthest a joint is from its capture, metres.
    pub deviation: f32,
    /// Collision weight of the whole character: 0 while it is a ghost.
    pub weight:    f32,
    /// Limbs that are let go in this frame.
    pub released:  u32,
    /// Bodies put back on the capture because their state was not usable.
    pub resets:    u32,
}

struct Body {
    def:        BodyDef,
    /// Centre of mass and reference point in the bone's frame.
    com_local:  Vec3,
    ref_local:  Vec3,
    inv_mass:   f32,
    inv_inertia: Vec3,
    /// Hull samples relative to the centre of mass, in the bone's axes.
    samples:    Vec<Vec3>,
    radius:     f32,
    // State.
    com:        Vec3,
    q:          Quat,
    /// Collision weight of this limb, and whether it has let go.
    weight:     f32,
    released:   bool,
    /// Depth this body is left in the collider in this frame, on top of `sink`.
    left_in:    f32,
}

impl Body {
    fn origin(&self) -> Vec3 { self.com - self.q * self.com_local }
    fn inv_inertia_world(&self, v: Vec3) -> Vec3 { self.q * (self.inv_inertia * (self.q.inverse() * v)) }
    fn inv_mass_at(&self, r: Vec3, n: Vec3) -> f32 {
        let rn = r.cross(n);
        self.inv_mass + rn.dot(self.inv_inertia_world(rn))
    }
    /// Push with `p` (mass times distance) at `r` from the centre of mass.
    fn push(&mut self, r: Vec3, p: Vec3) {
        self.com += p * self.inv_mass;
        self.turn(self.inv_inertia_world(r.cross(p)));
    }
    /// Turn by a small rotation vector about the centre of mass.
    fn turn(&mut self, w: Vec3) {
        let a = w.length();
        if a < 1e-9 { return; }
        self.q = (Quat::from_axis_angle(w / a, a) * self.q).normalize();
    }
    fn set_pose(&mut self, pose: &Pose) { self.q = pose.q; self.com = pose.p + pose.q * self.com_local; }
}

#[derive(Clone, Copy)]
struct Contact {
    body:   u32,
    /// Point of the body, from its centre of mass, in the bone's axes.
    local:  Vec3,
    /// Plane the point must stay in front of.
    point:  Vec3,
    normal: Vec3,
    /// Where the point was when the step began, for friction.
    start:  Vec3,
    /// How far in front of the plane the point would be at the end of the
    /// step with nothing holding it.
    free:   f32,
}

struct Pair { a: usize, b: usize, allow: f32 }

pub struct Solver {
    bodies:   Vec<Body>,
    pairs:    Vec<Pair>,
    pub params: Params,
    collider: Option<Arc<Bvh>>,
    contacts: Vec<Contact>,
    /// Bodies below each body, itself included.
    below:    Vec<Vec<usize>>,
}

impl Solver {
    /// `bind` is a pose in which the character does not collide with
    /// itself, usually the pose the skin was bound in. Parts that overlap
    /// there are allowed to overlap that much always.
    pub fn new(defs: Vec<BodyDef>, params: Params, bind: &[Pose], collider: Option<Arc<Bvh>>) -> Solver {
        let mut bodies: Vec<Body> = defs.into_iter().enumerate().map(|(i, def)| {
            let mass = def.mass.max(1e-4);
            let inertia = def.hull.inertia(mass).max(Vec3::splat(mass * 1e-5)) * def.turn_resist.max(1.0);
            let com_local = def.hull.com;
            // Contacts are judged from a point that is safely this body's own:
            // just inside its joint, or its middle for a body with no parent.
            let ref_local = if def.parent.is_some() { com_local * 0.15 } else { com_local };
            let pose = bind.get(i).copied().unwrap_or(Pose::IDENTITY);
            Body {
                samples: def.hull.samples.iter().map(|s| *s - com_local).collect(),
                radius: def.hull.radius,
                com_local, ref_local,
                inv_mass: 1.0 / mass,
                inv_inertia: Vec3::ONE / inertia,
                com: pose.p + pose.q * com_local, q: pose.q,
                weight: 1.0, released: false, left_in: 0.0,
                def,
            }
        }).collect();
        for (i, b) in bodies.iter_mut().enumerate() {
            if b.def.parent.map(|p| p >= i).unwrap_or(false) { b.def.parent = None; }
        }
        let n = bodies.len();
        let mut below: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
        for i in (0..n).rev() {
            if let Some(p) = bodies[i].def.parent { let kids = below[i].clone(); below[p].extend(kids); }
        }
        let mut solver = Solver { bodies, pairs: vec![], params, collider, contacts: vec![], below };
        // Pairs that may collide: not jointed to each other, and not sunk
        // deep into each other where the skin was bound.
        // Links between two bodies along the skeleton.
        let chain = |mut i: usize| { let mut up = vec![i]; while let Some(p) = solver.bodies[i].def.parent { up.push(p); i = p; } up };
        let links = |a: usize, b: usize| -> usize {
            let (ca, cb) = (chain(a), chain(b));
            for (da, x) in ca.iter().enumerate() { if let Some(db) = cb.iter().position(|y| y == x) { return da + db; } }
            usize::MAX
        };
        for a in 0..n { for b in a + 1..n {
            let (ta, tb) = (solver.bodies[a].def.trunk, solver.bodies[b].def.trunk);
            // Neighbours overlap by nature, and the trunk does not fold into itself.
            if ta && tb { continue; }
            // So does a limb where it grows out of the trunk: an upper arm
            // lies against the ribs, a thigh against the hips.
            let near = links(a, b);
            let rooted = |limb: usize| solver.bodies[limb].def.parent.map(|p| solver.bodies[p].def.trunk).unwrap_or(false);
            if near <= 1 || (tb && !ta && rooted(a) && near <= 3) || (ta && !tb && rooted(b) && near <= 3) { continue; }
            let depth = solver.overlap(a, b).max(solver.overlap(b, a));
            if depth > 0.06 { continue; }
            solver.pairs.push(Pair { a, b, allow: depth.max(0.0) });
        } }
        solver
    }

    pub fn body_count(&self) -> usize { self.bodies.len() }

    /// Deepest each body is inside the planes it was last held against, metres.
    pub fn residuals(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; self.bodies.len()];
        for c in &self.contacts {
            let b = &self.bodies[c.body as usize];
            let s = c.normal.dot(b.com + b.q * c.local - c.point) + b.def.sink + b.left_in;
            out[c.body as usize] = out[c.body as usize].max(-s);
        }
        out
    }
    pub fn def(&self, i: usize) -> &BodyDef { &self.bodies[i].def }
    pub fn poses(&self) -> Vec<Pose> { self.bodies.iter().map(|b| Pose { p: b.origin(), q: b.q }).collect() }

    /// Put every body on the capture.
    pub fn reset(&mut self, target: &[Pose]) {
        for (b, t) in self.bodies.iter_mut().zip(target) { b.set_pose(t); b.weight = 1.0; b.released = false; }
        self.contacts.clear();
    }

    /// Deepest that samples of `a` are inside the hull of `b`, as they stand.
    fn overlap(&self, a: usize, b: usize) -> f32 {
        let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
        if (ba.com - bb.com).length() > ba.radius + bb.radius { return 0.0; }
        let inv = bb.q.inverse();
        let mut deepest = 0.0f32;
        for s in &ba.samples {
            let x = ba.com + ba.q * *s;
            let (d, _) = bb.def.hull.signed(inv * (x - bb.com) + bb.com_local);
            deepest = deepest.max(-d);
        }
        deepest
    }

    /// How deep the capture of a body is in the collider, seen from its
    /// middle: the furthest any part of it lies behind a surface, measured
    /// square to that surface, and the share of it that lies behind one.
    /// `stride` looks at every nth sample only.
    pub fn target_cut(&self, i: usize, pose: &Pose, stride: usize) -> (f32, f32) {
        let Some(bvh) = &self.collider else { return (0.0, 0.0) };
        let b = &self.bodies[i];
        let com = pose.p + pose.q * b.com_local;
        if !bvh.near_sphere(com, b.radius) { return (0.0, 0.0); }
        let (mut deepest, mut behind, mut seen) = (0.0f32, 0usize, 0usize);
        for s in b.samples.iter().step_by(stride.max(1)) {
            let x = com + pose.q * *s;
            seen += 1;
            if let Some(hit) = bvh.segment(com, x) {
                behind += 1;
                deepest = deepest.max(hit.normal.dot(hit.point - x).abs());
            }
        }
        (deepest, behind as f32 / seen.max(1) as f32)
    }

    pub fn target_depth(&self, i: usize, pose: &Pose, stride: usize) -> f32 { self.target_cut(i, pose, stride).0 }

    /// Does the capture take a core body through a surface in this frame:
    /// its middle across one, or more than half of it behind one? Then the
    /// character cannot be kept out of the collider. Sitting low in a seat
    /// is neither. Walking through a door is both.
    pub fn blocked(&self, from: &[Pose], to: &[Pose]) -> bool { self.core_state(from, to).0 }

    /// What the capture does to the core bodies in a frame: whether it
    /// takes one through a surface, whether one touches the collider at
    /// all, and how deep each lies in it (zero for bodies that are not core).
    pub fn core_state(&self, from: &[Pose], to: &[Pose]) -> (bool, bool, Vec<f32>) {
        let mut depths = vec![0.0f32; self.bodies.len()];
        let Some(bvh) = &self.collider else { return (false, false, depths) };
        let (mut through, mut touching) = (false, false);
        for (i, b) in self.bodies.iter().enumerate() {
            if !b.def.core { continue; }
            let (c0, c1) = (from[i].p + from[i].q * b.com_local, to[i].p + to[i].q * b.com_local);
            if (c1 - c0).length_squared() > 1e-10 && bvh.segment(c0, c1).is_some() { through = true; }
            let (depth, share) = self.target_cut(i, &to[i], 3);
            if share > 0.5 { through = true; }
            if depth > b.def.sink + 0.005 { touching = true; }
            depths[i] = depth;
        }
        (through, through || touching, depths)
    }

    /// Depth each body is left in the collider in the frame to come.
    pub fn leave_in(&mut self, depths: &[f32]) {
        for (b, d) in self.bodies.iter_mut().zip(depths) { b.left_in = d.max(0.0); }
    }

    /// Advance one frame, from the capture at `from` to the capture at `to`.
    /// `before` and `after` are the frames around them, for the curve the
    /// steps in between follow. `w0` and `w1` are the collision weight of the
    /// whole character at the two ends: 1 collides, 0 is a ghost.
    pub fn step(&mut self, before: &[Pose], from: &[Pose], to: &[Pose], after: &[Pose], w0: f32, w1: f32) -> FrameStats {
        let n_bodies = self.bodies.len();
        let mut stats = FrameStats { weight: w1, ..Default::default() };
        if n_bodies == 0 || from.len() < n_bodies || to.len() < n_bodies { return stats; }
        let p = self.params.clone();

        // Steps: nothing moves further than max_step in one. Only bodies
        // that have something to run into count: one near the collider, or
        // one that is off its capture already.
        let mut travel = 0.0f32;
        let mut busy = false;
        for (i, b) in self.bodies.iter().enumerate() {
            let turn = from[i].q.angle_between(to[i].q);
            let moved = (to[i].p - from[i].p).length() + turn * b.radius;
            let off = (b.origin() - from[i].p).length() > 1e-4 || b.q.angle_between(from[i].q) > 1e-4;
            let near = w0.max(w1) > 0.0 && self.collider.as_ref().map(|c| c.near_sphere(b.com, b.radius + moved + p.margin + 0.01)).unwrap_or(false);
            if off || near { travel = travel.max(moved); busy = true; }
        }
        // The way back to the capture counts too. A body comes back no
        // faster than it is pushed out, so one that is held against a
        // surface is not thrown at it and caught again in every frame.
        let back_most = p.release_speed / p.fps.max(1.0);
        for (i, b) in self.bodies.iter().enumerate() {
            travel = travel.max(((b.origin() - from[i].p).length() * b.def.follow).min(back_most * if b.def.core { CORE_HOLD } else { 1.0 }));
        }
        let steps = if busy { ((travel / p.max_step.max(1e-4)).ceil() as usize).clamp(1, p.max_substeps.max(1)) } else { 1 };
        stats.substeps = steps as u32;
        let detect_every = steps.div_ceil(2);
        let release = p.release_speed / p.fps.max(1.0) / steps as f32;
        let iterations = p.iterations.max(1);
        let soft = 1.0 - (1.0 - p.soft.clamp(0.0, 0.999)).powf(1.0 / (steps * iterations) as f32);
        // No single step moves a body further than this, so what is found
        // near it before the step is all it can reach in the step.
        let limb_stride = (p.max_step.max(1e-4) * 1.5).min(back_most / steps as f32).max(1e-5);
        let stride = (p.max_step.max(1e-4) * 1.5).min(back_most * CORE_HOLD / steps as f32).max(1e-5);

        let curve = |i: usize, u: f32| -> Pose {
            // Catmull-Rom through the four frames for the position.
            let (p0, p1, p2, p3) = (before[i].p, from[i].p, to[i].p, after[i].p);
            let (u2, u3) = (u * u, u * u * u);
            let pos = 0.5 * ((2.0 * p1) + (p2 - p0) * u + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u3);
            Pose { p: pos, q: from[i].q.slerp(to[i].q, u).normalize() }
        };
        let smooth = before.len() >= n_bodies && after.len() >= n_bodies;
        let at = |i: usize, u: f32| if smooth { curve(i, u) } else { from[i].lerp(&to[i], u) };

        let mut prev: Vec<Pose> = (0..n_bodies).map(|i| at(i, 0.0)).collect();
        let mut target = prev.clone();
        for step in 0..steps {
            let u = (step + 1) as f32 / steps as f32;
            for i in 0..n_bodies { target[i] = at(i, u); }
            let weight = w0 + (w1 - w0) * u;

            // 1. What is near, found where the bodies are now: a state that
            //    is known to be on the right side of every surface.
            if weight > 0.0 && step % detect_every == 0 {
                self.detect(p.margin + stride * 2.0 * detect_every as f32 + 0.004);
            } else if weight <= 0.0 {
                self.contacts.clear();
            }
            for c in self.contacts.iter_mut() {
                let b = &self.bodies[c.body as usize];
                c.start = b.com + b.q * c.local;
            }

            // 2. Carried along with the capture, then drawn back toward it.
            for (i, b) in self.bodies.iter_mut().enumerate() {
                let dq = (target[i].q * prev[i].q.inverse()).normalize();
                let (c0, c1) = (prev[i].p + prev[i].q * b.com_local, target[i].p + target[i].q * b.com_local);
                b.com = c1 + dq * (b.com - c0);
                b.q = (dq * b.q).normalize();
                let k = 1.0 - (1.0 - b.def.follow.clamp(0.0, 1.0)).powf(1.0 / steps as f32);
                let kt = 1.0 - (1.0 - b.def.follow_turn.clamp(0.0, 1.0)).powf(1.0 / steps as f32);
                let origin = b.origin();
                let back = (target[i].p - origin) * k;
                let len = back.length();
                // The trunk holds its place harder than a limb pushes: a leg
                // pressed up from below bends at the knee, it does not lift the hips.
                let most = if b.def.core { stride } else { limb_stride };
                let back = if len > most { back * (most / len) } else { back };
                let angle = b.q.angle_between(target[i].q);
                let turn = (kt * angle).min(most / b.radius.max(0.02));
                if angle > 1e-6 { b.q = b.q.slerp(target[i].q, turn / angle).normalize(); }
                b.com = origin + back + b.q * b.com_local;
            }

            for c in self.contacts.iter_mut() {
                let b = &self.bodies[c.body as usize];
                c.free = c.normal.dot(b.com + b.q * c.local - c.point) + b.def.sink + b.left_in;
            }

            // 3. Joints, limits, contacts, in turn.
            for _ in 0..iterations {
                self.solve_joints(&target);
                self.solve_limits(&target);
                if p.self_collision && weight > 0.0 { self.solve_self(release / iterations as f32 * weight); }
                if weight > 0.0 { self.solve_contacts(release, soft, weight); }
            }
            self.solve_joints(&target);
            std::mem::swap(&mut prev, &mut target);
        }
        let target = to;

        // Anything unusable goes back on the capture.
        for (i, b) in self.bodies.iter_mut().enumerate() {
            let far = (b.origin() - target[i].p).length();
            if !(b.com.is_finite() && b.q.is_finite()) || far > 2.0 {
                b.set_pose(&target[i]);
                stats.resets += 1;
            }
        }

        // Limbs held too far from the capture let go, and take hold again
        // when the capture is clear of the collider.
        let mut hold = vec![false; n_bodies];
        // A limb's distance is counted from where the trunk has taken it.
        let carried = self.bodies[0].origin() - target[0].p;
        for i in 0..n_bodies {
            stats.deviation = stats.deviation.max((self.bodies[i].origin() - target[i].p).length());
            let far = (self.bodies[i].origin() - target[i].p - carried).length();
            if self.bodies[i].def.core { continue; }
            if far > p.limb_limit && !self.bodies[i].released {
                for j in self.below[i].clone() { self.bodies[j].released = true; }
                stats.released += 1;
            }
        }
        for i in 0..n_bodies {
            if !self.bodies[i].released { continue; }
            hold[i] = self.bodies[i].weight > 0.0 || self.target_depth(i, &target[i], 2) > p.ghost_depth * 0.5;
        }
        for i in 0..n_bodies {
            let b = &mut self.bodies[i];
            if !b.released { b.weight = (b.weight + 1.0 / p.fade_in.max(1) as f32).min(1.0); continue; }
            if b.weight > 0.0 { b.weight = (b.weight - 1.0 / p.fade_out.max(1) as f32).max(0.0); }
            else if !hold[i] { b.released = false; }
        }

        stats.contacts = self.contacts.len() as u32;
        for c in &self.contacts {
            let b = &self.bodies[c.body as usize];
            if b.weight < 1.0 || w1 < 1.0 { continue; }
            let s = c.normal.dot(b.com + b.q * c.local - c.point) + b.def.sink + b.left_in;
            stats.residual = stats.residual.max(-s);
        }
        stats
    }

    /// Find, for every sample of every body near the collider, the plane it
    /// has to stay in front of.
    fn detect(&mut self, reach: f32) {
        self.contacts.clear();
        let Some(bvh) = self.collider.clone() else { return };
        let deep = reach.max(0.2);
        // Where each body is judged from: a point known to be on the right
        // side of every surface. For the first body that is its own middle.
        // For the others it is the point by their joint, as long as that can
        // be reached from the parent's without crossing a surface. A body
        // whose joint is already through a surface is judged from its
        // parent's point, so the whole of it is seen to be on the wrong side.
        let mut views: Vec<Vec3> = Vec::with_capacity(self.bodies.len());
        for b in &self.bodies {
            let own = b.origin() + b.q * b.ref_local;
            views.push(match b.def.parent {
                Some(p) if bvh.segment(views[p], own).is_some() => views[p],
                _ => own,
            });
        }
        let views = &views;
        let work = |range: std::ops::Range<usize>| -> Vec<Contact> {
            let mut out = vec![];
            for i in range {
                let b = &self.bodies[i];
                if b.weight <= 0.0 || !bvh.near_sphere(b.com, b.radius + reach) { continue; }
                // A sample is inside the collider when a surface lies between
                // it and the point the body is judged from, or the body's
                // middle if that can be seen from there.
                let from = views[i];
                let middle = bvh.segment(from, b.com).is_none().then_some(b.com);
                let visible = |view: Vec3, p: Vec3| match bvh.segment(view, p) {
                    None => true,
                    Some(h) => (1.0 - h.t) * (p - view).length() < 0.002,
                };
                for s in &b.samples {
                    let x = b.com + b.q * *s;
                    let crossed = bvh.segment(from, x).map(|h| (h, from))
                        .or_else(|| middle.and_then(|m| bvh.segment(m, x).map(|h| (h, m))));
                    if let Some((hit, view)) = crossed {
                        // Out by the shortest way, when that way leads to the
                        // side the body is on. Otherwise back through the
                        // surface that was crossed.
                        let near = bvh.closest(x, deep).filter(|n| n.dist > 1e-5 && visible(view, n.point));
                        let (point, n) = match near {
                            Some(near) => (near.point, (near.point - x) / near.dist),
                            None => (hit.point, if hit.normal.dot(view - hit.point) >= 0.0 { hit.normal } else { -hit.normal }),
                        };
                        out.push(Contact { body: i as u32, local: *s, point, normal: n, start: x, free: 0.0 });
                    } else if let Some(near) = bvh.closest(x, reach) {
                        let d = x - near.point;
                        let n = if near.dist > 1e-5 { d / near.dist }
                                else if near.normal.dot(from - near.point) >= 0.0 { near.normal } else { -near.normal };
                        out.push(Contact { body: i as u32, local: *s, point: near.point, normal: n, start: x, free: 0.0 });
                    }
                }
            }
            out
        };
        let n = self.bodies.len();
        let threads = self.params.threads.clamp(1, n.max(1));
        if threads <= 1 {
            self.contacts = work(0..n);
        } else {
            let per = n.div_ceil(threads);
            let parts: Vec<Vec<Contact>> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..threads).map(|t| {
                    let range = (t * per).min(n)..((t + 1) * per).min(n);
                    let work = &work;
                    scope.spawn(move || work(range))
                }).collect();
                handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
            });
            self.contacts = parts.into_iter().flatten().collect();
        }
    }

    fn solve_contacts(&mut self, release: f32, soft: f32, weight: f32) {
        let (margin, friction) = (self.params.margin, self.params.friction);
        for k in 0..self.contacts.len() {
            let c = self.contacts[k];
            let b = &mut self.bodies[c.body as usize];
            let w = weight * b.weight;
            if w <= 0.0 { continue; }
            let r = b.q * c.local;
            let s = c.normal.dot(b.com + r - c.point) + b.def.sink + b.left_in;
            // A point that was outside when the step began is kept outside.
            // One that was already inside is let out at the release speed,
            // and never let further in.
            let s0 = c.normal.dot(c.start - c.point) + b.def.sink + b.left_in;
            let floor = if s0 >= 0.0 { 0.0 } else { (s0 + release).min(0.0) };
            // At full weight the point is held at that floor. At less, it is
            // let part of the way to where it would go if nothing held it,
            // so a character turning into a ghost sinks in by degrees.
            let floor = if floor > c.free { c.free + (floor - c.free) * w } else { floor };
            let (push, touching) = if s < floor { (floor - s, true) }
                                   else if s >= 0.0 && s < margin { ((margin - s) * soft * w, false) }
                                   else { continue };
            if push <= 0.0 { continue; }
            let lambda = push / b.inv_mass_at(r, c.normal);
            b.push(r, c.normal * lambda);
            if !touching || friction <= 0.0 { continue; }
            // Pressed into the surface: resist sliding along it, up to what
            // the pressure allows.
            let r = b.q * c.local;
            let slid = b.com + r - c.start;
            let along = slid - c.normal * c.normal.dot(slid);
            let len = along.length();
            if len < 1e-7 { continue; }
            let back = (friction * push).min(len) * w;
            let dir = -along / len;
            let lambda = back / b.inv_mass_at(r, dir);
            b.push(r, dir * lambda);
        }
    }

    fn solve_self(&mut self, release: f32) {
        let slack = self.params.self_slack;
        for k in 0..self.pairs.len() {
            let (a, b, allow) = (self.pairs[k].a, self.pairs[k].b, self.pairs[k].allow + slack);
            let w = self.bodies[a].weight.min(self.bodies[b].weight);
            if w <= 0.0 { continue; }
            if (self.bodies[a].com - self.bodies[b].com).length() > self.bodies[a].radius + self.bodies[b].radius { continue; }
            self.push_apart(a, b, allow, release * w);
            self.push_apart(b, a, allow, release * w);
        }
    }

    /// Move samples of `a` that are inside the hull of `b` out of it, both
    /// bodies giving way by their mass.
    fn push_apart(&mut self, a: usize, b: usize, allow: f32, release: f32) {
        for k in 0..self.bodies[a].samples.len() {
            let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
            let inv = bb.q.inverse();
            let ra = ba.q * ba.samples[k];
            let x = ba.com + ra;
            if (x - bb.com).length_squared() > bb.radius * bb.radius { continue; }
            let Some((d, dir)) = bb.def.hull.deeper_than(inv * (x - bb.com) + bb.com_local, allow) else { continue };
            let depth = d - allow;
            let n = bb.q * dir;
            let rb = x - bb.com;
            let lambda = depth.min(release) / (ba.inv_mass_at(ra, n) + bb.inv_mass_at(rb, n));
            self.bodies[a].push(ra, n * lambda);
            self.bodies[b].push(rb, -n * lambda);
        }
    }

    /// Hold each body's joint on its parent.
    fn solve_joints(&mut self, target: &[Pose]) {
        for i in 0..self.bodies.len() {
            let Some(pi) = self.bodies[i].def.parent else { continue };
            // Where the capture has the joint, in the parent's frame.
            let anchor = target[pi].q.inverse() * (target[i].p - target[pi].p);
            let (parent, child) = (&self.bodies[pi], &self.bodies[i]);
            let on_parent = parent.origin() + parent.q * anchor;
            let on_child = child.origin();
            let gap = on_parent - on_child;
            let len = gap.length();
            if len < 1e-7 { continue; }
            let n = gap / len;
            let (rp, rc) = (on_parent - parent.com, on_child - child.com);
            let lambda = len / (parent.inv_mass_at(rp, n) + child.inv_mass_at(rc, n));
            self.bodies[i].push(rc, n * lambda);
            self.bodies[pi].push(rp, -n * lambda);
        }
    }

    /// Keep each joint within its range of the capture.
    fn solve_limits(&mut self, target: &[Pose]) {
        for i in 0..self.bodies.len() {
            let Some(pi) = self.bodies[i].def.parent else { continue };
            let def = &self.bodies[i].def;
            let rel_t = (target[pi].q.inverse() * target[i].q).normalize();
            let rel = (self.bodies[pi].q.inverse() * self.bodies[i].q).normalize();
            let dev = (rel_t.inverse() * rel).normalize();
            let mut rest = dev;
            let mut flex = Quat::IDENTITY;
            if let Some(h) = &def.hinge {
                let (r, f) = swing_twist(dev, h.axis);
                let bend_t = twist_angle((h.reference.inverse() * rel_t).normalize(), h.axis);
                // The range always admits the capture itself.
                let (lo, hi) = ((h.min - bend_t).min(0.0), (h.max - bend_t).max(0.0));
                flex = Quat::from_axis_angle(h.axis, twist_angle(f, h.axis).clamp(lo, hi));
                rest = r;
            }
            let (swing, twist) = swing_twist(rest, def.twist_axis);
            let twist = Quat::from_axis_angle(def.twist_axis, twist_angle(twist, def.twist_axis).clamp(-def.twist, def.twist));
            let swing = clamp_angle(swing, def.swing);
            let mut want = (swing * twist * flex).normalize();
            let mut ranged = false;
            if let Some(r) = &def.range {
                let w = within_range(r, rel_t, want);
                ranged = w.angle_between(want) > 1e-5;
                want = w;
            }
            if want.angle_between(dev) < 1e-5 { continue; }
            // Turn child and parent toward each other, by their inertia.
            let pq = self.bodies[pi].q;
            let fix = (pq * (rel_t * want) * rel.inverse() * pq.inverse()).normalize();
            let (axis, angle) = axis_angle(fix);
            if angle.abs() < 1e-6 { continue; }
            // A wrist or an ankle at the end of its range turns the hand or
            // the foot back, not the forearm or the calf: what pushed the
            // hand there moves the limb through the contact instead.
            let wp = if ranged { 0.0 } else { axis.dot(self.bodies[pi].inv_inertia_world(axis)) };
            let wc = axis.dot(self.bodies[i].inv_inertia_world(axis));
            let total = (wp + wc).max(1e-12);
            // Part of the way in each pass: a joint at its limit and a
            // contact that both insist in full trade places for ever.
            let angle = angle * LIMIT_SHARE;
            self.bodies[i].turn(axis * (angle * wc / total));
            self.bodies[pi].turn(-axis * (angle * wp / total));
        }
    }
}

const LIMIT_SHARE: f32 = 0.5;
/// How many times faster the trunk comes back to its capture than a limb.
const CORE_HOLD: f32 = 3.0;

/// `want` (a change from the capture `rel_t`) cut back so the joint stays
/// within its range, or within the capture where that is further out.
pub fn within_range(r: &Range, rel_t: Quat, want: Quat) -> Quat {
    let side_axis = r.along.cross(r.flex_axis);
    let norm = |v: Vec3| ((v.dot(r.flex_axis) / r.flex.max(1e-3)).powi(2) + (v.dot(side_axis) / r.side.max(1e-3)).powi(2)).sqrt();
    let (sw_t, tw_t) = swing_twist((r.neutral.inverse() * rel_t).normalize(), r.along);
    let room = norm(rotvec(sw_t)).max(1.0);
    let most_twist = r.twist.max(twist_angle(tw_t, r.along).abs());
    let d = (r.neutral.inverse() * rel_t * want).normalize();
    let (sw, tw) = swing_twist(d, r.along);
    let v = rotvec(sw);
    let k = norm(v);
    let t = twist_angle(tw, r.along);
    if k <= room && t.abs() <= most_twist { return want; }
    let sw = if k > room { from_rotvec(v * (room / k)) } else { sw };
    let tw = Quat::from_axis_angle(r.along, t.clamp(-most_twist, most_twist));
    (rel_t.inverse() * r.neutral * sw * tw).normalize()
}

pub fn rotvec(q: Quat) -> Vec3 {
    let q = if q.w < 0.0 { -q } else { q };
    let v = Vec3::new(q.x, q.y, q.z);
    let s = v.length();
    if s < 1e-9 { Vec3::ZERO } else { v / s * 2.0 * s.atan2(q.w) }
}

pub fn from_rotvec(v: Vec3) -> Quat {
    let a = v.length();
    if a < 1e-9 { Quat::IDENTITY } else { Quat::from_axis_angle(v / a, a) }
}

/// Split a rotation into a swing and a twist about `axis`: q = swing * twist.
pub fn swing_twist(q: Quat, axis: Vec3) -> (Quat, Quat) {
    let v = Vec3::new(q.x, q.y, q.z);
    let p = axis * v.dot(axis);
    let twist = Quat::from_xyzw(p.x, p.y, p.z, q.w);
    let twist = if twist.length_squared() < 1e-12 { Quat::IDENTITY } else { twist.normalize() };
    ((q * twist.inverse()).normalize(), twist)
}

/// Signed angle of a rotation about `axis`, in -pi..pi.
pub fn twist_angle(q: Quat, axis: Vec3) -> f32 {
    let s = Vec3::new(q.x, q.y, q.z).dot(axis);
    let a = 2.0 * s.atan2(q.w);
    if a > std::f32::consts::PI { a - std::f32::consts::TAU } else if a < -std::f32::consts::PI { a + std::f32::consts::TAU } else { a }
}

fn axis_angle(q: Quat) -> (Vec3, f32) {
    let q = if q.w < 0.0 { -q } else { q };
    let v = Vec3::new(q.x, q.y, q.z);
    let s = v.length();
    if s < 1e-8 { return (Vec3::X, 0.0); }
    (v / s, 2.0 * s.atan2(q.w))
}

fn clamp_angle(q: Quat, max: f32) -> Quat {
    let (axis, angle) = axis_angle(q);
    if angle <= max { q } else { Quat::from_axis_angle(axis, max) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrist() -> Range {
        Range { neutral: Quat::IDENTITY, along: Vec3::NEG_Y, flex_axis: Vec3::X, flex: 75f32.to_radians(), side: 25f32.to_radians(), twist: 90f32.to_radians() }
    }

    #[test]
    fn a_wrist_bends_forward_freely_and_sideways_a_little() {
        let r = wrist();
        // From neutral: 60 degrees of flexion is within range, 60 to the side is not.
        let flex = Quat::from_rotation_x(60f32.to_radians());
        assert!(within_range(&r, Quat::IDENTITY, flex).angle_between(flex) < 1e-5);
        let side = Quat::from_rotation_z(60f32.to_radians());
        let got = within_range(&r, Quat::IDENTITY, side);
        assert!((got.angle_between(Quat::IDENTITY).to_degrees() - 25.0).abs() < 0.5, "{}", got.angle_between(Quat::IDENTITY).to_degrees());
    }

    #[test]
    fn the_capture_is_never_cut_back_only_what_goes_past_it() {
        let r = wrist();
        // The capture itself 40 degrees to the side: kept, and no further.
        let cap = Quat::from_rotation_z(40f32.to_radians());
        assert!(within_range(&r, cap, Quat::IDENTITY).angle_between(Quat::IDENTITY) < 1e-5);
        let more = Quat::from_rotation_z(20f32.to_radians());
        let got = cap * within_range(&r, cap, more);
        assert!((got.angle_between(Quat::IDENTITY).to_degrees() - 40.0).abs() < 0.5);
    }
}
