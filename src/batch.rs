//! Running Write FBX nodes, for the current file or for every file of the
//! folder loaders upstream. The work runs on its own thread so the window
//! stays responsive; progress is read back through `BatchState`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;

use crate::fbx_loader::list_fbx;
use crate::fbx_writer::{resolve_path, write_fbx};
use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, NodeType};

#[derive(Default)]
pub struct Progress {
    pub running: bool,
    pub done:    usize,
    pub total:   usize,
    pub written: usize,
    pub failed:  usize,
    pub log:     Vec<String>,
}

#[derive(Resource, Default, Clone)]
pub struct BatchState(pub Arc<Mutex<Progress>>);

/// Every Write FBX node of the graph.
pub fn write_nodes(graph: &NodeGraphState) -> Vec<NodeId> {
    graph.nodes.iter()
        .filter(|n| matches!(n.node_type, NodeType::WriteFbx { .. }))
        .map(|n| n.id)
        .collect()
}

/// Folder loaders that feed any of the given nodes.
pub fn upstream_folder_loaders(graph: &NodeGraphState, targets: &[NodeId]) -> Vec<NodeId> {
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut todo: Vec<NodeId> = targets.to_vec();
    let mut out = vec![];
    while let Some(id) = todo.pop() {
        if !seen.insert(id) { continue; }
        let Some(node) = graph.nodes.iter().find(|n| n.id == id) else { continue };
        if matches!(node.node_type, NodeType::LoadFbxDir { .. }) { out.push(id); }
        todo.extend(node.inputs.iter().filter_map(|i| i.connected_output.map(|(src, _)| src)));
    }
    out
}

/// Number of files a batch over these loaders covers.
pub fn file_count(graph: &NodeGraphState, loaders: &[NodeId]) -> usize {
    loaders.iter().filter_map(|id| graph.nodes.iter().find(|n| n.id == *id)).map(|n| match &n.node_type {
        NodeType::LoadFbxDir { dir, .. } => list_fbx(dir).len(),
        _ => 0,
    }).max().unwrap_or(0)
}

/// Run the given Write nodes once against the graph as it stands.
fn write_once(graph: &NodeGraphState, targets: &[NodeId], progress: &Mutex<Progress>) {
    for id in targets {
        let Some(node) = graph.nodes.iter().find(|n| n.id == *id) else { continue };
        let NodeType::WriteFbx { path, mesh } = &node.node_type else { continue };
        let line = match graph.eval_anim(*id) {
            None => Err(format!("{}: no clip reaches this node", node.name)),
            Some(clip) => {
                let out = resolve_path(path, &clip);
                match write_fbx(&out, &clip, true, *mesh) {
                    Ok(s) => Ok(format!(
                        "{}  ({} joints, {}{})",
                        out.display(), s.joints,
                        if s.frames > 0 { format!("{} frames", s.frames) } else { "pose only".into() },
                        if s.vertices > 0 { format!(", mesh {} verts", s.vertices) } else { String::new() },
                    )),
                    Err(e) => Err(format!("{}: {}: {e}", node.name, out.display())),
                }
            }
        };
        let mut p = progress.lock().unwrap();
        match line {
            Ok(l)  => { p.written += 1; p.log.push(l); }
            Err(l) => { p.failed += 1;  p.log.push(format!("FAILED  {l}")); }
        }
    }
}

/// Blocking version of the batch. `all_files` steps every upstream folder
/// loader through its whole folder.
pub fn run(graph: &NodeGraphState, targets: &[NodeId], all_files: bool, progress: &Mutex<Progress>) {
    let loaders = upstream_folder_loaders(graph, targets);
    let count   = if all_files { file_count(graph, &loaders) } else { 0 };

    if !all_files || loaders.is_empty() {
        progress.lock().unwrap().total = 1;
        write_once(graph, targets, progress);
        progress.lock().unwrap().done = 1;
    } else {
        progress.lock().unwrap().total = count;
        let mut g = graph.clone();
        for i in 0..count {
            let mut name = String::new();
            for n in g.nodes.iter_mut().filter(|n| loaders.contains(&n.id)) {
                if let NodeType::LoadFbxDir { dir, index, .. } = &mut n.node_type {
                    *index = i as u32;
                    if let Some(f) = list_fbx(dir).get(i) {
                        name = f.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                    }
                }
            }
            progress.lock().unwrap().log.push(format!("[{}/{}] {}", i + 1, count, name));
            write_once(&g, targets, progress);
            progress.lock().unwrap().done = i + 1;
        }
    }
    let mut p = progress.lock().unwrap();
    let summary = format!("done: {} written, {} failed", p.written, p.failed);
    p.log.push(summary);
    p.running = false;
}

