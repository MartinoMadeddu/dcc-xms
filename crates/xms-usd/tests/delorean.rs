use std::path::Path;
use xms_scene::{Path as PrimPath, PrimKind, Scene};

fn count(sc: &Scene, p: &PrimPath, meshes: &mut usize, materials: &mut usize, total: &mut usize) {
    for c in sc.children(p) {
        *total += 1;
        match sc.get(c).map(|prim| &prim.kind) {
            Some(PrimKind::Mesh(_)) => *meshes += 1,
            Some(PrimKind::Material(_)) => *materials += 1,
            _ => {}
        }
        count(sc, c, meshes, materials, total);
    }
}

#[test]
fn delorean() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/DeLorean.usdz");
    let tr = xms_usd::translate(&file, Default::default()).expect("translate");
    let (mut meshes, mut materials, mut total) = (0, 0, 0);
    count(&tr.scene, &PrimPath::root(), &mut meshes, &mut materials, &mut total);
    for w in &tr.warnings {
        println!("note: {w}");
    }
    println!("{total} prims · {meshes} meshes · {materials} materials · {:.2} s", tr.seconds);
    assert_eq!(meshes, 43);
}