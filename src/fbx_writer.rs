//! Binary FBX export (version 7500).
//!
//! Writes a clip as a skeleton with baked Translation / Rotation curves, and
//! its bound mesh, if any, as geometry with a skin deformer and a bind pose.
//! The layout follows what Motive and the FBX SDK write: Y-up, centimetres,
//! XYZ Euler rotations, one key per frame.
//!
//! The file is built as a tree of nodes in memory and serialised in one pass.

use std::io::Write;
use std::path::Path;

use bevy::math::{EulerRot, Mat4, Quat, Vec3};
use flate2::{write::ZlibEncoder, Compression};

use crate::core::anim::{AnimData, FrameRate, SkinMesh};

/// FBX time units per second.
const KTIME: i128 = 46_186_158_000;
/// Internal unit is the metre; files are written in centimetres.
const CM: f32 = 100.0;
const VERSION: u32 = 7500;

const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
// File id, creation time and footer id belong together. This is a matching
// set taken from a file written by the FBX SDK.
const FILE_ID: [u8; 16] = [43, 183, 46, 233, 180, 38, 205, 195, 185, 198, 189, 46, 168, 44, 255, 246];
const CREATION_TIME: &str = "2026-07-31 10:44:37:785";
const FOOT_ID: [u8; 16] = [250, 188, 168, 13, 215, 206, 209, 98, 181, 115, 249, 136, 19, 248, 40, 118];
const FOOT_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

pub struct WriteStats {
    pub joints:   usize,
    /// Frames of animation written (0 for a static pose).
    pub frames:   usize,
    pub vertices: usize,
}

// ============================================================================
// NODE TREE
// ============================================================================

enum Prop {
    Bool(bool),
    I32(i32),
    F64(f64),
    I64(i64),
    Str(Vec<u8>),
    Raw(Vec<u8>),
    ArrF32(Vec<f32>),
    ArrF64(Vec<f64>),
    ArrI32(Vec<i32>),
    ArrI64(Vec<i64>),
}

struct Node {
    name:  &'static str,
    props: Vec<Prop>,
    kids:  Vec<Node>,
}

fn node(name: &'static str, props: Vec<Prop>, kids: Vec<Node>) -> Node { Node { name, props, kids } }
fn leaf(name: &'static str, props: Vec<Prop>) -> Node { Node { name, props, kids: vec![] } }
fn s(v: &str) -> Prop { Prop::Str(v.as_bytes().to_vec()) }

/// Object name with its class, the way FBX stores it: "name\0\x01Class".
fn named(name: &str, class: &str) -> Prop {
    let mut v = name.as_bytes().to_vec();
    v.extend_from_slice(b"\x00\x01");
    v.extend_from_slice(class.as_bytes());
    Prop::Str(v)
}

/// Properties70 entry.
fn p(name: &str, ty: &str, sub: &str, flags: &str, values: Vec<Prop>) -> Node {
    let mut props = vec![s(name), s(ty), s(sub), s(flags)];
    props.extend(values);
    leaf("P", props)
}
fn p_int(name: &str, v: i32) -> Node { p(name, "int", "Integer", "", vec![Prop::I32(v)]) }
fn p_enum(name: &str, v: i32) -> Node { p(name, "enum", "", "", vec![Prop::I32(v)]) }
fn p_double(name: &str, v: f64) -> Node { p(name, "double", "Number", "", vec![Prop::F64(v)]) }
fn p_time(name: &str, v: i64) -> Node { p(name, "KTime", "Time", "", vec![Prop::I64(v)]) }
fn p_vec(name: &str, flags: &str, v: [f64; 3]) -> Node {
    p(name, name, "", flags, v.iter().map(|x| Prop::F64(*x)).collect())
}

fn array_prop(out: &mut Vec<u8>, tag: u8, count: usize, raw: Vec<u8>) -> std::io::Result<()> {
    out.push(tag);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    if raw.len() > 128 {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&raw)?;
        let z = enc.finish()?;
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
    } else {
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&raw);
    }
    Ok(())
}

fn write_prop(out: &mut Vec<u8>, prop: &Prop) -> std::io::Result<()> {
    match prop {
        Prop::Bool(v) => { out.push(b'C'); out.push(*v as u8); }
        Prop::I32(v)  => { out.push(b'I'); out.extend_from_slice(&v.to_le_bytes()); }
        Prop::F64(v)  => { out.push(b'D'); out.extend_from_slice(&v.to_le_bytes()); }
        Prop::I64(v)  => { out.push(b'L'); out.extend_from_slice(&v.to_le_bytes()); }
        Prop::Str(v)  => { out.push(b'S'); out.extend_from_slice(&(v.len() as u32).to_le_bytes()); out.extend_from_slice(v); }
        Prop::Raw(v)  => { out.push(b'R'); out.extend_from_slice(&(v.len() as u32).to_le_bytes()); out.extend_from_slice(v); }
        Prop::ArrF32(v) => array_prop(out, b'f', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())?,
        Prop::ArrF64(v) => array_prop(out, b'd', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())?,
        Prop::ArrI32(v) => array_prop(out, b'i', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())?,
        Prop::ArrI64(v) => array_prop(out, b'l', v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect())?,
    }
    Ok(())
}

