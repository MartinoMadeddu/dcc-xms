//! What is what in a human skeleton.
//!
//! Skeletons come with many naming conventions: Unreal and MetaHuman,
//! Mixamo, HumanIK, 3ds Max Biped, Blender Rigify, Unity and VRM, VRoid,
//! Character Creator, Daz Genesis 8 and 9, SMPL, Xsens, OptiTrack, Rokoko,
//! Kinect, CMU, Roblox, ARKit, Source and others. They also differ in where
//! the joints that twist a limb are: on the limb, between the main joints
//! (in line: HumanIK roll joints, Daz twist joints), or beside them
//! (floating: Unreal twist joints, Character Creator, HumanIK leaf joints).
//!
//! A joint's name is cut into words, its side and the part of the body it
//! names are read from them, and the hierarchy decides the rest: the main
//! joint of a limb is the one the next part hangs from, everything between
//! two main joints is in line with the limb, a twist joint beside it floats.
//! The spine is what lies between the hips and the shoulders, the neck what
//! lies between the shoulders and the head.
//!
//! Anything can be set by hand: a Characterize node puts picks on the clip,
//! and they win over what the names say.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::anim::{AnimData, Joint};

/// The joints a skeleton is characterised by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    Hips, Chest, Neck, Head,
    LeftClavicle, LeftUpperArm, LeftLowerArm, LeftHand,
    RightClavicle, RightUpperArm, RightLowerArm, RightHand,
    LeftThigh, LeftCalf, LeftFoot, LeftToe,
    RightThigh, RightCalf, RightFoot, RightToe,
}

pub const SLOTS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part { Hips, Chest, Neck, Head, Clavicle, UpperArm, LowerArm, Hand, Thigh, Calf, Foot, Toe }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side { Centre, Left, Right }

impl Slot {
    pub const ALL: [Slot; SLOTS] = [
        Slot::Hips, Slot::Chest, Slot::Neck, Slot::Head,
        Slot::LeftClavicle, Slot::LeftUpperArm, Slot::LeftLowerArm, Slot::LeftHand,
        Slot::RightClavicle, Slot::RightUpperArm, Slot::RightLowerArm, Slot::RightHand,
        Slot::LeftThigh, Slot::LeftCalf, Slot::LeftFoot, Slot::LeftToe,
        Slot::RightThigh, Slot::RightCalf, Slot::RightFoot, Slot::RightToe,
    ];

    pub fn index(self) -> usize { Slot::ALL.iter().position(|s| *s == self).unwrap() }

    pub fn of(part: Part, side: Side) -> Option<Slot> {
        Slot::ALL.iter().copied().find(|s| s.part() == part && s.side() == side)
    }

    pub fn part(self) -> Part {
        use Slot::*;
        match self {
            Hips => Part::Hips, Chest => Part::Chest, Neck => Part::Neck, Head => Part::Head,
            LeftClavicle | RightClavicle => Part::Clavicle,
            LeftUpperArm | RightUpperArm => Part::UpperArm,
            LeftLowerArm | RightLowerArm => Part::LowerArm,
            LeftHand | RightHand => Part::Hand,
            LeftThigh | RightThigh => Part::Thigh,
            LeftCalf | RightCalf => Part::Calf,
            LeftFoot | RightFoot => Part::Foot,
            LeftToe | RightToe => Part::Toe,
        }
    }

    pub fn side(self) -> Side {
        use Slot::*;
        match self {
            Hips | Chest | Neck | Head => Side::Centre,
            LeftClavicle | LeftUpperArm | LeftLowerArm | LeftHand | LeftThigh | LeftCalf | LeftFoot | LeftToe => Side::Left,
            _ => Side::Right,
        }
    }

    /// Name in the panel, without the side.
    pub fn label(self) -> &'static str {
        match self.part() {
            Part::Hips => "Hips", Part::Chest => "Chest", Part::Neck => "Neck", Part::Head => "Head",
            Part::Clavicle => "Clavicle", Part::UpperArm => "Upper arm", Part::LowerArm => "Forearm", Part::Hand => "Hand",
            Part::Thigh => "Thigh", Part::Calf => "Calf", Part::Foot => "Foot", Part::Toe => "Toe",
        }
    }
}

/// A slot set by hand, by joint name. An empty name leaves the slot empty.
pub type Picks = Vec<(Slot, String)>;

/// A joint that twists a limb, and where along the limb it sits: 0 at the
/// joint the segment starts from, 1 at the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Twist { pub joint: usize, pub in_line: bool, pub at: f32 }

/// One arm or one leg.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limb {
    pub arm:   bool,
    pub left:  bool,
    /// Clavicle of an arm. Legs have none.
    pub root:  Option<usize>,
    /// Upper arm or thigh, forearm or calf, hand or foot.
    pub upper: Option<usize>,
    pub mid:   Option<usize>,
    pub end:   Option<usize>,
    /// Toe of a leg.
    pub tip:   Option<usize>,
    /// Twist joints of the upper and of the lower segment.
    pub upper_twist: Vec<Twist>,
    pub mid_twist:   Vec<Twist>,
}

impl Limb {
    pub fn complete(&self) -> bool { self.upper.is_some() && self.mid.is_some() && self.end.is_some() }
}

/// The parts of a human skeleton, as joint indices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Human {
    /// The naming convention the skeleton looks like.
    pub convention: &'static str,
    pub slots: [Option<usize>; SLOTS],
    /// From above the hips up to the chest.
    pub spine: Vec<usize>,
    /// From above the chest up to below the head.
    pub neck:  Vec<usize>,
    /// Left arm, right arm, left leg, right leg.
    pub limbs: [Limb; 4],
    /// Slots that were set by hand.
    pub picked: Vec<Slot>,
}

impl Human {
    pub fn get(&self, s: Slot) -> Option<usize> { self.slots[s.index()] }
    /// The same joints in the same parts, however they were found.
    pub fn same_skeleton(&self, o: &Human) -> bool { self.slots == o.slots && self.spine == o.spine && self.neck == o.neck && self.limbs == o.limbs }
    pub fn found(&self) -> usize { self.slots.iter().filter(|s| s.is_some()).count() }
    /// Enough to stand on: hips, and an arm or a leg.
    pub fn usable(&self) -> bool { self.get(Slot::Hips).is_some() && self.limbs.iter().any(|l| l.complete()) }

    /// The skeleton of a clip, with the picks it carries.
    pub fn of(clip: &AnimData) -> Human {
        let picks = clip.human.as_deref().cloned().unwrap_or_default();
        Human::detect(&clip.joints, &picks)
    }

