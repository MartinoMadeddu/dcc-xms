//! Compact files for a clip and for a mesh.
//!
//! An FBX take of a long capture is hundreds of megabytes, most of it
//! curve keys in double precision. The same clip as this program holds it,
//! per-frame transforms, with what never changes written once and
//! rotations in 16 bits a component, is a tenth of that and loads at once.
//!
//! - `.xmsclip`: skeleton, every frame, and the skin with its weights.
//! - `.xmsmesh`: positions and triangles.
//!
//! Both are gzip streams. Load FBX and Load FBX Mesh read them by their
//! extension.

use std::io::{Read, Write};
use std::sync::Arc;

use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::Transform;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};

use crate::core::anim::{AnimData, FrameRate, Joint, SkinMesh, Track};
use crate::types::MeshData;

pub const CLIP_EXT: &str = "xmsclip";
pub const MESH_EXT: &str = "xmsmesh";

pub fn is_clip(path: &str) -> bool { path.to_ascii_lowercase().ends_with(".xmsclip") }
pub fn is_mesh(path: &str) -> bool { path.to_ascii_lowercase().ends_with(".xmsmesh") }

struct Out(Vec<u8>);
impl Out {
    fn u8(&mut self, v: u8) { self.0.push(v); }
    fn u16(&mut self, v: u16) { self.0.extend(v.to_le_bytes()); }
    fn u32(&mut self, v: u32) { self.0.extend(v.to_le_bytes()); }
    fn i64(&mut self, v: i64) { self.0.extend(v.to_le_bytes()); }
    fn f32(&mut self, v: f32) { self.0.extend(v.to_le_bytes()); }
    fn v3(&mut self, v: Vec3) { for x in v.to_array() { self.f32(x); } }
    fn quat(&mut self, q: Quat) { for x in q.to_array() { self.f32(x); } }
    fn text(&mut self, s: &str) { self.u32(s.len() as u32); self.0.extend(s.as_bytes()); }
    fn finish(self, path: &str, magic: &[u8; 8]) -> Result<(), String> {
        if let Some(dir) = std::path::Path::new(path).parent() { let _ = std::fs::create_dir_all(dir); }
        let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        let mut gz = GzEncoder::new(std::io::BufWriter::new(file), Compression::new(6));
        gz.write_all(magic).and_then(|_| gz.write_all(&self.0)).map_err(|e| e.to_string())?;
        gz.finish().map_err(|e| e.to_string())?;
        Ok(())
    }
}

struct In { data: Vec<u8>, at: usize }
impl In {
    fn open(path: &str, magic: &[u8; 8]) -> Result<In, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{e}"))?;
        let mut data = vec![];
        GzDecoder::new(std::io::BufReader::new(file)).read_to_end(&mut data).map_err(|e| format!("not a readable file: {e}"))?;
        if data.len() < 8 || &data[..8] != magic { return Err("not a file of this kind".into()); }
        Ok(In { data, at: 8 })
    }
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.at + n > self.data.len() { return Err("file ends early".into()); }
        self.at += n;
        Ok(&self.data[self.at - n..self.at])
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, String> { let b = self.take(2)?; Ok(u16::from_le_bytes([b[0], b[1]])) }
    fn u32(&mut self) -> Result<u32, String> { let b = self.take(4)?; Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }
    fn i64(&mut self) -> Result<i64, String> { let b = self.take(8)?; Ok(i64::from_le_bytes(b.try_into().unwrap())) }
    fn f32(&mut self) -> Result<f32, String> { let b = self.take(4)?; Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]])) }
    fn v3(&mut self) -> Result<Vec3, String> { Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?)) }
    fn quat(&mut self) -> Result<Quat, String> { Ok(Quat::from_xyzw(self.f32()?, self.f32()?, self.f32()?, self.f32()?)) }
    fn text(&mut self) -> Result<String, String> { let n = self.u32()? as usize; Ok(String::from_utf8_lossy(self.take(n)?).into_owned()) }
    /// A count that the rest of the file could hold at `each` bytes apiece.
    fn count(&mut self, each: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n.saturating_mul(each.max(1)) > self.data.len() - self.at { return Err("file ends early".into()); }
        Ok(n)
    }
}

