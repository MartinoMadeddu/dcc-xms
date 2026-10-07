//! FBX import through ufbx. Reads the node hierarchy and bakes one take to
//! dense per-frame local transforms. A mesh bound to the skeleton comes
//! with it, with its weights. Meshes on their own are read by `load_meshes`.
//!
//! ufbx converts the file to right-handed Y-up, metres, and folds pre/post
//! rotations and rotation orders into the local transform it evaluates.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::Transform;

use crate::core::anim::{AnimData, FrameRate, Joint, SkinMesh, Track};
use crate::types::MeshData;

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

fn to_mat4(m: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols_array(&[
        m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0,
        m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0,
        m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0,
        m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0,
    ])
}

/// Triangles and quads of a mesh as vertex indices. Larger polygons are cut into a fan.
fn mesh_faces(mesh: &ufbx::Mesh, mut face: impl FnMut(&[u32])) {
    for f in mesh.faces.iter() {
        let (b, n) = (f.index_begin as usize, f.num_indices as usize);
        if n < 3 || b + n > mesh.vertex_indices.len() { continue; }
        let all: &[u32] = &mesh.vertex_indices;
        let idx = &all[b..b + n];
        if n <= 4 { face(idx); } else { for k in 1..n - 1 { face(&[idx[0], idx[k], idx[k + 1]]); } }
    }
}

/// The meshes bound to the skeleton, as one, with up to four weights per vertex.
fn read_skin(scene: &ufbx::Scene, joint_of: &HashMap<usize, usize>, order: &[usize]) -> Option<SkinMesh> {
    let mut skin = SkinMesh::default();
    // Joints no mesh is bound to rest where the file has them.
    skin.bind = order.iter().map(|i| to_mat4(&scene.nodes[*i].node_to_world)).collect();
    let mut bound = vec![false; order.len()];
    for mesh in scene.meshes.iter() {
        let Some(deformer) = mesh.skin_deformers.iter().next() else { continue };
        let Some(node) = mesh.element.instances.iter().next() else { continue };
        if deformer.vertices.len() < mesh.num_vertices { continue; }
        // Mesh space to the world, as the mesh stands in the file.
        let to_world = to_mat4(&node.geometry_to_world);
        // Joint of each cluster. Its bind matrix follows from where the mesh
        // is and how the cluster sees the mesh.
        let cluster_joint: Vec<Option<usize>> = deformer.clusters.iter().map(|c| {
            let j = *joint_of.get(&(c.bone_node.as_ref()?.element.typed_id as usize))?;
            if !bound[j] {
                let to_bone = to_mat4(&c.geometry_to_bone);
                if to_bone.determinant().abs() > 1e-12 { skin.bind[j] = to_world * to_bone.inverse(); bound[j] = true; }
            }
            Some(j)
        }).collect();
        let base = skin.positions.len() as u32;
        for (v, p) in mesh.vertices.iter().enumerate() {
            skin.positions.push(to_world.transform_point3(Vec3::new(p.x as f32, p.y as f32, p.z as f32)));
            let sv = &deformer.vertices[v];
            let mut top = [(0u32, 0.0f32); 4];
            for k in sv.weight_begin as usize..(sv.weight_begin + sv.num_weights) as usize {
                let Some(w) = deformer.weights.get(k) else { break };
                let Some(Some(j)) = cluster_joint.get(w.cluster_index as usize) else { continue };
                let entry = (*j as u32, w.weight as f32);
                // Keep the four heaviest, heaviest first.
                let mut at = 4;
                for s in (0..4).rev() { if entry.1 > top[s].1 { at = s; } }
                if at < 4 { for s in (at + 1..4).rev() { top[s] = top[s - 1]; } top[at] = entry; }
            }
            let sum: f32 = top.iter().map(|t| t.1).sum();
            if sum > 1e-8 { for t in top.iter_mut() { t.1 /= sum; } } else { top[0] = (0, 1.0); }
            skin.joint.push(top[0].0);
            skin.weights.push(top);
        }
        mesh_faces(mesh, |idx| {
            let i = |k: usize| base + idx[k];
            skin.faces.push(if idx.len() == 3 { [i(0), i(1), i(2), i(2)] } else { [i(0), i(1), i(2), i(3)] });
        });
    }
    if skin.positions.is_empty() { return None; }
    // Smooth normals from the faces.
    let mut normals = vec![Vec3::ZERO; skin.positions.len()];
    for f in &skin.faces {
        let p = |k: usize| skin.positions[f[k] as usize];
        let n = (p(1) - p(0)).cross(p(2) - p(0)) + if SkinMesh::is_tri(f) { Vec3::ZERO } else { (p(2) - p(0)).cross(p(3) - p(0)) };
        for k in 0..4 { normals[f[k] as usize] += n; }
    }
    skin.normals = normals.into_iter().map(|n| n.normalize_or(Vec3::Y)).collect();
    Some(skin)
}