/// Record header: end offset, property count, property bytes (u64 each in
/// version 7500), name length (u8).
const HEADER: usize = 25;

fn write_node(out: &mut Vec<u8>, n: &Node) -> std::io::Result<()> {
    let start = out.len();
    out.extend_from_slice(&[0u8; HEADER]);
    out.extend_from_slice(n.name.as_bytes());
    let props_start = out.len();
    for prop in &n.props { write_prop(out, prop)?; }
    let props_len = out.len() - props_start;
    // A node with children, or with nothing at all, ends with a null record.
    if !n.kids.is_empty() || n.props.is_empty() {
        for k in &n.kids { write_node(out, k)?; }
        out.extend_from_slice(&[0u8; HEADER]);
    }
    let end = out.len() as u64;
    out[start..start + 8].copy_from_slice(&end.to_le_bytes());
    out[start + 8..start + 16].copy_from_slice(&(n.props.len() as u64).to_le_bytes());
    out[start + 16..start + 24].copy_from_slice(&(props_len as u64).to_le_bytes());
    out[start + 24] = n.name.len() as u8;
    Ok(())
}

// ============================================================================
// CONVERSIONS
// ============================================================================

/// FBX time of an absolute frame.
fn ktime(frame: i64, rate: FrameRate) -> i64 {
    let num = rate.num.max(1) as i128;
    let den = rate.den.max(1) as i128;
    let v   = frame as i128 * KTIME * den;
    ((v + v.signum() * num / 2) / num) as i64
}

/// FBX SDK time mode for a rate, and the custom rate when there is no match.
fn time_mode(rate: FrameRate, drop_frame: bool) -> (i32, f64) {
    let mode = match (rate.num, rate.den) {
        (120, 1) => 1, (100, 1) => 2, (60, 1) => 3, (50, 1) => 4, (48, 1) => 5,
        (30, 1) => 6,
        (30000, 1001) => if drop_frame { 8 } else { 9 },
        (25, 1) => 10, (24, 1) => 11, (1000, 1) => 12, (24000, 1001) => 13,
        (96, 1) => 15, (72, 1) => 16, (60000, 1001) => 17,
        _ => 14,
    };
    (mode, if mode == 14 { rate.fps() } else { -1.0 })
}

/// Quaternion to FBX XYZ Euler angles in degrees (R = Rz * Ry * Rx).
fn euler_xyz(q: Quat) -> [f64; 3] {
    let (z, y, x) = q.to_euler(EulerRot::ZYX);
    [x.to_degrees() as f64, y.to_degrees() as f64, z.to_degrees() as f64]
}

/// Pick the representation of `e` closest to `prev`, so curves stay
/// continuous across the +-180 wrap and interpolate cleanly between keys.
fn unroll(prev: [f64; 3], e: [f64; 3]) -> [f64; 3] {
    let near = |v: f64, r: f64| v + ((r - v) / 360.0).round() * 360.0;
    let a = [near(e[0], prev[0]), near(e[1], prev[1]), near(e[2], prev[2])];
    // Same rotation: (x + 180, 180 - y, z + 180).
    let b = [near(e[0] + 180.0, prev[0]), near(180.0 - e[1], prev[1]), near(e[2] + 180.0, prev[2])];
    let d = |c: [f64; 3]| (0..3).map(|i| (c[i] - prev[i]).abs()).sum::<f64>();
    if d(b) < d(a) { b } else { a }
}

fn matrix(m: Mat4) -> Prop {
    let mut a = m.to_cols_array();
    for i in 12..15 { a[i] *= CM; }
    Prop::ArrF64(a.iter().map(|x| *x as f64).collect())
}

// ============================================================================
// SCENE
// ============================================================================

struct Ids(i64);
impl Ids {
    fn next(&mut self) -> i64 { self.0 += 1; self.0 }
}

fn oo(src: i64, dst: i64) -> Node {
    leaf("C", vec![s("OO"), Prop::I64(src), Prop::I64(dst)])
}
fn op(src: i64, dst: i64, prop: &str) -> Node {
    leaf("C", vec![s("OP"), Prop::I64(src), Prop::I64(dst), s(prop)])
}

