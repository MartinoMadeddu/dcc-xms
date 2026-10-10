//! Writing what a scene network changed as a USD override layer.
//!
//! The layer sublayers the file each stage was read from and holds only
//! opinions over it: `over` prims with the attributes that changed. Opened on
//! its own (in usdview, rray, Imago), it composes to what Imago shows. What
//! changed comes from `crate::edits`: attributes no longer shared with the
//! loaded source.
//!
//! - Moved prims get their own transform again (`xformOp:transform`), so
//!   they keep following their parents.
//! - Changed geometry writes the changed attributes: `points` (and
//!   `extent`), topology, normals, widths, and primvars with their
//!   interpolation and indices. Attributes a prim lost are blocked (`None`).
//! - Geometry that can no longer be traced to its source (several prims
//!   merged into one) is written whole, in the prim's own space.
//! - Pruned prims are deactivated (`active = false`).
//! - Prims that did not come from a stage are written as new prims (`def`).
//! - Material binding and purpose changes are written as such.
//!
//! Prims inside a native instance cannot carry opinions in USD (instance
//! proxies are read-only), and point-instancer prototypes are changed
//! through their instancer: their edits are reported, not written, for now.
//!
//! Values are written at the current frame, as single values.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::math::{DMat4, Mat4};

use crate::core::geo::{Attr, Column, Context, Geo, Role, Topology, NORMALS, POINTS, WIDTHS};
use crate::edits::{edit_set, PrimEdit};
use crate::types::{CurveBasis, CurveWrap, NamedMesh, Placement, Purpose};
use crate::usd_scene::{StageNode, StageTree};

/// The last result of each Write USD node, for its properties panel.
static LAST: std::sync::Mutex<Vec<(crate::types::NodeId, Result<WriteReport, String>)>> = std::sync::Mutex::new(Vec::new());

pub fn remember(node: crate::types::NodeId, result: Result<WriteReport, String>) {
    let mut last = LAST.lock().unwrap();
    last.retain(|(n, _)| *n != node);
    last.push((node, result));
}

pub fn last(node: crate::types::NodeId) -> Option<Result<WriteReport, String>> {
    LAST.lock().unwrap().iter().find(|(n, _)| *n == node).map(|(_, r)| r.clone())
}

/// What a write did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WriteReport {
    /// Prims with opinions over their stage.
    pub changed:     usize,
    /// Prims deactivated.
    pub deactivated: usize,
    /// Prims written as new.
    pub added:       usize,
    /// Edits that could not be written, and why.
    pub skipped:     Vec<String>,
}

/// Write the override layer for these primitives to `out`.
pub fn write_override(prims: &[NamedMesh], out: &Path) -> Result<WriteReport, String> {
    if let Some(dir) = out.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let (text, report) = override_layer(prims, out)?;
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(report)
}