    /// Read a skeleton from its names and hierarchy, then put the picks on it.
    pub fn detect(joints: &[Joint], picks: &Picks) -> Human {
        let n = joints.len();
        let parsed: Vec<Option<Parsed>> = joints.iter().map(|j| if j.is_bone { parse(&j.name) } else { None }).collect();
        // Names of joint places (SMPL, Kinect, some BVH): the shoulder joint
        // turns the upper arm, the hip joint the thigh, the foot joint the toes.
        let positional = parsed.iter().flatten().any(|p| p.word == "elbow")
            && parsed.iter().flatten().any(|p| p.word == "wrist")
            && !parsed.iter().flatten().any(|p| matches!(p.word.as_str(), "forearm" | "lowerarm"));
        let has_ankle = parsed.iter().flatten().any(|p| p.word == "ankle");
        let parts: Vec<Option<(Part, Side, bool)>> = parsed.iter().map(|p| {
            let p = p.as_ref()?;
            let part = part_of(&p.word, p.side, positional, has_ankle)?;
            Some((part, p.side, p.twist))
        }).collect();
        let kids = children(joints);
        let depth = depths(joints);
        let up = |j: usize| -> Vec<usize> {
            let mut out = vec![j];
            let mut c = joints[j].parent;
            while let Some(p) = c { if p >= n || out.contains(&p) { break; } out.push(p); c = joints[p].parent; }
            out
        };
        let is = |j: usize, part: Part, side: Side| matches!(parts[j], Some((p, s, false)) if p == part && s == side);
        let named = |j: &str| -> Option<Option<usize>> {
            if j.is_empty() { return Some(None); }
            joints.iter().position(|x| x.name == j).map(Some)
        };
        let pick = |s: Slot| -> Option<Option<usize>> { picks.iter().rev().find(|p| p.0 == s).and_then(|p| named(&p.1)) };

        let mut h = Human::default();
        // Limbs: from the end up.
        for (k, (arm, side)) in [(true, Side::Left), (true, Side::Right), (false, Side::Left), (false, Side::Right)].into_iter().enumerate() {
            let (pu, pm, pe) = if arm { (Part::UpperArm, Part::LowerArm, Part::Hand) } else { (Part::Thigh, Part::Calf, Part::Foot) };
            let slot = |p: Part| Slot::of(p, side).unwrap();
            let mut best: (usize, Option<usize>, Option<usize>, Option<usize>) = (0, None, None, None);
            for e in 0..n {
                if !is(e, pe, side) { continue; }
                let path = up(e);
                // Highest of each part on the way up, each below the next.
                let topmost = |part: Part, below: usize| path.iter().copied().filter(|j| depth[*j] < depth[below] && is(*j, part, side)).min_by_key(|j| depth[*j]);
                let mid = topmost(pm, e);
                let upper = mid.and_then(|m| topmost(pu, m)).or_else(|| topmost(pu, e));
                // The highest end joint under the middle one: a wrist above a hand.
                let end = path.iter().copied().filter(|j| is(*j, pe, side) && mid.map(|m| depth[*j] > depth[m]).unwrap_or(true)).min_by_key(|j| depth[*j]).unwrap_or(e);
                let score = 1 + mid.is_some() as usize + upper.is_some() as usize;
                if score > best.0 { best = (score, upper, mid, Some(end)); }
            }
            let mut limb = Limb { arm, left: side == Side::Left, upper: best.1, mid: best.2, end: best.3, ..Default::default() };
            if limb.end.is_none() {
                // No end, but maybe the rest.
                limb.mid = (0..n).filter(|j| is(*j, pm, side)).min_by_key(|j| depth[*j]);
                limb.upper = (0..n).filter(|j| is(*j, pu, side)).min_by_key(|j| depth[*j]);
            }
            if let Some(v) = pick(slot(pu)) { limb.upper = v; }
            if let Some(v) = pick(slot(pm)) { limb.mid = v; }
            if let Some(v) = pick(slot(pe)) { limb.end = v; }
            if arm {
                let pr = Part::Clavicle;
                limb.root = limb.upper.and_then(|u| up(u).into_iter().skip(1).filter(|j| is(*j, pr, side)).min_by_key(|j| depth[*j]));
                if let Some(v) = pick(slot(pr)) { limb.root = v; }
            } else {
                limb.tip = limb.end.and_then(|e| descendants(&kids, e).into_iter().filter(|j| *j != e && is(*j, Part::Toe, side)).min_by_key(|j| depth[*j]));
                if let Some(v) = pick(slot(Part::Toe)) { limb.tip = v; }
            }
            h.limbs[k] = limb;
        }

        // The chest: where both arms hang from. The hips: where the legs
        // and the chest hang from, named so if one of them is.
        let lca = |js: &[usize]| -> Option<usize> {
            let first = up(*js.first()?);
            first.into_iter().find(|a| js.iter().all(|j| up(*j).contains(a)))
        };
        let arm_tops: Vec<usize> = h.limbs[..2].iter().filter_map(|l| l.root.or(l.upper)).collect();
        let mut chest = match arm_tops.as_slice() {
            [a, b] => lca(&[*a, *b]),
            [a] => joints[*a].parent,
            _ => None,
        };
        if let Some(v) = pick(Slot::Chest) { chest = v; }
        let mut anchors: Vec<usize> = h.limbs[2..].iter().filter_map(|l| l.upper).collect();
        if let Some(c) = chest { anchors.push(c); }
        let mut hips = None;
        if !anchors.is_empty() {
            let over_all = |j: usize| anchors.iter().all(|a| up(*a).contains(&j));
            hips = (0..n).filter(|j| is(*j, Part::Hips, Side::Centre) && over_all(*j)).max_by_key(|j| depth[*j]);
            if hips.is_none() && anchors.len() >= 2 { hips = lca(&anchors).filter(|j| joints[*j].is_bone || joints[*j].parent.is_some()); }
        }
        if hips.is_none() { hips = (0..n).filter(|j| is(*j, Part::Hips, Side::Centre)).min_by_key(|j| depth[*j]); }
        if let Some(v) = pick(Slot::Hips) { hips = v; }
        // A chest above the hips only.
        if let (Some(c), Some(hp)) = (chest, hips) { if c == hp || !up(c).contains(&hp) { chest = None; } }

        // The head: named so, above the chest. Else the end of the trunk.
        let above_chest = |j: usize| chest.map(|c| j != c && up(j).contains(&c)).unwrap_or(true);
        let limb_joints: Vec<usize> = h.limbs.iter().flat_map(|l| [l.root, l.upper]).flatten().collect();
        let mut head = (0..n).filter(|j| is(*j, Part::Head, Side::Centre) && above_chest(*j)).min_by_key(|j| depth[*j]);
        if head.is_none() {
            if let Some(c) = chest {
                // Follow the trunk up, away from the arms, to its last named bone.
                let mut cur = c;
                loop {
                    let next = kids[cur].iter().copied()
                        .filter(|k| joints[*k].is_bone && !limb_joints.iter().any(|l| up(*l).contains(k)) && !is_end(&joints[*k].name))
                        .max_by_key(|k| descendants(&kids, *k).len());
                    match next { Some(k) => cur = k, None => break }
                }
                if cur != c { head = Some(cur); }
            }
        }
        if let Some(v) = pick(Slot::Head) { head = v; }

        // Spine and neck: what lies between.
        let between = |low: Option<usize>, high: Option<usize>| -> Vec<usize> {
            let (Some(l), Some(hh)) = (low, high) else { return vec![] };
            let path = up(hh);
            let Some(at) = path.iter().position(|j| *j == l).filter(|a| *a > 0) else { return vec![] };
            let mut out: Vec<usize> = path[1..at].iter().copied().filter(|j| joints[*j].is_bone).collect();
            out.reverse();
            out
        };
        h.spine = between(hips, chest);
        if let Some(c) = chest { h.spine.push(c); }
        h.neck = between(chest, head);
        let mut neck = h.neck.first().copied();
        if let Some(v) = pick(Slot::Neck) {
            neck = v;
            // The neck starts at the pick.
            if let Some(v) = v { if let Some(i) = h.neck.iter().position(|j| *j == v) { h.neck.drain(..i); } }
        }

        // Twist joints: in line between main joints, or beside them.
        for limb in h.limbs.iter_mut() {
            let side = if limb.left { Side::Left } else { Side::Right };
            let (pu, pm) = if limb.arm { (Part::UpperArm, Part::LowerArm) } else { (Part::Thigh, Part::Calf) };
            let seg = |from: Option<usize>, to: Option<usize>, part: Part| -> Vec<Twist> {
                let (Some(a), Some(b)) = (from, to) else { return vec![] };
                let line = between(Some(a), Some(b));
                let mut out: Vec<Twist> = line.iter().map(|j| Twist { joint: *j, in_line: true, at: 0.0 }).collect();
                for j in descendants(&kids, a) {
                    if j == a || line.contains(&j) || up(j).contains(&b) { continue; }
                    let Some((p, s, true)) = parts[j] else { continue };
                    if p != part || s != side { continue; }
                    out.push(Twist { joint: j, in_line: false, at: 0.0 });
                }
                out
            };
            limb.upper_twist = seg(limb.upper, limb.mid, pu);
            limb.mid_twist = seg(limb.mid, limb.end, pm);
        }

        h.slots[Slot::Hips.index()] = hips;
        h.slots[Slot::Chest.index()] = chest;
        h.slots[Slot::Neck.index()] = neck;
        h.slots[Slot::Head.index()] = head;
        for limb in &h.limbs {
            let side = if limb.left { Side::Left } else { Side::Right };
            let set = |slots: &mut [Option<usize>; SLOTS], p: Part, v: Option<usize>| slots[Slot::of(p, side).unwrap().index()] = v;
            if limb.arm {
                set(&mut h.slots, Part::Clavicle, limb.root);
                set(&mut h.slots, Part::UpperArm, limb.upper);
                set(&mut h.slots, Part::LowerArm, limb.mid);
                set(&mut h.slots, Part::Hand, limb.end);
            } else {
                set(&mut h.slots, Part::Thigh, limb.upper);
                set(&mut h.slots, Part::Calf, limb.mid);
                set(&mut h.slots, Part::Foot, limb.end);
                set(&mut h.slots, Part::Toe, limb.tip);
            }
        }
        h.picked = Slot::ALL.iter().copied().filter(|s| pick(*s).is_some()).collect();
        h.convention = convention(joints, &h);
        h
    }

