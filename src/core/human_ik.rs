//! Human limits: a clip that something else has moved (the collision solve)
//! put back within what a human body can do.
//!
//! Each arm and leg goes through a two-bone IK solve. The wrist or the
//! ankle stays where it was put. The elbow or the knee bends about the one
//! axis it bends about in the capture, the same way round, never past what
//! a human joint allows; where it points (the swivel about the line from
//! shoulder to wrist) stays near where it points in the capture. The hand
//! and the foot keep their turn as far as the wrist and the ankle allow:
//! flexion and extension, and much less from side to side. Twist joints,
//! in line or beside the limb, take their share of any change of twist.
//!
//! A limit never makes a frame worse than the capture: where the capture
//! itself goes past a limit, the limit for that frame is the capture.

use std::sync::Arc;

use bevy::math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::anim::{AnimData, Track};
use super::human::{Human, Limb};

/// What the Body Collide node sets for the human pass.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub on: bool,
    /// Furthest an elbow or a knee may point away from where it points in the capture, degrees.
    pub swivel: f32,
    /// Furthest a hand or a foot may turn away from its capture at the wrist or ankle, degrees.
    pub give: f32,
}

impl Default for Limits {
    fn default() -> Self { Limits { on: true, swivel: 90.0, give: 45.0 } }
}

/// Ranges of a human limb, degrees.
#[derive(Clone, Copy, Debug)]
struct Range {
    /// Most an elbow or a knee bends.
    bend: f32,
    /// Wrist or ankle: flexion either way, and side to side.
    flex: f32,
    side: f32,
    /// Twist of the hand or foot about its length, either way from where
    /// the skin was bound; change of twist of the forearm or calf from the capture.
    end_twist: f32,
    mid_twist: f32,
    /// Turn of the upper bone about its length away from the capture.
    upper_twist: f32,
}

const ARM: Range = Range { bend: 150.0, flex: 75.0, side: 25.0, end_twist: 90.0, mid_twist: 30.0, upper_twist: 50.0 };
const LEG: Range = Range { bend: 155.0, flex: 50.0, side: 25.0, end_twist: 30.0, mid_twist: 15.0, upper_twist: 35.0 };

/// A limb, measured once for the whole clip.
struct Prep {
    limb:    Limb,
    range:   Range,
    /// Axis the elbow or knee bends about, in the frame of the upper bone,
    /// pointing so that bending is a turn the positive way about it.
    hinge:   Option<Vec3>,
    /// Hand or foot: its long axis and its flexion axis, in its own frame,
    /// and its turn relative to the forearm or calf where the skin was bound.
    end_axis: Vec3,
    flex_axis: Option<Vec3>,
    neutral: Quat,
    /// A direction square to the lower bone, in its frame, that the hinge
    /// points along where the skin was bound: twist is counted from it.
    twist_ref: Option<Vec3>,
}

fn rot(m: &Mat4) -> Quat { m.to_scale_rotation_translation().1.normalize() }
fn pos(m: &Mat4) -> Vec3 { m.w_axis.truncate() }

fn swing_twist(q: Quat, axis: Vec3) -> (Quat, Quat) {
    let v = Vec3::new(q.x, q.y, q.z);
    let p = axis * v.dot(axis);
    let twist = Quat::from_xyzw(p.x, p.y, p.z, q.w);
    let twist = if twist.length_squared() < 1e-12 { Quat::IDENTITY } else { twist.normalize() };
    ((q * twist.inverse()).normalize(), twist)
}

fn twist_angle(q: Quat, axis: Vec3) -> f32 {
    let a = 2.0 * Vec3::new(q.x, q.y, q.z).dot(axis).atan2(q.w);
    wrap(a)
}

fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let a = a % TAU;
    if a > PI { a - TAU } else if a < -PI { a + TAU } else { a }
}

fn rotvec(q: Quat) -> Vec3 {
    let q = if q.w < 0.0 { -q } else { q };
    let v = Vec3::new(q.x, q.y, q.z);
    let s = v.length();
    if s < 1e-9 { Vec3::ZERO } else { v / s * 2.0 * s.atan2(q.w) }
}

fn from_rotvec(v: Vec3) -> Quat {
    let a = v.length();
    if a < 1e-9 { Quat::IDENTITY } else { Quat::from_axis_angle(v / a, a) }
}

/// Signed angle from a to b about axis, all square to the axis.
fn angle_about(a: Vec3, b: Vec3, axis: Vec3) -> f32 { a.cross(b).dot(axis).atan2(a.dot(b)) }