const CLIP_MAGIC: &[u8; 8] = b"XMSCLIP1";
const MESH_MAGIC: &[u8; 8] = b"XMSMESH1";

pub fn write_clip(clip: &AnimData, path: &str) -> Result<(), String> {
    let mut o = Out(vec![]);
    o.text(&clip.name);
    o.u32(clip.rate.num); o.u32(clip.rate.den); o.u8(clip.drop_frame as u8);
    o.i64(clip.start_frame);
    let frames = clip.frames.max(1);
    o.u32(frames as u32);
    o.u32(clip.joints.len() as u32);
    for (j, joint) in clip.joints.iter().enumerate() {
        o.text(&joint.name);
        o.u32(joint.parent.map(|p| p as u32 + 1).unwrap_or(0));
        o.u8(joint.is_bone as u8);
        o.v3(joint.rest.translation); o.quat(joint.rest.rotation); o.v3(joint.rest.scale);
        o.quat(joint.zero_rot);
        // What moves is written per frame, what does not is written once.
        let track = &clip.tracks[j];
        if track.len() < frames { o.u8(0); continue; }
        let first = track[0];
        let moves = |f: &dyn Fn(&Transform) -> bool| track.iter().any(|t| f(t));
        let (mt, mr, ms) = (
            moves(&|t| t.translation != first.translation),
            moves(&|t| t.rotation != first.rotation),
            moves(&|t| t.scale != first.scale));
        o.u8(1 | (mt as u8) << 1 | (mr as u8) << 2 | (ms as u8) << 3);
        if mt { for t in track.iter().take(frames) { o.v3(t.translation); } } else { o.v3(first.translation); }
        if mr {
            for t in track.iter().take(frames) {
                let q = t.rotation.normalize();
                for x in q.to_array() { o.u16(((x.clamp(-1.0, 1.0) * 32767.0).round() as i16) as u16); }
            }
        } else { o.quat(first.rotation); }
        if ms { for t in track.iter().take(frames) { o.v3(t.scale); } } else { o.v3(first.scale); }
    }
    match &clip.skin {
        None => o.u8(0),
        Some(s) => {
            o.u8(1);
            o.u32(s.positions.len() as u32);
            for p in &s.positions { o.v3(*p); }
            let weighted = s.weights.len() == s.positions.len();
            o.u8(weighted as u8);
            for v in 0..s.positions.len() {
                if weighted { for (j, w) in s.weights[v] { o.u16(j as u16); o.u8((w.clamp(0.0, 1.0) * 255.0).round() as u8); } }
                else { o.u16(s.joint[v] as u16); }
            }
            o.u32(s.faces.len() as u32);
            for f in &s.faces { for i in f { o.u32(*i); } }
            o.u32(s.bind.len() as u32);
            for m in &s.bind { for x in m.to_cols_array() { o.f32(x); } }
        }
    }
    o.finish(path, CLIP_MAGIC)
}