fn curve(id: i64, times: &[i64], values: Vec<f32>) -> Node {
    let n = times.len() as i32;
    node("AnimationCurve", vec![Prop::I64(id), named("", "AnimCurve"), s("")], vec![
        leaf("Default", vec![Prop::F64(values.first().copied().unwrap_or(0.0) as f64)]),
        leaf("KeyVer", vec![Prop::I32(4009)]),
        leaf("KeyTime", vec![Prop::ArrI64(times.to_vec())]),
        leaf("KeyValueFloat", vec![Prop::ArrF32(values)]),
        // One attribute block shared by every key: cubic, flat user tangents,
        // default weights. Same block the FBX SDK writes for baked data.
        leaf("KeyAttrFlags", vec![Prop::ArrI32(vec![8456])]),
        leaf("KeyAttrDataFloat", vec![Prop::ArrF32(vec![0.0, 0.0, f32::from_bits(218_434_821), 0.0])]),
        leaf("KeyAttrRefCount", vec![Prop::ArrI32(vec![n])]),
    ])
}

fn geometry(id: i64, name: &str, skin: &SkinMesh) -> Node {
    let verts: Vec<f64> = skin.positions.iter()
        .flat_map(|v| [(v.x * CM) as f64, (v.y * CM) as f64, (v.z * CM) as f64])
        .collect();
    let mut index: Vec<i32> = vec![];
    let mut normals: Vec<f64> = vec![];
    for f in &skin.faces {
        let n = if SkinMesh::is_tri(f) { 3 } else { 4 };
        for (k, vi) in f[..n].iter().enumerate() {
            // The last index of a polygon is stored bit-inverted.
            index.push(if k == n - 1 { !(*vi as i32) } else { *vi as i32 });
            let nr = skin.normals[*vi as usize];
            normals.extend([nr.x as f64, nr.y as f64, nr.z as f64]);
        }
    }
    let layer_el = |ty: &str| node("LayerElement", vec![], vec![
        leaf("Type", vec![s(ty)]), leaf("TypedIndex", vec![Prop::I32(0)]),
    ]);
    node("Geometry", vec![Prop::I64(id), named(name, "Geometry"), s("Mesh")], vec![
        leaf("Vertices", vec![Prop::ArrF64(verts)]),
        leaf("PolygonVertexIndex", vec![Prop::ArrI32(index)]),
        leaf("GeometryVersion", vec![Prop::I32(124)]),
        node("LayerElementNormal", vec![Prop::I32(0)], vec![
            leaf("Version", vec![Prop::I32(101)]),
            leaf("Name", vec![s("")]),
            leaf("MappingInformationType", vec![s("ByPolygonVertex")]),
            leaf("ReferenceInformationType", vec![s("Direct")]),
            leaf("Normals", vec![Prop::ArrF64(normals)]),
        ]),
        node("LayerElementSmoothing", vec![Prop::I32(0)], vec![
            leaf("Version", vec![Prop::I32(102)]),
            leaf("Name", vec![s("")]),
            leaf("MappingInformationType", vec![s("ByPolygon")]),
            leaf("ReferenceInformationType", vec![s("Direct")]),
            leaf("Smoothing", vec![Prop::ArrI32(vec![1; skin.faces.len()])]),
        ]),
        node("Layer", vec![Prop::I32(0)], vec![
            leaf("Version", vec![Prop::I32(100)]),
            layer_el("LayerElementNormal"),
            layer_el("LayerElementSmoothing"),
        ]),
    ])
}

