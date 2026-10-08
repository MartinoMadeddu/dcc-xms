//! Animation data that flows through the node graph.
//!
//! A node outputs a whole clip (skeleton + every frame). Time is not an input
//! to evaluation: the timeline and the viewport sample the clip at the
//! playhead. Frame numbers are absolute, counted from timecode 00:00:00:00.
//!
//! Clips at different rates are lined up by timecode, not by elapsed seconds:
//! 01:00:00:00 at 30 fps and at 29.97 fps are the same position.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use bevy::math::{EulerRot, Mat4, Quat, Vec3};
use bevy::prelude::Transform;

// ============================================================================
// FRAME RATE
// ============================================================================

/// Rational frame rate: `num / den` frames per second (29.97 = 30000/1001).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

/// Rates offered in the UI.
pub const RATE_PRESETS: &[(&str, FrameRate)] = &[
    ("23.976", FrameRate { num: 24000,  den: 1001 }),
    ("24",     FrameRate { num: 24,     den: 1 }),
    ("25",     FrameRate { num: 25,     den: 1 }),
    ("29.97",  FrameRate { num: 30000,  den: 1001 }),
    ("30",     FrameRate { num: 30,     den: 1 }),
    ("48",     FrameRate { num: 48,     den: 1 }),
    ("50",     FrameRate { num: 50,     den: 1 }),
    ("59.94",  FrameRate { num: 60000,  den: 1001 }),
    ("60",     FrameRate { num: 60,     den: 1 }),
    ("100",    FrameRate { num: 100,    den: 1 }),
    ("120",    FrameRate { num: 120,    den: 1 }),
    ("240",    FrameRate { num: 240,    den: 1 }),
];

impl FrameRate {
    pub const fn new(num: u32, den: u32) -> Self { Self { num, den } }

    pub fn fps(&self) -> f64 {
        self.num.max(1) as f64 / self.den.max(1) as f64
    }

    /// Nominal integer rate used for timecode (29.97 counts as 30).
    pub fn timebase(&self) -> i64 {
        (self.fps().round() as i64).max(1)
    }

    /// True for the NTSC rates where drop-frame timecode is defined.
    pub fn supports_drop_frame(&self) -> bool {
        self.den == 1001 && (self.num == 30000 || self.num == 60000)
    }

    /// Snap a floating-point rate to a preset, or keep it as a millirate.
    pub fn from_fps(fps: f64) -> Self {
        for (_, r) in RATE_PRESETS {
            if (r.fps() - fps).abs() < 0.005 { return *r; }
        }
        if fps <= 0.0 || !fps.is_finite() { return Self::new(30, 1); }
        if (fps - fps.round()).abs() < 1e-6 {
            Self::new(fps.round() as u32, 1)
        } else {
            Self::new((fps * 1000.0).round() as u32, 1000)
        }
    }

    pub fn label(&self) -> String {
        for (name, r) in RATE_PRESETS {
            if r == self { return (*name).to_string(); }
        }
        let f = self.fps();
        if (f - f.round()).abs() < 1e-6 { format!("{}", f.round() as i64) } else { format!("{f:.3}") }
    }
}

// ============================================================================
// TIMECODE
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub negative: bool,
    pub hours:    i64,
    pub minutes:  i64,
    pub seconds:  i64,
    pub frames:   i64,
    pub drop:     bool,
}

impl fmt::Display for Timecode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sep = if self.drop { ';' } else { ':' };
        write!(f, "{}{:02}:{:02}:{:02}{}{:02}",
            if self.negative { "-" } else { "" },
            self.hours, self.minutes, self.seconds, sep, self.frames)
    }
}

/// Absolute frame number to timecode.
pub fn frame_to_timecode(frame: i64, rate: FrameRate, drop_frame: bool) -> Timecode {
    let tb       = rate.timebase();
    let drop     = drop_frame && rate.supports_drop_frame();
    let negative = frame < 0;
    let mut f    = frame.abs();

    if drop {
        let d        = tb / 15;                 // 2 at 29.97, 4 at 59.94
        let per_min  = tb * 60 - d;
        let per_10m  = tb * 600 - d * 9;
        let tens     = f / per_10m;
        let rem      = f % per_10m;
        f += d * 9 * tens;
        if rem > d { f += d * ((rem - d) / per_min); }
    }

    Timecode {
        negative,
        hours:   f / (tb * 3600),
        minutes: (f / (tb * 60)) % 60,
        seconds: (f / tb) % 60,
        frames:  f % tb,
        drop,
    }
}

/// Timecode fields to absolute frame number.
pub fn timecode_to_frame(h: i64, m: i64, s: i64, f: i64, rate: FrameRate, drop_frame: bool) -> i64 {
    let tb    = rate.timebase();
    let mut n = (h * 3600 + m * 60 + s) * tb + f;
    if drop_frame && rate.supports_drop_frame() {
        let d          = tb / 15;
        let total_mins = h * 60 + m;
        n -= d * (total_mins - total_mins / 10);
    }
    n
}

/// Position on the timecode axis in seconds: `h*3600 + m*60 + s + ff/timebase`.
/// This is what two clips at different rates have in common.
pub fn frame_to_tc_seconds(frame: i64, rate: FrameRate, drop_frame: bool) -> f64 {
    let tc = frame_to_timecode(frame, rate, drop_frame);
    let t  = (tc.hours * 3600 + tc.minutes * 60 + tc.seconds) as f64
           + tc.frames as f64 / rate.timebase() as f64;
    if tc.negative { -t } else { t }
}

