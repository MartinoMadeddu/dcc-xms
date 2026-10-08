//! Clip tools for motion capture work: mirror, smooth, in place, transform,
//! blend, loop, retarget, time warp, prune and floor.
//!
//! They follow what MotionBuilder, Houdini KineFX and Maya HumanIK are most
//! used for, reduced to operators on a baked clip. Each one returns a new
//! clip and leaves its input alone.

use std::sync::Arc;

use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::Transform;

use super::anim::{AnimData, Joint, Track};

fn lerp(a: &Transform, b: &Transform, t: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation:    a.rotation.slerp(b.rotation, t),
        scale:       a.scale.lerp(b.scale, t),
    }
}

fn smoothstep(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// Reflect a world transform in the plane x = 0.
fn reflect_rotation(q: Quat) -> Quat { Quat::from_xyzw(q.x, -q.y, -q.z, q.w) }
fn reflect_point(p: Vec3) -> Vec3 { Vec3::new(-p.x, p.y, p.z) }

/// Joint name without its namespace or the prefix shared by the skeleton.
fn short_name<'a>(name: &'a str, prefix: &str) -> &'a str {
    let n = name.strip_prefix(prefix).unwrap_or(name);
    n.rsplit(':').next().unwrap_or(n)
}

/// Name as used to match joints between skeletons: lower case, letters and
/// digits only.
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// The name of the joint on the other side, if this is a left or right joint.
pub fn other_side(name: &str) -> Option<String> {
    for (a, b) in [("Left", "Right"), ("left", "right"), ("LEFT", "RIGHT"), ("L_", "R_"), ("_L", "_R"), ("_l", "_r"), (".L", ".R"), (".l", ".r")] {
        for (from, to) in [(a, b), (b, a)] {
            let hit = match from {
                // Short markers only count at the start or the end of the name.
                "L_" | "R_" => name.rsplit(':').next().map(|n| n.starts_with(from)).unwrap_or(false)
                    .then(|| name.rfind(from)).flatten().filter(|i| *i == name.len() - name.rsplit(':').next().unwrap().len()),
                "_L" | "_R" | "_l" | "_r" | ".L" | ".R" | ".l" | ".r" => name.ends_with(from).then(|| name.len() - from.len()),
                _ => name.find(from),
            };
            if let Some(i) = hit {
                return Some(format!("{}{}{}", &name[..i], to, &name[i + from.len()..]));
            }
        }
    }
    None
}

impl AnimData {
    fn frame_count(&self) -> usize { self.frames.max(1) }

    /// Every track written out in full, one sample per frame.
    fn dense_tracks(&self) -> Vec<Track> {
        let n = self.frame_count();
        (0..self.joints.len()).map(|j| (0..n).map(|i| self.local(j, i)).collect()).collect()
    }