    /// Where along its segment each twist joint sits, from the rest pose.
    pub fn place_twists(&mut self, rest: &[bevy::math::Mat4]) {
        let p = |j: usize| rest[j].w_axis.truncate();
        for limb in self.limbs.iter_mut() {
            for (list, a, b) in [(&mut limb.upper_twist, limb.upper, limb.mid), (&mut limb.mid_twist, limb.mid, limb.end)] {
                let (Some(a), Some(b)) = (a, b) else { continue };
                let d = p(b) - p(a);
                let l2 = d.length_squared().max(1e-12);
                for t in list.iter_mut() { t.at = ((p(t.joint) - p(a)).dot(d) / l2).clamp(0.0, 1.0); }
            }
        }
    }

    /// Joints of one skeleton matched to joints of another by what they are:
    /// for each joint of `other`, the joint of `self` that plays its part.
    /// Chains of different lengths (a spine of three against one of five)
    /// are matched end to end.
    pub fn pairs(&self, other: &Human, other_len: usize) -> Vec<Option<usize>> {
        let mut out = vec![None; other_len];
        for s in Slot::ALL {
            if let (Some(a), Some(b)) = (self.get(s), other.get(s)) { if b < other_len { out[b] = Some(a); } }
        }
        let chain = |out: &mut Vec<Option<usize>>, mine: &[usize], theirs: &[usize]| {
            if mine.is_empty() { return; }
            for (k, b) in theirs.iter().enumerate() {
                if out[*b].is_some() { continue; }
                let at = if theirs.len() <= 1 { mine.len() - 1 } else { (k * (mine.len() - 1) + (theirs.len() - 1) / 2) / (theirs.len() - 1) };
                out[*b] = Some(mine[at.min(mine.len() - 1)]);
            }
        };
        chain(&mut out, &self.spine, &other.spine);
        chain(&mut out, &self.neck, &other.neck);
        out
    }
}

/// A clip with its picks.
pub fn with_picks(clip: &AnimData, picks: &Picks) -> AnimData {
    AnimData { human: if picks.is_empty() { None } else { Some(Arc::new(picks.clone())) }, ..clip.clone() }
}

// ── Names ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Parsed { side: Side, word: String, twist: bool }

/// Words of a name: "mixamorig:LeftForeArm" is left, fore, arm;
/// "Bip001 L UpperArm" is bip, 001, l, upper, arm; "lShldrBend" is l, shldr, bend.
pub fn words(name: &str) -> Vec<String> {
    let name = name.rsplit(|c| c == ':' || c == '|').next().unwrap_or(name);
    let mut out = vec![];
    for piece in name.split(|c: char| c == '_' || c == '.' || c == '-' || c == ' ') {
        if piece.is_empty() { continue; }
        let chars: Vec<char> = piece.chars().collect();
        let all_caps = !chars.iter().any(|c| c.is_lowercase());
        let mut cur = String::new();
        for (i, c) in chars.iter().enumerate() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let cut = match prev {
                None => false,
                Some(p) => {
                    (p.is_lowercase() && c.is_uppercase())
                    || (p.is_ascii_digit() != c.is_ascii_digit())
                    || (!all_caps && p.is_uppercase() && c.is_uppercase() && next.map(|x| x.is_lowercase()).unwrap_or(false))
                    // "LUArm", "RHand": a side letter at the front.
                    || (!all_caps && i == 1 && matches!(p, 'L' | 'R') && c.is_uppercase())
                }
            };
            if cut && !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
            cur.push(*c);
        }
        if !cur.is_empty() { out.push(cur); }
    }
    out.into_iter().map(|w| w.to_ascii_lowercase()).collect()
}

/// Words that say nothing about the part: prefixes of conventions, kinds of node.
const NOISE: &[&str] = &[
    "mixamorig", "mixamo", "valve", "biped", "bip", "cc", "base", "j", "def", "character", "jnt", "joint",
    "bind", "bn", "b", "skel", "sk", "rig", "c", "jt", "deform", "bone", "bend", "m", "x",
];

fn parse(name: &str) -> Option<Parsed> {
    // Vertebrae of Xsens and others are spine, not a left side.
    let compact: String = name.rsplit(|c| c == ':' || c == '|').next().unwrap_or(name).to_ascii_lowercase();
    if compact.len() <= 3 && matches!(compact.as_bytes().first(), Some(b'l' | b't' | b'c')) && compact[1..].chars().all(|c| c.is_ascii_digit()) && compact.len() > 1 {
        return Some(Parsed { side: Side::Centre, word: "spine".into(), twist: false });
    }
    let mut side = Side::Centre;
    let mut twist = false;
    let mut word = String::new();
    for w in words(name) {
        match w.as_str() {
            "left" | "l" => side = Side::Left,
            "right" | "r" => side = Side::Right,
            "twist" | "roll" | "leaf" => twist = true,
            w if w.chars().all(|c| c.is_ascii_digit()) => {}
            w if NOISE.contains(&w) => {}
            w => word.push_str(w),
        }
    }
    for (pre, s) in [("left", Side::Left), ("right", Side::Right)] {
        if side == Side::Centre && word.len() > pre.len() && word.starts_with(pre) { side = s; word.drain(..pre.len()); }
    }
    for (post, s) in [("left", Side::Left), ("right", Side::Right)] {
        if side == Side::Centre && word.len() > post.len() && word.ends_with(post) { side = s; word.truncate(word.len() - post.len()); }
    }
    for tail in ["twist", "roll"] {
        if word.len() > tail.len() && word.ends_with(tail) { twist = true; word.truncate(word.len() - tail.len()); }
    }
    if word.is_empty() { return None; }
    // "lhumerus", "rfemur": a side letter glued on.
    if side == Side::Centre && word.len() > 2 && matches!(&word[..1], "l" | "r") && is_limb_word(&word[1..]) && !is_limb_word(&word) {
        side = if word.starts_with('l') { Side::Left } else { Side::Right };
        word.drain(..1);
    }
    Some(Parsed { side, word, twist })
}

