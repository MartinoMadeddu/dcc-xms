//! FBX import through ufbx. Reads the node hierarchy and bakes one take to
//! dense per-frame local transforms. Geometry and skinning are not read.
//!
//! ufbx converts the file to right-handed Y-up, metres, and folds pre/post
//! rotations and rotation orders into the local transform it evaluates.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;

use crate::core::anim::{AnimData, FrameRate, Joint, Track};

#[derive(Clone)]
pub struct LoadedFbx {
    pub anim:  Arc<AnimData>,
    /// Names of every take (anim stack) in the file.
    pub takes: Arc<Vec<String>>,
}

fn to_transform(t: &ufbx::Transform) -> Transform {
    Transform {
        translation: Vec3::new(t.translation.x as f32, t.translation.y as f32, t.translation.z as f32),
        rotation:    Quat::from_xyzw(
            t.rotation.x as f32, t.rotation.y as f32, t.rotation.z as f32, t.rotation.w as f32,
        ).normalize(),
        scale:       Vec3::new(t.scale.x as f32, t.scale.y as f32, t.scale.z as f32),
    }
}

pub fn load_fbx(path: &str, take: u32) -> Result<LoadedFbx, String> {
    let opts = ufbx::LoadOpts {
        ignore_geometry:    true,
        ignore_embedded:    true,
        target_axes:        ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        space_conversion:   ufbx::SpaceConversion::AdjustTransforms,
        ..Default::default()
    };
    let scene = ufbx::load_file(path, opts)
        .map_err(|e| format!("{}", e.description.as_ref() as &str))?;

    // ── Which nodes become joints ────────────────────────────────────────────
    // Bones and every ancestor of a bone. A file with no bones at all (marker
    // or null-only capture) keeps every node. The scene root is never a joint.
    let n = scene.nodes.len();
    let any_bone = scene.nodes.iter().any(|nd| nd.bone.is_some());
    let mut keep = vec![!any_bone; n];
    if any_bone {
        for nd in scene.nodes.iter().filter(|nd| nd.bone.is_some()) {
            let mut cur: Option<&ufbx::Node> = Some(nd);
            while let Some(c) = cur {
                let i = c.element.typed_id as usize;
                if i >= n || keep[i] { break; }
                keep[i] = true;
                cur = c.parent.as_deref();
            }
        }
    }
    for nd in scene.nodes.iter() {
        if nd.is_root || nd.is_geometry_transform_helper || nd.is_scale_helper {
            let i = nd.element.typed_id as usize;
            if i < n { keep[i] = false; }
        }
    }

    // Parents before children.
    let mut order: Vec<usize> = (0..n).filter(|i| keep[*i]).collect();
    order.sort_by_key(|i| scene.nodes[*i].node_depth);
    let joint_of: HashMap<usize, usize> =
        order.iter().enumerate().map(|(j, i)| (*i, j)).collect();

    let joints: Vec<Joint> = order.iter().map(|i| {
        let nd = &scene.nodes[*i];
        let mut parent = None;
        let mut cur = nd.parent.as_deref();
        while let Some(p) = cur {
            if let Some(j) = joint_of.get(&(p.element.typed_id as usize)) { parent = Some(*j); break; }
            cur = p.parent.as_deref();
        }
        Joint {
            name:   nd.element.name.to_string(),
            parent,
            rest:   to_transform(&nd.local_transform),
        }
    }).collect();

    if joints.is_empty() {
        return Err("no transform nodes in file".into());
    }

    // ── Rate ─────────────────────────────────────────────────────────────────
    let rate = FrameRate::from_fps(scene.settings.frames_per_second);
    let drop_frame = matches!(
        scene.settings.time_mode,
        ufbx::TimeMode::NtscDropFrame | ufbx::TimeMode::E30FpsDrop
    ) && rate.supports_drop_frame();
    let fps = rate.fps();

    // ── Take ─────────────────────────────────────────────────────────────────
    let takes: Vec<String> = scene.anim_stacks.iter()
        .map(|s| s.element.name.to_string())
        .collect();

    let anim = if scene.anim_stacks.is_empty() {
        AnimData {
            name:        "(no animation)".into(),
            tracks:      Arc::new(vec![Vec::new(); joints.len()]),
            joints,
            rate,
            drop_frame,
            start_frame: 0,
            frames:      1,
        }
    } else {
        let stack  = &scene.anim_stacks[(take as usize).min(scene.anim_stacks.len() - 1)];
        let begin  = stack.time_begin;
        let end    = stack.time_end.max(begin);
        let frames = ((end - begin) * fps).round() as usize + 1;

        let tracks: Vec<Track> = order.iter().map(|i| {
            let nd = &scene.nodes[*i];
            (0..frames).map(|f| {
                let t = begin + f as f64 / fps;
                to_transform(&ufbx::evaluate_transform(&stack.anim, nd, t))
            }).collect()
        }).collect();

        AnimData {
            name:        stack.element.name.to_string(),
            joints,
            rate,
            drop_frame,
            start_frame: (begin * fps).round() as i64,
            frames,
            tracks:      Arc::new(tracks),
        }
    };

    Ok(LoadedFbx { anim: Arc::new(anim), takes: Arc::new(takes) })
}

