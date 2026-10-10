//! Example USD stages for the templates that show composed stages: files
//! that reference each other, a sublayer, a variant set, curves and points,
//! a point instancer, native instances and purposes.
//!
//! Generated, like the other USD examples, by `examples::write_all`:
//!
//!     cargo test write_example_files -- --ignored

use std::fmt::Write as _;

/// The stage examples, in `examples/stage/`.
pub const STREET:   &str = "stage/street.usda";
pub const STREET_BASE: &str = "stage/street_base.usda";
pub const STRANDS:  &str = "stage/strands.usda";
pub const FOREST:   &str = "stage/forest.usda";
pub const TABLES:   &str = "stage/tables.usda";
pub const LAMPS:    &str = "stage/lamps.usda";

/// Every stage example: file name and text.
pub fn all() -> Vec<(&'static str, String)> {
    vec![
        (STREET, street()),
        (STREET_BASE, street_base()),
        (STRANDS, strands()),
        (FOREST, forest()),
        (TABLES, tables()),
        (LAMPS, lamps()),
    ]
}

// ── Building blocks ──────────────────────────────────────────────────────────

/// A polygon mesh: points and faces, each face counter-clockwise seen from outside.
struct Poly { pts: Vec<[f32; 3]>, faces: Vec<Vec<u32>> }

impl Poly {
    fn add(&mut self, other: Poly) {
        let base = self.pts.len() as u32;
        self.pts.extend(other.pts);
        self.faces.extend(other.faces.into_iter().map(|f| f.into_iter().map(|i| i + base).collect()));
    }
}

/// A box from its low corner to its high corner.
fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> Poly {
    let [x0, y0, z0] = lo;
    let [x1, y1, z1] = hi;
    Poly {
        pts: vec![[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        faces: vec![vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7]],
    }
}

/// A cone standing on the ground plane of its own space, from `y` up to `y + h`.
fn cone(r: f32, y: f32, h: f32, sides: u32) -> Poly {
    let mut pts: Vec<[f32; 3]> = (0..sides).map(|i| {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        [r * a.cos(), y, r * a.sin()]
    }).collect();
    pts.push([0.0, y + h, 0.0]);
    let apex = sides;
    let mut faces: Vec<Vec<u32>> = (0..sides).map(|i| vec![(i + 1) % sides, i, apex]).collect();
    faces.push((0..sides).collect());
    Poly { pts, faces }
}

/// A closed cylinder from `y` up to `y + h`.
fn cylinder(r: f32, y: f32, h: f32, sides: u32) -> Poly {
    let ring = |yy: f32| (0..sides).map(move |i| {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        [r * a.cos(), yy, r * a.sin()]
    });
    let mut pts: Vec<[f32; 3]> = ring(y).collect();
    pts.extend(ring(y + h));
    let mut faces: Vec<Vec<u32>> = (0..sides).map(|i| { let j = (i + 1) % sides; vec![j, i, i + sides, j + sides] }).collect();
    faces.push((0..sides).collect());
    faces.push((0..sides).rev().map(|i| i + sides).collect());
    Poly { pts, faces }
}

/// A rounded stone: a sphere of `rings` by `sides`, squashed.
fn stone(r: f32, squash: f32, rings: u32, sides: u32) -> Poly {
    let mut pts = vec![[0.0, -r * squash, 0.0]];
    for k in 1..rings {
        let t = k as f32 / rings as f32 * std::f32::consts::PI;
        let (y, rr) = (-t.cos() * r * squash, t.sin() * r);
        for i in 0..sides {
            let a = i as f32 / sides as f32 * std::f32::consts::TAU;
            pts.push([rr * a.cos(), y, rr * a.sin()]);
        }
    }
    pts.push([0.0, r * squash, 0.0]);
    let top = pts.len() as u32 - 1;
    let at = |ring: u32, i: u32| 1 + (ring - 1) * sides + i % sides;
    let mut faces = vec![];
    for i in 0..sides { faces.push(vec![0, at(1, i), at(1, i + 1)]); }
    for k in 1..rings - 1 {
        for i in 0..sides { faces.push(vec![at(k, i), at(k + 1, i), at(k + 1, i + 1), at(k, i + 1)]); }
    }
    for i in 0..sides { faces.push(vec![at(rings - 1, i + 1), at(rings - 1, i), top]); }
    Poly { pts, faces }
}

fn num(v: f32) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn vec3(p: &[f32; 3]) -> String { format!("({}, {}, {})", num(p[0]), num(p[1]), num(p[2])) }

fn list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    items.iter().map(f).collect::<Vec<_>>().join(", ")
}