/// The override layer as text, for a layer to be written at `out` (paths to
/// the sources are relative to it).
pub fn override_layer(prims: &[NamedMesh], out: &Path) -> Result<(String, WriteReport), String> {
    // Each stage once.
    let mut trees: Vec<Arc<StageTree>> = vec![];
    for p in prims {
        if let Some(t) = &p.stage {
            if !trees.iter().any(|x| Arc::ptr_eq(x, t)) { trees.push(t.clone()); }
        }
    }
    let mut root = Spec::default();
    let mut report = WriteReport::default();

    for tree in &trees {
        let nodes: BTreeMap<&str, &StageNode> = tree.nodes.iter().map(|n| (n.path.as_str(), n)).collect();
        let set = edit_set(prims, tree);
        let by_path: BTreeMap<&str, &NamedMesh> = prims.iter()
            .filter(|p| p.stage.as_ref().is_some_and(|t| Arc::ptr_eq(t, tree)))
            .map(|p| (p.path.as_str(), p)).collect();

        for e in &set.prims {
            let (Some(p), Some(node)) = (by_path.get(e.path.as_str()), nodes.get(e.path.as_str())) else { continue };
            if let Some(why) = read_only(node) {
                report.skipped.push(format!("{}: {why}", e.path));
                continue;
            }
            let spec = root.at(&e.path);
            prim_opinions(spec, p, e, node, tree);
            report.changed += 1;
        }
        for path in &set.removed {
            if let Some(why) = nodes.get(path.as_str()).and_then(|n| read_only(n)) {
                report.skipped.push(format!("{path}: removed, but {why}"));
                continue;
            }
            root.at(path).meta.push("active = false".into());
            report.deactivated += 1;
        }
        for path in &set.added {
            if let Some(p) = by_path.get(path.as_str()) {
                new_prim(root.at(path), p, tree.root);
                report.added += 1;
            }
        }
    }
    // Prims that came from no stage at all.
    for p in prims.iter().filter(|p| p.stage.is_none()) {
        if p.path.is_empty() || !p.path.starts_with('/') { continue; }
        new_prim(root.at(&p.path), p, Mat4::IDENTITY);
        report.added += 1;
    }

    // The layer: the stages' files as sublayers, their axis and unit (stage
    // metadata only comes from the root layer), then the opinions.
    let mut text = String::from("#usda 1.0\n(\n    doc = \"Imago override layer\"\n");
    if let Some(t) = trees.first() {
        if let Some(mpu) = t.meters_per_unit { let _ = writeln!(text, "    metersPerUnit = {}", num64(mpu)); }
        if !t.up_axis.is_empty() { let _ = writeln!(text, "    upAxis = \"{}\"", t.up_axis); }
    }
    if !trees.is_empty() {
        let dir = out.parent().unwrap_or(Path::new("."));
        let layers: Vec<String> = trees.iter().map(|t| format!("@{}@", relative(&t.source, dir).display())).collect();
        let _ = writeln!(text, "    subLayers = [\n        {}\n    ]", layers.join(",\n        "));
    }
    text.push_str(")\n");
    for (name, child) in &root.children {
        text.push('\n');
        child.write(&mut text, name, 0);
    }
    Ok((text, report))
}

/// Why a prim cannot carry opinions, if it cannot.
fn read_only(node: &StageNode) -> Option<&'static str> {
    if node.proxy {
        Some("inside a native instance, which USD keeps read-only (edit the prototype, or de-instance it)")
    } else if node.prototype {
        Some("a point instancer's prototype (changing it through its instancer comes later)")
    } else {
        None
    }
}

// ── Prim specs ──────────────────────────────────────────────────────────────

/// A prim in the layer: `over` unless given a type (`def`), its metadata and
/// attribute lines, and its children by name.
#[derive(Default)]
struct Spec {
    def:      Option<&'static str>,
    meta:     Vec<String>,
    lines:    Vec<String>,
    children: BTreeMap<String, Spec>,
}

impl Spec {
    /// The spec at a path, made (as `over`s) on the way.
    fn at(&mut self, path: &str) -> &mut Spec {
        path.split('/').filter(|s| !s.is_empty()).fold(self, |s, name| s.children.entry(name.to_string()).or_default())
    }

    fn write(&self, out: &mut String, name: &str, depth: usize) {
        let pad = "    ".repeat(depth);
        match self.def {
            Some(ty) => { let _ = write!(out, "{pad}def {ty} \"{name}\""); }
            None => { let _ = write!(out, "{pad}over \"{name}\""); }
        }
        if !self.meta.is_empty() {
            let _ = write!(out, " (\n{}\n{pad})", self.meta.iter().map(|m| format!("{pad}    {m}")).collect::<Vec<_>>().join("\n"));
        }
        let _ = writeln!(out, "\n{pad}{{");
        for l in &self.lines { let _ = writeln!(out, "{pad}    {l}"); }
        for (i, (n, c)) in self.children.iter().enumerate() {
            if i > 0 || !self.lines.is_empty() { out.push('\n'); }
            c.write(out, n, depth + 1);
        }
        let _ = writeln!(out, "{pad}}}");
    }
}