// ── Cache ────────────────────────────────────────────────────────────────────
// Keyed by path + take, invalidated when the file's modification time changes.

type CacheKey   = (String, u32);
type CacheEntry = (Option<SystemTime>, Result<LoadedFbx, String>);

static CACHE: OnceLock<Mutex<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();

pub fn load_fbx_cached(path: &str, take: u32) -> Result<LoadedFbx, String> {
    if path.is_empty() { return Err("no file set".into()); }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key   = (path.to_string(), take);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some((t, r)) = cache.lock().unwrap().get(&key) {
        if *t == mtime { return r.clone(); }
    }

    let result = if mtime.is_none() {
        Err("file not found".to_string())
    } else {
        let r = load_fbx(path, take);
        if let Err(e) = &r { eprintln!("[LoadFbx] failed to load '{path}': {e}"); }
        r
    };

    let mut cache = cache.lock().unwrap();
    if cache.len() > 32 { cache.clear(); }
    cache.insert(key, (mtime, result.clone()));
    result
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ASCII FBX: Z-up, centimetres, 30 fps, two bones, one take that
    /// starts at 01:00:00:00 and moves the root 30 cm along X over one second.
    const FBX: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",2
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",1
		P: "FrontAxisSign", "int", "Integer", "",-1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
		P: "TimeMode", "enum", "", "",6
	}
}
Objects:  {
	NodeAttribute: 11, "NodeAttribute::", "LimbNode" {
		TypeFlags: "Skeleton"
	}
	NodeAttribute: 12, "NodeAttribute::", "LimbNode" {
		TypeFlags: "Skeleton"
	}
	Model: 1, "Model::Hips", "LimbNode" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,0,100
		}
	}
	Model: 2, "Model::rig:Spine", "LimbNode" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,0,20
		}
	}
	AnimationStack: 100, "AnimStack::Take 001", "" {
		Properties70:  {
			P: "LocalStart", "KTime", "Time", "",166270168800000
			P: "LocalStop", "KTime", "Time", "",166316354958000
			P: "ReferenceStart", "KTime", "Time", "",166270168800000
			P: "ReferenceStop", "KTime", "Time", "",166316354958000
		}
	}
	AnimationLayer: 101, "AnimLayer::BaseLayer", "" {
	}
	AnimationCurveNode: 200, "AnimCurveNode::T", "" {
		Properties70:  {
			P: "d|X", "Number", "", "A",0
			P: "d|Y", "Number", "", "A",0
			P: "d|Z", "Number", "", "A",100
		}
	}
	AnimationCurve: 300, "AnimCurve::", "" {
		Default: 0
		KeyVer: 4008
		KeyTime: *2 {
			a: 166270168800000,166316354958000
		}
		KeyValueFloat: *2 {
			a: 0,30
		}
		KeyAttrFlags: *1 {
			a: 4
		}
		KeyAttrDataFloat: *4 {
			a: 0,0,0,0
		}
		KeyAttrRefCount: *1 {
			a: 2
		}
	}
}
Connections:  {
	C: "OO",1,0
	C: "OO",2,1
	C: "OO",11,1
	C: "OO",12,2
	C: "OO",101,100
	C: "OO",200,101
	C: "OP",200,1, "Lcl Translation"
	C: "OP",300,200, "d|X"
}
"#;

    #[test]
    fn loads_skeleton_take_and_timecode() {
        let path = std::env::temp_dir().join("xms_fbx_loader_test.fbx");
        std::fs::write(&path, FBX).unwrap();
        let loaded = load_fbx(path.to_str().unwrap(), 0).unwrap();
        let a = &loaded.anim;

        assert_eq!(*loaded.takes, vec!["Take 001".to_string()]);
        assert_eq!(a.joints.len(), 2);
        assert_eq!(a.joints[0].name, "Hips");
        assert_eq!(a.joints[1].name, "rig:Spine");
        assert_eq!(a.joints[1].parent, Some(0));

        assert_eq!(a.rate, FrameRate::new(30, 1));
        assert_eq!(a.frames, 31);
        assert_eq!(a.timecode(a.start_frame).to_string(), "01:00:00:00");
        assert_eq!(a.timecode(a.end_frame()).to_string(), "01:00:01:00");

        // Z-up centimetres in the file, Y-up metres here.
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-4;
        let first = a.world_pose(0);
        let last  = a.world_pose(30);
        let mid   = a.world_pose(15);
        assert!(close(first[0].w_axis.truncate(), Vec3::new(0.0, 1.0, 0.0)), "{:?}", first[0].w_axis);
        assert!(close(first[1].w_axis.truncate(), Vec3::new(0.0, 1.2, 0.0)), "{:?}", first[1].w_axis);
        assert!(close(mid[0].w_axis.truncate(),   Vec3::new(0.15, 1.0, 0.0)), "{:?}", mid[0].w_axis);
        assert!(close(last[1].w_axis.truncate(),  Vec3::new(0.3, 1.2, 0.0)), "{:?}", last[1].w_axis);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(load_fbx_cached("/nonexistent/take.fbx", 0).is_err());
        assert!(load_fbx_cached("", 0).is_err());
    }
}