/// Frame under a timecode-axis position (the frame field is floored).
pub fn tc_seconds_to_frame(t: f64, rate: FrameRate, drop_frame: bool) -> i64 {
    let a     = t.abs();
    let whole = a.floor() as i64;
    let ff    = ((a - whole as f64) * rate.timebase() as f64 + 1e-6).floor() as i64;
    let f     = timecode_to_frame(whole / 3600, (whole / 60) % 60, whole % 60, ff, rate, drop_frame);
    if t < 0.0 { -f } else { f }
}

// ============================================================================
// SKELETON + CLIP
// ============================================================================

#[derive(Clone, Debug)]
pub struct Joint {
    pub name:   String,
    /// Index into `AnimData::joints`. Parents always come before children.
    pub parent: Option<usize>,
    /// Local rest transform, used when the joint has no samples.
    pub rest:   Transform,
    /// False for helper transforms (FBX nulls) above or between bones.
    pub is_bone: bool,
    /// Local rotation of the joint's neutral pose: what the rig shows when
    /// its animated rotation is zero. Identity unless the rig stores a fixed
    /// joint orientation (FBX PreRotation).
    pub zero_rot: Quat,
}

impl Joint {
    pub fn new(name: impl Into<String>, parent: Option<usize>, rest: Transform) -> Self {
        Self { name: name.into(), parent, rest, is_bone: true, zero_rot: Quat::IDENTITY }
    }
}

/// Mesh bound to a skeleton. Each vertex follows one joint, or, when
/// `weights` is filled in, a blend of up to four.
#[derive(Clone, Debug, Default)]
pub struct SkinMesh {
    /// Bind-pose positions, world space, metres.
    pub positions: Vec<Vec3>,
    pub normals:   Vec<Vec3>,
    /// Triangles and quads. A triangle repeats its last index.
    pub faces:     Vec<[u32; 4]>,
    /// Joint index per vertex.
    pub joint:     Vec<u32>,
    /// World matrix of every joint in the bind pose.
    pub bind:      Vec<Mat4>,
    /// Joints and weights per vertex, heaviest first, summing to one. Empty
    /// for a mesh whose vertices each follow `joint` alone.
    pub weights:   Vec<[(u32, f32); 4]>,
}

/// One manual correction applied by the Fix Pose node.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoseEdit {
    pub joint:       String,
    /// Extra rotation, degrees, XYZ, applied in the joint's parent space.
    pub rotation:    [f32; 3],
    /// Extra translation, centimetres.
    pub translation: [f32; 3],
}

/// Local transforms per frame for one joint. Empty means "static at rest".
pub type Track = Vec<Transform>;

#[derive(Clone, Debug)]
pub struct AnimData {
    /// Take / clip name.
    pub name:        String,
    pub joints:      Vec<Joint>,
    pub rate:        FrameRate,
    pub drop_frame:  bool,
    /// Absolute frame number of the first sample.
    pub start_frame: i64,
    /// Number of samples per animated track (at least 1).
    pub frames:      usize,
    /// One track per joint. Shared so that metadata-only nodes do not copy samples.
    pub tracks:      Arc<Vec<Track>>,
    /// File the clip was loaded from, without extension ("" if generated).
    pub source:      String,
    /// Folder of that file.
    pub source_dir:  String,
    /// Character name, set by the Split node.
    pub subject:     String,
    /// Optional mesh bound to the skeleton.
    pub skin:        Option<Arc<SkinMesh>>,
    /// Axes and unit of the file the clip came from. Written files use them
    /// again, so a clip goes back out with the skeleton it came in with.
    pub space:       Option<Arc<FileSpace>>,
}

/// The axes, unit and joint kinds of a source file.
///
/// Files are read into this program's space: Y up, metres. The conversion
/// is put on the joints at the top of the hierarchy, as ufbx does it; the
/// joints under them keep their values as written, in the file's unit. To
/// write the clip back as it came, `to_internal` is taken off those top
/// joints again and the file says the axes and unit it had.
#[derive(Clone, Debug, PartialEq)]
pub struct FileSpace {
    /// FBX axis numbers (0 X, 1 Y, 2 Z) and signs, as in GlobalSettings.
    pub up:          (i32, i32),
    pub front:       (i32, i32),
    pub coord:       (i32, i32),
    /// Centimetres per file unit (FBX UnitScaleFactor).
    pub unit_cm:     f64,
    /// File world to this program's world: the axis turn and the unit.
    pub to_internal: Mat4,
    /// Joints that are FBX "Root" skeleton nodes rather than "LimbNode".
    pub root_joints: Vec<String>,
    /// Name of the skinned mesh in the file.
    pub mesh_name:   String,
}

impl FileSpace {
    /// The space of a file with these axes and unit. `to_internal` turns
    /// the file's right, up and front onto +X, +Y, +Z and scales its unit
    /// to metres.
    pub fn new(up: (i32, i32), front: (i32, i32), coord: (i32, i32), unit_cm: f64) -> Self {
        let axis = |(a, sgn): (i32, i32)| {
            let mut v = Vec3::ZERO;
            v[a.clamp(0, 2) as usize] = if sgn < 0 { -1.0 } else { 1.0 };
            v
        };
        let basis = Mat4::from_cols(axis(coord).extend(0.0), axis(up).extend(0.0), axis(front).extend(0.0), bevy::math::Vec4::W);
        let unit = (unit_cm * 0.01) as f32;
        let to_internal = Mat4::from_scale(Vec3::splat(unit)) * basis.transpose();
        Self { up, front, coord, unit_cm, to_internal, root_joints: vec![], mesh_name: String::new() }
    }

    /// Y up, +Z front, +X right, centimetres: what this program writes for
    /// a clip that did not come from a file.
    pub fn y_up_cm() -> Self { Self::new((1, 1), (2, 1), (0, 1), 1.0) }