/// The opinions for one edited prim of a stage.
fn prim_opinions(spec: &mut Spec, p: &NamedMesh, e: &PrimEdit, node: &StageNode, tree: &StageTree) {
    let src = p.source.as_ref().expect("edits come from prims with a source");
    // Its placement when loaded: root · parents · its own transform.
    let loaded = match &src.place { Placement::One(m) => m.as_dmat4(), _ => DMat4::IDENTITY };

    if let Some(Placement::One(now)) = &e.place {
        // Same parents, new own transform: L' = L · loaded⁻¹ · now.
        let local = node.local * loaded.inverse() * now.as_dmat4();
        spec.lines.push(format!("matrix4d xformOp:transform = {}", matrix(&local)));
        let order = if node.reset_xform { "[\"!resetXformStack!\", \"xformOp:transform\"]" } else { "[\"xformOp:transform\"]" };
        spec.lines.push(format!("uniform token[] xformOpOrder = {order}"));
    }
    if let Some(m) = &e.material {
        bind(spec, m.as_deref());
    }
    if let Some(purpose) = e.purpose {
        spec.lines.push(format!("uniform token purpose = \"{}\"", purpose_token(purpose)));
    }

    if e.replaced {
        // The whole geometry, from the mesh as drawn, back in the prim's own space.
        let drawn = match &p.place { Placement::One(m) => m.as_dmat4(), _ => DMat4::IDENTITY };
        let to_local = (loaded.inverse() * drawn).as_mat4();
        let local = crate::node_graph::nodes::place_mesh(&p.mesh, &to_local);
        let geo = Geo::from_mesh(&local);
        geometry(spec, &geo, true);
        // What the source had and this does not: blocked.
        for ctx in Context::ALL {
            for (name, attr) in src.geo.attrs(ctx) {
                if geo.attr(ctx, name).is_none() { block(spec, name, attr); }
            }
        }
        return;
    }
    let Some(geo) = &p.geo else { return };
    if e.topology { topology(spec, &geo.topology, true); }
    for (ctx, name) in &e.attrs {
        if let Some(attr) = geo.attr(*ctx, name) { attribute(spec, *ctx, name, attr); }
        if &**name == POINTS { extent(spec, geo); }
    }
    for (ctx, name) in &e.removed {
        if let Some(attr) = src.geo.attr(*ctx, name) { block(spec, name, attr); }
    }
    let _ = tree;
}

/// A prim that is not in the stage (or not in any): written whole, as new.
fn new_prim(spec: &mut Spec, p: &NamedMesh, root: Mat4) {
    let geo = p.geo.as_deref().cloned().unwrap_or_else(|| Geo::from_mesh(&p.mesh));
    spec.def = Some(match geo.topology {
        Topology::Mesh { .. } => "Mesh",
        Topology::Curves { .. } => "BasisCurves",
        Topology::Points => "Points",
    });
    // Its placement, without the root correction the file's axes do not have.
    let place = match &p.place { Placement::One(m) => (root.as_dmat4().inverse() * m.as_dmat4()), _ => root.as_dmat4().inverse() };
    if place != DMat4::IDENTITY {
        spec.lines.push(format!("matrix4d xformOp:transform = {}", matrix(&place)));
        spec.lines.push("uniform token[] xformOpOrder = [\"xformOp:transform\"]".into());
    }
    if let Some(m) = &p.material { bind(spec, Some(m)); }
    if p.purpose != Purpose::Default {
        spec.lines.push(format!("uniform token purpose = \"{}\"", purpose_token(p.purpose)));
    }
    geometry(spec, &geo, false);
}

/// Every attribute and the topology of a geometry.
fn geometry(spec: &mut Spec, geo: &Geo, over: bool) {
    topology(spec, &geo.topology, over);
    for ctx in Context::ALL {
        for (name, attr) in geo.attrs(ctx) { attribute(spec, ctx, name, attr); }
    }
    extent(spec, geo);
}