/// Start a batch in the background. Does nothing while one is running.
pub fn start(state: &BatchState, graph: &NodeGraphState, targets: Vec<NodeId>, all_files: bool) {
    {
        let mut p = state.0.lock().unwrap();
        if p.running { return; }
        *p = Progress { running: true, ..Default::default() };
    }
    let graph = graph.clone();
    let shared = state.0.clone();
    std::thread::spawn(move || run(&graph, &targets, all_files, &shared));
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fbx_loader::load_fbx;
    use crate::graph_io::mocap_split_template;
    use crate::types::{NodeType, SplitPick};
    use bevy::math::Vec3;

    /// Whole pipeline on generated data: two characters in two files, split,
    /// written as animation and as skinned T-pose, for every file.
    #[test]
    fn template_batch_writes_every_file() {
        let dir = std::env::temp_dir().join("xms_batch_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Source takes: two test characters under helper roots.
        let c = crate::core::anim::create_test_clip(1.0, crate::core::anim::FrameRate::new(60, 1));
        let mut joints = vec![];
        let mut tracks = vec![];
        for name in ["Skeleton 001", "Skeleton 002"] {
            let base = joints.len();
            let mut root = crate::core::anim::Joint::new(format!("{name}_Root"), None, Default::default());
            root.is_bone = false;
            joints.push(root);
            tracks.push(vec![]);
            for (j, joint) in c.joints.iter().enumerate() {
                let mut joint = joint.clone();
                joint.name   = format!("{name}_{}", joint.name.trim_start_matches("Take01:"));
                joint.parent = Some(joint.parent.map(|p| base + 1 + p).unwrap_or(base));
                joints.push(joint);
                tracks.push(c.tracks[j].clone());
            }
        }
        let two = crate::core::anim::AnimData { joints, tracks: std::sync::Arc::new(tracks), ..c };
        for take in ["take_A", "take_B", "take_C"] {
            write_fbx(&dir.join(format!("{take}.fbx")), &two, true, true).unwrap();
        }

        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        for n in &mut g.nodes {
            if let NodeType::LoadFbxDir { dir: d, .. } = &mut n.node_type { *d = dir.to_string_lossy().to_string(); }
        }
        let targets = write_nodes(&g);
        assert_eq!(targets.len(), 4);
        assert_eq!(file_count(&g, &upstream_folder_loaders(&g, &targets)), 3);

        let progress = Mutex::new(Progress::default());
        run(&g, &targets, true, &progress);
        let p = progress.lock().unwrap();
        assert_eq!((p.written, p.failed, p.done), (12, 0, 3), "{:#?}", p.log);

        for take in ["take_A", "take_B", "take_C"] {
            for ch in ["Skeleton_001", "Skeleton_002"] {
                let anim = load_fbx(dir.join("split").join(format!("{take}_{ch}.fbx")).to_str().unwrap(), 0).unwrap().anim;
                assert_eq!(anim.joints.len(), 20);
                assert_eq!(anim.frames, 60);
                assert!(!anim.joints[0].is_bone && anim.joints[1].is_bone);
                assert!(anim.joints.iter().all(|j| j.name.starts_with(&ch.replace('_', " "))));

                let tpose = dir.join("split").join(format!("{take}_{ch}_tpose.fbx"));
                let scene = ufbx::load_file(tpose.to_str().unwrap(), ufbx::LoadOpts::default()).unwrap();
                assert_eq!(scene.anim_stacks.len(), 0);
                assert_eq!(scene.meshes.len(), 1);
                assert_eq!(scene.skin_deformers[0].clusters.len(), 19);
                // Same bones, same hierarchy, in the animation and the T-pose.
                let t = load_fbx(tpose.to_str().unwrap(), 0).unwrap().anim;
                let names = |a: &crate::core::anim::AnimData| a.joints.iter()
                    .map(|j| (j.name.clone(), j.parent.map(|p| a.joints[p].name.clone()))).collect::<Vec<_>>();
                assert_eq!(names(&t), names(&anim));
            }
        }

        // A second output that matches nothing fails loudly and leaves the rest alone.
        for n in &mut g.nodes {
            if let NodeType::SplitSkeleton { picks } = &mut n.node_type { picks[1] = SplitPick::Joint("nope".into()); }
        }
        let progress = Mutex::new(Progress::default());
        run(&g, &targets, false, &progress);
        let p = progress.lock().unwrap();
        assert_eq!((p.written, p.failed), (2, 2), "{:#?}", p.log);
    }

    /// Real capture. Set XMS_SAMPLE_DIR to a folder holding the take, and
    /// XMS_REFERENCE_DIR to the output of the reference Python script.
    #[test]
    fn sample_take_matches_reference() {
        let (Ok(dir), Ok(reference)) = (std::env::var("XMS_SAMPLE_DIR"), std::env::var("XMS_REFERENCE_DIR")) else { return };
        let _ = std::fs::remove_dir_all(std::path::Path::new(&dir).join("split"));

        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        for n in &mut g.nodes {
            if let NodeType::LoadFbxDir { dir: d, .. } = &mut n.node_type { *d = dir.clone(); }
        }
        let targets = write_nodes(&g);
        let progress = Mutex::new(Progress::default());
        let t0 = std::time::Instant::now();
        run(&g, &targets, true, &progress);
        let p = progress.lock().unwrap();
        println!("{:#?}\nbatch took {:?}", p.log, t0.elapsed());
        assert_eq!(p.failed, 0);

        let close = |a: Vec3, b: Vec3, eps: f32| (a - b).length() < eps;
        for ch in ["Skeleton_001", "Skeleton_002"] {
            // Animation: same joints, range and motion as the reference split.
            let name = format!("S3_13_001_{ch}.fbx");
            let mine = load_fbx(&format!("{dir}/split/{name}"), 0).unwrap().anim;
            let refr = load_fbx(&format!("{reference}/{name}"), 0).unwrap().anim;
            assert_eq!(mine.joints.len(), refr.joints.len());
            assert_eq!((mine.frames, mine.start_frame, mine.rate), (refr.frames, refr.start_frame, refr.rate));
            for (a, b) in mine.joints.iter().zip(&refr.joints) {
                assert_eq!((&a.name, a.parent, a.is_bone), (&b.name, b.parent, b.is_bone));
            }
            let mut worst = 0.0f32;
            let mut worst_rot = 0.0f32;
            for f in (0..mine.frames).step_by(37).chain([mine.frames - 1]) {
                let (pa, pb) = (mine.world_pose(f), refr.world_pose(f));
                for j in 0..mine.joints.len() {
                    worst = worst.max((pa[j].w_axis - pb[j].w_axis).length());
                    worst_rot = worst_rot.max(mine.local(j, f).rotation.angle_between(refr.local(j, f).rotation));
                }
            }
            println!("{ch}: worst position error {:.4} mm, worst rotation error {:.5} deg", worst * 1000.0, worst_rot.to_degrees());
            assert!(worst < 2e-4 && worst_rot < 2e-3);

            // T-pose: same bone positions and mesh size as the reference.
            let name = format!("S3_13_001_{ch}_tpose.fbx");
            let mine = load_fbx(&format!("{dir}/split/{name}"), 0).unwrap().anim;
            let refr = load_fbx(&format!("{reference}/{name}"), 0).unwrap().anim;
            let (pa, pb) = (mine.world_pose(0), refr.world_pose(0));
            for j in 0..mine.joints.len() {
                assert_eq!(mine.joints[j].name, refr.joints[j].name);
                assert!(close(pa[j].w_axis.truncate(), pb[j].w_axis.truncate(), 1e-5), "{}", mine.joints[j].name);
                assert!(mine.local(j, 0).rotation.angle_between(bevy::math::Quat::IDENTITY) < 1e-5);
            }
            let sa = ufbx::load_file(&format!("{dir}/split/{name}"), ufbx::LoadOpts::default()).unwrap();
            let sb = ufbx::load_file(&format!("{reference}/{name}"), ufbx::LoadOpts::default()).unwrap();
            assert_eq!(sa.meshes[0].num_vertices, sb.meshes[0].num_vertices);
            assert_eq!(sa.meshes[0].num_faces, sb.meshes[0].num_faces);
            assert_eq!(sa.skin_deformers[0].clusters.len(), sb.skin_deformers[0].clusters.len());
            assert_eq!(sa.anim_stacks.len(), 0);
            let mut worst_v = 0.0f64;
            for (a, b) in sa.meshes[0].vertices.iter().zip(sb.meshes[0].vertices.iter()) {
                worst_v = worst_v.max(((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt());
            }
            println!("{ch}: T-pose mesh {} verts, worst vertex difference {:.5} cm", sa.meshes[0].num_vertices, worst_v);
            assert!(worst_v < 1e-3);
        }
    }
}