/// Every mesh of a file, in the world, as triangles: name of its node and
/// the mesh. For models with no skeleton, such as a set to collide with.
pub fn load_meshes(path: &str) -> Result<Vec<(String, Arc<MeshData>)>, String> {
    let opts = ufbx::LoadOpts {
        ignore_embedded:    true,
        ignore_animation:   true,
        target_axes:        ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        space_conversion:   ufbx::SpaceConversion::AdjustTransforms,
        ..Default::default()
    };
    let scene = ufbx::load_file(path, opts)
        .map_err(|e| format!("{}", e.description.as_ref() as &str))?;
    let mut out = vec![];
    for mesh in scene.meshes.iter() {
        for node in mesh.element.instances.iter() {
            let to_world = to_mat4(&node.geometry_to_world);
            let vertices: Vec<[f32; 3]> = mesh.vertices.iter()
                .map(|p| to_world.transform_point3(Vec3::new(p.x as f32, p.y as f32, p.z as f32)).to_array()).collect();
            let mut indices: Vec<u32> = Vec::with_capacity(mesh.num_triangles * 3);
            // A mirrored node turns its faces inside out: turn them back.
            let flip = to_world.determinant() < 0.0;
            mesh_faces(mesh, |idx| {
                for k in 1..idx.len() - 1 {
                    if flip { indices.extend([idx[0], idx[k + 1], idx[k]]); } else { indices.extend([idx[0], idx[k], idx[k + 1]]); }
                }
            });
            if indices.is_empty() { continue; }
            let mut data = MeshData::from_triangles(vertices, indices);
            data.face_count = mesh.num_faces;
            data.compute_normals();
            data.primvars.clear();
            out.push((node.element.name.to_string(), Arc::new(data)));
        }
    }
    if out.is_empty() { return Err("no meshes in file".into()); }
    Ok(out)
}

type MeshEntry = (Option<SystemTime>, Result<Vec<(String, Arc<MeshData>)>, String>);
static MESHES: OnceLock<Mutex<HashMap<String, MeshEntry>>> = OnceLock::new();

/// `load_meshes`, kept until the file changes.
pub fn load_meshes_cached(path: &str) -> Result<Vec<(String, Arc<MeshData>)>, String> {
    if path.is_empty() { return Err("no file set".into()); }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let cache = MESHES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((t, r)) = cache.lock().unwrap().get(path) {
        if *t == mtime { return r.clone(); }
    }
    let result = if mtime.is_none() { Err("file not found".to_string()) } else { load_meshes(path) };
    let mut cache = cache.lock().unwrap();
    if cache.len() > 3 { cache.clear(); }
    cache.insert(path.to_string(), (mtime, result.clone()));
    result
}

pub fn load_fbx(path: &str, take: u32) -> Result<LoadedFbx, String> {
    let opts = ufbx::LoadOpts {
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

    // Hierarchy order: each joint is followed by its whole subtree.
    let mut order: Vec<usize> = vec![];
    let mut stack: Vec<&ufbx::Node> = vec![&scene.root_node];
    while let Some(nd) = stack.pop() {
        let i = nd.element.typed_id as usize;
        if i < n && keep[i] { order.push(i); }
        for c in nd.children.iter().rev() { stack.push(c); }
    }
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
        // Rotation the rig shows when its animated rotation is zero: the
        // rest rotation with the node's own Euler rotation taken back out.
        // Exact when the node has no post-rotation.
        let rest  = to_transform(&nd.local_transform);
        let euler = ufbx::euler_to_quat(nd.euler_rotation, nd.rotation_order);
        let euler = Quat::from_xyzw(euler.x as f32, euler.y as f32, euler.z as f32, euler.w as f32);
        let zero  = (rest.rotation * euler.inverse()).normalize();
        Joint {
            name:     nd.element.name.to_string(),
            parent,
            rest,
            is_bone:  nd.bone.is_some(),
            zero_rot: if zero.angle_between(Quat::IDENTITY) < 1e-4 { Quat::IDENTITY } else { zero },
        }
    }).collect();

    if joints.is_empty() {
        return Err("no transform nodes in file".into());
    }

    let skin = read_skin(&scene, &joint_of, &order).map(Arc::new);

    let p    = std::path::Path::new(path);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir  = p.parent().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();

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
            source:      stem.clone(),
            source_dir:  dir.clone(),
            subject:     String::new(),
            skin:        skin.clone(),
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
            source:      stem.clone(),
            source_dir:  dir.clone(),
            subject:     String::new(),
            skin:        skin.clone(),
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
    if cache.len() > 3 { cache.clear(); }   // clips are large
    cache.insert(key, (mtime, result.clone()));
    result
}

/// FBX files directly inside a folder, sorted by name.
pub fn list_fbx(dir: &str) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case("fbx")).unwrap_or(false))
        .collect();
    files.sort();
    files
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