    /// World matrices of the rest pose.
    pub fn rest_world(&self) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.joints.len());
        for (j, joint) in self.joints.iter().enumerate() {
            let local = joint.rest.compute_matrix();
            out.push(match joint.parent { Some(p) if p < j => out[p] * local, _ => local });
        }
        out
    }

    /// Prefix shared by the names of all bones, such as "Skeleton 001_".
    pub fn name_prefix(&self) -> String {
        let names: Vec<&str> = self.joints.iter().filter(|j| j.is_bone).map(|j| j.name.as_str()).collect();
        let Some(first) = names.first() else { return String::new() };
        let mut len = first.len();
        for n in &names {
            len = len.min(first.bytes().zip(n.bytes()).take_while(|(a, b)| a == b).count());
        }
        // Cut back to a separator, so "LeftArm" and "LeftLeg" do not lose "Left".
        let cut = first[..len].rfind(|c| c == ':' || c == '_' || c == ' ').map(|i| i + 1).unwrap_or(0);
        first[..cut].to_string()
    }

    /// Local transforms that give these world rotations, with each joint
    /// keeping the local translation and scale it is given.
    fn locals_from_world_rotations(&self, world_rot: &[Quat], local_t: &[Vec3], scale: &[Vec3]) -> Vec<Transform> {
        (0..self.joints.len()).map(|j| {
            let parent = self.joints[j].parent.filter(|p| *p < j).map(|p| world_rot[p]).unwrap_or(Quat::IDENTITY);
            Transform { translation: local_t[j], rotation: (parent.inverse() * world_rot[j]).normalize(), scale: scale[j] }
        }).collect()
    }

    // ── Mirror ───────────────────────────────────────────────────────────────

    /// Mirror the motion left to right. Left and right joints swap roles;
    /// the pose is reflected in the plane x = 0. Works from each joint's
    /// change against the rest pose, so the joints' own axes do not matter.
    pub fn mirrored(&self) -> AnimData {
        let nj = self.joints.len();
        let pair: Vec<usize> = (0..nj).map(|j| {
            other_side(&self.joints[j].name)
                .and_then(|n| self.joints.iter().position(|x| x.name == n))
                .unwrap_or(j)
        }).collect();
        let rest = self.rest_world();
        let rest_rot: Vec<Quat> = rest.iter().map(|m| m.to_scale_rotation_translation().1).collect();
        let n = self.frame_count();
        let mut tracks: Vec<Track> = vec![Vec::with_capacity(n); nj];
        for i in 0..n {
            let world = self.world_pose(i);
            let mut rot = vec![Quat::IDENTITY; nj];
            let mut pos = vec![Vec3::ZERO; nj];
            let mut out_world = vec![Mat4::IDENTITY; nj];
            for j in 0..nj {
                let p = pair[j];
                let (_, wr, wt) = world[p].to_scale_rotation_translation();
                // Change of the paired joint against its rest, reflected.
                let delta = reflect_rotation(wr * rest_rot[p].inverse());
                rot[j] = (delta * rest_rot[j]).normalize();
                pos[j] = reflect_point(wt);
            }
            for j in 0..nj {
                let joint = &self.joints[j];
                let own = self.local(j, i);
                let parent = joint.parent.filter(|p| *p < j);
                let parent_rot = parent.map(|p| rot[p]).unwrap_or(Quat::IDENTITY);
                // Centre joints and roots follow the reflected position; a
                // left or right joint keeps the offset from its own parent.
                let translation = if pair[j] == j || parent.is_none() {
                    match parent {
                        Some(p) => out_world[p].inverse().transform_point3(pos[j]),
                        None => pos[j],
                    }
                } else {
                    self.local(j, i.min(n - 1)).translation.length() * joint.rest.translation.normalize_or_zero()
                };
                let local = Transform { translation, rotation: (parent_rot.inverse() * rot[j]).normalize(), scale: own.scale };
                out_world[j] = parent.map(|p| out_world[p]).unwrap_or(Mat4::IDENTITY) * local.compute_matrix();
                tracks[j].push(local);
            }
        }
        AnimData { tracks: Arc::new(tracks), skin: None, ..self.clone() }
    }

    // ── Smooth ───────────────────────────────────────────────────────────────

    /// Gaussian filter over time. `radius` is in frames; `amount` blends
    /// between the original (0) and the filtered motion (1).
    pub fn smoothed(&self, radius: u32, amount: f32, translations: bool) -> AnimData {
        if radius == 0 || amount <= 0.0 { return self.clone(); }
        let n = self.frame_count();
        let r = radius as i64;
        let sigma = (radius as f32 / 2.0).max(0.5);
        let weights: Vec<f32> = (-r..=r).map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp()).collect();
        let tracks: Vec<Track> = self.tracks.iter().map(|t| {
            if t.len() < 2 { return t.clone(); }
            (0..n.min(t.len())).map(|i| {
                let here = t[i];
                let (mut q, mut p, mut total) = (Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), Vec3::ZERO, 0.0);
                for (w, k) in weights.iter().zip(-r..=r) {
                    let s = &t[(i as i64 + k).clamp(0, t.len() as i64 - 1) as usize];
                    // Keep neighbours on the same side of the quaternion sphere.
                    let sq = if s.rotation.dot(here.rotation) < 0.0 { -s.rotation } else { s.rotation };
                    q = q + sq * *w;
                    p += s.translation * *w;
                    total += *w;
                }
                let filtered = Transform {
                    translation: if translations { p / total } else { here.translation },
                    rotation: if q.length_squared() > 1e-12 { q.normalize() } else { here.rotation },
                    scale: here.scale,
                };
                lerp(&here, &filtered, amount.clamp(0.0, 1.0))
            }).collect()
        }).collect();
        AnimData { tracks: Arc::new(tracks), ..self.clone() }
    }

    // ── In place / root motion ───────────────────────────────────────────────

    /// Hold the hips over the spot where they start. With `keep_height` the
    /// up and down motion stays. With `to_root`, the travel that is taken
    /// off the hips is put on the joint above them instead, if there is one.
    pub fn in_place(&self, keep_height: bool, to_root: bool) -> AnimData {
        let Some(hip) = self.hip_joint() else { return self.clone() };
        let n = self.frame_count();
        let mut tracks = self.dense_tracks();
        let root = self.joints[hip].parent;
        let start = self.world_pose(0)[hip].w_axis.truncate();
        for i in 0..n {
            let world = self.world_pose(i);
            let at = world[hip].w_axis.truncate();
            let mut travel = at - start;
            if keep_height { travel.y = 0.0; }
            let parent = root.map(|p| world[p]).unwrap_or(Mat4::IDENTITY);
            match root.filter(|_| to_root) {
                Some(r) => {
                    // Root carries the travel; hips keep their place under it.
                    let grand = self.joints[r].parent.map(|g| world[g]).unwrap_or(Mat4::IDENTITY);
                    let root_world = Mat4::from_translation(travel) * world[r];
                    let moved_root = Transform::from_matrix(grand.inverse() * root_world);
                    tracks[r][i] = moved_root;
                    tracks[hip][i] = Transform::from_matrix(root_world.inverse() * world[hip]);
                }
                None => {
                    let target = Mat4::from_translation(-travel) * world[hip];
                    tracks[hip][i] = Transform::from_matrix(parent.inverse() * target);
                }
            }
        }
        AnimData { tracks: Arc::new(tracks), ..self.clone() }
    }

    // ── Transform ────────────────────────────────────────────────────────────

    /// Move, turn (degrees, about Y, then X, then Z) and scale the whole clip.
    pub fn transformed(&self, translate: Vec3, rotate_deg: Vec3, scale: f32) -> AnimData {
        let x = Mat4::from_scale_rotation_translation(
            Vec3::splat(scale.max(1e-6)),
            Quat::from_euler(bevy::math::EulerRot::YXZ, rotate_deg.y.to_radians(), rotate_deg.x.to_radians(), rotate_deg.z.to_radians()),
            translate,
        );
        let n = self.frame_count();
        let mut tracks: Vec<Track> = (*self.tracks).clone();
        let mut joints = self.joints.clone();
        for j in 0..self.joints.len() {
            if self.joints[j].parent.is_some() { continue; }
            tracks[j] = (0..n).map(|i| Transform::from_matrix(x * self.local(j, i).compute_matrix())).collect();
            joints[j].rest = Transform::from_matrix(x * self.joints[j].rest.compute_matrix());
        }
        AnimData { joints, tracks: Arc::new(tracks), skin: None, ..self.clone() }
    }

    // ── Blend ────────────────────────────────────────────────────────────────

    /// This clip followed by `next`, cross-faded over `blend` frames. Joints
    /// are matched by name; a joint missing from `next` holds its last pose.
    /// With `align`, `next` is moved so its hips start where this clip's end.
    pub fn blended(&self, next: &AnimData, blend: u32, align: bool) -> AnimData {
        let (na, nb) = (self.frame_count(), next.frame_count());
        let blend = (blend as usize).min(na).min(nb);
        let total = na + nb - blend;
        let map: Vec<Option<usize>> = self.joints.iter()
            .map(|j| next.joints.iter().position(|x| x.name == j.name)).collect();

        // Offset that puts the hips of `next` under the hips at the join.
        let mut offset = Vec3::ZERO;
        if align {
            if let (Some(ha), Some(hb)) = (self.hip_joint(), self.hip_joint().and_then(|h| map[h])) {
                let end = self.world_pose(na - blend.max(1))[ha].w_axis.truncate();
                let start = next.world_pose(0)[hb].w_axis.truncate();
                offset = Vec3::new(end.x - start.x, 0.0, end.z - start.z);
            }
        }
        let b_root_shift = |jb: usize, t: Transform| -> Transform {
            if next.joints[jb].parent.is_some() { return t; }
            Transform { translation: t.translation + offset, ..t }
        };

        let tracks: Vec<Track> = (0..self.joints.len()).map(|j| {
            (0..total).map(|i| {
                let a = self.local(j, i.min(na - 1));
                let Some(jb) = map[j] else { return a };
                let start_b = na - blend;
                if i < start_b { return a; }
                let b = b_root_shift(jb, next.local(jb, i - start_b));
                if blend == 0 || i >= na { return b; }
                lerp(&a, &b, smoothstep((i - start_b + 1) as f32 / (blend + 1) as f32))
            }).collect()
        }).collect();
        AnimData { frames: total, tracks: Arc::new(tracks), ..self.clone() }
    }

    // ── Loop ─────────────────────────────────────────────────────────────────

    /// Make the clip cycle: over the last `blend` frames the pose is eased
    /// into the pose of the first frame. Travel of the root joints is kept.
    pub fn looped(&self, blend: u32) -> AnimData {
        let n = self.frame_count();
        let blend = (blend as usize).min(n.saturating_sub(1));
        if blend == 0 { return self.clone(); }
        let mut tracks = self.dense_tracks();
        for j in 0..self.joints.len() {
            let first = self.local(j, 0);
            for k in 0..blend {
                let i = n - blend + k;
                let w = smoothstep((k + 1) as f32 / blend as f32);
                let here = tracks[j][i];
                let mut eased = lerp(&here, &first, w);
                // A joint that travels (root, hips) keeps its position.
                if self.joints[j].parent.is_none() || Some(j) == self.hip_joint() {
                    eased.translation = Vec3::new(here.translation.x, eased.translation.y, here.translation.z);
                }
                tracks[j][i] = eased;
            }
        }
        AnimData { tracks: Arc::new(tracks), ..self.clone() }
    }

    // ── Retarget ─────────────────────────────────────────────────────────────

    /// Put this clip's motion on another skeleton.
    ///
    /// Joints are matched by name, ignoring namespaces, the prefix each
    /// skeleton shares across its bones, case and punctuation. Each matched
    /// joint takes the source joint's change against its rest pose, applied
    /// to its own rest pose. Where the rest poses differ (arms out against
    /// arms down), each bone is first lined up with the source bone at rest.
    /// Hips travel is scaled by the ratio of hip heights.
    /// Returns the clip and the number of joints matched.
    pub fn retargeted(&self, target: &AnimData) -> (AnimData, usize) {
        let (sp, tp) = (self.name_prefix(), target.name_prefix());
        let source_keys: Vec<String> = self.joints.iter().map(|j| key(short_name(&j.name, &sp))).collect();
        let map: Vec<Option<usize>> = target.joints.iter().map(|j| {
            if !j.is_bone { return None; }
            let k = key(short_name(&j.name, &tp));
            source_keys.iter().position(|s| *s == k && !k.is_empty())
        }).collect();
        let matched = map.iter().filter(|m| m.is_some()).count();

        let src_rest = self.rest_world();
        let tgt_rest = target.rest_world();
        let rot = |m: &Mat4| m.to_scale_rotation_translation().1;
        let (src_hip, tgt_hip) = (self.hip_joint(), target.hip_joint());
        let ratio = match (src_hip, tgt_hip) {
            (Some(s), Some(t)) if src_rest[s].w_axis.y.abs() > 1e-4 => tgt_rest[t].w_axis.y / src_rest[s].w_axis.y,
            _ => 1.0,
        };

        let n = self.frame_count();
        let nj = target.joints.len();
        let scale: Vec<Vec3> = target.joints.iter().map(|j| j.rest.scale).collect();
        let mut tracks: Vec<Track> = vec![Vec::with_capacity(n); nj];

        // The two skeletons may rest in different poses (arms out against
        // arms down). For each matched joint with one matched joint below
        // it, find the turn that points its bone the way the source bone
        // points at rest. Joints at the end of a chain take their parent's.
        let p_of = |m: &Mat4| m.w_axis.truncate();
        let mut align = vec![Quat::IDENTITY; nj];
        for j in 0..nj {
            let Some(sj) = map[j] else { continue };
            let below: Vec<usize> = (0..nj).filter(|c| target.joints[*c].parent == Some(j) && map[*c].is_some()).collect();
            align[j] = match below.as_slice() {
                [c] => {
                    let dt = (p_of(&tgt_rest[*c]) - p_of(&tgt_rest[j])).normalize_or_zero();
                    let ds = (p_of(&src_rest[map[*c].unwrap()]) - p_of(&src_rest[sj])).normalize_or_zero();
                    if dt == Vec3::ZERO || ds == Vec3::ZERO { Quat::IDENTITY } else { Quat::from_rotation_arc(dt, ds) }
                }
                [] => target.joints[j].parent.map(|p| align[p]).unwrap_or(Quat::IDENTITY),
                _ => Quat::IDENTITY,
            };
        }
        for i in 0..n {
            let world = self.world_pose(i);
            let mut world_rot = vec![Quat::IDENTITY; nj];
            let mut local_t: Vec<Vec3> = target.joints.iter().map(|j| j.rest.translation).collect();
            for j in 0..nj {
                let parent = target.joints[j].parent.filter(|p| *p < j);
                world_rot[j] = match map[j] {
                    Some(s) => (rot(&world[s]) * rot(&src_rest[s]).inverse() * align[j] * rot(&tgt_rest[j])).normalize(),
                    // Unmatched: stay at rest under the parent.
                    None => (parent.map(|p| world_rot[p]).unwrap_or(Quat::IDENTITY) * target.joints[j].rest.rotation).normalize(),
                };
            }
            if let (Some(s), Some(t)) = (src_hip, tgt_hip) {
                if map[t] == Some(s) || map[t].is_some() {
                    let s = map[t].unwrap_or(s);
                    let travel = (world[s].w_axis.truncate() - src_rest[s].w_axis.truncate()) * ratio;
                    let want = tgt_rest[t].w_axis.truncate() + travel;
                    // Hips position in the space of their parent, which stays at rest.
                    let parent = target.joints[t].parent.map(|p| tgt_rest[p]).unwrap_or(Mat4::IDENTITY);
                    local_t[t] = parent.inverse().transform_point3(want);
                }
            }
            // Joints above the hips keep their rest rotation, so the parent
            // space used for the hips position holds.
            if let Some(t) = tgt_hip {
                for j in 0..t { world_rot[j] = rot(&tgt_rest[j]); }
            }
            let locals = target.locals_from_world_rotations(&world_rot, &local_t, &scale);
            for (j, l) in locals.into_iter().enumerate() { tracks[j].push(l); }
        }
        (AnimData {
            name:        self.name.clone(),
            joints:      target.joints.clone(),
            rate:        self.rate,
            drop_frame:  self.drop_frame,
            start_frame: self.start_frame,
            frames:      n,
            tracks:      Arc::new(tracks),
            source:      self.source.clone(),
            source_dir:  self.source_dir.clone(),
            subject:     target.subject.clone(),
            skin:        target.skin.clone(),
        }, matched)
    }

    // ── Time warp ────────────────────────────────────────────────────────────

    /// Play faster or slower, or backwards. The rate stays; the number of
    /// frames changes. Poses between frames are interpolated.
    pub fn time_warped(&self, speed: f32, reverse: bool) -> AnimData {
        let speed = speed.clamp(0.01, 100.0);
        let n = self.frame_count();
        let frames = (((n - 1) as f32 / speed).round() as usize + 1).max(1);
        let tracks: Vec<Track> = self.tracks.iter().map(|t| {
            if t.is_empty() { return vec![]; }
            (0..frames).map(|i| {
                let mut src = (i as f32 * speed).min((t.len() - 1) as f32);
                if reverse { src = (t.len() - 1) as f32 - src; }
                let i0 = (src.floor() as usize).min(t.len() - 1);
                let i1 = (i0 + 1).min(t.len() - 1);
                lerp(&t[i0], &t[i1], src - i0 as f32)
            }).collect()
        }).collect();
        AnimData { frames, tracks: Arc::new(tracks), ..self.clone() }
    }

    // ── Prune ────────────────────────────────────────────────────────────────

    /// Remove every joint whose name matches the pattern (comma-separated
    /// regular expressions, see `core::pattern`), and everything below it.
    pub fn pruned(&self, words: &str) -> AnimData {
        let pattern = crate::core::pattern::NamePattern::new(words);
        if pattern.is_empty() { return self.clone(); }
        let mut gone = vec![false; self.joints.len()];
        for (j, joint) in self.joints.iter().enumerate() {
            gone[j] = joint.parent.map(|p| gone[p]).unwrap_or(false) || pattern.matches(&joint.name);
        }
        // Never remove everything.
        if gone.iter().all(|g| *g) { return self.clone(); }
        let mut new_index = vec![usize::MAX; self.joints.len()];
        let mut joints: Vec<Joint> = vec![];
        let mut tracks: Vec<Track> = vec![];
        for j in 0..self.joints.len() {
            if gone[j] { continue; }
            new_index[j] = joints.len();
            let mut joint = self.joints[j].clone();
            joint.parent = joint.parent.map(|p| new_index[p]);
            joints.push(joint);
            tracks.push(self.tracks.get(j).cloned().unwrap_or_default());
        }
        AnimData { joints, tracks: Arc::new(tracks), skin: None, ..self.clone() }
    }

    // ── Floor ────────────────────────────────────────────────────────────────

    /// Lowest point any bone reaches over the whole clip.
    pub fn lowest_point(&self) -> f32 {
        let mut low = f32::MAX;
        for i in 0..self.frame_count() {
            for (j, m) in self.world_pose(i).iter().enumerate() {
                if self.joints[j].is_bone { low = low.min(m.w_axis.y); }
            }
        }
        if low == f32::MAX { 0.0 } else { low }
    }

    /// Move the clip up or down so its lowest point sits at `height`.
    pub fn floored(&self, height: f32) -> AnimData {
        self.transformed(Vec3::new(0.0, height - self.lowest_point(), 0.0), Vec3::ZERO, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::{create_test_clip, FrameRate};

    fn clip() -> AnimData { create_test_clip(2.0, FrameRate::new(30, 1)) }
    fn joint(c: &AnimData, name: &str) -> usize { c.joints.iter().position(|j| j.name.ends_with(name)).unwrap() }
    fn pos(c: &AnimData, frame: usize, name: &str) -> Vec3 { c.world_pose(frame)[joint(c, name)].w_axis.truncate() }
    fn close(a: Vec3, b: Vec3, eps: f32) -> bool { (a - b).length() < eps }

    #[test]
    fn left_and_right_names_pair_up() {
        assert_eq!(other_side("Take01:LeftArm").as_deref(), Some("Take01:RightArm"));
        assert_eq!(other_side("RightToe").as_deref(), Some("LeftToe"));
        assert_eq!(other_side("L_Hand").as_deref(), Some("R_Hand"));
        assert_eq!(other_side("rig:R_Hand").as_deref(), Some("rig:L_Hand"));
        assert_eq!(other_side("Hand_L").as_deref(), Some("Hand_R"));
        assert_eq!(other_side("hand.r").as_deref(), Some("hand.l"));
        assert_eq!(other_side("Spine"), None);
        assert_eq!(other_side("Head"), None);
    }

    #[test]
    fn mirror_reflects_the_pose_and_twice_is_identity() {
        let c = clip();
        let m = c.mirrored();
        assert_eq!((m.frames, m.joints.len()), (c.frames, c.joints.len()));
        for f in [0, 7, 20, 44] {
            for (a, b) in [("LeftHand", "RightHand"), ("RightFoot", "LeftFoot"), ("Head", "Head"), ("Hips", "Hips"), ("LeftToe", "RightToe")] {
                let want = reflect_point(pos(&c, f, b));
                assert!(close(pos(&m, f, a), want, 1e-4), "frame {f} {a}: {} vs {want}", pos(&m, f, a));
            }
        }
        let back = m.mirrored();
        for f in [0, 13, 59] {
            for j in 0..c.joints.len() {
                let (a, b) = (c.world_pose(f)[j], back.world_pose(f)[j]);
                assert!(close(a.w_axis.truncate(), b.w_axis.truncate(), 1e-4));
                assert!(a.to_scale_rotation_translation().1.angle_between(b.to_scale_rotation_translation().1) < 1e-3);
            }
        }
    }

    #[test]
    fn smoothing_takes_out_jitter() {
        let c = clip();
        let mut tracks = (*c.tracks).clone();
        let arm = joint(&c, "LeftArm");
        // One frame knocked off.
        tracks[arm][30].rotation = tracks[arm][30].rotation * Quat::from_rotation_x(0.8);
        let noisy = AnimData { tracks: Arc::new(tracks), ..c.clone() };
        let err = |x: &AnimData| pos(x, 30, "LeftHand").distance(pos(&c, 30, "LeftHand"));
        let smooth = noisy.smoothed(3, 1.0, true);
        assert!(err(&noisy) > 0.2);
        assert!(err(&smooth) < err(&noisy) * 0.4, "{} vs {}", err(&smooth), err(&noisy));
        // No radius or no amount: unchanged.
        assert!(close(pos(&noisy.smoothed(0, 1.0, true), 30, "LeftHand"), pos(&noisy, 30, "LeftHand"), 1e-6));
        assert!(close(pos(&noisy.smoothed(3, 0.0, true), 30, "LeftHand"), pos(&noisy, 30, "LeftHand"), 1e-6));
        assert_eq!(smooth.frames, c.frames);
    }

    /// The test clip walking forwards at one metre a second.
    fn travelling() -> AnimData {
        let c = clip();
        let mut tracks = (*c.tracks).clone();
        for (i, t) in tracks[0].iter_mut().enumerate() { t.translation.z += i as f32 / 30.0; }
        AnimData { tracks: Arc::new(tracks), ..c }
    }

    #[test]
    fn in_place_holds_the_hips() {
        let c = travelling();
        assert!((pos(&c, 59, "Hips").z - pos(&c, 0, "Hips").z - 59.0 / 30.0).abs() < 1e-4);
        let p = c.in_place(true, false);
        for f in [0, 20, 59] {
            let (a, b) = (pos(&p, f, "Hips"), pos(&c, f, "Hips"));
            assert!((a.x - pos(&c, 0, "Hips").x).abs() < 1e-4 && (a.z - pos(&c, 0, "Hips").z).abs() < 1e-4);
            assert!((a.y - b.y).abs() < 1e-5, "height kept");
            // The pose around the hips is the same.
            assert!(close(pos(&p, f, "Head") - a, pos(&c, f, "Head") - b, 1e-4));
        }
        let flat = c.in_place(false, false);
        assert!(close(pos(&flat, 40, "Hips"), pos(&c, 0, "Hips"), 1e-4));
    }

    #[test]
    fn root_motion_moves_to_the_root_joint() {
        // Hips under a root that stands still.
        let c = travelling();
        let mut joints = vec![Joint::new("Root", None, Transform::IDENTITY)];
        joints[0].is_bone = false;
        joints.extend(c.joints.iter().map(|j| Joint { parent: Some(j.parent.map(|p| p + 1).unwrap_or(0)), ..j.clone() }));
        let mut tracks = vec![vec![]];
        tracks.extend((*c.tracks).clone());
        let rooted = AnimData { joints, tracks: Arc::new(tracks), ..c.clone() };
        let out = rooted.in_place(true, true);
        for f in [0, 31, 59] {
            // The figure is where it was.
            for name in ["Hips", "Head", "LeftToe"] { assert!(close(pos(&out, f, name), pos(&rooted, f, name), 1e-4)); }
            // The root now carries the travel.
            assert!((pos(&out, f, "Root").z - f as f32 / 30.0).abs() < 1e-4);
            assert!((pos(&out, f, "Hips").z - pos(&out, f, "Root").z - pos(&rooted, 0, "Hips").z).abs() < 1e-4);
        }
    }

    #[test]
    fn transform_moves_turns_and_scales_the_clip() {
        let c = clip();
        let t = c.transformed(Vec3::new(1.0, 0.0, 2.0), Vec3::new(0.0, 90.0, 0.0), 2.0);
        for f in [0, 25] {
            for name in ["Hips", "LeftHand", "Head"] {
                let p = pos(&c, f, name);
                let want = Vec3::new(p.z, p.y, -p.x) * 2.0 + Vec3::new(1.0, 0.0, 2.0);
                assert!(close(pos(&t, f, name), want, 1e-4), "{name}: {} vs {want}", pos(&t, f, name));
            }
        }
        assert!((c.floored(0.5).lowest_point() - 0.5).abs() < 1e-4);
        assert!(c.floored(0.0).lowest_point().abs() < 1e-4);
    }

    #[test]
    fn blend_joins_two_clips() {
        let a = travelling();
        let b = clip().trimmed(0, 30);
        let out = a.blended(&b, 10, true);
        assert_eq!(out.frames, 60 + 30 - 10);
        // Start is A, end is B.
        assert!(close(pos(&out, 0, "LeftHand"), pos(&a, 0, "LeftHand"), 1e-5));
        assert!(close(pos(&out, 49, "LeftHand"), pos(&a, 49, "LeftHand"), 1e-5));
        // Aligned: B carries on from where A's hips were, not from the origin.
        let end = pos(&out, 79, "Hips");
        assert!((end.z - pos(&a, 50, "Hips").z).abs() < 0.05, "{end}");
        // No jump across the join.
        for f in 45..70 {
            assert!(pos(&out, f + 1, "Hips").distance(pos(&out, f, "Hips")) < 0.08, "jump at {f}");
        }
        // Not aligned: B plays where it was recorded.
        let raw = a.blended(&b, 10, false);
        assert!(close(pos(&raw, 79, "Hips"), pos(&b, 29, "Hips"), 1e-5));
        // No cross-fade: a plain cut.
        assert_eq!(a.blended(&b, 0, false).frames, 90);
    }

    #[test]
    fn loop_ends_where_it_starts() {
        let c = clip().trimmed(0, 7);     // cut mid-stride so the ends differ
        let gap = |x: &AnimData| pos(x, x.frames - 1, "LeftHand").distance(pos(x, 0, "LeftHand"));
        assert!(gap(&c) > 0.05);
        let l = c.looped(12);
        assert!(gap(&l) < 1e-4);
        assert_eq!(l.frames, c.frames);
        assert!(close(pos(&l, 10, "LeftHand"), pos(&c, 10, "LeftHand"), 1e-6));
        // A travelling clip keeps its travel.
        let t = travelling().looped(10);
        assert!((pos(&t, 59, "Hips").z - pos(&travelling(), 59, "Hips").z).abs() < 1e-5);
    }

    /// The test skeleton under other names, twice the size.
    fn big_skeleton() -> AnimData {
        let c = clip();
        let joints = c.joints.iter().map(|j| {
            let mut out = j.clone();
            out.name = j.name.replace("Take01:", "Giant_");
            out.rest.translation *= 2.0;
            out
        }).collect();
        AnimData { joints, frames: 1, tracks: Arc::new(vec![vec![]; c.joints.len()]), ..c }
    }

    #[test]
    fn retarget_carries_the_motion_to_another_skeleton() {
        let c = travelling();
        let target = big_skeleton();
        assert_eq!(c.name_prefix(), "Take01:");
        assert_eq!(target.name_prefix(), "Giant_");
        let (out, matched) = c.retargeted(&target);
        assert_eq!(matched, 19);
        assert_eq!((out.frames, out.joints.len()), (c.frames, 19));
        assert!(out.joints[0].name.starts_with("Giant_"));
        for f in [0, 17, 59] {
            // Twice the skeleton: twice the pose, twice the travel.
            for name in ["Hips", "Head", "LeftHand", "RightToe"] {
                let want = pos(&c, f, name) * 2.0;
                assert!(close(pos(&out, f, name), want, 2e-3), "frame {f} {name}: {} vs {want}", pos(&out, f, name));
            }
        }
        // Bone lengths are the target's own.
        let len = |x: &AnimData, f| pos(x, f, "LeftForeArm").distance(pos(x, f, "LeftHand"));
        assert!((len(&out, 30) - 0.52).abs() < 1e-4);
        // A skeleton with nothing in common: nothing matched, pose at rest.
        let mut alien = big_skeleton();
        for j in &mut alien.joints { j.name = format!("x{}", j.name.len()); }
        let (out, matched) = c.retargeted(&alien);
        assert_eq!(matched, 0);
        assert!(close(out.world_pose(20)[4].w_axis.truncate(), alien.rest_world()[4].w_axis.truncate(), 1e-4));
    }

    #[test]
    fn retarget_copes_with_a_different_rest_pose() {
        // Same skeleton, but resting with the arms straight out to the sides.
        let c = clip();
        let mut target = AnimData { frames: 1, tracks: Arc::new(vec![vec![]; c.joints.len()]), ..c.clone() };
        for j in &mut target.joints {
            j.name = j.name.replace("Take01:", "T_");
            let side = if j.name.contains("Left") { 1.0 } else { -1.0 };
            if j.name.ends_with("ForeArm") || j.name.ends_with("Hand") {
                j.rest.translation = Vec3::new(side * j.rest.translation.length(), 0.0, 0.0);
            }
        }
        assert!((target.rest_world()[joint(&target, "LeftHand")].w_axis.y - c.rest_world()[joint(&c, "LeftHand")].w_axis.y).abs() > 0.4);
        let (out, matched) = c.retargeted(&target);
        assert_eq!(matched, 19);
        // Same bone lengths, so the arms land where the source arms are.
        for f in [0, 12, 40] {
            for name in ["LeftForeArm", "LeftHand", "RightHand", "Head", "LeftToe"] {
                assert!(close(pos(&out, f, name), pos(&c, f, name), 2e-3), "frame {f} {name}: {} vs {}", pos(&out, f, name), pos(&c, f, name));
            }
        }
    }

    #[test]
    fn time_warp_changes_speed_and_direction() {
        let c = clip();
        let fast = c.time_warped(2.0, false);
        assert_eq!(fast.frames, 31);
        assert!(close(pos(&fast, 10, "LeftHand"), pos(&c, 20, "LeftHand"), 1e-5));
        let slow = c.time_warped(0.5, false);
        assert_eq!(slow.frames, 119);
        assert!(close(pos(&slow, 40, "LeftHand"), pos(&c, 20, "LeftHand"), 1e-5));
        let back = c.time_warped(1.0, true);
        assert_eq!(back.frames, 60);
        assert!(close(pos(&back, 0, "LeftHand"), pos(&c, 59, "LeftHand"), 1e-5));
        assert!(close(pos(&back, 59, "LeftHand"), pos(&c, 0, "LeftHand"), 1e-5));
    }

    #[test]
    fn prune_takes_regular_expressions() {
        let c = clip();
        // Left side only, anchored at the end so "LeftToeBase" style names would stay.
        let p = c.pruned(":Left.*Arm$");
        assert!(p.joints.iter().all(|j| !(j.name.contains("Left") && j.name.contains("Arm"))));
        assert!(p.joints.iter().any(|j| j.name.contains("Right") && j.name.contains("Arm")));
        assert!(p.joints.len() < c.joints.len());
    }

    #[test]
    fn prune_drops_joints_and_what_hangs_below() {
        let c = clip();
        let p = c.pruned("toe, forearm");
        // Toes: 2. Forearms and the hands below them: 4.
        assert_eq!(p.joints.len(), 19 - 6);
        assert!(p.joints.iter().all(|j| !j.name.contains("Toe") && !j.name.contains("Hand")));
        for (j, joint) in p.joints.iter().enumerate() { assert!(joint.parent.map(|q| q < j).unwrap_or(true)); }
        assert!(close(pos(&p, 22, "LeftFoot"), pos(&c, 22, "LeftFoot"), 1e-5));
        assert_eq!(c.pruned("").joints.len(), 19);
        assert_eq!(c.pruned("take01").joints.len(), 19);   // would remove everything: refused
    }
}