/// A Mesh prim, indented by `ind` spaces. `extra` goes inside it.
fn mesh(name: &str, p: &Poly, ind: usize, extra: &str) -> String {
    let pad = " ".repeat(ind);
    let counts = list(&p.faces, |f| f.len().to_string());
    let idx = p.faces.iter().flatten().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
    let mut s = format!("{pad}def Mesh \"{name}\"\n{pad}{{\n");
    let _ = writeln!(s, "{pad}    point3f[] points = [{}]", list(&p.pts, vec3));
    let _ = writeln!(s, "{pad}    int[] faceVertexCounts = [{counts}]");
    let _ = writeln!(s, "{pad}    int[] faceVertexIndices = [{idx}]");
    let _ = writeln!(s, "{pad}    uniform token subdivisionScheme = \"none\"");
    for line in extra.lines() { let _ = writeln!(s, "{pad}    {line}"); }
    let _ = writeln!(s, "{pad}}}");
    s
}

fn translate(t: [f32; 3]) -> String {
    format!("double3 xformOp:translate = {}\nuniform token[] xformOpOrder = [\"xformOp:translate\"]", vec3(&t))
}

fn header(default_prim: &str, extra: &str) -> String {
    format!("#usda 1.0\n(\n    defaultPrim = \"{default_prim}\"\n    upAxis = \"Y\"\n    metersPerUnit = 1\n{extra})\n\n")
}

/// A small, repeatable random number generator: the files come out the same every time.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }
}

// ── Composition: a street ────────────────────────────────────────────────────

/// The base of the street, brought in by the root layer as a sublayer:
/// the ground and two kerbs.
pub fn street_base() -> String {
    let mut s = header("Street", "");
    s += "def Xform \"Street\"\n{\n";
    s += &mesh("Road", &cuboid([-6.0, -0.05, -2.0], [6.0, 0.0, 2.0]), 4, "");
    s += &mesh("PavementNorth", &cuboid([-6.0, -0.05, 2.0], [6.0, 0.12, 3.5]), 4, "");
    s += &mesh("PavementSouth", &cuboid([-6.0, -0.05, -3.5], [6.0, 0.12, -2.0]), 4, "");
    s += "}\n";
    s
}

/// The street as the program sees it: the base as a sublayer, three tables
/// referenced from `table.usda`, an override that moves one pavement in the
/// root layer, and a variant set that dresses the tables or not.
pub fn street() -> String {
    let mut s = header("Street", "    subLayers = [\n        @./street_base.usda@\n    ]\n");
    s += "over \"Street\" (\n    variants = {\n        string dressing = \"cafe\"\n    }\n    prepend variantSets = \"dressing\"\n)\n{\n";
    // An opinion in the root layer, stronger than the sublayer's.
    s += "    over \"PavementSouth\"\n    {\n        double3 xformOp:translate = (0, 0, -0.8)\n        uniform token[] xformOpOrder = [\"xformOp:translate\"]\n    }\n\n";
    for (k, x) in [-3.0f32, 0.0, 3.0].iter().enumerate() {
        let _ = write!(s, "    def Xform \"Table{}\" (\n        prepend references = @../table.usda@\n    )\n    {{\n", k + 1);
        for line in translate([*x, 0.12, 2.75]).lines() { let _ = writeln!(s, "        {line}"); }
        s += "    }\n\n";
    }
    s += "    variantSet \"dressing\" = {\n";
    s += "        \"cafe\" {\n";
    for (k, x) in [-3.0f32, 0.0, 3.0].iter().enumerate() {
        let _ = write!(s, "            def Xform \"Shapes{}\" (\n                prepend references = @../shapes.usda@\n            )\n            {{\n", k + 1);
        let _ = writeln!(s, "                double3 xformOp:translate = {}", vec3(&[*x, 0.89, 2.75]));
        s += "                float3 xformOp:scale = (0.25, 0.25, 0.25)\n";
        s += "                uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:scale\"]\n            }\n";
    }
    s += "        }\n        \"empty\" {\n        }\n    }\n}\n";
    s
}