/// Write `clip` to `path`. With `animation` false, or a single-frame clip,
/// the file holds the pose of the first frame and no animation.
pub fn write_fbx(path: &Path, clip: &AnimData, animation: bool) -> std::io::Result<WriteStats> {
    if clip.joints.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "clip has no joints"));
    }
    let frames    = clip.frames.max(1);
    let animated  = animation && frames > 1;
    let take      = if clip.name.is_empty() { "Take 001" } else { clip.name.as_str() };
    let t_start   = ktime(clip.start_frame, clip.rate);
    let t_stop    = ktime(clip.end_frame(), clip.rate);
    let mut ids   = Ids(1_000_000);
    let mut objects: Vec<Node> = vec![];
    let mut conns:   Vec<Node> = vec![];

    // ── Skeleton ─────────────────────────────────────────────────────────────
    let joint_ids: Vec<i64> = clip.joints.iter().map(|_| ids.next()).collect();
    for (j, joint) in clip.joints.iter().enumerate() {
        let (class, flags) = if joint.is_bone { ("LimbNode", "Skeleton") } else { ("Null", "Null") };
        let attr = ids.next();
        objects.push(node("NodeAttribute", vec![Prop::I64(attr), named(&joint.name, "NodeAttribute"), s(class)], vec![
            leaf("TypeFlags", vec![s(flags)]),
        ]));

        let has_track = animated && clip.tracks.get(j).map(|t| t.len() > 1).unwrap_or(false);
        let flag = if has_track { "A+" } else { "A" };
        let l = clip.local(j, 0);
        let mut props = vec![
            p_int("DefaultAttributeIndex", 0),
            p_vec("Lcl Translation", flag,
                [(l.translation.x * CM) as f64, (l.translation.y * CM) as f64, (l.translation.z * CM) as f64]),
            p_vec("Lcl Rotation", flag, euler_xyz(l.rotation)),
        ];
        if (l.scale - Vec3::ONE).abs().max_element() > 1e-6 {
            props.push(p_vec("Lcl Scaling", "A", [l.scale.x as f64, l.scale.y as f64, l.scale.z as f64]));
        }
        objects.push(node("Model", vec![Prop::I64(joint_ids[j]), named(&joint.name, "Model"), s(class)], vec![
            leaf("Version", vec![Prop::I32(232)]),
            node("Properties70", vec![], props),
            leaf("Shading", vec![Prop::Bool(true)]),
            leaf("Culling", vec![s("CullingOff")]),
        ]));
        conns.push(oo(joint_ids[j], joint.parent.map(|p| joint_ids[p]).unwrap_or(0)));
        conns.push(oo(attr, joint_ids[j]));
    }

    // ── Animation ────────────────────────────────────────────────────────────
    if animated {
        let stack = ids.next();
        let layer = ids.next();
        objects.push(node("AnimationStack", vec![Prop::I64(stack), named(take, "AnimStack"), s("")], vec![
            node("Properties70", vec![], vec![
                p_time("LocalStart", t_start), p_time("LocalStop", t_stop),
                p_time("ReferenceStart", t_start), p_time("ReferenceStop", t_stop),
            ]),
        ]));
        objects.push(node("AnimationLayer", vec![Prop::I64(layer), named("Base Layer", "AnimLayer"), s("")], vec![]));
        conns.push(oo(layer, stack));

        let times: Vec<i64> = (0..frames).map(|i| ktime(clip.start_frame + i as i64, clip.rate)).collect();
        for (j, _) in clip.joints.iter().enumerate() {
            let Some(track) = clip.tracks.get(j).filter(|t| t.len() > 1) else { continue };
            let n = frames.min(track.len());

            let mut tr: [Vec<f32>; 3] = Default::default();
            let mut ro: [Vec<f32>; 3] = Default::default();
            let mut prev = euler_xyz(track[0].rotation);
            for t in &track[..n] {
                let e = unroll(prev, euler_xyz(t.rotation));
                prev = e;
                for a in 0..3 {
                    tr[a].push(t.translation[a] * CM);
                    ro[a].push(e[a] as f32);
                }
            }

            for (label, prop, channels) in [("T", "Lcl Translation", tr), ("R", "Lcl Rotation", ro)] {
                let cn = ids.next();
                objects.push(node("AnimationCurveNode", vec![Prop::I64(cn), named(label, "AnimCurveNode"), s("")], vec![
                    node("Properties70", vec![], ["d|X", "d|Y", "d|Z"].iter().zip(&channels).map(|(ch, v)| {
                        p(ch, "Number", "", "A", vec![Prop::F64(v[0] as f64)])
                    }).collect()),
                ]));
                conns.push(oo(cn, layer));
                conns.push(op(cn, joint_ids[j], prop));
                for (ch, values) in ["d|X", "d|Y", "d|Z"].iter().zip(channels) {
                    let c = ids.next();
                    objects.push(curve(c, &times[..n], values));
                    conns.push(op(c, cn, ch));
                }
            }
        }
    }

    // ── Mesh, skin, bind pose ────────────────────────────────────────────────
    let mut vertices = 0;
    if let Some(skin) = clip.skin.as_ref().filter(|s| !s.positions.is_empty() && s.bind.len() == clip.joints.len()) {
        vertices = skin.positions.len();
        let base = if clip.subject.is_empty() { take } else { clip.subject.as_str() };
        let mesh_name = format!("{base}_Mesh");
        let (mesh, geo, deformer, pose) = (ids.next(), ids.next(), ids.next(), ids.next());

        objects.push(geometry(geo, &mesh_name, skin));
        objects.push(node("Model", vec![Prop::I64(mesh), named(&mesh_name, "Model"), s("Mesh")], vec![
            leaf("Version", vec![Prop::I32(232)]),
            node("Properties70", vec![], vec![p_int("DefaultAttributeIndex", 0)]),
            leaf("Shading", vec![Prop::Bool(true)]),
            leaf("Culling", vec![s("CullingOff")]),
        ]));
        objects.push(node("Deformer", vec![Prop::I64(deformer), named(&mesh_name, "Deformer"), s("Skin")], vec![
            leaf("Version", vec![Prop::I32(101)]),
            leaf("Link_DeformAcuracy", vec![Prop::F64(50.0)]),
        ]));
        conns.extend([oo(mesh, 0), oo(geo, mesh), oo(deformer, geo)]);

        let mut by_joint: Vec<Vec<i32>> = vec![vec![]; clip.joints.len()];
        for (vi, j) in skin.joint.iter().enumerate() {
            if let Some(list) = by_joint.get_mut(*j as usize) { list.push(vi as i32); }
        }
        for (j, verts) in by_joint.into_iter().enumerate() {
            if verts.is_empty() { continue; }
            let cluster = ids.next();
            let weights = vec![1.0f64; verts.len()];
            objects.push(node("Deformer", vec![Prop::I64(cluster), named(&clip.joints[j].name, "SubDeformer"), s("Cluster")], vec![
                leaf("Version", vec![Prop::I32(100)]),
                leaf("UserData", vec![s(""), s("")]),
                leaf("Indexes", vec![Prop::ArrI32(verts)]),
                leaf("Weights", vec![Prop::ArrF64(weights)]),
                // Mesh space to joint space at bind time, and the joint's world matrix.
                leaf("Transform", vec![matrix(skin.bind[j].inverse())]),
                leaf("TransformLink", vec![matrix(skin.bind[j])]),
            ]));
            conns.extend([oo(cluster, deformer), oo(joint_ids[j], cluster)]);
        }

        let mut pose_nodes = vec![node("PoseNode", vec![], vec![
            leaf("Node", vec![Prop::I64(mesh)]), leaf("Matrix", vec![matrix(Mat4::IDENTITY)]),
        ])];
        for (j, m) in skin.bind.iter().enumerate() {
            pose_nodes.push(node("PoseNode", vec![], vec![
                leaf("Node", vec![Prop::I64(joint_ids[j])]), leaf("Matrix", vec![matrix(*m)]),
            ]));
        }
        let mut kids = vec![
            leaf("Type", vec![s("BindPose")]),
            leaf("Version", vec![Prop::I32(100)]),
            leaf("NbPoseNodes", vec![Prop::I32(pose_nodes.len() as i32)]),
        ];
        kids.extend(pose_nodes);
        objects.push(node("Pose", vec![Prop::I64(pose), named("BindPose", "Pose"), s("BindPose")], kids));
    }

    // ── Definitions ──────────────────────────────────────────────────────────
    let mut counts: Vec<(&'static str, i32)> = vec![];
    for o in &objects {
        match counts.iter_mut().find(|c| c.0 == o.name) {
            Some(c) => c.1 += 1,
            None    => counts.push((o.name, 1)),
        }
    }
    let total = 1 + counts.iter().map(|c| c.1).sum::<i32>();
    let mut defs = vec![
        leaf("Version", vec![Prop::I32(100)]),
        leaf("Count", vec![Prop::I32(total)]),
        node("ObjectType", vec![s("GlobalSettings")], vec![leaf("Count", vec![Prop::I32(1)])]),
    ];
    for (ty, n) in &counts {
        defs.push(node("ObjectType", vec![s(ty)], vec![leaf("Count", vec![Prop::I32(*n)])]));
    }

    // ── Sections ─────────────────────────────────────────────────────────────
    let (mode, custom_rate) = time_mode(clip.rate, clip.drop_frame);
    let top = vec![
        node("FBXHeaderExtension", vec![], vec![
            leaf("FBXHeaderVersion", vec![Prop::I32(1003)]),
            leaf("FBXVersion", vec![Prop::I32(VERSION as i32)]),
            leaf("EncryptionType", vec![Prop::I32(0)]),
            node("CreationTimeStamp", vec![], vec![
                leaf("Version", vec![Prop::I32(1000)]),
                leaf("Year", vec![Prop::I32(2026)]), leaf("Month", vec![Prop::I32(7)]), leaf("Day", vec![Prop::I32(31)]),
                leaf("Hour", vec![Prop::I32(10)]), leaf("Minute", vec![Prop::I32(44)]), leaf("Second", vec![Prop::I32(37)]),
                leaf("Millisecond", vec![Prop::I32(785)]),
            ]),
            leaf("Creator", vec![s("XMS FBX writer")]),
            node("SceneInfo", vec![named("GlobalInfo", "SceneInfo"), s("UserData")], vec![
                leaf("Type", vec![s("UserData")]),
                leaf("Version", vec![Prop::I32(100)]),
                node("MetaData", vec![], vec![
                    leaf("Version", vec![Prop::I32(100)]),
                    leaf("Title", vec![s("XMS Scene")]), leaf("Subject", vec![s("Exported from XMS")]),
                    leaf("Author", vec![s("")]), leaf("Keywords", vec![s("")]),
                    leaf("Revision", vec![s("")]), leaf("Comment", vec![s("")]),
                ]),
                node("Properties70", vec![], vec![]),
            ]),
        ]),
        leaf("FileId", vec![Prop::Raw(FILE_ID.to_vec())]),
        leaf("CreationTime", vec![s(CREATION_TIME)]),
        leaf("Creator", vec![s("XMS FBX writer")]),
        node("GlobalSettings", vec![], vec![
            leaf("Version", vec![Prop::I32(1000)]),
            node("Properties70", vec![], vec![
                p_int("UpAxis", 1), p_int("UpAxisSign", 1),
                p_int("FrontAxis", 2), p_int("FrontAxisSign", 1),
                p_int("CoordAxis", 0), p_int("CoordAxisSign", 1),
                p_int("OriginalUpAxis", -1), p_int("OriginalUpAxisSign", 1),
                p_double("UnitScaleFactor", 1.0), p_double("OriginalUnitScaleFactor", 1.0),
                p("AmbientColor", "ColorRGB", "Color", "", vec![Prop::F64(0.0), Prop::F64(0.0), Prop::F64(0.0)]),
                p("DefaultCamera", "KString", "", "", vec![s("Producer Perspective")]),
                p_enum("TimeMode", mode), p_enum("TimeProtocol", 2), p_enum("SnapOnFrameMode", 0),
                p_time("TimeSpanStart", if animated { t_start } else { 0 }),
                p_time("TimeSpanStop", if animated { t_stop } else { KTIME as i64 }),
                p_double("CustomFrameRate", custom_rate),
                p("TimeMarker", "Compound", "", "", vec![]),
                p_int("CurrentTimeMarker", -1),
            ]),
        ]),
        node("Documents", vec![], vec![
            leaf("Count", vec![Prop::I32(1)]),
            node("Document", vec![Prop::I64(ids.next()), s("Scene"), s("Scene")], vec![
                node("Properties70", vec![], vec![
                    p("SourceObject", "object", "", "", vec![]),
                    p("ActiveAnimStackName", "KString", "", "", vec![s(if animated { take } else { "" })]),
                ]),
                leaf("RootNode", vec![Prop::I64(0)]),
            ]),
        ]),
        node("References", vec![], vec![]),
        node("Definitions", vec![], defs),
        node("Objects", vec![], objects),
        node("Connections", vec![], conns),
        node("Takes", vec![], {
            let mut k = vec![leaf("Current", vec![s(if animated { take } else { "" })])];
            if animated {
                k.push(node("Take", vec![s(take)], vec![
                    leaf("FileName", vec![s(&format!("{}.tak", take.replace(' ', "_")))]),
                    leaf("LocalTime", vec![Prop::I64(t_start), Prop::I64(t_stop)]),
                    leaf("ReferenceTime", vec![Prop::I64(t_start), Prop::I64(t_stop)]),
                ]));
            }
            k
        }),
    ];

    // ── Serialise ────────────────────────────────────────────────────────────
    let mut out: Vec<u8> = Vec::with_capacity(1 << 20);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    for n in &top { write_node(&mut out, n)?; }
    out.extend_from_slice(&[0u8; HEADER]);
    out.extend_from_slice(&FOOT_ID);
    out.extend_from_slice(&[0u8; 4]);
    let pad = 16 - out.len() % 16;
    out.extend(std::iter::repeat(0u8).take(pad));
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0u8; 120]);
    out.extend_from_slice(&FOOT_MAGIC);

    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() { std::fs::create_dir_all(dir)?; }
    }
    std::fs::write(path, out)?;

    Ok(WriteStats { joints: clip.joints.len(), frames: if animated { frames } else { 0 }, vertices })
}

