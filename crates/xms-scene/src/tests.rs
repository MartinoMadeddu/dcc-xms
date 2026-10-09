use crate::*;

fn p(s: &str) -> Path {
    Path::new(s).unwrap()
}

fn xform(path: &str, samples: Vec<(f64, [f64; 3])>) -> Prim {
    let mut prim = Prim::new(p(path), PrimKind::Group);
    prim.local_xform = Sampled::from_samples(samples.into_iter().map(|(t, v)| (t, Mat4d::translation(v))).collect());
    prim
}

#[test]
fn paths() {
    let a = p("/World/Lion/body/");
    assert_eq!(a.as_str(), "/World/Lion/body");
    assert_eq!(a.name(), "body");
    assert_eq!(a.parent().unwrap().as_str(), "/World/Lion");
    assert_eq!(p("/World").parent().unwrap(), Path::root());
    assert!(Path::root().parent().is_none());
    assert_eq!(Path::root().child("World").unwrap().as_str(), "/World");
    assert!(a.has_prefix(&p("/World")));
    assert!(!p("/WorldX").has_prefix(&p("/World")));
    assert!(Path::new("World").is_err());
    assert!(Path::new("/World/a.b").is_err());
    assert!(Path::new("/World//x").is_err());
}

#[test]
fn sampled_interpolation_and_holding() {
    let s = Sampled::from_samples(vec![(2.0, 20.0f32), (0.0, 0.0), (1.0, 10.0), (1.0, 12.0)]);
    assert_eq!(s.samples().len(), 3, "sorted, duplicate time replaced");
    assert_eq!(s.at(-5.0), Some(0.0), "held before the first sample");
    assert_eq!(s.at(0.5), Some(6.0));
    assert_eq!(s.at(1.0), Some(12.0));
    assert_eq!(s.at(9.0), Some(20.0), "held after the last sample");
    assert!(Sampled::<f32>::default().at(0.0).is_none());
    assert!(!Sampled::constant(3.0f32).is_animated());
    // Arrays of different lengths hold instead of interpolating
    let v = Sampled::from_samples(vec![(0.0, vec![[0.0f32; 3]]), (1.0, vec![[1.0; 3], [2.0; 3]])]);
    assert_eq!(v.at(0.5).unwrap().len(), 1);
    // Booleans hold
    let b = Sampled::from_samples(vec![(0.0, true), (1.0, false)]);
    assert_eq!(b.at(0.99), Some(true));
    assert_eq!(b.at(1.0), Some(false));
}