// ── Curves and points ────────────────────────────────────────────────────────

/// One BasisCurves prim of each basis, a patch of grass as linear strands,
/// and a spiral of points of growing width.
pub fn strands() -> String {
    let mut s = header("Strands", "");
    s += "def Xform \"Strands\"\n{\n";
    let curve = |name: &str, kind: &str, pts: &[[f32; 3]], counts: &[u32], width: f32| {
        format!("    def BasisCurves \"{name}\"\n    {{\n{kind}        int[] curveVertexCounts = [{}]\n        point3f[] points = [{}]\n        float[] widths = [{}] (\n            interpolation = \"constant\"\n        )\n    }}\n\n",
            list(counts, |c| c.to_string()), list(pts, vec3), num(width))
    };
    // A zig-zag, straight between its points.
    let zig: Vec<[f32; 3]> = (0..7).map(|i| [-4.0 + i as f32 * 0.4, if i % 2 == 0 { 0.2 } else { 1.0 }, 0.0]).collect();
    s += &curve("Linear", "        uniform token type = \"linear\"\n", &zig, &[7], 0.03);
    // Two Bezier segments: 3 points a segment, plus the first.
    let bez = [[-1.2f32, 0.2, 0.0], [-1.2, 1.4, 0.0], [0.0, 1.4, 0.0], [0.0, 0.6, 0.0], [0.0, -0.2, 0.0], [1.2, -0.2, 0.0], [1.2, 1.0, 0.0]];
    s += &curve("Bezier", "        uniform token type = \"cubic\"\n        uniform token basis = \"bezier\"\n", &bez, &[7], 0.03);
    // Catmull-Rom through every point.
    let cr: Vec<[f32; 3]> = (0..8).map(|i| [2.0 + i as f32 * 0.35, 0.6 + 0.4 * (i as f32 * 1.3).sin(), 0.0]).collect();
    s += &curve("CatmullRom", "        uniform token type = \"cubic\"\n        uniform token basis = \"catmullRom\"\n", &cr, &[8], 0.03);
    // A closed B-spline ring.
    let ring: Vec<[f32; 3]> = (0..8).map(|i| { let a = i as f32 / 8.0 * std::f32::consts::TAU; [5.5 + 0.7 * a.cos(), 0.7 + 0.7 * a.sin(), 0.0] }).collect();
    s += &curve("Ring", "        uniform token type = \"cubic\"\n        uniform token basis = \"bspline\"\n        uniform token wrap = \"periodic\"\n", &ring, &[8], 0.03);
    // Grass: 600 strands of 5 points, bending in the wind.
    let mut rng = Lcg(7);
    let (mut pts, mut counts) = (vec![], vec![]);
    for _ in 0..600 {
        let (x, z) = (-3.0 + rng.next() * 6.0, -3.5 + rng.next() * 2.0);
        let h = 0.25 + rng.next() * 0.35;
        let lean = 0.1 + rng.next() * 0.15;
        for k in 0..5 {
            let t = k as f32 / 4.0;
            pts.push([x + lean * t * t, h * t, z + 0.03 * t]);
        }
        counts.push(5);
    }
    s += &curve("Grass", "        uniform token type = \"linear\"\n", &pts, &counts, 0.004);
    // A spiral of 400 points, wider as it climbs.
    let mut spiral = vec![];
    let mut widths = vec![];
    for i in 0..400 {
        let t = i as f32 / 400.0;
        let a = t * 6.0 * std::f32::consts::TAU;
        spiral.push([(0.3 + 0.8 * t) * a.cos(), 0.05 + 2.0 * t, -2.5 + (0.3 + 0.8 * t) * a.sin()]);
        widths.push(0.01 + 0.05 * t);
    }
    let _ = write!(s, "    def Points \"Spiral\"\n    {{\n        double3 xformOp:translate = (6, 0, 0)\n        uniform token[] xformOpOrder = [\"xformOp:translate\"]\n        point3f[] points = [{}]\n        float[] widths = [{}] (\n            interpolation = \"vertex\"\n        )\n    }}\n",
        list(&spiral, vec3), list(&widths, |w| num(*w)));
    s += "}\n";
    s
}