fn is_limb_word(w: &str) -> bool {
    [Part::Clavicle, Part::UpperArm, Part::LowerArm, Part::Hand, Part::Thigh, Part::Calf, Part::Foot, Part::Toe]
        .iter().any(|p| part_of(w, Side::Left, false, false) == Some(*p) || part_of(w, Side::Left, true, true) == Some(*p))
}

fn part_of(word: &str, side: Side, positional: bool, has_ankle: bool) -> Option<Part> {
    let sided = side != Side::Centre;
    let part = match word {
        "pelvis" | "hips" | "hip" | "lowertorso" if !sided => Part::Hips,
        "head" if !sided => Part::Head,
        "neck" if !sided => Part::Neck,
        "clavicle" | "collar" => Part::Clavicle,
        "shoulder" if positional => Part::UpperArm,
        "shoulder" => Part::Clavicle,
        "upperarm" | "arm" | "uparm" | "shldr" | "humerus" | "uarm" => Part::UpperArm,
        "lowerarm" | "forearm" | "elbow" | "radius" | "farm" | "fore" => Part::LowerArm,
        "wrist" => Part::Hand,
        "hand" if !positional => Part::Hand,
        "hip" if positional => Part::Thigh,
        "thigh" | "upleg" | "upperleg" | "femur" => Part::Thigh,
        "calf" | "leg" | "lowerleg" | "shin" | "knee" | "tibia" => Part::Calf,
        "ankle" => Part::Foot,
        "foot" if positional && has_ankle => Part::Toe,
        "foot" => Part::Foot,
        "toe" | "toes" | "toebase" | "ball" => Part::Toe,
        _ => return None,
    };
    let limb = !matches!(part, Part::Hips | Part::Chest | Part::Neck | Part::Head);
    (limb == sided).then_some(part)
}

fn is_end(name: &str) -> bool {
    let w = words(name);
    w.iter().any(|x| matches!(x.as_str(), "end" | "nub" | "top" | "site" | "tip"))
}

fn children(joints: &[Joint]) -> Vec<Vec<usize>> {
    let mut kids = vec![vec![]; joints.len()];
    for (j, x) in joints.iter().enumerate() { if let Some(p) = x.parent { if p < joints.len() && p != j { kids[p].push(j); } } }
    kids
}

fn depths(joints: &[Joint]) -> Vec<usize> {
    let mut d = vec![0usize; joints.len()];
    for j in 0..joints.len() { if let Some(p) = joints[j].parent { if p < j { d[j] = d[p] + 1; } } }
    d
}

fn descendants(kids: &[Vec<usize>], j: usize) -> Vec<usize> {
    let mut out = vec![j];
    let mut i = 0;
    while i < out.len() { let c = out[i]; out.extend(kids[c].iter().copied()); i += 1; }
    out
}