    /// Z up, -Y front, +X right, centimetres: Unreal's space.
    pub fn z_up_cm() -> Self { Self::new((2, 1), (1, -1), (0, 1), 1.0) }
}

impl AnimData {
    pub fn end_frame(&self) -> i64 { self.start_frame + self.frames.max(1) as i64 - 1 }

    /// Real duration, first sample to last.
    pub fn duration_seconds(&self) -> f64 {
        (self.frames.max(1) - 1) as f64 / self.rate.fps()
    }

    pub fn timecode(&self, frame: i64) -> Timecode {
        frame_to_timecode(frame, self.rate, self.drop_frame)
    }

    /// Timecode-axis position of an absolute frame.
    pub fn tc_seconds(&self, frame: i64) -> f64 {
        frame_to_tc_seconds(frame, self.rate, self.drop_frame)
    }

    /// Absolute frame under a timecode-axis position (not clamped).
    pub fn frame_at(&self, tc_seconds: f64) -> i64 {
        tc_seconds_to_frame(tc_seconds, self.rate, self.drop_frame)
    }

    /// Sample index under a timecode-axis position, clamped to the clip.
    pub fn index_at(&self, tc_seconds: f64) -> usize {
        let rel = self.frame_at(tc_seconds) - self.start_frame;
        rel.clamp(0, self.frames.max(1) as i64 - 1) as usize
    }

    pub fn local(&self, joint: usize, index: usize) -> Transform {
        match self.tracks.get(joint) {
            Some(t) if !t.is_empty() => t[index.min(t.len() - 1)],
            _ => self.joints[joint].rest,
        }
    }

    /// World-space matrices of every joint at one sample index.
    pub fn world_pose(&self, index: usize) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.joints.len());
        for (j, joint) in self.joints.iter().enumerate() {
            let local = self.local(j, index).compute_matrix();
            let world = match joint.parent {
                Some(p) if p < j => out[p] * local,
                _ => local,
            };
            out.push(world);
        }
        out
    }

    /// Depth of each joint in the hierarchy (roots are 0).
    pub fn joint_depths(&self) -> Vec<usize> {
        let mut d = vec![0usize; self.joints.len()];
        for (j, joint) in self.joints.iter().enumerate() {
            if let Some(p) = joint.parent { if p < j { d[j] = d[p] + 1; } }
        }
        d
    }

    // ── Operators ────────────────────────────────────────────────────────────

    /// Rename joints. Samples are shared with the input.
    pub fn renamed(&self, find: &str, replace: &str, strip_namespace: bool, prefix: &str) -> AnimData {
        // `find` is one regular expression, case ignored. `replace` may use
        // $1, $2 for its groups.
        let find = (!find.trim().is_empty()).then(|| crate::core::pattern::compile(find.trim()));
        let mut out = self.clone();
        for j in &mut out.joints {
            let mut n = j.name.clone();
            if strip_namespace {
                if let Some(i) = n.rfind(':') { n = n[i + 1..].to_string(); }
            }
            if let Some(re) = &find { n = re.replace_all(&n, replace).into_owned(); }
            if !prefix.is_empty() { n = format!("{prefix}{n}"); }
            j.name = n;
        }
        out
    }

    /// Cut `head` frames from the start and `tail` frames from the end.
    /// The remaining frames keep their absolute frame numbers.
    pub fn trimmed(&self, head: usize, tail: usize) -> AnimData {
        let n    = self.frames.max(1);
        let head = head.min(n - 1);
        let tail = tail.min(n - 1 - head);
        let keep = n - head - tail;
        let tracks: Vec<Track> = self.tracks.iter().map(|t| {
            if t.is_empty() { return vec![]; }
            let a = head.min(t.len() - 1);
            let b = (head + keep).min(t.len());
            t[a..b.max(a + 1)].to_vec()
        }).collect();
        AnimData {
            start_frame: self.start_frame + head as i64,
            frames:      keep,
            tracks:      Arc::new(tracks),
            ..self.clone()
        }
    }

    /// Resample to a new rate. Start timecode and duration are preserved.
    pub fn resampled(&self, rate: FrameRate) -> AnimData {
        if rate == self.rate { return self.clone(); }
        let old_fps = self.rate.fps();
        let new_fps = rate.fps();
        let frames  = (self.duration_seconds() * new_fps + 1e-6).floor() as usize + 1;
        let tracks: Vec<Track> = self.tracks.iter().map(|t| {
            if t.is_empty() { return vec![]; }
            (0..frames).map(|i| {
                let src = i as f64 / new_fps * old_fps;
                let i0  = (src.floor() as usize).min(t.len() - 1);
                let i1  = (i0 + 1).min(t.len() - 1);
                lerp_transform(&t[i0], &t[i1], (src - i0 as f64) as f32)
            }).collect()
        }).collect();
        let drop = self.drop_frame && rate.supports_drop_frame();
        AnimData {
            rate,
            drop_frame:  drop,
            start_frame: tc_seconds_to_frame(self.tc_seconds(self.start_frame), rate, drop),
            frames,
            tracks:      Arc::new(tracks),
            ..self.clone()
        }
    }

    /// Keep every sample and relabel the rate. Playback speed changes; the
    /// start timecode fields are preserved.
    pub fn reinterpreted(&self, rate: FrameRate) -> AnimData {
        let drop  = self.drop_frame && rate.supports_drop_frame();
        let start = tc_seconds_to_frame(self.tc_seconds(self.start_frame), rate, drop);
        AnimData { rate, drop_frame: drop, start_frame: start, ..self.clone() }
    }

    /// Move the clip so its first frame sits at the given timecode.
    pub fn with_start_timecode(&self, h: u32, m: u32, s: u32, f: u32, drop_frame: bool) -> AnimData {
        let drop = drop_frame && self.rate.supports_drop_frame();
        AnimData {
            drop_frame:  drop,
            start_frame: timecode_to_frame(h as i64, m as i64, s as i64, f as i64, self.rate, drop),
            ..self.clone()
        }
    }
}