// ── A point instancer: a forest ──────────────────────────────────────────────

/// Trees per side of the forest: this squared is how many copies are drawn.
pub const FOREST_SIDE: usize = 70;

/// A forest of pines and stones, one PointInstancer with two prototypes:
/// 4,900 copies, each turned and scaled on its own.
pub fn forest() -> String {
    let mut s = header("Forest", "");
    s += "def Xform \"Forest\"\n{\n";
    let half = FOREST_SIDE as f32 * 0.6;
    s += &mesh("Ground", &cuboid([-half - 1.0, -0.1, -half - 1.0], [half + 1.0, 0.0, half + 1.0]), 4, "");
    let mut rng = Lcg(11);
    let (mut pos, mut idx, mut rot, mut scale) = (vec![], vec![], vec![], vec![]);
    for i in 0..FOREST_SIDE {
        for j in 0..FOREST_SIDE {
            let x = -half + (i as f32 + rng.next()) * 1.2;
            let z = -half + (j as f32 + rng.next()) * 1.2;
            let stone = rng.next() < 0.15;
            idx.push(if stone { 1 } else { 0 });
            pos.push([x, 0.0, z]);
            // A turn about the up axis, as a half-precision quaternion: (real, i, j, k).
            let a = rng.next() * std::f32::consts::TAU;
            rot.push(format!("({}, 0, {}, 0)", num((a * 0.5).cos()), num((a * 0.5).sin())));
            let k = if stone { 0.5 + rng.next() * 0.8 } else { 0.7 + rng.next() * 0.6 };
            scale.push([k, k * if stone { 0.7 } else { 1.0 }, k]);
        }
    }
    s += "    def PointInstancer \"Trees\"\n    {\n";
    s += "        rel prototypes = [</Forest/Trees/Prototypes/Pine>, </Forest/Trees/Prototypes/Stone>]\n";
    let _ = writeln!(s, "        int[] protoIndices = [{}]", list(&idx, |i| i.to_string()));
    let _ = writeln!(s, "        point3f[] positions = [{}]", list(&pos, vec3));
    let _ = writeln!(s, "        quath[] orientations = [{}]", rot.join(", "));
    let _ = writeln!(s, "        float3[] scales = [{}]", list(&scale, vec3));
    s += "\n        def Scope \"Prototypes\"\n        {\n";
    s += "            def Xform \"Pine\"\n            {\n";
    s += &mesh("Trunk", &cylinder(0.08, 0.0, 0.5, 8), 16, "");
    let mut crown = cone(0.55, 0.4, 0.9, 10);
    crown.add(cone(0.42, 0.9, 0.8, 10));
    crown.add(cone(0.28, 1.4, 0.7, 10));
    s += &mesh("Crown", &crown, 16, "");
    s += "            }\n\n            def Xform \"Stone\"\n            {\n";
    s += &mesh("Rock", &stone(0.35, 0.6, 6, 10), 16, "");
    s += "            }\n        }\n    }\n}\n";
    s
}

// ── Native instances: a hall of tables ───────────────────────────────────────

/// Tables per side of the hall.
pub const TABLES_SIDE: usize = 8;