/// Which convention a skeleton follows, by the names it has.
fn convention(joints: &[Joint], h: &Human) -> &'static str {
    let names: Vec<String> = joints.iter().map(|j| j.name.to_ascii_lowercase()).collect();
    let short: Vec<&str> = names.iter().map(|n| n.rsplit(|c| c == ':' || c == '|').next().unwrap_or(n)).collect();
    let any = |f: &dyn Fn(&str) -> bool| short.iter().any(|n| f(n)) || names.iter().any(|n| f(n));
    let has = |s: &str| any(&|n: &str| n.contains(s));
    let is = |s: &str| short.iter().any(|n| *n == s);
    if has("mixamorig") { return "Mixamo"; }
    if has("valvebiped") { return "Source (ValveBiped)"; }
    if any(&|n: &str| n.starts_with("bip0")) { return "3ds Max Biped"; }
    if has("cc_base_") { return "Character Creator"; }
    if has("j_bip_") { return "VRoid"; }
    if any(&|n: &str| n.starts_with("def-")) { return "Blender Rigify"; }
    if any(&|n: &str| n.ends_with("_jnt")) { return "DeepMotion"; }
    if has("shldrbend") { return "Daz Genesis 8"; }
    if is("l_upperarm") && is("l_shin") { return "Daz Genesis 9"; }
    if is("upperarm_l") && is("spine_05") { return "Unreal 5 / MetaHuman"; }
    if is("upperarm_l") && is("calf_l") { return "Unreal (UE4)"; }
    if is("lowertorso") && is("uppertorso") { return "Roblox R15"; }
    if is("hips_joint") { return "Apple ARKit"; }
    if is("left_elbow") && is("left_wrist") { return "SMPL"; }
    if is("elbowleft") { return "Kinect"; }
    if is("elbow_left") { return "Azure Kinect"; }
    if is("t8") && is("l5") { return "Xsens MVN"; }
    if is("luarm") { return "OptiTrack Motive"; }
    if is("lhumerus") { return "CMU (ASF)"; }
    if is("lshldr") { return "CMU (BVH)"; }
    if is("lhipjoint") { return "CMU (BVH)"; }
    if is("upperarm01.l") { return "MakeHuman"; }
    if is("upperarm_l") && is("upperleg_l") { return "Bandai Namco"; }
    if has("leftarmroll") || has("leafleft") { return "HumanIK"; }
    if is("leftupperarm") && is("leftupleg") { return "Rokoko"; }
    if is("leftupperarm") && is("leftupperleg") { return "Unity / VRM humanoid"; }
    if is("leftforearm") && is("lefttoe") { return "LaFAN1"; }
    if is("leftforearm") && is("leftupleg") { return "HumanIK / Mixamo names"; }
    if h.usable() { return "Read from part names"; }
    "Not recognised"
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::Transform;

    /// A skeleton from (name, parent name) pairs.
    fn skeleton(defs: &[(&str, &str)]) -> Vec<Joint> {
        let mut out: Vec<Joint> = vec![];
        for (name, parent) in defs {
            let p = if parent.is_empty() { None } else { Some(out.iter().position(|j| j.name == *parent).unwrap_or_else(|| panic!("{parent} before {name}"))) };
            out.push(Joint::new(name.to_string(), p, Transform::IDENTITY));
        }
        out
    }

    fn name(js: &[Joint], j: Option<usize>) -> &str { j.map(|j| js[j].name.as_str()).unwrap_or("-") }

    /// Every slot, as joint names, in Slot::ALL order.
    fn check(conv: &str, js: &[Joint], want: [&str; SLOTS]) -> Human {
        let h = Human::detect(js, &vec![]);
        for (s, w) in Slot::ALL.iter().zip(want) {
            assert_eq!(name(js, h.get(*s)), w, "{conv}: {s:?}");
        }
        assert_eq!(h.convention, conv);
        h
    }

    /// A biped from a name for each slot and extras in between, by template.
    fn chain(defs: &[(&str, &str)]) -> Vec<Joint> { skeleton(defs) }

    #[test]
    fn words_are_cut_at_case_digits_and_marks() {
        assert_eq!(words("mixamorig:LeftForeArm"), ["left", "fore", "arm"]);
        assert_eq!(words("Bip001 L UpperArm"), ["bip", "001", "l", "upper", "arm"]);
        assert_eq!(words("lShldrBend"), ["l", "shldr", "bend"]);
        assert_eq!(words("LUArm"), ["l", "u", "arm"]);
        assert_eq!(words("CC_Base_L_Upperarm"), ["cc", "base", "l", "upperarm"]);
        assert_eq!(words("DEF-upper_arm.L.001"), ["def", "upper", "arm", "l", "001"]);
        assert_eq!(words("SHOULDER_LEFT"), ["shoulder", "left"]);
        assert_eq!(words("upperarm_twistCor_01_r"), ["upperarm", "twist", "cor", "01", "r"]);
    }

    #[test]
    fn unreal_mannequin_and_metahuman() {
        let js = chain(&[
            ("root", ""), ("pelvis", "root"), ("spine_01", "pelvis"), ("spine_02", "spine_01"), ("spine_03", "spine_02"),
            ("spine_04", "spine_03"), ("spine_05", "spine_04"), ("neck_01", "spine_05"), ("neck_02", "neck_01"), ("head", "neck_02"),
            ("FACIAL_C_FacialRoot", "head"),
            ("clavicle_l", "spine_05"), ("upperarm_l", "clavicle_l"), ("upperarm_correctiveRoot_l", "upperarm_l"), ("upperarm_bck_l", "upperarm_correctiveRoot_l"),
            ("lowerarm_l", "upperarm_l"), ("hand_l", "lowerarm_l"), ("index_metacarpal_l", "hand_l"), ("index_01_l", "index_metacarpal_l"),
            ("wrist_inner_l", "hand_l"), ("lowerarm_twist_02_l", "lowerarm_l"), ("lowerarm_twist_01_l", "lowerarm_l"), ("lowerarm_in_l", "lowerarm_l"),
            ("upperarm_twist_01_l", "upperarm_l"), ("upperarm_twistCor_01_l", "upperarm_twist_01_l"), ("upperarm_twist_02_l", "upperarm_l"),
            ("clavicle_pec_l", "spine_05"), ("spine_04_latissimus_l", "spine_04"),
            ("clavicle_r", "spine_05"), ("upperarm_r", "clavicle_r"), ("lowerarm_r", "upperarm_r"), ("hand_r", "lowerarm_r"),
            ("thigh_l", "pelvis"), ("calf_l", "thigh_l"), ("foot_l", "calf_l"), ("ball_l", "foot_l"), ("bigtoe_01_l", "ball_l"),
            ("calf_twist_01_l", "calf_l"), ("calf_knee_l", "calf_l"), ("thigh_twist_01_l", "thigh_l"), ("thigh_twistCor_01_l", "thigh_twist_01_l"),
            ("thigh_r", "pelvis"), ("calf_r", "thigh_r"), ("foot_r", "calf_r"), ("ball_r", "foot_r"),
            ("ik_foot_root", "root"), ("ik_foot_l", "ik_foot_root"), ("ik_hand_root", "root"), ("ik_hand_l", "ik_hand_root"),
        ]);
        let h = check("Unreal 5 / MetaHuman", &js, [
            "pelvis", "spine_05", "neck_01", "head",
            "clavicle_l", "upperarm_l", "lowerarm_l", "hand_l", "clavicle_r", "upperarm_r", "lowerarm_r", "hand_r",
            "thigh_l", "calf_l", "foot_l", "ball_l", "thigh_r", "calf_r", "foot_r", "ball_r",
        ]);
        let names = |v: &[usize]| v.iter().map(|j| js[*j].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&h.spine), ["spine_01", "spine_02", "spine_03", "spine_04", "spine_05"]);
        assert_eq!(names(&h.neck), ["neck_01", "neck_02"]);
        // Floating twist joints, beside the limb.
        let tw = |v: &[Twist]| v.iter().map(|t| (js[t.joint].name.clone(), t.in_line)).collect::<Vec<_>>();
        assert_eq!(tw(&h.limbs[0].upper_twist), [("upperarm_twist_01_l".to_string(), false), ("upperarm_twist_02_l".to_string(), false)]);
        assert_eq!(tw(&h.limbs[0].mid_twist), [("lowerarm_twist_02_l".to_string(), false), ("lowerarm_twist_01_l".to_string(), false)]);
        assert_eq!(tw(&h.limbs[2].upper_twist), [("thigh_twist_01_l".to_string(), false)]);
        assert_eq!(tw(&h.limbs[2].mid_twist), [("calf_twist_01_l".to_string(), false)]);
    }

    #[test]
    fn humanik_with_roll_joints_in_line_and_leaf_joints_beside() {
        let js = chain(&[
            ("Character1_Reference", ""), ("Character1_Hips", "Character1_Reference"),
            ("Character1_Spine", "Character1_Hips"), ("Character1_Spine1", "Character1_Spine"), ("Character1_Spine2", "Character1_Spine1"),
            ("Character1_Neck", "Character1_Spine2"), ("Character1_Head", "Character1_Neck"),
            ("Character1_LeftShoulder", "Character1_Spine2"), ("Character1_LeftArm", "Character1_LeftShoulder"),
            ("Character1_LeftArmRoll", "Character1_LeftArm"), ("Character1_LeftForeArm", "Character1_LeftArmRoll"),
            ("Character1_LeftForeArmRoll", "Character1_LeftForeArm"), ("Character1_LeftHand", "Character1_LeftForeArmRoll"),
            ("Character1_LeafLeftArmRoll1", "Character1_LeftArm"),
            ("Character1_RightShoulder", "Character1_Spine2"), ("Character1_RightArm", "Character1_RightShoulder"),
            ("Character1_RightForeArm", "Character1_RightArm"), ("Character1_RightHand", "Character1_RightForeArm"),
            ("Character1_LeftUpLeg", "Character1_Hips"), ("Character1_LeftUpLegRoll", "Character1_LeftUpLeg"), ("Character1_LeftLeg", "Character1_LeftUpLegRoll"),
            ("Character1_LeftFoot", "Character1_LeftLeg"), ("Character1_LeftToeBase", "Character1_LeftFoot"),
            ("Character1_RightUpLeg", "Character1_Hips"), ("Character1_RightLeg", "Character1_RightUpLeg"),
            ("Character1_RightFoot", "Character1_RightLeg"), ("Character1_RightToeBase", "Character1_RightFoot"),
        ]);
        let h = check("HumanIK", &js, [
            "Character1_Hips", "Character1_Spine2", "Character1_Neck", "Character1_Head",
            "Character1_LeftShoulder", "Character1_LeftArm", "Character1_LeftForeArm", "Character1_LeftHand",
            "Character1_RightShoulder", "Character1_RightArm", "Character1_RightForeArm", "Character1_RightHand",
            "Character1_LeftUpLeg", "Character1_LeftLeg", "Character1_LeftFoot", "Character1_LeftToeBase",
            "Character1_RightUpLeg", "Character1_RightLeg", "Character1_RightFoot", "Character1_RightToeBase",
        ]);
        let tw = |v: &[Twist]| v.iter().map(|t| (js[t.joint].name.as_str(), t.in_line)).collect::<Vec<_>>();
        assert_eq!(tw(&h.limbs[0].upper_twist), [("Character1_LeftArmRoll", true), ("Character1_LeafLeftArmRoll1", false)]);
        assert_eq!(tw(&h.limbs[0].mid_twist), [("Character1_LeftForeArmRoll", true)]);
        assert_eq!(tw(&h.limbs[2].upper_twist), [("Character1_LeftUpLegRoll", true)]);
    }

    /// The usual biped, in one convention's names: hips, spine..., neck,
    /// head, then for each side clavicle, upper arm, forearm, hand, thigh,
    /// calf, foot, toe. `extra` joints go in after their parent.
    fn biped(names: [&str; 20], spine: &[&str], extra: &[(&str, &str)]) -> Vec<Joint> {
        let [hips, chest, neck, head, lc, lu, ll, lh, rc, ru, rl, rh, lt, lk, lf, lo, rt, rk, rf, ro] = names;
        let mut defs: Vec<(&str, &str)> = vec![(hips, "")];
        let mut prev = hips;
        for s in spine { defs.push((s, prev)); prev = s; }
        defs.push((chest, prev));
        defs.push((neck, chest));
        defs.push((head, neck));
        for (c, u, l, h) in [(lc, lu, ll, lh), (rc, ru, rl, rh)] {
            let top = if c.is_empty() { chest } else { defs.push((c, chest)); c };
            defs.extend([(u, top), (l, u), (h, l)]);
        }
        for (t, k, f, o) in [(lt, lk, lf, lo), (rt, rk, rf, ro)] { defs.extend([(t, hips), (k, t), (f, k), (o, f)]); }
        let mut joints = skeleton(&defs);
        for (n, p) in extra {
            let pi = joints.iter().position(|j| j.name == *p).unwrap();
            joints.push(Joint::new(n.to_string(), Some(pi), Transform::IDENTITY));
        }
        joints
    }

    #[test]
    fn the_common_conventions_are_read_from_their_names() {
        let cases: Vec<(&str, [&str; 20], Vec<&str>)> = vec![
            ("Mixamo", ["mixamorig:Hips", "mixamorig:Spine2", "mixamorig:Neck", "mixamorig:Head",
                "mixamorig:LeftShoulder", "mixamorig:LeftArm", "mixamorig:LeftForeArm", "mixamorig:LeftHand",
                "mixamorig:RightShoulder", "mixamorig:RightArm", "mixamorig:RightForeArm", "mixamorig:RightHand",
                "mixamorig:LeftUpLeg", "mixamorig:LeftLeg", "mixamorig:LeftFoot", "mixamorig:LeftToeBase",
                "mixamorig:RightUpLeg", "mixamorig:RightLeg", "mixamorig:RightFoot", "mixamorig:RightToeBase"],
                vec!["mixamorig:Spine", "mixamorig:Spine1"]),
            ("Unity / VRM humanoid", ["Hips", "UpperChest", "Neck", "Head", "LeftShoulder", "LeftUpperArm", "LeftLowerArm", "LeftHand",
                "RightShoulder", "RightUpperArm", "RightLowerArm", "RightHand", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "LeftToes",
                "RightUpperLeg", "RightLowerLeg", "RightFoot", "RightToes"], vec!["Spine", "Chest"]),
            ("Unity / VRM humanoid", ["hips", "upperChest", "neck", "head", "leftShoulder", "leftUpperArm", "leftLowerArm", "leftHand",
                "rightShoulder", "rightUpperArm", "rightLowerArm", "rightHand", "leftUpperLeg", "leftLowerLeg", "leftFoot", "leftToes",
                "rightUpperLeg", "rightLowerLeg", "rightFoot", "rightToes"], vec!["spine", "chest"]),
            ("3ds Max Biped", ["Bip001 Pelvis", "Bip001 Spine2", "Bip001 Neck", "Bip001 Head",
                "Bip001 L Clavicle", "Bip001 L UpperArm", "Bip001 L Forearm", "Bip001 L Hand",
                "Bip001 R Clavicle", "Bip001 R UpperArm", "Bip001 R Forearm", "Bip001 R Hand",
                "Bip001 L Thigh", "Bip001 L Calf", "Bip001 L Foot", "Bip001 L Toe0",
                "Bip001 R Thigh", "Bip001 R Calf", "Bip001 R Foot", "Bip001 R Toe0"], vec!["Bip001 Spine", "Bip001 Spine1"]),
            ("Blender Rigify", ["DEF-spine", "DEF-spine.003", "DEF-spine.004", "DEF-spine.006",
                "DEF-shoulder.L", "DEF-upper_arm.L", "DEF-forearm.L", "DEF-hand.L",
                "DEF-shoulder.R", "DEF-upper_arm.R", "DEF-forearm.R", "DEF-hand.R",
                "DEF-thigh.L", "DEF-shin.L", "DEF-foot.L", "DEF-toe.L",
                "DEF-thigh.R", "DEF-shin.R", "DEF-foot.R", "DEF-toe.R"], vec!["DEF-spine.001", "DEF-spine.002"]),
            ("Character Creator", ["CC_Base_Hip", "CC_Base_Spine02", "CC_Base_NeckTwist01", "CC_Base_Head",
                "CC_Base_L_Clavicle", "CC_Base_L_Upperarm", "CC_Base_L_Forearm", "CC_Base_L_Hand",
                "CC_Base_R_Clavicle", "CC_Base_R_Upperarm", "CC_Base_R_Forearm", "CC_Base_R_Hand",
                "CC_Base_L_Thigh", "CC_Base_L_Calf", "CC_Base_L_Foot", "CC_Base_L_ToeBase",
                "CC_Base_R_Thigh", "CC_Base_R_Calf", "CC_Base_R_Foot", "CC_Base_R_ToeBase"], vec!["CC_Base_Waist", "CC_Base_Spine01"]),
            ("VRoid", ["J_Bip_C_Hips", "J_Bip_C_UpperChest", "J_Bip_C_Neck", "J_Bip_C_Head",
                "J_Bip_L_Shoulder", "J_Bip_L_UpperArm", "J_Bip_L_LowerArm", "J_Bip_L_Hand",
                "J_Bip_R_Shoulder", "J_Bip_R_UpperArm", "J_Bip_R_LowerArm", "J_Bip_R_Hand",
                "J_Bip_L_UpperLeg", "J_Bip_L_LowerLeg", "J_Bip_L_Foot", "J_Bip_L_ToeBase",
                "J_Bip_R_UpperLeg", "J_Bip_R_LowerLeg", "J_Bip_R_Foot", "J_Bip_R_ToeBase"], vec!["J_Bip_C_Spine", "J_Bip_C_Chest"]),
            ("Daz Genesis 9", ["hip", "spine4", "neck1", "head", "l_shoulder", "l_upperarm", "l_forearm", "l_hand",
                "r_shoulder", "r_upperarm", "r_forearm", "r_hand", "l_thigh", "l_shin", "l_foot", "l_toes",
                "r_thigh", "r_shin", "r_foot", "r_toes"], vec!["spine1", "spine2", "spine3"]),
            ("SMPL", ["pelvis", "spine3", "neck", "head", "left_collar", "left_shoulder", "left_elbow", "left_wrist",
                "right_collar", "right_shoulder", "right_elbow", "right_wrist", "left_hip", "left_knee", "left_ankle", "left_foot",
                "right_hip", "right_knee", "right_ankle", "right_foot"], vec!["spine1", "spine2"]),
            ("Xsens MVN", ["Pelvis", "T8", "Neck", "Head", "LeftShoulder", "LeftUpperArm", "LeftForeArm", "LeftHand",
                "RightShoulder", "RightUpperArm", "RightForeArm", "RightHand", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "LeftToe",
                "RightUpperLeg", "RightLowerLeg", "RightFoot", "RightToe"], vec!["L5", "L3", "T12"]),
            ("OptiTrack Motive", ["Hip", "Chest", "Neck", "Head", "LShoulder", "LUArm", "LFArm", "LHand",
                "RShoulder", "RUArm", "RFArm", "RHand", "LThigh", "LShin", "LFoot", "LToe", "RThigh", "RShin", "RFoot", "RToe"], vec!["Ab"]),
            ("Rokoko", ["hip", "chest", "neck", "head", "leftShoulder", "leftUpperArm", "leftLowerArm", "leftHand",
                "rightShoulder", "rightUpperArm", "rightLowerArm", "rightHand", "leftUpLeg", "leftLeg", "leftFoot", "leftToe",
                "rightUpLeg", "rightLeg", "rightFoot", "rightToe"], vec!["spine"]),
            ("Apple ARKit", ["hips_joint", "spine_7_joint", "neck_1_joint", "head_joint",
                "left_shoulder_1_joint", "left_arm_joint", "left_forearm_joint", "left_hand_joint",
                "right_shoulder_1_joint", "right_arm_joint", "right_forearm_joint", "right_hand_joint",
                "left_upLeg_joint", "left_leg_joint", "left_foot_joint", "left_toes_joint",
                "right_upLeg_joint", "right_leg_joint", "right_foot_joint", "right_toes_joint"], vec!["spine_1_joint", "spine_4_joint"]),
            ("Source (ValveBiped)", ["ValveBiped.Bip01_Pelvis", "ValveBiped.Bip01_Spine4", "ValveBiped.Bip01_Neck1", "ValveBiped.Bip01_Head1",
                "ValveBiped.Bip01_L_Clavicle", "ValveBiped.Bip01_L_UpperArm", "ValveBiped.Bip01_L_Forearm", "ValveBiped.Bip01_L_Hand",
                "ValveBiped.Bip01_R_Clavicle", "ValveBiped.Bip01_R_UpperArm", "ValveBiped.Bip01_R_Forearm", "ValveBiped.Bip01_R_Hand",
                "ValveBiped.Bip01_L_Thigh", "ValveBiped.Bip01_L_Calf", "ValveBiped.Bip01_L_Foot", "ValveBiped.Bip01_L_Toe0",
                "ValveBiped.Bip01_R_Thigh", "ValveBiped.Bip01_R_Calf", "ValveBiped.Bip01_R_Foot", "ValveBiped.Bip01_R_Toe0"], vec!["ValveBiped.Bip01_Spine"]),
            ("Bandai Namco", ["Hips", "Chest", "Neck", "Head", "Shoulder_L", "UpperArm_L", "LowerArm_L", "Hand_L",
                "Shoulder_R", "UpperArm_R", "LowerArm_R", "Hand_R", "UpperLeg_L", "LowerLeg_L", "Foot_L", "Toes_L",
                "UpperLeg_R", "LowerLeg_R", "Foot_R", "Toes_R"], vec!["Spine"]),
            ("Roblox R15", ["LowerTorso", "UpperTorso", "Neck", "Head", "", "LeftUpperArm", "LeftLowerArm", "LeftHand",
                "", "RightUpperArm", "RightLowerArm", "RightHand", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "LeftToe",
                "RightUpperLeg", "RightLowerLeg", "RightFoot", "RightToe"], vec![]),
            ("Azure Kinect", ["PELVIS", "SPINE_CHEST", "NECK", "HEAD", "CLAVICLE_LEFT", "SHOULDER_LEFT", "ELBOW_LEFT", "WRIST_LEFT",
                "CLAVICLE_RIGHT", "SHOULDER_RIGHT", "ELBOW_RIGHT", "WRIST_RIGHT", "HIP_LEFT", "KNEE_LEFT", "ANKLE_LEFT", "FOOT_LEFT",
                "HIP_RIGHT", "KNEE_RIGHT", "ANKLE_RIGHT", "FOOT_RIGHT"], vec!["SPINE_NAVAL"]),
            ("CMU (ASF)", ["root", "upperback", "lowerneck", "head", "lclavicle", "lhumerus", "lradius", "lwrist",
                "rclavicle", "rhumerus", "rradius", "rwrist", "lfemur", "ltibia", "lfoot", "ltoes",
                "rfemur", "rtibia", "rfoot", "rtoes"], vec!["lowerback"]),
            ("CMU (BVH)", ["hip", "chest", "neck", "head", "lCollar", "lShldr", "lForeArm", "lHand",
                "rCollar", "rShldr", "rForeArm", "rHand", "lThigh", "lShin", "lFoot", "lToe",
                "rThigh", "rShin", "rFoot", "rToe"], vec!["abdomen"]),
            ("DeepMotion", ["hips_JNT", "spine2_JNT", "neck_JNT", "head_JNT", "l_shoulder_JNT", "l_arm_JNT", "l_forearm_JNT", "l_hand_JNT",
                "r_shoulder_JNT", "r_arm_JNT", "r_forearm_JNT", "r_hand_JNT", "l_upleg_JNT", "l_leg_JNT", "l_foot_JNT", "l_toebase_JNT",
                "r_upleg_JNT", "r_leg_JNT", "r_foot_JNT", "r_toebase_JNT"], vec!["spine_JNT", "spine1_JNT"]),
            ("LaFAN1", ["Hips", "Spine2", "Neck", "Head", "LeftShoulder", "LeftArm", "LeftForeArm", "LeftHand",
                "RightShoulder", "RightArm", "RightForeArm", "RightHand", "LeftUpLeg", "LeftLeg", "LeftFoot", "LeftToe",
                "RightUpLeg", "RightLeg", "RightFoot", "RightToe"], vec!["Spine", "Spine1"]),
        ];
        for (conv, names, spine) in cases {
            let js = biped(names, &spine, &[]);
            let want: [&str; SLOTS] = std::array::from_fn(|k| if names[k].is_empty() { "-" } else { names[k] });
            let h = check(conv, &js, want);
            assert_eq!(h.spine.len(), spine.len() + 1, "{conv}");
        }
    }

    #[test]
    fn hips_are_where_legs_and_spine_meet_and_twists_in_line_are_found() {
        // Daz Genesis 8: the hip carries the spine, the pelvis only the legs;
        // the bend joints are the limb, the twist joints are in line.
        let js = skeleton(&[
            ("hip", ""), ("pelvis", "hip"), ("abdomenLower", "hip"), ("abdomenUpper", "abdomenLower"), ("chestLower", "abdomenUpper"),
            ("chestUpper", "chestLower"), ("neckLower", "chestUpper"), ("neckUpper", "neckLower"), ("head", "neckUpper"),
            ("lCollar", "chestUpper"), ("lShldrBend", "lCollar"), ("lShldrTwist", "lShldrBend"), ("lForearmBend", "lShldrTwist"),
            ("lForearmTwist", "lForearmBend"), ("lHand", "lForearmTwist"),
            ("rCollar", "chestUpper"), ("rShldrBend", "rCollar"), ("rShldrTwist", "rShldrBend"), ("rForearmBend", "rShldrTwist"),
            ("rForearmTwist", "rForearmBend"), ("rHand", "rForearmTwist"),
            ("lThighBend", "pelvis"), ("lThighTwist", "lThighBend"), ("lShin", "lThighTwist"), ("lFoot", "lShin"), ("lMetatarsals", "lFoot"), ("lToe", "lMetatarsals"),
            ("rThighBend", "pelvis"), ("rThighTwist", "rThighBend"), ("rShin", "rThighTwist"), ("rFoot", "rShin"), ("rToe", "rFoot"),
        ]);
        let h = check("Daz Genesis 8", &js, [
            "hip", "chestUpper", "neckLower", "head", "lCollar", "lShldrBend", "lForearmBend", "lHand",
            "rCollar", "rShldrBend", "rForearmBend", "rHand", "lThighBend", "lShin", "lFoot", "lToe", "rThighBend", "rShin", "rFoot", "rToe",
        ]);
        assert_eq!(h.spine.len(), 4);
        let tw = |v: &[Twist]| v.iter().map(|t| (js[t.joint].name.as_str(), t.in_line)).collect::<Vec<_>>();
        assert_eq!(tw(&h.limbs[0].upper_twist), [("lShldrTwist", true)]);
        assert_eq!(tw(&h.limbs[0].mid_twist), [("lForearmTwist", true)]);
        assert_eq!(tw(&h.limbs[2].upper_twist), [("lThighTwist", true)]);

        // MakeHuman: numbered segments in line, a root that is the hips.
        let js = skeleton(&[
            ("root", ""), ("spine05", "root"), ("spine04", "spine05"), ("spine03", "spine04"), ("neck01", "spine03"), ("head", "neck01"),
            ("clavicle.L", "spine03"), ("shoulder01.L", "clavicle.L"), ("upperarm01.L", "shoulder01.L"), ("upperarm02.L", "upperarm01.L"),
            ("lowerarm01.L", "upperarm02.L"), ("lowerarm02.L", "lowerarm01.L"), ("wrist.L", "lowerarm02.L"),
            ("clavicle.R", "spine03"), ("shoulder01.R", "clavicle.R"), ("upperarm01.R", "shoulder01.R"), ("lowerarm01.R", "upperarm01.R"), ("wrist.R", "lowerarm01.R"),
            ("pelvis.L", "root"), ("upperleg01.L", "pelvis.L"), ("upperleg02.L", "upperleg01.L"), ("lowerleg01.L", "upperleg02.L"), ("foot.L", "lowerleg01.L"), ("toe1-1.L", "foot.L"),
            ("pelvis.R", "root"), ("upperleg01.R", "pelvis.R"), ("lowerleg01.R", "upperleg01.R"), ("foot.R", "lowerleg01.R"),
        ]);
        let h = check("MakeHuman", &js, [
            "root", "spine03", "neck01", "head", "clavicle.L", "upperarm01.L", "lowerarm01.L", "wrist.L",
            "clavicle.R", "upperarm01.R", "lowerarm01.R", "wrist.R", "upperleg01.L", "lowerleg01.L", "foot.L", "toe1-1.L",
            "upperleg01.R", "lowerleg01.R", "foot.R", "-",
        ]);
        let tw = |v: &[Twist]| v.iter().map(|t| (js[t.joint].name.as_str(), t.in_line)).collect::<Vec<_>>();
        assert_eq!(tw(&h.limbs[0].upper_twist), [("upperarm02.L", true)]);
        assert_eq!(tw(&h.limbs[0].mid_twist), [("lowerarm02.L", true)]);
    }

    #[test]
    fn biped_twist_joints_float_beside_the_limb() {
        let js = biped(["Bip001 Pelvis", "Bip001 Spine1", "Bip001 Neck", "Bip001 Head",
            "Bip001 L Clavicle", "Bip001 L UpperArm", "Bip001 L Forearm", "Bip001 L Hand",
            "Bip001 R Clavicle", "Bip001 R UpperArm", "Bip001 R Forearm", "Bip001 R Hand",
            "Bip001 L Thigh", "Bip001 L Calf", "Bip001 L Foot", "Bip001 L Toe0",
            "Bip001 R Thigh", "Bip001 R Calf", "Bip001 R Foot", "Bip001 R Toe0"], &["Bip001 Spine"],
            &[("Bip001 LUpArmTwist", "Bip001 L UpperArm"), ("Bip001 L ForeTwist", "Bip001 L Forearm"), ("Bip001 L Finger0", "Bip001 L Hand")]);
        let h = Human::detect(&js, &vec![]);
        assert_eq!(h.limbs[0].upper_twist.iter().map(|t| js[t.joint].name.as_str()).collect::<Vec<_>>(), ["Bip001 LUpArmTwist"]);
        assert_eq!(h.limbs[0].mid_twist.iter().map(|t| js[t.joint].name.as_str()).collect::<Vec<_>>(), ["Bip001 L ForeTwist"]);
        assert!(h.limbs.iter().all(|l| l.upper_twist.iter().chain(&l.mid_twist).all(|t| !t.in_line)));
    }

    #[test]
    fn picks_win_and_unknown_names_can_be_set_by_hand() {
        // Names that say nothing: nothing is found until it is picked.
        let names = ["b0", "b4", "b5", "b6", "b7", "b8", "b9", "b10", "b11", "b12", "b13", "b14", "b15", "b16", "b17", "b18", "b19", "b20", "b21", "b22"];
        let js = biped(names, &["b1", "b2", "b3"], &[]);
        let h = Human::detect(&js, &vec![]);
        assert!(!h.usable());
        assert_eq!(h.convention, "Not recognised");
        let picks: Picks = Slot::ALL.iter().zip(names).filter(|(s, _)| !matches!(s, Slot::Chest | Slot::Neck)).map(|(s, n)| (*s, n.to_string())).collect();
        let h = Human::detect(&js, &picks);
        for (s, n) in Slot::ALL.iter().zip(names) { assert_eq!(name(&js, h.get(*s)), n, "{s:?}"); }
        assert_eq!(h.spine.len(), 4);
        assert_eq!(h.picked.len(), 18);
        // An empty pick empties a slot found by name.
        let js = biped(["Hips", "Chest", "Neck", "Head", "LeftShoulder", "LeftArm", "LeftForeArm", "LeftHand",
            "RightShoulder", "RightArm", "RightForeArm", "RightHand", "LeftUpLeg", "LeftLeg", "LeftFoot", "LeftToe",
            "RightUpLeg", "RightLeg", "RightFoot", "RightToe"], &["Spine"], &[]);
        let h = Human::detect(&js, &vec![(Slot::LeftToe, String::new()), (Slot::RightHand, "RightForeArm".into())]);
        assert_eq!(h.get(Slot::LeftToe), None);
        assert_eq!(name(&js, h.get(Slot::RightHand)), "RightForeArm");
    }

    #[test]
    fn chains_of_different_lengths_are_matched_end_to_end() {
        let a = biped(["Hips", "Chest", "Neck", "Head", "LeftShoulder", "LeftArm", "LeftForeArm", "LeftHand",
            "RightShoulder", "RightArm", "RightForeArm", "RightHand", "LeftUpLeg", "LeftLeg", "LeftFoot", "LeftToe",
            "RightUpLeg", "RightLeg", "RightFoot", "RightToe"], &["Spine"], &[]);
        let b = biped(["pelvis", "spine_05", "neck_01", "head", "clavicle_l", "upperarm_l", "lowerarm_l", "hand_l",
            "clavicle_r", "upperarm_r", "lowerarm_r", "hand_r", "thigh_l", "calf_l", "foot_l", "ball_l",
            "thigh_r", "calf_r", "foot_r", "ball_r"], &["spine_01", "spine_02", "spine_03", "spine_04"], &[]);
        let (ha, hb) = (Human::detect(&a, &vec![]), Human::detect(&b, &vec![]));
        let pairs = ha.pairs(&hb, b.len());
        let of = |n: &str| pairs[b.iter().position(|j| j.name == n).unwrap()].map(|i| a[i].name.as_str());
        assert_eq!(of("upperarm_l"), Some("LeftArm"));
        assert_eq!(of("ball_r"), Some("RightToe"));
        assert_eq!(of("spine_05"), Some("Chest"));
        assert_eq!(of("spine_01"), Some("Spine"));
        assert!(pairs.iter().all(|p| p.is_some()));
    }
}