// ── Characters ───────────────────────────────────────────────────────────────

impl AnimData {
    /// Joints below `root`, root first, in hierarchy order.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        let mut inside = vec![false; self.joints.len()];
        let mut out = vec![];
        for (j, joint) in self.joints.iter().enumerate() {
            if j == root || joint.parent.map(|p| inside[p]).unwrap_or(false) {
                inside[j] = true;
                out.push(j);
            }
        }
        out
    }

    /// Roots of the characters in the clip: top-level joints with at least
    /// one bone below them.
    pub fn character_roots(&self) -> Vec<usize> {
        (0..self.joints.len())
            .filter(|j| self.joints[*j].parent.is_none())
            .filter(|j| self.subtree(*j).iter().any(|i| self.joints[*i].is_bone))
            .collect()
    }

    /// Character name for a root joint: its name without a trailing "_Root",
    /// spaces replaced so it is safe in a file name.
    pub fn character_name(&self, root: usize) -> String {
        let n = &self.joints[root].name;
        let n = if n.to_lowercase().ends_with("_root") { &n[..n.len() - 5] } else { n.as_str() };
        n.trim().replace(' ', "_")
    }

    /// Keep one joint and everything below it.
    pub fn split(&self, root: usize) -> AnimData {
        let ids = self.subtree(root);
        let mut new_index = vec![usize::MAX; self.joints.len()];
        for (n, j) in ids.iter().enumerate() { new_index[*j] = n; }
        let joints = ids.iter().map(|j| {
            let mut joint = self.joints[*j].clone();
            joint.parent = if *j == root { None } else { joint.parent.map(|p| new_index[p]) };
            joint
        }).collect();
        let tracks = ids.iter().map(|j| self.tracks.get(*j).cloned().unwrap_or_default()).collect();
        AnimData {
            joints,
            tracks:  Arc::new(tracks),
            subject: self.character_name(root),
            skin:    None,
            ..self.clone()
        }
    }

    // ── Poses ────────────────────────────────────────────────────────────────

    /// First bone of the skeleton: the joint that carries the hip height.
    pub fn hip_joint(&self) -> Option<usize> {
        self.joints.iter().position(|j| j.is_bone)
    }

    /// Single-frame neutral pose: every rotation at the rig's zero, helper
    /// roots at the origin, hips centred. `hip_height` (metres) replaces the
    /// rest height of the hips when given.
    pub fn auto_tpose(&self, hip_height: Option<f32>) -> AnimData {
        let hip = self.hip_joint();
        let tracks: Vec<Track> = self.joints.iter().enumerate().map(|(j, joint)| {
            let mut t = joint.rest;
            t.rotation = joint.zero_rot;
            let above_hips = hip.map(|h| j < h && !joint.is_bone).unwrap_or(false);
            if joint.parent.is_none() || above_hips {
                t.translation = Vec3::ZERO;
            }
            if Some(j) == hip {
                t.translation = Vec3::new(0.0, hip_height.unwrap_or(t.translation.y), 0.0);
            }
            vec![t]
        }).collect();
        AnimData { frames: 1, tracks: Arc::new(tracks), skin: None, ..self.clone() }
    }

    /// Apply manual corrections to every frame.
    pub fn pose_fixed(&self, edits: &[PoseEdit]) -> AnimData {
        let mut tracks: Vec<Track> = (*self.tracks).clone();
        for e in edits {
            // A joint of exactly that name wins. Otherwise the text is a
            // pattern and the edit goes to every joint it matches.
            let targets: Vec<usize> = match self.joints.iter().position(|x| x.name == e.joint) {
                Some(j) => vec![j],
                None => {
                    let p = crate::core::pattern::NamePattern::new(&e.joint);
                    (0..self.joints.len()).filter(|j| p.matches(&self.joints[*j].name)).collect()
                }
            };
            let rot = Quat::from_euler(
                EulerRot::ZYX,
                e.rotation[2].to_radians(), e.rotation[1].to_radians(), e.rotation[0].to_radians(),
            );
            let tr = Vec3::from_array(e.translation) * 0.01;
            for j in targets {
                if tracks[j].is_empty() { tracks[j] = vec![self.joints[j].rest; self.frames.max(1)]; }
                for t in tracks[j].iter_mut() {
                    t.rotation     = (rot * t.rotation).normalize();
                    t.translation += tr;
                }
            }
        }
        AnimData { tracks: Arc::new(tracks), skin: None, ..self.clone() }
    }

    // ── Proxy mesh ───────────────────────────────────────────────────────────

    /// Sphere per bone and cylinder per bone-to-child link, built in the pose
    /// of the first frame and bound rigidly. `thickness` scales the radii.
    pub fn with_proxy_skin(&self, thickness: f32) -> AnimData {
        let bind = self.world_pose(0);
        let pos: Vec<Vec3> = bind.iter().map(|m| m.w_axis.truncate()).collect();
        let mut kids: Vec<Vec<usize>> = vec![vec![]; self.joints.len()];
        for (j, joint) in self.joints.iter().enumerate() {
            if let Some(p) = joint.parent { if joint.is_bone { kids[p].push(j); } }
        }
        let mut mesh = SkinMesh { bind, ..Default::default() };
        let k = thickness.max(0.01);
        for (j, joint) in self.joints.iter().enumerate() {
            if !joint.is_bone { continue; }
            let mut lens: Vec<f32> = kids[j].iter().map(|c| pos[j].distance(pos[*c])).collect();
            if let Some(p) = joint.parent {
                if self.joints[p].is_bone { lens.push(pos[j].distance(pos[p])); }
            }
            lens.retain(|l| *l > 1e-6);
            let shortest = lens.iter().copied().fold(f32::MAX, f32::min);
            let r = if lens.is_empty() { 0.01 } else { (0.18 * shortest).clamp(0.0035, 0.03) };
            mesh.add_sphere(pos[j], r * k, j as u32);
            for c in &kids[j] {
                let len = pos[j].distance(pos[*c]);
                if len > 1e-6 {
                    mesh.add_cylinder(pos[j], pos[*c], (0.09 * len).clamp(0.002, 0.016) * k, j as u32);
                }
            }
        }
        AnimData { skin: Some(Arc::new(mesh)), ..self.clone() }
    }
}