/// Expand the tokens of an output path for one clip:
/// `{dir}` source folder, `{file}` source file name, `{char}` character,
/// `{take}` take name. Adds ".fbx" when the result has no extension.
pub fn resolve_path(pattern: &str, clip: &AnimData) -> std::path::PathBuf {
    let safe = |v: &str| v.trim().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file = if clip.source.is_empty() { safe(&clip.name) } else { safe(&clip.source) };
    let dir  = if clip.source_dir.is_empty() { ".".to_string() } else { clip.source_dir.clone() };
    let mut out = pattern.trim()
        .replace("{dir}", &dir)
        .replace("{file}", &file)
        .replace("{char}", &safe(&clip.subject))
        .replace("{take}", &safe(&clip.name));
    if !out.to_lowercase().ends_with(".fbx") { out.push_str(".fbx"); }
    std::path::PathBuf::from(out)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::{create_test_clip, PoseEdit};
    use crate::fbx_loader::load_fbx;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("xms_writer_{name}.fbx"))
    }

    fn close(a: Vec3, b: Vec3, eps: f32) -> bool { (a - b).length() < eps }

    /// Index in `back` of each joint of `c`. Readers return joints in
    /// hierarchy order, which need not be the order they were written in.
    fn remap(c: &AnimData, back: &AnimData) -> Vec<usize> {
        c.joints.iter()
            .map(|j| back.joints.iter().position(|b| b.name == j.name).expect("joint lost"))
            .collect()
    }

    #[test]
    fn animation_round_trips_through_ufbx() {
        for rate in [FrameRate::new(30, 1), FrameRate::new(120, 1), FrameRate::new(30000, 1001)] {
            let c = create_test_clip(2.0, rate);
            let path = tmp(&format!("anim_{}", rate.num));
            let stats = write_fbx(&path, &c, true).unwrap();
            assert_eq!(stats.frames, c.frames);

            let back = load_fbx(path.to_str().unwrap(), 0).unwrap().anim;
            assert_eq!(back.joints.len(), c.joints.len());
            assert_eq!(back.rate, c.rate);
            assert_eq!(back.frames, c.frames);
            assert_eq!(back.start_frame, c.start_frame);
            assert_eq!(back.name, c.name);
            let map = remap(&c, &back);
            for (j, b) in c.joints.iter().enumerate() {
                let a = &back.joints[map[j]];
                assert_eq!(a.parent.map(|p| back.joints[p].name.clone()), b.parent.map(|p| c.joints[p].name.clone()));
                assert!(a.is_bone);
            }
            for f in [0, 1, c.frames / 3, c.frames / 2, c.frames - 1] {
                let (pa, pb) = (back.world_pose(f), c.world_pose(f));
                for j in 0..c.joints.len() {
                    assert!(close(pa[map[j]].w_axis.truncate(), pb[j].w_axis.truncate(), 2e-4),
                        "rate {rate:?} frame {f} joint {j}: {:?} vs {:?}", pa[map[j]].w_axis, pb[j].w_axis);
                    let (qa, qb) = (back.local(map[j], f).rotation, c.local(j, f).rotation);
                    assert!(qa.angle_between(qb) < 2e-3, "rotation frame {f} joint {j}");
                }
            }
        }
    }

    #[test]
    fn large_rotations_stay_continuous() {
        // A joint spinning through several turns must not jump by 360 between keys.
        let mut c = create_test_clip(2.0, FrameRate::new(30, 1));
        let mut tracks = (*c.tracks).clone();
        for (i, t) in tracks[13].iter_mut().enumerate() {
            t.rotation = Quat::from_rotation_z(i as f32 * 0.4) * Quat::from_rotation_y(i as f32 * 0.15);
        }
        c.tracks = std::sync::Arc::new(tracks);
        let mut prev = euler_xyz(c.tracks[13][0].rotation);
        for t in c.tracks[13].iter() {
            let e = unroll(prev, euler_xyz(t.rotation));
            assert!((0..3).all(|a| (e[a] - prev[a]).abs() < 60.0), "{prev:?} -> {e:?}");
            prev = e;
        }
        let path = tmp("spin");
        write_fbx(&path, &c, true).unwrap();
        let back = load_fbx(path.to_str().unwrap(), 0).unwrap().anim;
        let j = remap(&c, &back)[13];
        for f in 0..c.frames {
            assert!(back.local(j, f).rotation.angle_between(c.local(13, f).rotation) < 2e-3, "frame {f}");
        }
    }

    #[test]
    fn static_pose_has_no_animation() {
        let t = create_test_clip(2.0, FrameRate::new(30, 1)).auto_tpose(None);
        let path = tmp("static");
        assert_eq!(write_fbx(&path, &t, true).unwrap().frames, 0);
        let loaded = load_fbx(path.to_str().unwrap(), 0).unwrap();
        assert!(loaded.takes.is_empty());
        assert_eq!(loaded.anim.frames, 1);
        let (pa, pb) = (loaded.anim.world_pose(0), t.world_pose(0));
        let map = remap(&t, &loaded.anim);
        for j in 0..t.joints.len() {
            assert!(close(pa[map[j]].w_axis.truncate(), pb[j].w_axis.truncate(), 1e-4));
        }
    }

    /// Reads the written file back through ufbx's own scene view, to check
    /// what a strict reader sees: node types, mesh, skin clusters, bind pose.
    #[test]
    fn skinned_tpose_is_a_valid_skeletal_mesh() {
        let mut c = create_test_clip(1.0, FrameRate::new(30, 1));
        c.joints[0].is_bone = false;          // helper root above the bones
        c.subject = "Skeleton_001".into();
        let t = c.auto_tpose(None)
            .pose_fixed(&[PoseEdit { joint: "Take01:LeftArm".into(), rotation: [0.0, 0.0, 80.0], translation: [0.0; 3] }])
            .with_proxy_skin(1.0);
        let skin = t.skin.clone().unwrap();
        let path = tmp("skin");
        let stats = write_fbx(&path, &t, true).unwrap();
        assert_eq!(stats.vertices, skin.positions.len());

        let scene = ufbx::load_file(path.to_str().unwrap(), ufbx::LoadOpts::default()).unwrap();
        assert_eq!(scene.metadata.version, 7500);
        assert!(!scene.metadata.ascii);
        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.skin_deformers.len(), 1);
        assert_eq!(scene.anim_stacks.len(), 0);

        let mesh = &scene.meshes[0];
        assert_eq!(mesh.num_vertices, skin.positions.len());
        assert_eq!(mesh.num_faces, skin.faces.len());
        assert_eq!(mesh.skin_deformers.len(), 1);
        let v0 = mesh.vertices[0];
        assert!(close(Vec3::new(v0.x as f32, v0.y as f32, v0.z as f32), skin.positions[0] * CM, 1e-3));

        // One cluster per bone, none for the helper root.
        let sd = &scene.skin_deformers[0];
        assert_eq!(sd.clusters.len(), 18);
        let total: usize = sd.clusters.iter().map(|c| c.num_weights).sum();
        assert_eq!(total, skin.positions.len());
        for cl in sd.clusters.iter() {
            let bone = cl.bone_node.as_ref().expect("cluster without bone");
            assert!(bone.bone.is_some(), "{} is not a bone", bone.element.name);
            let j = t.joints.iter().position(|x| x.name == *bone.element.name).unwrap();
            // geometry_to_bone maps bind-space vertices into the bone: the
            // bone's own origin must land on zero.
            let m = &cl.geometry_to_bone;
            let o = skin.bind[j].w_axis.truncate() * CM;
            let x = m.m00 as f32 * o.x + m.m01 as f32 * o.y + m.m02 as f32 * o.z + m.m03 as f32;
            let y = m.m10 as f32 * o.x + m.m11 as f32 * o.y + m.m12 as f32 * o.z + m.m13 as f32;
            let z = m.m20 as f32 * o.x + m.m21 as f32 * o.y + m.m22 as f32 * o.z + m.m23 as f32;
            assert!(Vec3::new(x, y, z).length() < 1e-2, "{}: {x} {y} {z}", bone.element.name);
        }

        // The root is a null, everything below it a bone, and the node's
        // world position matches the bind pose.
        let root = scene.nodes.iter().find(|n| *n.element.name == *"Take01:Hips").unwrap();
        assert!(root.bone.is_none());
        let arm = scene.nodes.iter().find(|n| *n.element.name == *"Take01:LeftForeArm").unwrap();
        assert!(arm.bone.is_some());
        let w = &arm.node_to_world;
        let j = t.joints.iter().position(|x| x.name == "Take01:LeftForeArm").unwrap();
        assert!(close(Vec3::new(w.m03 as f32, w.m13 as f32, w.m23 as f32), skin.bind[j].w_axis.truncate() * CM, 1e-2));
        assert_eq!(scene.poses.len(), 1);
        assert!(scene.poses[0].is_bind_pose);
        assert_eq!(scene.poses[0].bone_poses.len(), t.joints.len() + 1);
    }

    #[test]
    fn output_path_tokens() {
        let mut c = create_test_clip(1.0, FrameRate::new(30, 1));
        c.source = "S3_13_001".into();
        c.source_dir = "/data/day01".into();
        c.subject = "Skeleton_002".into();
        assert_eq!(
            resolve_path("{dir}/split/{file}_{char}_tpose.fbx", &c),
            std::path::PathBuf::from("/data/day01/split/S3_13_001_Skeleton_002_tpose.fbx"));
        assert_eq!(resolve_path("/out/{take}", &c), std::path::PathBuf::from("/out/TestWalk.fbx"));
    }

    #[test]
    fn ktime_matches_fbx() {
        assert_eq!(ktime(1, FrameRate::new(30, 1)), 1_539_538_600);
        assert_eq!(ktime(120, FrameRate::new(120, 1)), 46_186_158_000);
        assert_eq!(ktime(30000, FrameRate::new(30000, 1001)), 46_186_158_000 * 1001);
    }
}