fn perp(v: Vec3, axis: Vec3) -> Vec3 { v - axis * axis.dot(v) }

fn smooth(a: f32, b: f32, x: f32) -> f32 { let t = ((x - a) / (b - a)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// Where the skin was bound, else the rest pose.
fn neutral_world(clip: &AnimData) -> Vec<Mat4> {
    match &clip.skin {
        Some(s) if s.bind.len() == clip.joints.len() => s.bind.clone(),
        _ => clip.rest_world(),
    }
}

/// Fingers, by name: the bases of the index and little fingers, and of all
/// of them.
fn finger_bases(clip: &AnimData, hand: usize) -> (Option<usize>, Option<usize>, Vec<usize>) {
    let n = clip.joints.len();
    let under = |j: usize, top: usize| { let mut c = Some(j); let mut d = 0; while let Some(x) = c { if x == top { return Some(d); } c = clip.joints[x].parent; d += 1; if d > 3 { break; } } None };
    let (mut index, mut little, mut all) = (None::<(usize, usize)>, None::<(usize, usize)>, vec![]);
    for j in 0..n {
        let Some(d) = under(j, hand).filter(|d| *d >= 1) else { continue };
        let w = super::human::words(&clip.joints[j].name);
        let has = |s: &str| w.iter().any(|x| x == s || x.starts_with(s));
        let finger = has("index") || has("middle") || has("ring") || has("pinky") || has("little") || (has("finger") && !w.iter().any(|x| x == "0"));
        if !finger || has("thumb") || w.iter().any(|x| matches!(x.as_str(), "bulge" | "side" | "half" | "in" | "out" | "inn" | "palm" | "mcp" | "pip" | "dip" | "slide" | "end" | "nub")) { continue; }
        let digit = w.iter().find(|x| x.chars().all(|c| c.is_ascii_digit())).cloned().unwrap_or_default();
        let is_index = has("index") || (has("finger") && digit.starts_with('1'));
        let is_little = has("pinky") || has("little") || (has("finger") && digit.starts_with('4'));
        if is_index && index.map(|(_, dd)| d < dd).unwrap_or(true) { index = Some((j, d)); }
        if is_little && little.map(|(_, dd)| d < dd).unwrap_or(true) { little = Some((j, d)); }
        if d <= 2 { all.push(j); }
    }
    (index.map(|x| x.0), little.map(|x| x.0), all)
}

impl Prep {
    fn new(capture: &AnimData, limb: &Limb) -> Option<Prep> {
        let (u, m, e) = (limb.upper?, limb.mid?, limb.end?);
        let range = if limb.arm { ARM } else { LEG };
        // The hinge: the normal of the plane of the two bones, in the frame
        // of the upper one, over the frames where the joint is well bent.
        let frames = capture.frames.max(1);
        let count = frames.min(600);
        let mut sum = Vec3::ZERO;
        let mut weight = 0.0f32;
        for k in 0..count {
            let w = capture.world_pose(k * (frames - 1).max(1) / (count - 1).max(1));
            let (s, el, wr) = (pos(&w[u]), pos(&w[m]), pos(&w[e]));
            let c = (el - s).normalize_or_zero().cross((wr - el).normalize_or_zero());
            let l = c.length();
            if l > 20f32.to_radians().sin() { sum += rot(&w[u]).inverse() * c / l * l; weight += l; }
        }
        let neutral = neutral_world(capture);
        let hinge = if weight > 0.5 && sum.length() > 0.5 * weight {
            Some(sum.normalize())
        } else {
            // Hardly bent in the clip: as it is bent where the skin was bound.
            let (s, el, wr) = (pos(&neutral[u]), pos(&neutral[m]), pos(&neutral[e]));
            let c = (el - s).normalize_or_zero().cross((wr - el).normalize_or_zero());
            (c.length() > 5f32.to_radians().sin()).then(|| (rot(&neutral[u]).inverse() * c).normalize())
        };
        let (ql, qe) = (rot(&neutral[m]), rot(&neutral[e]));
        let along_mid = (pos(&neutral[e]) - pos(&neutral[m])).normalize_or_zero();
        // Long axis of the hand or foot: to the fingers or the toes.
        let mut end_axis = qe.inverse() * along_mid;
        let mut flex_axis = None;
        if limb.arm {
            let (index, little, all) = finger_bases(capture, e);
            if !all.is_empty() {
                let mid = all.iter().map(|j| pos(&neutral[*j])).sum::<Vec3>() / all.len() as f32;
                let d = qe.inverse() * (mid - pos(&neutral[e]));
                if d.length() > 1e-4 { end_axis = d.normalize(); }
            }
            if let (Some(i), Some(l)) = (index, little) {
                let lat = perp(qe.inverse() * (pos(&neutral[i]) - pos(&neutral[l])), end_axis);
                if lat.length() > 1e-4 { flex_axis = Some(lat.normalize()); }
            }
        } else if let Some(t) = limb.tip {
            let d = qe.inverse() * (pos(&neutral[t]) - pos(&neutral[e]));
            if d.length() > 1e-4 { end_axis = d.normalize(); }
        }
        // Else the wrist and ankle bend about the axis the elbow and knee do.
        if flex_axis.is_none() {
            if let Some(h) = hinge {
                let a = perp(qe.inverse() * (rot(&neutral[u]) * h), end_axis);
                if a.length() > 0.2 { flex_axis = Some(a.normalize()); }
            }
        }
        if !end_axis.is_finite() || end_axis.length() < 0.5 { end_axis = Vec3::Y; }
        let twist_ref = hinge.and_then(|h| {
            let qu = rot(&neutral[u]);
            let b_u = qu.inverse() * (pos(&neutral[m]) - pos(&neutral[u])).normalize_or_zero();
            let a = perp(h, b_u).normalize_or_zero();
            let b_l = ql.inverse() * along_mid;
            let k = perp(ql.inverse() * (qu * a), b_l);
            (k.length() > 0.1).then(|| k.normalize())
        });
        Some(Prep { limb: limb.clone(), range, hinge, end_axis, flex_axis, neutral: (ql.inverse() * qe).normalize(), twist_ref })
    }
}

/// The range of each wrist and ankle, for the collision solve: the end
/// joint, the joint above it, and the range in the solver's terms.
pub fn end_ranges(capture: &AnimData, human: &Human) -> Vec<(usize, usize, xms_ragdoll::Range)> {
    human.limbs.iter().filter_map(|l| {
        let p = Prep::new(capture, l)?;
        let flex_axis = p.flex_axis?;
        Some((l.end?, l.mid?, xms_ragdoll::Range { neutral: p.neutral, along: p.end_axis, flex_axis, flex: p.range.flex.to_radians(), side: p.range.side.to_radians(), twist: p.range.end_twist.to_radians() }))
    }).collect()
}

/// What the pass did, for the panel and the tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    pub frames: usize,
    /// Frames in which some elbow or knee bent the wrong way, before and after.
    pub flips_before: usize,
    pub flips_after:  usize,
    /// Frames in which some wrist or ankle was past its range, before and after.
    pub ends_before: usize,
    pub ends_after:  usize,
    /// Furthest a wrist or ankle was moved, metres.
    pub moved_end: f32,
    /// Frames in which a joint was sent round for its roll, or for its swivel.
    pub roll_frames: usize,
    pub swivel_frames: usize,
}

/// `moved` is `capture` after something changed its rotations. The result
/// is `moved` within human limits.
///
/// `blocked`, when given, says whether a segment crosses the set: an elbow
/// or knee is not sent through it.
pub fn humanize(capture: &AnimData, moved: &AnimData, human: &Human, limits: &Limits, blocked: Option<&(dyn Fn(Vec3, Vec3) -> bool + Sync)>) -> (AnimData, Report) {
    let mut report = Report { frames: moved.frames.max(1), ..Default::default() };
    if !limits.on || capture.joints.len() != moved.joints.len() { return (moved.clone(), report); }
    let preps: Vec<Prep> = human.limbs.iter().filter_map(|l| Prep::new(capture, l)).collect();
    if preps.is_empty() { return (moved.clone(), report); }
    let n = moved.joints.len();
    let frames = moved.frames.max(1).min(capture.frames.max(1));
    let mut tracks: Vec<Track> = (*moved.tracks).clone();
    // Joints the pass may write get a sample per frame.
    for p in &preps {
        let l = &p.limb;
        let mut js: Vec<usize> = [l.upper, l.mid, l.end].into_iter().flatten().collect();
        js.extend(l.upper_twist.iter().chain(&l.mid_twist).map(|t| t.joint));
        for j in js { if tracks[j].len() < frames { tracks[j] = (0..frames).map(|f| moved.local(j, f)).collect(); } }
    }
    let mut human = human.clone();
    human.place_twists(&neutral_world(capture));
    let twists_of = |p: &Prep| -> (Vec<super::human::Twist>, Vec<super::human::Twist>) {
        let l = human.limbs.iter().find(|x| x.arm == p.limb.arm && x.left == p.limb.left).unwrap();
        (l.upper_twist.clone(), l.mid_twist.clone())
    };
    let twists: Vec<_> = preps.iter().map(twists_of).collect();

    // Where each joint was last sent round to, so the next frame starts there.
    let mut last_phi = vec![0.0f32; preps.len()];
    for f in 0..frames {
        let wc = capture.world_pose(f);
        let mut w = moved.world_pose(f);
        let (mut flipped_before, mut flipped_after, mut end_before, mut end_after) = (false, false, false, false);
        let (mut roll_hit, mut swivel_hit) = (false, false);
        for (pi, (p, (upper_tw, mid_tw))) in preps.iter().zip(&twists).enumerate() {
            let (u, m, e) = (p.limb.upper.unwrap(), p.limb.mid.unwrap(), p.limb.end.unwrap());
            let r = p.range;
            let parent_rot = |w: &[Mat4], j: usize| capture.joints[j].parent.filter(|x| *x < n).map(|x| rot(&w[x])).unwrap_or(Quat::IDENTITY);
            let (s, el, wr) = (pos(&w[u]), pos(&w[m]), pos(&w[e]));
            let (sc, ec, wcp) = (pos(&wc[u]), pos(&wc[m]), pos(&wc[e]));
            let (qu, ql, qe) = (rot(&w[u]), rot(&w[m]), rot(&w[e]));
            let (quc, qlc, qec) = (rot(&wc[u]), rot(&wc[m]), rot(&wc[e]));
            let (qp, qpc) = (parent_rot(&w, u), parent_rot(&wc, u));
            let (l1, l2) = ((el - s).length(), (wr - el).length());
            if l1 < 1e-5 || l2 < 1e-5 { continue; }
            let b_u = qu.inverse() * (el - s) / l1;
            let b_l = ql.inverse() * (wr - el) / l2;

            // ── Elbow or knee ────────────────────────────────────────────
            // Three corrections, each nothing while its limit holds:
            // the swivel, the roll of the upper bone against the plane the
            // joint bends in, the twist of the lower bone. The wrist or
            // ankle stays where it is through all three.
            let mut qu_new = qu;
            let mut ql_new = ql;
            if let (Some(hinge), Some(k)) = (p.hinge, p.twist_ref) {
                let a = perp(hinge, b_u).normalize_or_zero();
                let k = perp(k, b_l).normalize_or_zero();
                let bend_of = |s: Vec3, e: Vec3, w: Vec3| (e - s).angle_between(w - e);
                let (bend, bend_c) = (bend_of(s, el, wr), bend_of(sc, ec, wcp));
                // How much the bend plane means: nothing for a straight limb.
                let plane = |b: f32| smooth(6f32.to_radians(), 14f32.to_radians(), b);
                let (w_s, w_c) = (plane(bend), plane(bend_c));
                let dir = (wr - s).normalize_or_zero();
                let carry = qp * qpc.inverse();
                // Roll of the upper bone against its bend plane.
                let roll_of = |s: Vec3, e: Vec3, w: Vec3, q: Quat| {
                    let (d1, d2) = ((e - s).normalize_or_zero(), (w - e).normalize_or_zero());
                    angle_about(d1.cross(d2).normalize_or_zero(), perp(q * a, d1).normalize_or_zero(), d1)
                };
                if roll_of(s, el, wr, qu).abs() > 90f32.to_radians() && bend > 15f32.to_radians() { flipped_before = true; }

                let most = limits.swivel.max(0.0).to_radians();
                let excess = |x: f32, m: f32| if x > m { x - m } else if x < -m { x + m } else { 0.0 };
                // How far the lower bone leaves the plane its hinge allows, in
                // the capture: a carrying angle, give or take.
                let off_plane_c = (quc * a).dot((wcp - ec).normalize_or_zero()).clamp(-1.0, 1.0).asin();
                let plane_give = 15f32.to_radians();
                // The upper bone's roll against the bend plane, put back to the
                // nearest roll at which the lower bone stays that near its
                // hinge plane and bends forward. What it takes.
                let roll_c = roll_of(sc, ec, wcp, quc);
                let roll_fix = |roll: f32| -> f32 {
                    let sb = bend.sin().abs();
                    // Never tighter than the capture itself.
                    let lim = 85f32.to_radians().max(if w_c > 0.0 { roll_c.abs() + 0.02 } else { 0.0 });
                    let (lo, hi) = if sb < 1e-3 { (-lim, lim) } else {
                        let x = (-(off_plane_c + plane_give).sin() / sb).clamp(-1.0, 1.0).asin();
                        let y = (-(off_plane_c - plane_give).sin() / sb).clamp(-1.0, 1.0).asin();
                        (x.min(y).max(-lim), x.max(y).min(lim))
                    };
                    let r = wrap(roll);
                    r - r.clamp(lo, hi.max(lo))
                };
                // The capture's swivel, carried from its own line to the wrist
                // over to this one.
                let dir_c = (carry * (wcp - sc)).normalize_or_zero();
                let pole_c = perp(carry * (ec - sc), dir_c);
                let pole_ref = if dir_c == Vec3::ZERO || dir == Vec3::ZERO { Vec3::ZERO }
                    else { perp(Quat::from_rotation_arc(dir_c, dir) * pole_c, dir).normalize_or_zero() };
                let swivel_off = |pole: Vec3| if w_c > 0.0 && pole_ref != Vec3::ZERO { excess(angle_about(pole_ref, pole, dir), most) } else { 0.0 };
                let pole_s = perp(el - s, dir).normalize_or_zero();
                let roll_s = roll_of(s, el, wr, qu);
                let wrong_roll = w_s * roll_fix(roll_s).abs();
                let wrong_swivel = w_s * w_c * swivel_off(pole_s).abs();
                if wrong_roll > 1e-4 { roll_hit = true; }
                let wrong = wrong_roll + wrong_swivel;

                // 1. Bent the wrong way, or pointing far from the capture.
                //    The shoulder or hip turns the upper bone about its length
                //    until the joint bends about its hinge, as far as it may
                //    turn from the capture; past that the joint goes round the
                //    line to the wrist (which stays put). Least change wins.
                let upper_most = r.upper_twist.to_radians();
                let qu_ref = (carry * quc).normalize();
                if wrong > 1e-4 && pole_s != Vec3::ZERO && dir != Vec3::ZERO {
                    let dist = (wr - s).length();
                    let cos_a = ((l1 * l1 + dist * dist - l2 * l2) / (2.0 * l1 * dist.max(1e-6))).clamp(-1.0, 1.0);
                    let sin_a = (1.0 - cos_a * cos_a).sqrt();
                    // The upper bone swung to the joint's new place, then
                    // turned about its length as far as the roll needs and
                    // the shoulder or hip allows. What it could not turn is left.
                    let at = |phi: f32| {
                        let pole = Quat::from_axis_angle(dir, phi) * pole_s;
                        let d1 = (dir * cos_a + pole * sin_a).normalize();
                        let q = (Quat::from_rotation_arc((qu * b_u).normalize(), d1) * qu).normalize();
                        let roll = roll_of(s, s + d1 * l1, wr, q);
                        let full = w_s * roll_fix(roll);
                        let turned = |t: f32| (Quat::from_axis_angle(d1, -full * t) * q).normalize();
                        let twist_of = |q: Quat| twist_angle((qu_ref.inverse() * q).normalize(), b_u);
                        let budget = upper_most.max(twist_of(q).abs());
                        let mut t = 1.0f32;
                        if twist_of(turned(1.0)).abs() > budget {
                            let (mut lo, mut hi) = (0.0f32, 1.0f32);
                            for _ in 0..10 { let m = 0.5 * (lo + hi); if twist_of(turned(m)).abs() > budget { hi = m; } else { lo = m; } }
                            t = lo;
                        }
                        let q = turned(t);
                        (pole, d1, q, full * t, full * (1.0 - t))
                    };
                    let prev = last_phi[pi];
                    let cost = |phi: f32| {
                        let (pole, _, _, fix, left) = at(phi);
                        10.0 * left * left + (w_c * swivel_off(pole)).powi(2)
                            + 0.05 * phi * phi + 0.02 * (phi - prev).powi(2) + 0.01 * fix * fix
                    };
                    // Through the set is no way: the joint, or a bone that was
                    // clear of it, must not cross a surface to get there.
                    let clear = |phi: f32| match blocked {
                        None => true,
                        Some(b) => {
                            let (_, d1, _, _, _) = at(phi);
                            let e = s + d1 * l1;
                            !(b(el, e) || (b(s, e) && !b(s, el)) || (b(e, wr) && !b(el, wr)))
                        }
                    };
                    let mut ranked: Vec<(f32, f32)> = (-90..=90).map(|k| { let phi = (k as f32 * 2.0).to_radians(); (cost(phi), phi) }).collect();
                    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut best = (cost(0.0), 0.0f32);
                    // Not round to a place worse than staying put.
                    if let Some(c) = ranked.iter().filter(|c| c.0 <= best.0).take(24).find(|c| clear(c.1)) {
                        best = *c;
                        for k in -10..=10 {
                            let phi = c.1 + (k as f32 * 0.2).to_radians();
                            let v = cost(phi);
                            if v < best.0 && clear(phi) { best = (v, phi); }
                        }
                    }
                    if best.1.abs() > 1e-4 { swivel_hit = true; }
                    last_phi[pi] = best.1;
                    let (_, d1, q, _, _) = at(best.1);
                    qu_new = q;
                    let d2 = (wr - (s + d1 * l1)).normalize_or_zero();
                    if d2 != Vec3::ZERO { ql_new = (Quat::from_rotation_arc((ql * b_l).normalize(), d2) * ql).normalize(); }
                } else {
                    last_phi[pi] = 0.0;
                }

                // 2. Straight, the bend plane says nothing: the upper bone's
                //    roll stays near the capture's instead.
                let d1 = (qu_new * b_u).normalize();
                let off_roll = angle_about(perp(carry * quc * a, d1).normalize_or_zero(), perp(qu_new * a, d1).normalize_or_zero(), d1);
                let turn = (1.0 - w_s) * excess(off_roll, most);
                if turn.abs() > 1e-6 && dir != Vec3::ZERO {
                    let r = Quat::from_axis_angle(dir, -turn);
                    qu_new = (r * qu_new).normalize();
                    ql_new = (r * ql_new).normalize();
                }
                let el1 = s + qu_new * b_u * l1;

                // 3. Twist of the lower bone about its length, against the hinge.
                let twist_of = |e: Vec3, w: Vec3, qu: Quat, ql: Quat| {
                    let d2 = (w - e).normalize_or_zero();
                    angle_about(perp(qu * a, d2).normalize_or_zero(), perp(ql * k, d2).normalize_or_zero(), d2)
                };
                let tw = twist_of(el1, wr, qu_new, ql_new);
                let tw_c = twist_of(ec, wcp, quc, qlc);
                let fix = excess(wrap(tw - tw_c), r.mid_twist.to_radians());
                let d2 = (wr - el1).normalize_or_zero();
                if fix.abs() > 1e-6 && d2 != Vec3::ZERO { ql_new = (Quat::from_axis_angle(d2, -fix) * ql_new).normalize(); }

                // 4. No further than a human joint bends: the lower bone
                //    turns back about the hinge, and the wrist with it.
                let bend_max = r.bend.to_radians().max(bend_c + 0.02);
                let bend_now = (qu_new * b_u).angle_between(ql_new * b_l);
                if bend_now > bend_max {
                    let axis = (qu_new * b_u).cross(ql_new * b_l).normalize_or_zero();
                    if axis != Vec3::ZERO { ql_new = (Quat::from_axis_angle(axis, bend_max - bend_now) * ql_new).normalize(); }
                }
            }
            // ── Wrist or ankle ────────────────────────────────────────────
            let rel = ql_new.inverse() * qe;
            let rel_c = qlc.inverse() * qec;
            let dev = (p.neutral.inverse() * rel).normalize();
            let dev_c = (p.neutral.inverse() * rel_c).normalize();
            let ax = p.end_axis;
            let (sw, tw) = swing_twist(dev, ax);
            let (sw_c, tw_c) = swing_twist(dev_c, ax);
            let (tau, tau_c) = (twist_angle(tw, ax), twist_angle(tw_c, ax));
            // Twist about the hand's or foot's length: within its range from
            // where the skin was bound (the capture's, where that is further),
            // and within the give of the capture.
            let give = limits.give.max(0.0).to_radians();
            let most_twist = r.end_twist.to_radians().max(tau_c.abs());
            let tau = (tau_c + wrap(tau - tau_c).clamp(-give, give)).clamp(-most_twist, most_twist);
            let (mut v, v_c) = (rotvec(sw), rotvec(sw_c));
            let norm = |v: Vec3| match p.flex_axis {
                Some(fx) => { let g = ax.cross(fx); ((v.dot(fx) / r.flex.to_radians()).powi(2) + (v.dot(g) / r.side.to_radians()).powi(2)).sqrt() }
                None => v.length() / 60f32.to_radians(),
            };
            let room = norm(v_c).max(1.0);
            if norm(v) > room * 1.0005 { end_before = true; }
            for _ in 0..4 {
                let off = v - v_c;
                if off.length() > give { v = v_c + off * (give / off.length()); }
                let k = norm(v);
                if k > room { v *= room / k; }
            }
            if norm(v) > room * 1.0005 { end_after = true; }
            let rel_new = p.neutral * from_rotvec(v) * Quat::from_axis_angle(ax, tau);
            let qe_new = (ql_new * rel_new).normalize();

            // ── Written back, joint by joint down the limb ───────────────
            let set = |w: &mut Vec<Mat4>, tracks: &mut Vec<Track>, j: usize, world_rot: Quat| {
                let parent = capture.joints[j].parent.filter(|x| *x < n).map(|x| w[x]).unwrap_or(Mat4::IDENTITY);
                let mut local = tracks[j][f];
                local.rotation = (rot(&parent).inverse() * world_rot).normalize();
                tracks[j][f] = local;
                w[j] = parent * local.compute_matrix();
            };
            let refresh = |w: &mut Vec<Mat4>, tracks: &Vec<Track>, from: usize, to: usize| {
                // Joints between `from` and `to`, top down, from their samples.
                let mut path = vec![];
                let mut c = capture.joints[to].parent;
                while let Some(x) = c { if x == from { break; } path.push(x); c = capture.joints[x].parent; }
                for j in path.into_iter().rev() {
                    let parent = capture.joints[j].parent.map(|x| w[x]).unwrap_or(Mat4::IDENTITY);
                    let local = tracks[j].get(f).copied().unwrap_or_else(|| moved.local(j, f));
                    w[j] = parent * local.compute_matrix();
                }
            };
            let refresh_one = |w: &mut Vec<Mat4>, tracks: &Vec<Track>, j: usize| {
                let parent = capture.joints[j].parent.map(|x| w[x]).unwrap_or(Mat4::IDENTITY);
                let local = tracks[j].get(f).copied().unwrap_or_else(|| moved.local(j, f));
                w[j] = parent * local.compute_matrix();
            };
            // Twist about an axis given in the world, put on a joint's own turn.
            let twist_joint = |w: &mut Vec<Mat4>, tracks: &mut Vec<Track>, j: usize, axis_world: Vec3, angle: f32| {
                if angle.abs() < 1e-6 { return; }
                let axis = (rot(&w[j]).inverse() * axis_world).normalize_or_zero();
                if axis == Vec3::ZERO { return; }
                let mut local = tracks[j][f];
                local.rotation = (local.rotation * Quat::from_axis_angle(axis, angle)).normalize();
                tracks[j][f] = local;
            };

            set(&mut w, &mut tracks, u, qu_new);
            // Upper twist joints: the end at the shoulder or hip keeps the
            // turn it had, the share toward the elbow or knee goes with the bone.
            let rel_old = (qpc.inverse() * quc).normalize();
            let rel_now = (parent_rot(&w, u).inverse() * qu_new).normalize();
            let delta_u = twist_angle((rel_old.inverse() * rel_now).normalize(), b_u);
            for t in upper_tw {
                if t.in_line { refresh(&mut w, &tracks, u, t.joint); }
                refresh_one(&mut w, &tracks, t.joint);
                twist_joint(&mut w, &mut tracks, t.joint, qu_new * b_u, -(1.0 - t.at) * delta_u);
                refresh_one(&mut w, &tracks, t.joint);
            }
            refresh(&mut w, &tracks, u, m);
            set(&mut w, &mut tracks, m, ql_new);
            // Lower twist joints follow the hand or foot by their place.
            let delta_e = twist_angle((rel_c.inverse() * rel_new).normalize(), (rel_new.inverse() * b_l).normalize_or_zero());
            for t in mid_tw {
                if t.in_line { refresh(&mut w, &tracks, m, t.joint); }
                refresh_one(&mut w, &tracks, t.joint);
                twist_joint(&mut w, &mut tracks, t.joint, ql_new * b_l, t.at * delta_e);
                refresh_one(&mut w, &tracks, t.joint);
            }
            refresh(&mut w, &tracks, m, e);
            set(&mut w, &mut tracks, e, qe_new);
            report.moved_end = report.moved_end.max((pos(&w[e]) - wr).length());
            if let Some(hinge) = p.hinge {
                let (s2, e2, w2) = (pos(&w[u]), pos(&w[m]), pos(&w[e]));
                let b2 = rot(&w[u]).inverse() * (e2 - s2).normalize_or_zero();
                let a = perp(hinge, b2).normalize_or_zero();
                let (d1, d2) = ((e2 - s2).normalize_or_zero(), (w2 - e2).normalize_or_zero());
                let roll = angle_about(d1.cross(d2).normalize_or_zero(), perp(rot(&w[u]) * a, d1).normalize_or_zero(), d1);
                if d1.angle_between(d2) > 15f32.to_radians() && roll.abs() > 90f32.to_radians() { flipped_after = true; }
            }
        }
        report.flips_before += flipped_before as usize;
        report.flips_after += flipped_after as usize;
        report.ends_before += end_before as usize;
        report.ends_after += end_after as usize;
        report.roll_frames += roll_hit as usize;
        report.swivel_frames += swivel_hit as usize;
    }
    (AnimData { tracks: Arc::new(tracks), ..moved.clone() }, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::{create_test_clip, FrameRate};

    fn walk() -> AnimData { create_test_clip(2.0, FrameRate::new(30, 1)) }

    fn idx(c: &AnimData, name: &str) -> usize { c.joints.iter().position(|j| j.name.ends_with(name)).unwrap() }

    #[test]
    fn a_clip_left_alone_comes_back_as_it_was() {
        let c = walk();
        let h = Human::of(&c);
        let (out, report) = humanize(&c, &c, &h, &Limits::default(), None);
        assert_eq!(report.flips_before, 0);
        assert_eq!(report.flips_after, 0);
        for f in [0, 13, 40] {
            let (a, b) = (c.world_pose(f), out.world_pose(f));
            for j in 0..c.joints.len() {
                assert!((a[j].w_axis - b[j].w_axis).length() < 1e-4, "frame {f} joint {}", c.joints[j].name);
                assert!(rot(&a[j]).angle_between(rot(&b[j])) < 1e-3, "frame {f} joint {}", c.joints[j].name);
            }
        }
    }

    /// A forearm turned the wrong way about the elbow, and a hand bent 90
    /// degrees sideways: both come back within range, the wrist stays put.
    #[test]
    fn an_elbow_bent_backward_and_a_wrist_bent_sideways_are_put_right() {
        let c = walk();
        let h = Human::of(&c);
        let (fore, hand) = (idx(&c, "LeftForeArm"), idx(&c, "LeftHand"));
        let mut tracks = (*c.tracks).clone();
        for f in 0..c.frames {
            // The walk bends the elbow about -X. Bend it about +X instead.
            let r = tracks[fore][f].rotation;
            tracks[fore][f].rotation = Quat::from_rotation_x(-2.0 * r.to_axis_angle().1 * r.x.signum()) * r;
            // And the hand 90 degrees about the bone's own side axis (Z).
            tracks[hand][f].rotation = Quat::from_rotation_z(90f32.to_radians()) * tracks[hand][f].rotation;
        }
        let bad = AnimData { tracks: Arc::new(tracks), ..c.clone() };
        let (out, report) = humanize(&c, &bad, &h, &Limits::default(), None);
        assert!(report.flips_before > 0, "{report:?}");
        assert_eq!(report.flips_after, 0, "{report:?}");
        assert!(report.ends_before > 0 && report.ends_after == 0, "{report:?}");
        let upper = idx(&c, "LeftArm");
        for f in 0..c.frames {
            let (b, o) = (bad.world_pose(f), out.world_pose(f));
            // The wrist is where the bad clip put it (it can be reached).
            assert!((pos(&b[hand]) - pos(&o[hand])).length() < 2e-3, "frame {f}");
            // Bones keep their length.
            assert!(((pos(&o[fore]) - pos(&o[upper])).length() - 0.28).abs() < 1e-4);
            // The hand is within 45 degrees of the forearm's line.
            let hand_dir = rot(&o[hand]) * Vec3::NEG_Y;
            let fore_dir = (pos(&o[hand]) - pos(&o[fore])).normalize();
            assert!(hand_dir.angle_between(fore_dir) < 45f32.to_radians(), "frame {f}: {}", hand_dir.angle_between(fore_dir).to_degrees());
        }
    }

    #[test]
    fn off_gives_the_clip_back_untouched() {
        let c = walk();
        let (out, _) = humanize(&c, &c, &Human::of(&c), &Limits { on: false, ..Default::default() }, None);
        assert!(Arc::ptr_eq(&out.tracks, &c.tracks));
    }
}