pub fn read_clip(path: &str) -> Result<AnimData, String> {
    let mut i = In::open(path, CLIP_MAGIC)?;
    let name = i.text()?;
    let rate = FrameRate::new(i.u32()?, i.u32()?);
    let drop_frame = i.u8()? != 0;
    let start_frame = i.i64()?;
    let frames = i.count(0)?.max(1);
    let n = i.count(60)?;
    let mut joints = Vec::with_capacity(n);
    let mut tracks: Vec<Track> = Vec::with_capacity(n);
    for _ in 0..n {
        let jname = i.text()?;
        let parent = match i.u32()? { 0 => None, p => Some(p as usize - 1) };
        let is_bone = i.u8()? != 0;
        let rest = Transform { translation: i.v3()?, rotation: i.quat()?, scale: i.v3()? };
        let zero_rot = i.quat()?;
        joints.push(Joint { name: jname, parent, rest, is_bone, zero_rot });
        let flags = i.u8()?;
        if flags & 1 == 0 { tracks.push(vec![]); continue; }
        let mut track = vec![Transform::IDENTITY; frames];
        if flags & 2 != 0 { for t in track.iter_mut() { t.translation = i.v3()?; } } else { let v = i.v3()?; for t in track.iter_mut() { t.translation = v; } }
        if flags & 4 != 0 {
            for t in track.iter_mut() {
                let mut q = [0.0f32; 4];
                for x in q.iter_mut() { *x = (i.u16()? as i16) as f32 / 32767.0; }
                t.rotation = Quat::from_array(q).normalize();
            }
        } else { let q = i.quat()?; for t in track.iter_mut() { t.rotation = q; } }
        if flags & 8 != 0 { for t in track.iter_mut() { t.scale = i.v3()?; } } else { let v = i.v3()?; for t in track.iter_mut() { t.scale = v; } }
        tracks.push(track);
    }
    let skin = if i.u8()? == 0 { None } else {
        let mut s = SkinMesh::default();
        let verts = i.count(12)?;
        for _ in 0..verts { s.positions.push(i.v3()?); }
        let weighted = i.u8()? != 0;
        for _ in 0..verts {
            if weighted {
                let mut w = [(0u32, 0.0f32); 4];
                for e in w.iter_mut() { *e = (i.u16()? as u32, i.u8()? as f32 / 255.0); }
                let sum: f32 = w.iter().map(|e| e.1).sum();
                if sum > 0.0 { for e in w.iter_mut() { e.1 /= sum; } } else { w[0].1 = 1.0; }
                s.joint.push(w[0].0);
                s.weights.push(w);
            } else { s.joint.push(i.u16()? as u32); }
        }
        let faces = i.count(16)?;
        for _ in 0..faces { s.faces.push([i.u32()?, i.u32()?, i.u32()?, i.u32()?]); }
        let binds = i.count(64)?;
        for _ in 0..binds { let mut m = [0.0f32; 16]; for x in m.iter_mut() { *x = i.f32()?; } s.bind.push(Mat4::from_cols_array(&m)); }
        if s.faces.iter().flatten().any(|v| *v as usize >= verts) { return Err("a face names a vertex that is not there".into()); }
        // Normals are made again from the faces.
        let mut normals = vec![Vec3::ZERO; verts];
        for f in &s.faces {
            let p = |k: usize| s.positions[f[k] as usize];
            let nrm = (p(1) - p(0)).cross(p(2) - p(0)) + if SkinMesh::is_tri(f) { Vec3::ZERO } else { (p(2) - p(0)).cross(p(3) - p(0)) };
            for k in 0..4 { normals[f[k] as usize] += nrm; }
        }
        s.normals = normals.into_iter().map(|v| v.normalize_or(Vec3::Y)).collect();
        Some(Arc::new(s))
    };
    let p = std::path::Path::new(path);
    Ok(AnimData {
        name, joints, rate, drop_frame, start_frame, frames,
        tracks: Arc::new(tracks),
        source: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        source_dir: p.parent().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        subject: String::new(),
        skin,
    })
}

pub fn write_mesh(mesh: &MeshData, path: &str) -> Result<(), String> {
    let mut o = Out(Vec::with_capacity(mesh.vertices.len() * 12 + mesh.indices.len() * 4 + 16));
    o.u32(mesh.vertices.len() as u32);
    for v in &mesh.vertices { for x in v { o.f32(*x); } }
    o.u32(mesh.indices.len() as u32);
    for i in &mesh.indices { o.u32(*i); }
    o.finish(path, MESH_MAGIC)
}