fn topology(spec: &mut Spec, t: &Topology, over: bool) {
    match t {
        Topology::Points => {}
        Topology::Mesh { counts, indices, left_handed, .. } => {
            spec.lines.push(format!("int[] faceVertexCounts = {}", list(counts.iter(), |c| c.to_string())));
            spec.lines.push(format!("int[] faceVertexIndices = {}", list(indices.iter(), |i| i.to_string())));
            if *left_handed || over {
                let o = if *left_handed { "leftHanded" } else { "rightHanded" };
                spec.lines.push(format!("uniform token orientation = \"{o}\""));
            }
        }
        Topology::Curves { counts, basis, wrap } => {
            let (ty, b) = match basis {
                CurveBasis::Linear => ("linear", "bezier"),
                CurveBasis::Bezier => ("cubic", "bezier"),
                CurveBasis::Bspline => ("cubic", "bspline"),
                CurveBasis::CatmullRom => ("cubic", "catmullRom"),
            };
            let w = match wrap { CurveWrap::Nonperiodic => "nonperiodic", CurveWrap::Periodic => "periodic", CurveWrap::Pinned => "pinned" };
            spec.lines.push(format!("int[] curveVertexCounts = {}", list(counts.iter(), |c| c.to_string())));
            spec.lines.push(format!("uniform token type = \"{ty}\""));
            spec.lines.push(format!("uniform token basis = \"{b}\""));
            spec.lines.push(format!("uniform token wrap = \"{w}\""));
        }
    }
}

/// The USD attribute name of an attribute: its own for the schema's
/// (`points`, `normals`, `widths`), `primvars:` for the rest.
fn usd_name(name: &str) -> String {
    match name {
        POINTS | NORMALS | WIDTHS => name.to_string(),
        _ => format!("primvars:{name}"),
    }
}

/// An attribute, with its interpolation and indices.
fn attribute(spec: &mut Spec, ctx: Context, name: &str, attr: &Attr) {
    let Some((ty, values)) = typed_values(&attr.column, attr.role) else { return };
    let usd = usd_name(name);
    if name == POINTS {
        spec.lines.push(format!("point3f[] points = {values}"));
        return;
    }
    spec.lines.push(format!("{ty}[] {usd} = {values} (\n    interpolation = \"{}\"\n)", ctx.usd_interpolation()));
    if let Some(ix) = &attr.indices {
        spec.lines.push(format!("int[] {usd}:indices = {}", list(ix.iter(), |i| i.to_string())));
    }
}

/// An attribute the prim no longer has: blocked, so the weaker opinion is
/// not seen through.
fn block(spec: &mut Spec, name: &str, attr: &Attr) {
    let Some((ty, _)) = typed_values(&attr.column, attr.role) else { return };
    let usd = usd_name(name);
    spec.lines.push(format!("{ty}[] {usd} = None"));
    if attr.indices.is_some() { spec.lines.push(format!("int[] {usd}:indices = None")); }
}

/// The bounds of the points, which USD wants to match them.
fn extent(spec: &mut Spec, geo: &Geo) {
    if let Some((lo, hi)) = geo.bounds() {
        spec.lines.push(format!("float3[] extent = [{}, {}]", v3(&lo), v3(&hi)));
    }
}

fn bind(spec: &mut Spec, material: Option<&str>) {
    spec.meta.push("prepend apiSchemas = [\"MaterialBindingAPI\"]".into());
    match material {
        Some(m) => spec.lines.push(format!("rel material:binding = <{m}>")),
        None => spec.lines.push("rel material:binding = None".into()),
    }
}

fn purpose_token(p: Purpose) -> &'static str {
    match p {
        Purpose::Default => "default",
        Purpose::Render { .. } => "render",
        Purpose::Proxy => "proxy",
        Purpose::Guide => "guide",
    }
}