#[test]
fn samples_within_shutter() {
    let s = Sampled::from_samples(vec![(0.0, 0.0f32), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
    let t = |open: f64, close: f64| s.within(open, close).iter().map(|x| x.0).collect::<Vec<_>>();
    assert_eq!(t(0.5, 1.5), vec![0.0, 1.0, 2.0], "bracketing samples on both sides");
    assert_eq!(t(1.0, 2.0), vec![1.0, 2.0]);
    assert_eq!(t(5.0, 6.0), vec![3.0], "after the last sample: held");
    assert_eq!(t(-2.0, -1.0), vec![0.0], "before the first sample: held");
}

#[test]
fn world_transforms_and_reset() {
    let mut s = Scene::new();
    s.insert(xform("/A", vec![(0.0, [1.0, 0.0, 0.0])]));
    s.insert(xform("/A/B", vec![(0.0, [0.0, 2.0, 0.0])]));
    let w = s.world_xform_at(&p("/A/B"), 0.0);
    assert_eq!(w.transform_point([0.0; 3]), [1.0, 2.0, 0.0]);
    s.edit(&p("/A/B"), Dirty::TRANSFORM, |prim| prim.reset_xform_stack = true);
    assert_eq!(s.world_xform_at(&p("/A/B"), 0.0).transform_point([0.0; 3]), [0.0, 2.0, 0.0]);
}

#[test]
fn motion_samples_follow_animated_ancestors() {
    let mut s = Scene::new();
    s.insert(xform("/A", vec![(0.0, [0.0; 3]), (1.0, [10.0, 0.0, 0.0])]));
    s.insert(xform("/A/B", vec![(0.0, [0.0, 1.0, 0.0])]));
    let m = s.world_xform_samples(&p("/A/B"), 0.25, 0.75);
    assert_eq!(m.len(), 2, "interval ends");
    assert_eq!(m[0].1.transform_point([0.0; 3]), [2.5, 1.0, 0.0]);
    assert_eq!(m[1].1.transform_point([0.0; 3]), [7.5, 1.0, 0.0]);
    // A sample inside the interval is included
    s.edit(&p("/A"), Dirty::TRANSFORM, |prim| {
        prim.local_xform = Sampled::from_samples(vec![
            (0.0, Mat4d::translation([0.0; 3])),
            (0.5, Mat4d::translation([1.0, 0.0, 0.0])),
            (1.0, Mat4d::translation([0.0; 3])),
        ])
    });
    let m = s.world_xform_samples(&p("/A/B"), 0.25, 0.75);
    assert_eq!(m.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0.25, 0.5, 0.75]);
    // Static: one sample
    s.insert(xform("/C", vec![(0.0, [1.0; 3])]));
    assert_eq!(s.world_xform_samples(&p("/C"), 0.25, 0.75).len(), 1);
}

#[test]
fn insert_remove_and_changes() {
    let mut s = Scene::new();
    s.insert(Prim::new(p("/World/Lion/body"), PrimKind::Mesh(Box::default())));
    assert!(s.contains(&p("/World")) && s.contains(&p("/World/Lion")), "ancestors created");
    let c = s.take_changes();
    assert_eq!(c.added.len(), 3);
    assert!(c.dirty.is_empty(), "added prims aren't also dirty");
    assert!(s.take_changes().is_empty());

    // Inherited dirty bits reach descendants; others don't
    s.edit(&p("/World"), Dirty::TRANSFORM | Dirty::PARAMS, |_| {});
    let c = s.take_changes();
    let get = |path: &str| c.dirty.iter().find(|d| d.0.as_str() == path).map(|d| d.1);
    assert_eq!(get("/World"), Some(Dirty::TRANSFORM | Dirty::PARAMS));
    assert_eq!(get("/World/Lion/body"), Some(Dirty::TRANSFORM));

    // Removing a subtree lists every prim; add-then-remove cancels out
    s.insert(Prim::new(p("/World/Tmp"), PrimKind::Group));
    s.remove(&p("/World/Tmp"));
    s.remove(&p("/World/Lion"));
    let c = s.take_changes();
    assert!(c.added.is_empty());
    assert_eq!(c.removed.len(), 2);
    assert_eq!(s.children(&p("/World")).len(), 0);

    // Added, removed and added again: reported once
    s.insert(Prim::new(p("/World/X"), PrimKind::Group));
    s.remove(&p("/World/X"));
    s.insert(Prim::new(p("/World/X"), PrimKind::Group));
    assert_eq!(s.take_changes().added, vec![p("/World/X")]);
}

#[test]
fn inherited_binding_and_visibility() {
    let mut s = Scene::new();
    let mut lion = Prim::new(p("/World/Lion"), PrimKind::Group);
    lion.material_binding = Some(p("/Looks/Gold"));
    lion.visible = Sampled::from_samples(vec![(0.0, true), (10.0, false)]);
    s.insert(lion);
    s.insert(Prim::new(p("/World/Lion/body"), PrimKind::Mesh(Box::default())));
    assert_eq!(s.resolved_material(&p("/World/Lion/body")), Some(p("/Looks/Gold")));
    assert!(s.visible_at(&p("/World/Lion/body"), 5.0));
    assert!(!s.visible_at(&p("/World/Lion/body"), 10.0));
    assert_eq!(s.traverse(&Path::root()).iter().map(|x| x.as_str().to_string()).collect::<Vec<_>>(), vec!["/", "/World", "/World/Lion", "/World/Lion/body"]);
}

#[test]
fn quaternion_nlerp() {
    let a: Quatf = [0.0, 0.0, 0.0, 1.0];
    let b: Quatf = [0.0, 0.0, -0.0, -1.0]; // same rotation, opposite sign
    let q = nlerp(&a, &b, 0.5);
    assert!((q[3].abs() - 1.0).abs() < 1e-6, "shortest arc, normalized");
    let c: Quatf = [0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2];
    let q = nlerp(&a, &c, 0.5);
    let len: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((len - 1.0).abs() < 1e-6);
}

#[test]
fn shutter_interval() {
    let t = SceneTime { time: 10.0, shutter_open: -0.25, shutter_close: 0.25 };
    assert_eq!(t.interval(), (9.75, 10.25));
    assert!(t.has_motion_blur());
    assert!(!SceneTime::default().has_motion_blur());
}

#[test]
fn summary_counts() {
    let mut s = Scene::new();
    let mut mesh = Mesh::default();
    mesh.points = Sampled::from_samples(vec![(0.0, vec![[0.0f32; 3]; 4]), (1.0, vec![[1.0; 3]; 4])]);
    mesh.face_vertex_counts = vec![4];
    mesh.subdivision.scheme = Some("catmullClark".into());
    s.insert(Prim::new(p("/World/m"), PrimKind::Mesh(Box::new(mesh))));
    s.insert(Prim::new(p("/World/ball"), PrimKind::Gprim(Box::new(Gprim::Sphere { radius: 1.0 }))));
    s.insert(xform("/World/spin", vec![(0.0, [0.0; 3]), (1.0, [1.0, 0.0, 0.0])]));
    let sum = s.summary();
    assert_eq!((sum.meshes, sum.mesh_faces, sum.mesh_points, sum.subdivision_meshes), (1, 1, 4, 1));
    assert_eq!(sum.gprims, 1);
    assert_eq!((sum.animated_points, sum.animated_xforms, sum.samples), (1, 1, 4));
    assert!(sum.to_string().contains("meshes 1"));
}