/// A hall of tables, each an instanceable reference to `table.usda`: one
/// prototype, shared by every table.
pub fn tables() -> String {
    let mut s = header("Hall", "");
    s += "def Xform \"Hall\"\n{\n";
    let w = TABLES_SIDE as f32 * 2.2;
    s += &mesh("Floor", &cuboid([-1.5, -0.05, -1.5], [w, 0.0, w * 0.6]), 4, "");
    for i in 0..TABLES_SIDE {
        for j in 0..TABLES_SIDE {
            let _ = write!(s, "    def Xform \"Table_{i}_{j}\" (\n        instanceable = true\n        prepend references = @../table.usda@\n    )\n    {{\n");
            let _ = writeln!(s, "        double3 xformOp:translate = {}", vec3(&[i as f32 * 2.2, 0.0, j as f32 * 1.3]));
            let _ = writeln!(s, "        float xformOp:rotateY = {}", if (i + j) % 2 == 0 { 0 } else { 180 });
            s += "        uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:rotateY\"]\n    }\n\n";
        }
    }
    s += "}\n";
    s
}

// ── Purposes: street lamps ───────────────────────────────────────────────────

/// Three street lamps, each with render geometry (fine), proxy geometry (a
/// box and a stick) and a guide (the cone of light, as lines).
pub fn lamps() -> String {
    let mut s = header("Lamps", "");
    s += "def Xform \"Lamps\"\n{\n";
    for (k, x) in [-2.5f32, 0.0, 2.5].iter().enumerate() {
        let _ = write!(s, "    def Xform \"Lamp{}\"\n    {{\n", k + 1);
        for line in translate([*x, 0.0, 0.0]).lines() { let _ = writeln!(s, "        {line}"); }
        s += "\n        def Xform \"Render\"\n        {\n            uniform token purpose = \"render\"\n";
        s += &mesh("Base", &cylinder(0.18, 0.0, 0.15, 32), 12, "");
        s += &mesh("Pole", &cylinder(0.05, 0.15, 2.6, 24), 12, "");
        let mut head = cylinder(0.09, 2.75, 0.12, 24);
        head.add(cone(0.35, 2.55, 0.32, 32));
        s += &mesh("Shade", &head, 12, "");
        s += "        }\n\n        def Xform \"Proxy\"\n        {\n            uniform token purpose = \"proxy\"\n";
        let mut proxy = cuboid([-0.18, 0.0, -0.18], [0.18, 0.15, 0.18]);
        proxy.add(cuboid([-0.06, 0.15, -0.06], [0.06, 2.75, 0.06]));
        proxy.add(cuboid([-0.35, 2.55, -0.35], [0.35, 2.9, 0.35]));
        s += &mesh("Blocks", &proxy, 12, "");
        s += "        }\n\n        def Xform \"Guide\"\n        {\n            uniform token purpose = \"guide\"\n";
        // The cone of light: lines from the lamp down to a circle on the ground.
        let mut pts = vec![];
        for i in 0..12 {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            pts.push([0.0, 2.55, 0.0]);
            pts.push([1.6 * a.cos(), 0.0, 1.6 * a.sin()]);
        }
        let _ = write!(s, "            def BasisCurves \"LightCone\"\n            {{\n                uniform token type = \"linear\"\n                int[] curveVertexCounts = [{}]\n                point3f[] points = [{}]\n            }}\n",
            list(&vec![2u32; 12], |c| c.to_string()), list(&pts, vec3));
        s += "        }\n    }\n\n";
    }
    s += "}\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every closed mesh of the examples encloses a positive volume: its
    /// faces point out.
    #[test]
    fn the_building_blocks_are_closed_and_face_out() {
        let vol = |p: &Poly| -> f32 {
            let mut v = 0.0;
            for f in &p.faces {
                for k in 1..f.len() - 1 {
                    let (a, b, c) = (p.pts[f[0] as usize], p.pts[f[k] as usize], p.pts[f[k + 1] as usize]);
                    v += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0;
                }
            }
            v
        };
        let closed = |p: &Poly| {
            let mut edges = std::collections::HashMap::new();
            for f in &p.faces { for k in 0..f.len() { *edges.entry((f[k], f[(k + 1) % f.len()])).or_insert(0) += 1; } }
            edges.iter().all(|((a, b), n)| *n == 1 && edges.get(&(*b, *a)) == Some(&1))
        };
        for (name, p) in [
            ("cuboid", cuboid([0.0; 3], [1.0, 2.0, 3.0])), ("cone", cone(1.0, 0.0, 2.0, 10)),
            ("cylinder", cylinder(0.5, 0.0, 1.0, 12)), ("stone", stone(1.0, 0.6, 6, 10)),
        ] {
            assert!(closed(&p), "{name} is not closed");
            assert!(vol(&p) > 0.0, "{name} faces in: {}", vol(&p));
        }
    }

    /// The stages, written out and read back through composition.
    #[test]
    fn the_stages_compose_as_described() {
        use crate::types::{Placement, Purpose};
        let dir = std::env::temp_dir().join("xms_stage_examples");
        let _ = std::fs::remove_dir_all(&dir);
        crate::examples::write_all(&dir).unwrap();
        let read = |name: &str| crate::usd_stage::read(&dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let names = |sc: &crate::usd_scene::UsdScene| sc.meshes.iter().map(|m| m.path.clone()).collect::<Vec<_>>();

        // The street: the sublayer's ground, three referenced tables, the
        // dressing of the selected variant, and the root layer's override.
        let street = read(STREET);
        let n = names(&street);
        assert!(n.iter().any(|p| p == "/Street/Road"), "{n:?}");
        assert_eq!(n.iter().filter(|p| p.starts_with("/Street/Table")).count(), 15, "{n:?}");
        assert_eq!(n.iter().filter(|p| p.starts_with("/Street/Shapes")).count(), 9, "{n:?}");
        let south = street.meshes.iter().find(|m| m.path == "/Street/PavementSouth").unwrap();
        // Prims are read in their own space, placed by their transform.
        let world = match &south.place { Placement::One(m) => *m, _ => bevy::math::Mat4::IDENTITY };
        let z_max = south.mesh.vertices.iter().map(|v| world.transform_point3(bevy::math::Vec3::from_array(*v)).z).fold(f32::MIN, f32::max);
        assert!((z_max - -2.8).abs() < 1e-3, "the override moves the pavement: {z_max}");

        // Curves of each basis, the grass, and the spiral of points.
        let strands = read(STRANDS);
        assert_eq!(strands.meshes.len(), 6, "{:?}", names(&strands));
        let grass = strands.meshes.iter().find(|m| m.path.ends_with("Grass")).unwrap();
        assert_eq!(grass.mesh.curve_counts.len(), 600);
        let spiral = strands.meshes.iter().find(|m| m.path.ends_with("Spiral")).unwrap();
        assert_eq!((spiral.mesh.points.len(), spiral.mesh.widths.len()), (400, 400));

        // The forest: one shared mesh per prototype mesh, every tree a placement.
        let forest = read(FOREST);
        let copies = |end: &str| forest.meshes.iter().filter(|m| m.path.ends_with(end)).map(|m| m.place.copies()).sum::<usize>();
        assert_eq!(copies("Trunk") + copies("Rock"), FOREST_SIDE * FOREST_SIDE);
        assert_eq!(copies("Crown"), copies("Trunk"));
        assert!(forest.meshes.iter().filter(|m| !m.path.ends_with("Ground")).all(|m| matches!(m.place, Placement::Many(_))));

        // The hall: every table is the same five meshes, placed.
        let hall = read(TABLES);
        let tables: Vec<_> = hall.meshes.iter().filter(|m| m.path.contains("Table_")).collect();
        assert_eq!(tables.len(), TABLES_SIDE * TABLES_SIDE * 5);
        let mut shared: Vec<*const crate::types::MeshData> = tables.iter().map(|m| std::sync::Arc::as_ptr(&m.mesh)).collect();
        shared.sort(); shared.dedup();
        assert_eq!(shared.len(), 5, "one prototype for every table");

        // The lamps: render, proxy and guide geometry, told apart.
        let lamps = read(LAMPS);
        let count = |f: &dyn Fn(Purpose) -> bool| lamps.meshes.iter().filter(|m| f(m.purpose)).count();
        assert_eq!(count(&|p| p == Purpose::Render { has_proxy: true }), 9);
        assert_eq!(count(&|p| p == Purpose::Proxy), 3);
        assert_eq!(count(&|p| p == Purpose::Guide), 3);
    }
}