impl SkinMesh {
    fn add(&mut self, p: Vec3, n: Vec3, joint: u32) -> u32 {
        self.positions.push(p);
        self.normals.push(n);
        self.joint.push(joint);
        self.positions.len() as u32 - 1
    }

    fn add_sphere(&mut self, c: Vec3, r: f32, joint: u32) {
        const SEG: usize = 10;
        const RINGS: usize = 6;
        let top = self.add(c + Vec3::Y * r, Vec3::Y, joint);
        let mut rows: Vec<Vec<u32>> = vec![];
        for j in 1..RINGS {
            let th = std::f32::consts::PI * j as f32 / RINGS as f32;
            rows.push((0..SEG).map(|i| {
                let ph = std::f32::consts::TAU * i as f32 / SEG as f32;
                let n  = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
                self.add(c + n * r, n, joint)
            }).collect());
        }
        let bot = self.add(c - Vec3::Y * r, -Vec3::Y, joint);
        for i in 0..SEG {
            let k = (i + 1) % SEG;
            self.faces.push([top, rows[0][k], rows[0][i], rows[0][i]]);
            for w in rows.windows(2) {
                self.faces.push([w[0][i], w[0][k], w[1][k], w[1][i]]);
            }
            let last = rows.last().unwrap();
            self.faces.push([bot, last[i], last[k], last[k]]);
        }
    }

    fn add_cylinder(&mut self, a: Vec3, b: Vec3, r: f32, joint: u32) {
        const SEG: usize = 8;
        let axis = (b - a).normalize();
        let refv = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let u = axis.cross(refv).normalize();
        let v = axis.cross(u);
        let ring: Vec<(u32, u32)> = (0..SEG).map(|i| {
            let ph = std::f32::consts::TAU * i as f32 / SEG as f32;
            let n  = u * ph.cos() + v * ph.sin();
            (self.add(a + n * r, n, joint), self.add(b + n * r, n, joint))
        }).collect();
        for i in 0..SEG {
            let k = (i + 1) % SEG;
            self.faces.push([ring[i].0, ring[k].0, ring[k].1, ring[i].1]);
        }
    }

    pub fn is_tri(face: &[u32; 4]) -> bool { face[2] == face[3] }

    /// Positions and normals with every vertex carried by its joint.
    /// `pose` holds the current world matrix of every joint.
    pub fn deformed(&self, pose: &[Mat4]) -> (Vec<Vec3>, Vec<Vec3>) {
        let skin: Vec<Mat4> = pose.iter().zip(&self.bind).map(|(p, b)| *p * b.inverse()).collect();
        let mut pos = Vec::with_capacity(self.positions.len());
        let mut nrm = Vec::with_capacity(self.positions.len());
        if self.weights.len() == self.positions.len() {
            // Linear blend: the weighted mean of where each joint takes the vertex.
            for i in 0..self.positions.len() {
                let (mut p, mut n) = (Vec3::ZERO, Vec3::ZERO);
                for (j, w) in self.weights[i] {
                    if w <= 0.0 { continue; }
                    let Some(m) = skin.get(j as usize) else { continue };
                    p += m.transform_point3(self.positions[i]) * w;
                    n += m.transform_vector3(self.normals[i]) * w;
                }
                pos.push(p);
                nrm.push(n.normalize_or_zero());
            }
            return (pos, nrm);
        }
        for i in 0..self.positions.len() {
            let m = skin.get(self.joint[i] as usize).copied().unwrap_or(Mat4::IDENTITY);
            pos.push(m.transform_point3(self.positions[i]));
            nrm.push(m.transform_vector3(self.normals[i]).normalize_or_zero());
        }
        (pos, nrm)
    }
}

fn lerp_transform(a: &Transform, b: &Transform, t: f32) -> Transform {
    if t <= 0.0 { return *a; }
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation:    a.rotation.slerp(b.rotation, t),
        scale:       a.scale.lerp(b.scale, t),
    }
}

// ============================================================================
// TEST CLIP GENERATOR
// ============================================================================