pub fn read_mesh(path: &str) -> Result<MeshData, String> {
    let mut i = In::open(path, MESH_MAGIC)?;
    let n = i.count(12)?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n { vertices.push([i.f32()?, i.f32()?, i.f32()?]); }
    let m = i.count(4)?;
    let mut indices = Vec::with_capacity(m);
    for _ in 0..m { let v = i.u32()?; if v as usize >= n { return Err("a triangle names a vertex that is not there".into()); } indices.push(v); }
    let mut mesh = MeshData::from_triangles(vertices, indices);
    mesh.compute_normals();
    mesh.primvars.clear();
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::create_test_clip;

    #[test]
    fn a_clip_comes_back_as_it_went() {
        let clip = create_test_clip(1.0, FrameRate::new(30000, 1001)).with_proxy_skin(1.0);
        let path = std::env::temp_dir().join("xms_packed_test.xmsclip");
        let path = path.to_str().unwrap();
        write_clip(&clip, path).unwrap();
        let back = read_clip(path).unwrap();
        assert_eq!((back.frames, back.start_frame, back.rate, back.joints.len()), (clip.frames, clip.start_frame, clip.rate, clip.joints.len()));
        assert!(back.joints.iter().zip(&clip.joints).all(|(a, b)| a.name == b.name && a.parent == b.parent && a.rest == b.rest));
        for f in [0, 7, clip.frames - 1] {
            let (a, b) = (clip.world_pose(f), back.world_pose(f));
            for j in 0..clip.joints.len() { assert!((a[j].w_axis - b[j].w_axis).length() < 2e-4, "frame {f} joint {j}"); }
        }
        let (s, t) = (clip.skin.as_ref().unwrap(), back.skin.as_ref().unwrap());
        assert_eq!((s.positions.len(), s.faces.len(), s.bind.len()), (t.positions.len(), t.faces.len(), t.bind.len()));
        assert_eq!(s.positions, t.positions);
        assert_eq!(s.joint, t.joint);
        // Not a clip, and a clip cut short, are errors, not crashes.
        assert!(read_clip("/nonexistent.xmsclip").is_err());
        assert!(read_mesh(path).is_err());
    }

    #[test]
    fn a_mesh_comes_back_as_it_went() {
        let mesh = crate::node_graph::nodes::create_sphere(1.0, 12);
        let path = std::env::temp_dir().join("xms_packed_test.xmsmesh");
        let path = path.to_str().unwrap();
        write_mesh(&mesh, path).unwrap();
        let back = read_mesh(path).unwrap();
        assert_eq!(back.vertices, mesh.vertices);
        assert_eq!(back.indices, mesh.indices);
        assert_eq!(back.normals.len(), back.vertices.len());
    }

    /// Makes the files of the Ragdoll template from the FBX files they came from:
    ///
    ///     XMS_PACK_CLIP=take.fbx XMS_PACK_SET=set.fbx cargo test pack_ragdoll_example -- --ignored --nocapture
    #[test]
    #[ignore]
    fn pack_ragdoll_example() {
        let (Ok(take), Ok(set)) = (std::env::var("XMS_PACK_CLIP"), std::env::var("XMS_PACK_SET")) else { return };
        let clip = crate::fbx_loader::load_fbx(&take, 0).unwrap().anim;
        let out = crate::examples::path(crate::examples::RAGDOLL_TAKE);
        write_clip(&clip, &out).unwrap();
        println!("{out}: {} bytes", std::fs::metadata(&out).unwrap().len());
        let meshes = crate::fbx_loader::load_meshes(&set).unwrap();
        let parts: Vec<&MeshData> = meshes.iter().map(|m| &*m.1).collect();
        let out = crate::examples::path(crate::examples::RAGDOLL_SET);
        write_mesh(&crate::node_graph::nodes::merge_all(&parts), &out).unwrap();
        println!("{out}: {} bytes", std::fs::metadata(&out).unwrap().len());
    }
}