/// The USD array type of a column, and its values as text.
fn typed_values(c: &Column, role: Role) -> Option<(&'static str, String)> {
    Some(match c {
        Column::Float(v) => ("float", list(v.iter(), |x| num(*x))),
        Column::Int(v) => ("int", list(v.iter(), |x| x.to_string())),
        Column::Bool(v) => ("bool", list(v.iter(), |x| x.to_string())),
        Column::Vec2(v) => (if role == Role::TexCoord { "texCoord2f" } else { "float2" }, list(v.iter(), |p| format!("({}, {})", num(p[0]), num(p[1])))),
        Column::Vec3(v) => (match role {
            Role::Point => "point3f",
            Role::Normal => "normal3f",
            Role::Vector => "vector3f",
            Role::Color => "color3f",
            _ => "float3",
        }, list(v.iter(), v3)),
        Column::Vec4(v) => (if role == Role::Color { "color4f" } else { "float4" },
            list(v.iter(), |p| format!("({}, {}, {}, {})", num(p[0]), num(p[1]), num(p[2]), num(p[3])))),
        Column::Quat(v) => ("quatf", list(v.iter(), |q| format!("({}, {}, {}, {})", num(q[3]), num(q[0]), num(q[1]), num(q[2])))),
        Column::Token(v) => ("token", list(v.iter(), |t| format!("\"{}\"", t.replace('"', "\\\"")))),
        Column::Mat4(_) => return None,
    })
}

// ── Text ────────────────────────────────────────────────────────────────────

fn list<T>(items: impl Iterator<Item = T>, f: impl Fn(T) -> String) -> String {
    format!("[{}]", items.map(f).collect::<Vec<_>>().join(", "))
}

/// A number as USD reads it: finite, shortest form.
fn num(x: f32) -> String { if x.is_finite() { format!("{x}") } else { "0".into() } }
fn num64(x: f64) -> String { if x.is_finite() { format!("{x}") } else { "0".into() } }

fn v3(p: &[f32; 3]) -> String { format!("({}, {}, {})", num(p[0]), num(p[1]), num(p[2])) }

/// A matrix as USD writes it: rows, with the translation in the last row.
/// glam's columns are USD's rows.
fn matrix(m: &DMat4) -> String {
    let c = m.to_cols_array_2d();
    let row = |r: &[f64; 4]| format!("({}, {}, {}, {})", num64(r[0]), num64(r[1]), num64(r[2]), num64(r[3]));
    format!("( {}, {}, {}, {} )", row(&c[0]), row(&c[1]), row(&c[2]), row(&c[3]))
}

