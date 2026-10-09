//! Which part of a human a joint is, from its name, and how that part may
//! move away from the capture.

/// Parts of the body that get a rigid body of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role { Pelvis, Spine, Neck, Head, Clavicle, UpperArm, LowerArm, Hand, Thigh, Calf, Foot, Toe, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side { Centre, Left, Right }

/// How one part behaves.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Share of the way back to the capture that is covered in one frame,
    /// for the place of the part and for its turn.
    pub follow:      f32,
    pub follow_turn: f32,
    /// How far the joint may swing and twist away from the capture, degrees.
    pub swing:       f32,
    pub twist:       f32,
    /// The joint above this part bends about one axis only.
    pub hinge:       bool,
    /// Blocking this part blocks the whole character: it cannot step aside.
    pub core:        bool,
    /// Radius of the stand-in capsule when the part has no skin, as a share
    /// of the bone's length.
    pub fallback:    f32,
}

impl Role {
    pub fn tuning(self) -> Tuning {
        let t = |follow, follow_turn, swing, twist, hinge, core, fallback| Tuning { follow, follow_turn, swing, twist, hinge, core, fallback };
        match self {
            Role::Pelvis   => t(0.70, 0.70,  0.0,  0.0, false, true,  0.9),
            Role::Spine    => t(0.60, 0.60, 12.0, 10.0, false, true,  0.9),
            Role::Neck     => t(0.55, 0.55, 20.0, 20.0, false, false, 0.6),
            Role::Head     => t(0.55, 0.80, 25.0, 25.0, false, false, 0.9),
            Role::Clavicle => t(0.55, 0.55, 14.0,  6.0, false, false, 0.35),
            Role::UpperArm => t(0.40, 0.40, 70.0, 50.0, false, false, 0.2),
            Role::LowerArm => t(0.35, 0.35, 10.0, 70.0, true,  false, 0.16),
            Role::Hand     => t(0.40, 0.80, 45.0, 30.0, false, false, 0.35),
            Role::Thigh    => t(0.45, 0.45, 50.0, 35.0, false, false, 0.2),
            Role::Calf     => t(0.40, 0.40,  8.0, 25.0, true,  false, 0.14),
            Role::Foot     => t(0.45, 0.85, 55.0, 30.0, false, false, 0.3),
            Role::Toe      => t(0.45, 0.85, 30.0, 10.0, false, false, 0.4),
            Role::Other    => t(0.45, 0.45, 30.0, 20.0, false, false, 0.25),
        }
    }

    /// How much harder the part is to turn than its shape makes it. The
    /// ends of the limbs and the head keep their turn when they are pushed:
    /// the limb moves, the hand does not spin, a lifted foot stays level.
    pub fn turn_resist(self) -> f32 { if matches!(self, Role::Hand | Role::Foot | Role::Toe | Role::Head) { 25.0 } else { 1.0 } }

    /// Part of the trunk: these never collide with each other.
    pub fn trunk(self) -> bool { matches!(self, Role::Pelvis | Role::Spine | Role::Neck | Role::Head | Role::Clavicle) }

    /// Part and side of a joint, from its name. Knows the Unreal mannequin
    /// and MetaHuman names (pelvis, spine_03, upperarm_l, calf_r, ball_l),
    /// the HumanIK and Mixamo names (Hips, Spine1, LeftForeArm, RightUpLeg,
    /// LeftToeBase) and the usual variations of both. Helper joints (twist,
    /// corrective, fingers) have no part: their skin goes to the part above.
    pub fn from_name(name: &str) -> Option<(Role, Side)> {
        // "ns:rig|LeftArm" is "LeftArm".
        let name = name.rsplit(|c| c == ':' || c == '|').next().unwrap_or(name);
        let lower = name.to_ascii_lowercase();
        let mut s = lower.as_str();
        let mut side = Side::Centre;
        let sided = |s: &str, tails: &[&str]| tails.iter().find(|t| s.ends_with(**t)).map(|t| s.len() - t.len());
        if let Some(rest) = s.strip_prefix("left") { side = Side::Left; s = rest; }
        else if let Some(rest) = s.strip_prefix("right") { side = Side::Right; s = rest; }
        else if let Some(rest) = s.strip_prefix("l_") { side = Side::Left; s = rest; }
        else if let Some(rest) = s.strip_prefix("r_") { side = Side::Right; s = rest; }
        else if let Some(cut) = sided(s, &["_l", ".l", "-l", "_left", "left"]) { side = Side::Left; s = &s[..cut]; }
        else if let Some(cut) = sided(s, &["_r", ".r", "-r", "_right", "right"]) { side = Side::Right; s = &s[..cut]; }
        // "spine_03" and "Spine3" are "spine".
        let word: String = s.trim_matches(|c: char| c == '_' || c == '.' || c == '-' || c == ' ' || c.is_ascii_digit())
            .chars().filter(|c| *c != '_' && *c != ' ').collect();
        let role = match word.as_str() {
            "pelvis" | "hips" | "hip" => Role::Pelvis,
            "spine" | "chest" | "upperchest" | "torso" | "abdomen" | "waist" => Role::Spine,
            "neck" => Role::Neck,
            "head" => Role::Head,
            "clavicle" | "shoulder" | "collar" => Role::Clavicle,
            "upperarm" | "arm" | "uparm" => Role::UpperArm,
            "lowerarm" | "forearm" | "elbow" => Role::LowerArm,
            "hand" | "wrist" => Role::Hand,
            "thigh" | "upleg" | "upperleg" => Role::Thigh,
            "calf" | "leg" | "lowerleg" | "shin" | "knee" => Role::Calf,
            "foot" | "ankle" => Role::Foot,
            "ball" | "toe" | "toebase" | "toes" => Role::Toe,
            _ => return None,
        };
        // The middle of the body has no side, the limbs must have one.
        let limb = !matches!(role, Role::Pelvis | Role::Spine | Role::Neck | Role::Head);
        if limb != (side != Side::Centre) { return None; }
        Some((role, side))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreal_and_humanik_names_are_known_and_helpers_are_not() {
        use Role::*; use Side::*;
        let yes = [
            ("pelvis", Pelvis, Centre), ("spine_03", Spine, Centre), ("neck_02", Neck, Centre), ("head", Head, Centre),
            ("clavicle_l", Clavicle, Left), ("upperarm_r", UpperArm, Right), ("lowerarm_l", LowerArm, Left),
            ("hand_r", Hand, Right), ("thigh_l", Thigh, Left), ("calf_r", Calf, Right), ("foot_l", Foot, Left), ("ball_r", Toe, Right),
            ("Hips", Pelvis, Centre), ("Spine1", Spine, Centre), ("mixamorig:LeftForeArm", LowerArm, Left),
            ("Take01:RightUpLeg", Thigh, Right), ("LeftLeg", Calf, Left), ("RightToeBase", Toe, Right), ("LeftShoulder", Clavicle, Left),
            ("LeftArm", UpperArm, Left), ("Chest", Spine, Centre),
        ];
        for (n, r, s) in yes { assert_eq!(Role::from_name(n), Some((r, s)), "{n}"); }
        let no = [
            "root", "upperarm_twist_01_l", "upperarm_correctiveRoot_r", "lowerarm_in_l", "index_02_l", "thumb_01_r",
            "spine_04_latissimus_r", "clavicle_pec_l", "calf_knee_r", "wrist_inner_l", "thigh_twistCor_02_r",
            "ik_hand_l", "LeftHandIndex1", "arm", "left", "",
        ];
        for n in no { assert_eq!(Role::from_name(n), None, "{n}"); }
    }
}