/// Procedural biped walking on the spot. Joint names carry a namespace so the
/// rename node has something to clean. Starts at 01:00:00:00.
pub fn create_test_clip(seconds: f32, rate: FrameRate) -> AnimData {
    use bevy::math::Quat;

    const NS: &str = "Take01:";
    // (name, parent, rest translation)
    let defs: [(&str, Option<usize>, [f32; 3]); 19] = [
        ("Hips",         None,     [ 0.00, 0.95, 0.0]),
        ("Spine",        Some(0),  [ 0.00, 0.12, 0.0]),
        ("Chest",        Some(1),  [ 0.00, 0.22, 0.0]),
        ("Neck",         Some(2),  [ 0.00, 0.22, 0.0]),
        ("Head",         Some(3),  [ 0.00, 0.14, 0.0]),
        ("LeftUpLeg",    Some(0),  [ 0.10,-0.05, 0.0]),
        ("LeftLeg",      Some(5),  [ 0.00,-0.43, 0.0]),
        ("LeftFoot",     Some(6),  [ 0.00,-0.42, 0.0]),
        ("LeftToe",      Some(7),  [ 0.00,-0.05, 0.14]),
        ("RightUpLeg",   Some(0),  [-0.10,-0.05, 0.0]),
        ("RightLeg",     Some(9),  [ 0.00,-0.43, 0.0]),
        ("RightFoot",    Some(10), [ 0.00,-0.42, 0.0]),
        ("RightToe",     Some(11), [ 0.00,-0.05, 0.14]),
        ("LeftArm",      Some(2),  [ 0.19, 0.16, 0.0]),
        ("LeftForeArm",  Some(13), [ 0.00,-0.28, 0.0]),
        ("LeftHand",     Some(14), [ 0.00,-0.26, 0.0]),
        ("RightArm",     Some(2),  [-0.19, 0.16, 0.0]),
        ("RightForeArm", Some(16), [ 0.00,-0.28, 0.0]),
        ("RightHand",    Some(17), [ 0.00,-0.26, 0.0]),
    ];

    let joints: Vec<Joint> = defs.iter().map(|(n, p, t)| Joint::new(
        format!("{NS}{n}"), *p, Transform::from_translation(Vec3::from_array(*t)),
    )).collect();

    let fps    = rate.fps();
    let frames = ((seconds.max(0.0) as f64 * fps).round() as usize).max(1);
    let mut tracks: Vec<Track> = joints.iter().map(|j| vec![j.rest; frames]).collect();

    for i in 0..frames {
        let t     = i as f64 / fps;
        let phase = (t * std::f64::consts::TAU) as f32;      // one stride per second
        let s     = phase.sin();
        let rot_x = |a: f32| Quat::from_rotation_x(a);

        tracks[0][i].translation.y = 0.95 + 0.02 * (2.0 * phase).cos();
        tracks[0][i].rotation      = Quat::from_rotation_y(0.08 * s);
        tracks[2][i].rotation      = Quat::from_rotation_y(-0.12 * s);

        tracks[5][i].rotation  = rot_x(-0.55 * s);
        tracks[6][i].rotation  = rot_x(0.9 * (-s).max(0.0));
        tracks[9][i].rotation  = rot_x(0.55 * s);
        tracks[10][i].rotation = rot_x(0.9 * s.max(0.0));

        tracks[13][i].rotation = rot_x(0.5 * s);
        tracks[14][i].rotation = rot_x(-0.35 - 0.2 * s);
        tracks[16][i].rotation = rot_x(-0.5 * s);
        tracks[17][i].rotation = rot_x(-0.35 + 0.2 * s);
    }

    AnimData {
        name:        "TestWalk".into(),
        joints,
        rate,
        drop_frame:  false,
        start_frame: timecode_to_frame(1, 0, 0, 0, rate, false),
        frames,
        tracks:      Arc::new(tracks),
        source:      String::new(),
        source_dir:  String::new(),
        subject:     String::new(),
        skin:        None,
        space:       None,
    }
}

// ============================================================================
// MEMO
// ============================================================================
//
// The graph is re-evaluated often (every panel asks for its own node), so an
// operator must not redo its work unless its parameters or its input changed.
// The key is the node's parameters plus the identity of the input clip.

struct MemoEntry {
    _input: Option<Arc<AnimData>>,   // kept alive so the pointer in the key stays unique
    output: Option<Arc<AnimData>>,
}

static MEMO: OnceLock<Mutex<HashMap<u64, MemoEntry>>> = OnceLock::new();