/// `path` relative to `dir` when they share a start, else as it is.
fn relative(path: &Path, dir: &Path) -> PathBuf {
    let (Ok(path), Ok(dir)) = (path.canonicalize(), dir.canonicalize()) else { return path.to_path_buf() };
    let (p, d): (Vec<_>, Vec<_>) = (path.components().collect(), dir.components().collect());
    let common = p.iter().zip(&d).take_while(|(a, b)| a == b).count();
    if common == 0 { return path; }
    let mut out = PathBuf::from(".");
    for _ in common..d.len() { out.push(".."); }
    for c in &p[common..] { out.push(c); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MeshData, Source};

    /// A stage on disk, loaded as Load USD gives it: packed primitives with
    /// their sources.
    fn load(name: &str, text: &str) -> (PathBuf, Vec<NamedMesh>) {
        let dir = std::env::temp_dir().join("xms_usd_write_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        (path.clone(), prims_of(&path))
    }

    fn prims_of(path: &Path) -> Vec<NamedMesh> {
        let scene = crate::usd_scene::load(path).unwrap();
        scene.meshes.iter().map(|m| NamedMesh {
            mesh: m.mesh.clone(), geo: m.geo.clone(), place: m.place.clone(), stage: Some(scene.tree.clone()),
            material: m.material.clone(), purpose: m.purpose,
            source: m.geo.clone().map(|geo| Arc::new(Source { geo, place: m.place.clone(), material: m.material.clone(), purpose: m.purpose })),
            ..NamedMesh::new(m.path.clone(), MeshData::default())
        }).collect()
    }

    const QUAD: &str = r#"
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
"#;

    fn stage_text() -> String {
        format!(r#"#usda 1.0
(
    upAxis = "Z"
    metersPerUnit = 0.01
)
def Xform "set" {{
    double3 xformOp:translate = (0, 0, 100)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Mesh "a" {{{QUAD}    }}
    def Mesh "b" {{{QUAD}    }}
    def Mesh "c" {{{QUAD}    }}
}}
"#)
    }

    #[test]
    fn an_untouched_stage_writes_no_opinions() {
        let (_, prims) = load("untouched.usda", &stage_text());
        let out = std::env::temp_dir().join("xms_usd_write_test/untouched_over.usda");
        let (text, report) = override_layer(&prims, &out).unwrap();
        assert_eq!(report, WriteReport::default());
        assert!(!text.contains("over "), "{text}");
        assert!(text.contains("upAxis = \"Z\"") && text.contains("metersPerUnit = 0.01") && text.contains("@./untouched.usda@"), "{text}");
    }

    #[test]
    fn edits_written_and_read_back_compose_to_what_was_shown() {
        let (_, mut prims) = load("edited.usda", &stage_text());
        // a: points moved. b: moved by a Transform. c: pruned.
        let mut g = (**prims[0].geo.as_ref().unwrap()).clone();
        g.points_mut()[2] = [1.0, 1.0, 5.0];
        prims[0].geo = Some(Arc::new(g));
        prims[1].place = prims[1].place.moved(Mat4::from_translation(bevy::math::Vec3::new(2.0, 0.0, 0.0)));
        let shown_b = prims[1].place.clone();
        prims.truncate(2);

        let out = std::env::temp_dir().join("xms_usd_write_test/edited_over.usda");
        let report = write_override(&prims, &out).unwrap();
        assert_eq!((report.changed, report.deactivated, report.added), (2, 1, 0), "{report:?}");

        // The override, opened on its own, composes over the original.
        let back = crate::usd_scene::load(&out).unwrap();
        let find = |p: &str| back.meshes.iter().find(|m| m.path == p);
        let a = find("/set/a").expect("a is there");
        assert_eq!(a.geo.as_ref().unwrap().points()[2], [1.0, 1.0, 5.0]);
        let b = find("/set/b").expect("b is there");
        let (Placement::One(now), Placement::One(was)) = (&b.place, &shown_b) else { panic!("placed") };
        assert!(now.abs_diff_eq(*was, 1e-4), "{now} vs {was}");
        assert!(find("/set/c").is_none(), "c is deactivated");
        // The same axis and unit as the original.
        assert_eq!((back.up_axis.as_str(), back.meters_per_unit), ("Z", Some(0.01)));
    }

    #[test]
    fn primvars_keep_their_interpolation_indices_and_type() {
        let mut spec = Spec::default();
        let st = Attr::indexed(Column::Vec2(Arc::new(vec![[0.0, 0.0], [1.0, 1.0]])), vec![0, 1, 1, 0], Role::TexCoord);
        attribute(&mut spec, Context::Corner, "st", &st);
        assert!(spec.lines[0].starts_with("texCoord2f[] primvars:st = [(0, 0), (1, 1)]"), "{}", spec.lines[0]);
        assert!(spec.lines[0].contains("interpolation = \"faceVarying\""));
        assert_eq!(spec.lines[1], "int[] primvars:st:indices = [0, 1, 1, 0]");
        block(&mut spec, "displayColor", &Attr::new(Column::Vec3(Arc::new(vec![[1.0, 0.0, 0.0]])), Role::Color));
        assert_eq!(spec.lines[2], "color3f[] primvars:displayColor = None");
    }
}