pub fn memo(
    params:  &str,
    input:   Option<&Arc<AnimData>>,
    compute: impl FnOnce() -> Option<AnimData>,
) -> Option<Arc<AnimData>> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    params.hash(&mut h);
    input.map(|a| Arc::as_ptr(a) as usize).hash(&mut h);
    let key = h.finish();

    let map = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(e) = map.lock().unwrap().get(&key) {
        return e.output.clone();
    }
    let output = compute().map(Arc::new);
    let mut map = map.lock().unwrap();
    if map.len() > 24 { map.clear(); }   // entries hold whole clips
    map.insert(key, MemoEntry { _input: input.cloned(), output: output.clone() });
    output
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: FrameRate = FrameRate::new(30000, 1001);

    #[test]
    fn non_drop_timecode() {
        let r = FrameRate::new(24, 1);
        assert_eq!(frame_to_timecode(0, r, false).to_string(), "00:00:00:00");
        assert_eq!(frame_to_timecode(86400, r, false).to_string(), "01:00:00:00");
        assert_eq!(frame_to_timecode(86400 + 24 * 61 + 5, r, false).to_string(), "01:01:01:05");
        assert_eq!(timecode_to_frame(1, 1, 1, 5, r, false), 86400 + 24 * 61 + 5);
    }

    #[test]
    fn drop_frame_timecode() {
        assert_eq!(frame_to_timecode(1799, NTSC, true).to_string(), "00:00:59;29");
        assert_eq!(frame_to_timecode(1800, NTSC, true).to_string(), "00:01:00;02");
        assert_eq!(frame_to_timecode(17982, NTSC, true).to_string(), "00:10:00;00");
        assert_eq!(frame_to_timecode(107892, NTSC, true).to_string(), "01:00:00;00");
        for f in [0i64, 1, 1799, 1800, 17981, 17982, 107892, 250_000] {
            let tc = frame_to_timecode(f, NTSC, true);
            assert_eq!(timecode_to_frame(tc.hours, tc.minutes, tc.seconds, tc.frames, NTSC, true), f);
        }
    }

    #[test]
    fn drop_frame_ignored_at_integer_rates() {
        let tc = frame_to_timecode(1800, FrameRate::new(30, 1), true);
        assert_eq!(tc.to_string(), "00:01:00:00");
    }

    #[test]
    fn rate_snapping() {
        assert_eq!(FrameRate::from_fps(29.97), NTSC);
        assert_eq!(FrameRate::from_fps(23.976), FrameRate::new(24000, 1001));
        assert_eq!(FrameRate::from_fps(120.0), FrameRate::new(120, 1));
        assert_eq!(NTSC.timebase(), 30);
    }

    #[test]
    fn trim_keeps_absolute_frames() {
        let c = create_test_clip(2.0, FrameRate::new(30, 1));
        assert_eq!(c.frames, 60);
        assert_eq!(c.timecode(c.start_frame).to_string(), "01:00:00:00");
        let t = c.trimmed(10, 5);
        assert_eq!(t.frames, 45);
        assert_eq!(t.start_frame, c.start_frame + 10);
        assert_eq!(t.end_frame(), c.end_frame() - 5);
        assert_eq!(t.local(5, 0), c.local(5, 10));
        let all = c.trimmed(1000, 1000);
        assert_eq!(all.frames, 1);
    }

    #[test]
    fn resample_preserves_time() {
        let c = create_test_clip(2.0, FrameRate::new(30, 1));
        let r = c.resampled(FrameRate::new(60, 1));
        assert_eq!(r.frames, 119);
        assert_eq!(r.timecode(r.start_frame).to_string(), "01:00:00:00");
        assert!((r.duration_seconds() - c.duration_seconds()).abs() < 1e-9);
        assert_eq!(r.local(5, 20), c.local(5, 10));
        let down = c.resampled(FrameRate::new(15, 1));
        assert_eq!(down.frames, 30);
        assert_eq!(down.local(5, 7), c.local(5, 14));
    }

    #[test]
    fn rates_line_up_by_timecode() {
        let c = create_test_clip(2.0, FrameRate::new(30, 1)).trimmed(20, 0);
        assert_eq!(c.timecode(c.start_frame).to_string(), "01:00:00:20");
        // 30 -> 60: frame field doubles.
        let hi = c.resampled(FrameRate::new(60, 1));
        assert_eq!(hi.timecode(hi.start_frame).to_string(), "01:00:00:40");
        // 30 -> 29.97: same label, even though elapsed seconds differ.
        let ntsc = c.resampled(NTSC);
        assert_eq!(ntsc.timecode(ntsc.start_frame).to_string(), "01:00:00:20");
        // A playhead on the 30 fps clip lands on the matching frame of each.
        let t = c.tc_seconds(c.start_frame + 15);
        assert_eq!(hi.frame_at(t) - hi.start_frame, 30);
        assert_eq!(ntsc.frame_at(t) - ntsc.start_frame, 15);
        // Round trip, including drop-frame.
        let df = c.resampled(NTSC).with_start_timecode(0, 59, 59, 0, true);
        for f in df.start_frame..df.start_frame + 120 {
            assert_eq!(df.frame_at(df.tc_seconds(f)), f);
        }
    }

    #[test]
    fn reinterpret_keeps_samples_and_timecode() {
        let c = create_test_clip(2.0, FrameRate::new(30, 1));
        let r = c.reinterpreted(FrameRate::new(24, 1));
        assert_eq!(r.frames, c.frames);
        assert_eq!(r.timecode(r.start_frame).to_string(), "01:00:00:00");
        assert!(Arc::ptr_eq(&r.tracks, &c.tracks));
    }

    #[test]
    fn rename_shares_samples() {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let r = c.renamed("Left", "L_", true, "mx_");
        assert_eq!(r.joints[0].name, "mx_Hips");
        assert_eq!(r.joints[5].name, "mx_L_UpLeg");
        assert!(Arc::ptr_eq(&r.tracks, &c.tracks));
    }

    #[test]
    fn rename_with_groups() {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let r = c.renamed("^(left|right)(.+)$", "${2}_$1", true, "");
        assert_eq!(r.joints[5].name, "UpLeg_Left");
        assert_eq!(r.joints[0].name, "Hips");
    }

    #[test]
    fn fix_pose_by_pattern_reaches_every_match() {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let edit = |joint: &str| PoseEdit { joint: joint.into(), rotation: [0.0; 3], translation: [0.0, 10.0, 0.0] };
        let moved = |a: &AnimData| (0..a.joints.len())
            .filter(|j| a.tracks[*j].first().map(|t| t.translation) != c.tracks[*j].first().map(|t| t.translation)
                        && !a.tracks[*j].is_empty())
            .count();
        let one  = c.pose_fixed(&[edit("LeftUpLeg")]);
        let many = c.pose_fixed(&[edit("UpLeg$")]);
        assert_eq!(moved(&one), 1);
        assert_eq!(moved(&many), 2);
    }

    #[test]
    fn set_start_timecode() {
        let c = create_test_clip(1.0, NTSC).with_start_timecode(10, 0, 0, 0, true);
        assert_eq!(c.timecode(c.start_frame).to_string(), "10:00:00;00");
    }

    #[test]
    fn world_pose_chains_parents() {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let pose = c.world_pose(0);
        let head = pose[4].w_axis.truncate();
        assert!((head.y - (0.95 + 0.02 + 0.12 + 0.22 + 0.22 + 0.14)).abs() < 1e-4);
    }

    /// Two test characters under helper roots, like a Motive export.
    fn two_characters() -> AnimData {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let n = c.joints.len();
        let mut joints = vec![];
        let mut tracks = vec![];
        for (ci, name) in ["Skeleton 001", "Skeleton 002"].iter().enumerate() {
            let base = joints.len();
            let mut root = Joint::new(format!("{name}_Root"), None, Transform::IDENTITY);
            root.is_bone = false;
            joints.push(root);
            tracks.push(vec![]);
            for (j, joint) in c.joints.iter().enumerate() {
                let mut joint = joint.clone();
                joint.name   = format!("{name}_{}", joint.name.trim_start_matches("Take01:"));
                joint.parent = Some(joint.parent.map(|p| base + 1 + p).unwrap_or(base));
                joints.push(joint);
                let mut t = c.tracks[j].clone();
                if j == 0 { for x in &mut t { x.translation.x += ci as f32 * 2.0; } }
                tracks.push(t);
            }
        }
        assert_eq!(joints.len(), 2 * (n + 1));
        AnimData { joints, tracks: Arc::new(tracks), source: "take".into(), ..c }
    }

    #[test]
    fn split_by_character_root() {
        let c = two_characters();
        let roots = c.character_roots();
        assert_eq!(roots, vec![0, 20]);
        assert_eq!(c.character_name(roots[1]), "Skeleton_002");
        let b = c.split(roots[1]);
        assert_eq!(b.joints.len(), 20);
        assert_eq!(b.subject, "Skeleton_002");
        assert_eq!(b.joints[0].parent, None);
        assert_eq!(b.joints[1].name, "Skeleton 002_Hips");
        assert_eq!(b.joints[1].parent, Some(0));
        assert_eq!(b.joints[6].parent, Some(1));
        assert_eq!(b.frames, c.frames);
        assert_eq!(b.local(1, 7), c.local(21, 7));
        // A bone picked as root works too.
        let leg = c.split(6);
        assert_eq!(leg.joints.len(), 4);
    }

    #[test]
    fn auto_tpose_zeroes_rotations_and_root() {
        let c = two_characters().split(20);
        let t = c.auto_tpose(None);
        assert_eq!(t.frames, 1);
        for j in 0..t.joints.len() {
            assert_eq!(t.local(j, 0).rotation, Quat::IDENTITY);
        }
        assert_eq!(t.local(0, 0).translation, Vec3::ZERO);
        // Hips: centred, rest height kept. The animated clip is offset in X.
        assert_eq!(t.local(1, 0).translation, Vec3::new(0.0, 0.95, 0.0));
        assert_eq!(c.auto_tpose(Some(1.1)).local(1, 0).translation, Vec3::new(0.0, 1.1, 0.0));
        // Bone offsets are untouched.
        assert_eq!(t.local(2, 0).translation, c.joints[2].rest.translation);
    }

    #[test]
    fn auto_tpose_uses_joint_orientation() {
        let mut c = create_test_clip(1.0, FrameRate::new(30, 1));
        let q = Quat::from_rotation_z(0.5);
        c.joints[13].zero_rot = q;
        assert_eq!(c.auto_tpose(None).local(13, 0).rotation, q);
    }

    #[test]
    fn pose_fix_rotates_named_joint() {
        let t = create_test_clip(1.0, FrameRate::new(30, 1)).auto_tpose(None);
        let before = t.world_pose(0);
        let fixed = t.pose_fixed(&[PoseEdit {
            joint: "Take01:LeftArm".into(), rotation: [0.0, 0.0, 90.0], translation: [0.0; 3],
        }]);
        let after = fixed.world_pose(0);
        // Forearm hung straight down from the arm; +90 about Z swings it to +X.
        let d = after[14].w_axis.truncate() - after[13].w_axis.truncate();
        assert!((d - Vec3::new(0.28, 0.0, 0.0)).length() < 1e-5, "{d:?}");
        assert_eq!(before[13].w_axis, after[13].w_axis);
        assert_eq!(before[16], after[16]);
    }

    #[test]
    fn proxy_skin_follows_joints() {
        let c = create_test_clip(1.0, FrameRate::new(30, 1));
        let s = c.auto_tpose(None).with_proxy_skin(1.0);
        let skin = s.skin.as_ref().unwrap();
        // 19 spheres of 52 vertices, 18 links of 16.
        assert_eq!(skin.positions.len(), 19 * 52 + 18 * 16);
        assert!(skin.joint.iter().all(|j| (*j as usize) < s.joints.len()));
        // In the bind pose nothing moves.
        let (p, _) = skin.deformed(&s.world_pose(0));
        assert!(p.iter().zip(&skin.positions).all(|(a, b)| (*a - *b).length() < 1e-5));
        // Move the hips: every vertex follows.
        let moved = s.pose_fixed(&[PoseEdit {
            joint: "Take01:Hips".into(), rotation: [0.0; 3], translation: [0.0, 50.0, 0.0],
        }]);
        let (p, _) = skin.deformed(&moved.world_pose(0));
        assert!(p.iter().zip(&skin.positions).all(|(a, b)| ((*a - *b) - Vec3::Y * 0.5).length() < 1e-5));
    }

    #[test]
    fn memo_reuses_result() {
        let a = memo("memo-test", None, || Some(create_test_clip(1.0, FrameRate::new(30, 1)))).unwrap();
        let b = memo("memo-test", None, || panic!("should be cached")).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
