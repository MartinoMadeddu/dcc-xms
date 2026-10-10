//! Geometry as typed attribute columns per context.
//!
//! The one geometry model for packed primitives, the nodes and ICE. A `Geo`
//! is a topology (points, polygons or curves) and, for each context, a set of
//! named attributes. Contexts are where an attribute's values live, and match
//! USD's primvar interpolations:
//!
//! | `Context`   | values per        | USD interpolation |
//! |-------------|-------------------|-------------------|
//! | `Object`    | one for the prim  | `constant`        |
//! | `Point`     | point             | `vertex`          |
//! | `Primitive` | polygon or curve  | `uniform`         |
//! | `Corner`    | polygon corner    | `faceVarying`     |
//!
//! Columns are shared: cloning a `Geo` copies pointers, and writing to a
//! column (through `Arc::make_mut`) copies that column only. So a node that
//! moves the points of a mesh with forty attributes copies one column, and
//! an attribute still pointing at the column it was loaded with is, by that
//! fact, unedited: edits are what no longer shares with the source.
//!
//! Names follow USD (`points`, `normals`, `st`, `displayColor`, `widths`, and
//! a primvar's own name for the rest), so reading and writing USD is a
//! one-to-one mapping. Interfaces can show friendlier aliases on top.
//!
//! `MeshData` stays for now as a view built from a `Geo` (`to_mesh`), so the
//! viewport, modelling and UV code keep working while they move over.

// Some of the API (subdivision schemes, quaternion and matrix columns) is
// for what reads and writes USD next.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::{CurveBasis, CurveWrap, MeshData, PrimVar, PrimVarInterp};

// ── Names ───────────────────────────────────────────────────────────────────

/// Point positions (USD `points`).
pub const POINTS: &str = "points";
/// Normals (USD `normals`, or `primvars:normals`).
pub const NORMALS: &str = "normals";
/// Texture coordinates (USD `primvars:st`).
pub const ST: &str = "st";
/// Point and curve widths (USD `widths`).
pub const WIDTHS: &str = "widths";
/// Names texture coordinates go by, `st` first (USD's convention).
pub const UV_NAMES: [&str; 5] = [ST, "st0", "UVMap", "map1", "uv"];

// ── Context ─────────────────────────────────────────────────────────────────

/// Where an attribute's values live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    Object,
    Point,
    Primitive,
    Corner,
}

impl Context {
    pub const ALL: [Context; 4] = [Context::Object, Context::Point, Context::Primitive, Context::Corner];

    fn slot(self) -> usize {
        match self { Context::Object => 0, Context::Point => 1, Context::Primitive => 2, Context::Corner => 3 }
    }

    /// The USD interpolation token for writing.
    pub fn usd_interpolation(self) -> &'static str {
        match self {
            Context::Object => "constant",
            Context::Point => "vertex",
            Context::Primitive => "uniform",
            Context::Corner => "faceVarying",
        }
    }

    /// From a USD interpolation token. `varying` reads as per point: for
    /// polygons and points it has as many values as there are points.
    pub fn from_usd(token: &str) -> Option<Context> {
        Some(match token {
            "constant" => Context::Object,
            "vertex" | "varying" => Context::Point,
            "uniform" => Context::Primitive,
            "faceVarying" => Context::Corner,
            _ => return None,
        })
    }
}

// ── Columns ─────────────────────────────────────────────────────────────────

/// The type of a column's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Bool, Int, Float, Vec2, Vec3, Vec4, Quat, Mat4, Token }

/// A typed, shared column of values.
#[derive(Clone, Debug)]
pub enum Column {
    Bool(Arc<Vec<bool>>),
    Int(Arc<Vec<i32>>),
    Float(Arc<Vec<f32>>),
    Vec2(Arc<Vec<[f32; 2]>>),
    Vec3(Arc<Vec<[f32; 3]>>),
    Vec4(Arc<Vec<[f32; 4]>>),
    /// (i, j, k, real), as USD stores quaternions.
    Quat(Arc<Vec<[f32; 4]>>),
    /// Row-major, as USD.
    Mat4(Arc<Vec<[f64; 16]>>),
    Token(Arc<Vec<Arc<str>>>),
}

macro_rules! column_access {
    ($get:ident, $get_mut:ident, $variant:ident, $t:ty) => {
        /// The values, when the column is of this type.
        pub fn $get(&self) -> Option<&[$t]> {
            match self { Column::$variant(v) => Some(v), _ => None }
        }
        /// The values to write, when the column is of this type. Copies them
        /// first if they are shared.
        pub fn $get_mut(&mut self) -> Option<&mut Vec<$t>> {
            match self { Column::$variant(v) => Some(Arc::make_mut(v)), _ => None }
        }
    };
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Bool(v) => v.len(),
            Column::Int(v) => v.len(),
            Column::Float(v) => v.len(),
            Column::Vec2(v) => v.len(),
            Column::Vec3(v) => v.len(),
            Column::Vec4(v) => v.len(),
            Column::Quat(v) => v.len(),
            Column::Mat4(v) => v.len(),
            Column::Token(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn kind(&self) -> Kind {
        match self {
            Column::Bool(_) => Kind::Bool,
            Column::Int(_) => Kind::Int,
            Column::Float(_) => Kind::Float,
            Column::Vec2(_) => Kind::Vec2,
            Column::Vec3(_) => Kind::Vec3,
            Column::Vec4(_) => Kind::Vec4,
            Column::Quat(_) => Kind::Quat,
            Column::Mat4(_) => Kind::Mat4,
            Column::Token(_) => Kind::Token,
        }
    }

    /// Whether two columns are the very same data (not merely equal values).
    /// An attribute whose column is still the source's is unedited.
    pub fn same(&self, other: &Column) -> bool {
        match (self, other) {
            (Column::Bool(a), Column::Bool(b)) => Arc::ptr_eq(a, b),
            (Column::Int(a), Column::Int(b)) => Arc::ptr_eq(a, b),
            (Column::Float(a), Column::Float(b)) => Arc::ptr_eq(a, b),
            (Column::Vec2(a), Column::Vec2(b)) => Arc::ptr_eq(a, b),
            (Column::Vec3(a), Column::Vec3(b)) => Arc::ptr_eq(a, b),
            (Column::Vec4(a), Column::Vec4(b)) => Arc::ptr_eq(a, b),
            (Column::Quat(a), Column::Quat(b)) => Arc::ptr_eq(a, b),
            (Column::Mat4(a), Column::Mat4(b)) => Arc::ptr_eq(a, b),
            (Column::Token(a), Column::Token(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    column_access!(bools, bools_mut, Bool, bool);
    column_access!(ints, ints_mut, Int, i32);
    column_access!(floats, floats_mut, Float, f32);
    column_access!(vec2s, vec2s_mut, Vec2, [f32; 2]);
    column_access!(vec3s, vec3s_mut, Vec3, [f32; 3]);
    column_access!(vec4s, vec4s_mut, Vec4, [f32; 4]);
    column_access!(quats, quats_mut, Quat, [f32; 4]);
    column_access!(mat4s, mat4s_mut, Mat4, [f64; 16]);
    column_access!(tokens, tokens_mut, Token, Arc<str>);
}

// ── Attributes ──────────────────────────────────────────────────────────────

/// What an attribute's values mean, for writing USD (`point3f` against
/// `normal3f`, `color3f`, `texCoord2f`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    None,
    Point,
    Normal,
    Vector,
    Color,
    TexCoord,
}

/// A named attribute: its values, and optionally indices into them (USD
/// indexed primvars, where shared values such as UVs on a seam are stored
/// once). With indices, the attribute has one index per element of its
/// context; without, one value.
#[derive(Clone, Debug)]
pub struct Attr {
    pub column:  Column,
    pub indices: Option<Arc<Vec<u32>>>,
    pub role:    Role,
}

impl Attr {
    pub fn new(column: Column, role: Role) -> Self {
        Self { column, indices: None, role }
    }

    pub fn indexed(column: Column, indices: Vec<u32>, role: Role) -> Self {
        Self { column, indices: Some(Arc::new(indices)), role }
    }

    /// How many elements of its context the attribute covers.
    pub fn len(&self) -> usize {
        self.indices.as_ref().map_or(self.column.len(), |i| i.len())
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// The value index for an element of the context.
    pub fn value_index(&self, element: usize) -> Option<usize> {
        match &self.indices {
            Some(ix) => ix.get(element).map(|&i| i as usize),
            None => Some(element),
        }
    }

    /// Whether this is still the very data of `other` (values and indices).
    pub fn same(&self, other: &Attr) -> bool {
        self.column.same(&other.column)
            && match (&self.indices, &other.indices) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

// ── Topology ────────────────────────────────────────────────────────────────

/// How a mesh is subdivided when rendered (USD `subdivisionScheme`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subdiv {
    /// USD's default.
    #[default]
    CatmullClark,
    Loop,
    Bilinear,
    None,
}

/// What the points make up.
#[derive(Clone, Debug, Default)]
pub enum Topology {
    #[default]
    Points,
    /// Polygons: corners per polygon, and the point of each corner.
    /// `left_handed`: corners wind clockwise (USD `orientation`).
    Mesh {
        counts:      Arc<Vec<u32>>,
        indices:     Arc<Vec<u32>>,
        left_handed: bool,
        subdiv:      Subdiv,
    },
    /// Curves: control points per curve, end to end through the points.
    Curves {
        counts: Arc<Vec<u32>>,
        basis:  CurveBasis,
        wrap:   CurveWrap,
    },
}

impl Topology {
    pub fn mesh(counts: Vec<u32>, indices: Vec<u32>) -> Self {
        Topology::Mesh { counts: Arc::new(counts), indices: Arc::new(indices), left_handed: false, subdiv: Subdiv::default() }
    }

    pub fn curves(counts: Vec<u32>, basis: CurveBasis, wrap: CurveWrap) -> Self {
        Topology::Curves { counts: Arc::new(counts), basis, wrap }
    }

    /// Polygons or curves.
    pub fn primitive_count(&self) -> usize {
        match self {
            Topology::Points => 0,
            Topology::Mesh { counts, .. } | Topology::Curves { counts, .. } => counts.len(),
        }
    }

    /// Polygon corners (curve control points, for curves).
    pub fn corner_count(&self) -> usize {
        match self {
            Topology::Points => 0,
            Topology::Mesh { indices, .. } => indices.len(),
            Topology::Curves { counts, .. } => counts.iter().map(|&c| c as usize).sum(),
        }
    }

    /// Whether two topologies are the very same data.
    pub fn same(&self, other: &Topology) -> bool {
        match (self, other) {
            (Topology::Points, Topology::Points) => true,
            (Topology::Mesh { counts: a, indices: ai, left_handed: al, subdiv: asd },
             Topology::Mesh { counts: b, indices: bi, left_handed: bl, subdiv: bsd }) =>
                Arc::ptr_eq(a, b) && Arc::ptr_eq(ai, bi) && al == bl && asd == bsd,
            (Topology::Curves { counts: a, basis: ab, wrap: aw }, Topology::Curves { counts: b, basis: bb, wrap: bw }) =>
                Arc::ptr_eq(a, b) && ab == bb && aw == bw,
            _ => false,
        }
    }
}

// ── Geo ─────────────────────────────────────────────────────────────────────

/// A topology and its attributes, per context.
#[derive(Clone, Debug, Default)]
pub struct Geo {
    pub topology: Topology,
    attrs: [BTreeMap<Arc<str>, Attr>; 4],
}

impl Geo {
    /// Points only.
    pub fn from_points(points: Vec<[f32; 3]>) -> Self {
        let mut g = Geo::default();
        g.set(Context::Point, POINTS, Attr::new(Column::Vec3(Arc::new(points)), Role::Point));
        g
    }

    /// Polygons: corners per polygon and the point of each corner.
    pub fn from_polygons(points: Vec<[f32; 3]>, counts: Vec<u32>, indices: Vec<u32>) -> Self {
        let mut g = Geo::from_points(points);
        g.topology = Topology::mesh(counts, indices);
        g
    }

    // ── Attributes ─────────────────────────────────────────────────────────

    pub fn attr(&self, ctx: Context, name: &str) -> Option<&Attr> {
        self.attrs[ctx.slot()].get(name)
    }

    pub fn attr_mut(&mut self, ctx: Context, name: &str) -> Option<&mut Attr> {
        self.attrs[ctx.slot()].get_mut(name)
    }

    pub fn set(&mut self, ctx: Context, name: &str, attr: Attr) {
        self.attrs[ctx.slot()].insert(Arc::from(name), attr);
    }

    pub fn remove(&mut self, ctx: Context, name: &str) -> Option<Attr> {
        self.attrs[ctx.slot()].remove(name)
    }

    /// The attributes of a context, by name.
    pub fn attrs(&self, ctx: Context) -> impl Iterator<Item = (&str, &Attr)> {
        self.attrs[ctx.slot()].iter().map(|(k, v)| (&**k, v))
    }

    /// Where an attribute is, in whichever context has it.
    pub fn find(&self, name: &str) -> Option<(Context, &Attr)> {
        Context::ALL.into_iter().find_map(|c| self.attr(c, name).map(|a| (c, a)))
    }

    // ── Points ─────────────────────────────────────────────────────────────

    pub fn points(&self) -> &[[f32; 3]] {
        self.attr(Context::Point, POINTS).and_then(|a| a.column.vec3s()).unwrap_or(&[])
    }

    /// The points to write: copied first if shared.
    pub fn points_mut(&mut self) -> &mut Vec<[f32; 3]> {
        if self.attr(Context::Point, POINTS).and_then(|a| a.column.vec3s()).is_none() {
            self.set(Context::Point, POINTS, Attr::new(Column::Vec3(Arc::new(vec![])), Role::Point));
        }
        self.attr_mut(Context::Point, POINTS).and_then(|a| a.column.vec3s_mut()).expect("points are a Vec3 column")
    }

    pub fn point_count(&self) -> usize { self.points().len() }

    /// How many elements a context has.
    pub fn count(&self, ctx: Context) -> usize {
        match ctx {
            Context::Object => 1,
            Context::Point => self.point_count(),
            Context::Primitive => self.topology.primitive_count(),
            Context::Corner => self.topology.corner_count(),
        }
    }

    /// Low and high corners of the points.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.points().iter();
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
        }))
    }

    /// Every attribute covers its context, every index points at a value,
    /// and the topology points at existing points.
    pub fn validate(&self) -> Result<(), String> {
        let points = self.point_count();
        match &self.topology {
            Topology::Points => {}
            Topology::Mesh { counts, indices, .. } => {
                let corners: usize = counts.iter().map(|&c| c as usize).sum();
                if corners != indices.len() {
                    return Err(format!("polygon corners: counts add up to {corners}, there are {} indices", indices.len()));
                }
                if let Some(bad) = indices.iter().find(|&&i| i as usize >= points) {
                    return Err(format!("polygon corner points at point {bad}, there are {points} points"));
                }
            }
            Topology::Curves { counts, .. } => {
                let cvs: usize = counts.iter().map(|&c| c as usize).sum();
                if cvs > points {
                    return Err(format!("curves need {cvs} points, there are {points}"));
                }
            }
        }
        for ctx in Context::ALL {
            let expected = self.count(ctx);
            for (name, attr) in self.attrs(ctx) {
                if attr.len() != expected {
                    return Err(format!("{name} ({ctx:?}) has {} elements, {expected} expected", attr.len()));
                }
                if let Some(ix) = &attr.indices {
                    let values = attr.column.len();
                    if let Some(bad) = ix.iter().find(|&&i| i as usize >= values) {
                        return Err(format!("{name}: index {bad} past its {values} values"));
                    }
                }
            }
        }
        Ok(())
    }

    // ── MeshData, both ways ────────────────────────────────────────────────

    /// The `MeshData` view of this geometry, for code that still works on
    /// `MeshData`: polygons (wound counter-clockwise) and their fan
    /// triangulation, normals per point, UVs per triangle corner, points and
    /// their widths, curves, and every other attribute with one to four
    /// float components as a `PrimVar`.
    pub fn to_mesh(&self) -> MeshData {
        let mut m = MeshData::default();
        match &self.topology {
            Topology::Points => {
                m.points = self.points().to_vec();
                if let Some(w) = self.attr(Context::Point, WIDTHS).and_then(|a| expand_floats(a, self.count(Context::Point))) {
                    m.widths = w;
                } else if let Some(w) = self.attr(Context::Object, WIDTHS).and_then(|a| a.column.floats()) {
                    m.widths = w.to_vec();
                }
            }
            Topology::Curves { counts, basis, wrap } => {
                m.curve_points = self.points().to_vec();
                m.curve_counts = counts.to_vec();
                m.curve_basis = *basis;
                m.curve_wrap = *wrap;
            }
            Topology::Mesh { counts, indices, left_handed, .. } => {
                m.vertices = self.points().to_vec();
                // Corners of each polygon, in the winding drawn.
                let mut polys: Vec<Vec<u32>> = Vec::with_capacity(counts.len());
                let mut corner_of: Vec<Vec<usize>> = Vec::with_capacity(counts.len());
                let mut at = 0usize;
                for &n in counts.iter() {
                    let n = n as usize;
                    let corners: Vec<usize> = if *left_handed { (0..n).rev().map(|k| at + k).collect() } else { (at..at + n).collect() };
                    polys.push(corners.iter().map(|&c| indices[c]).collect());
                    corner_of.push(corners);
                    at += n;
                }
                // UVs per triangle corner, from wherever `st` lives (or the
                // first of its other names, or any texture coordinates).
                let is_uv = |a: &&Attr| a.column.kind() == Kind::Vec2;
                let st = UV_NAMES.iter().find_map(|n| self.find(n).filter(|(_, a)| is_uv(a)))
                    .or_else(|| Context::ALL.into_iter().find_map(|c| {
                        self.attrs(c).find(|(_, a)| a.role == Role::TexCoord && is_uv(a)).map(|(_, a)| (c, a))
                    }));
                let uv = |ctx: Context, a: &Attr, poly: usize, corner: usize| -> [f32; 2] {
                    let element = match ctx {
                        Context::Object => 0,
                        Context::Point => indices[corner] as usize,
                        Context::Primitive => poly,
                        Context::Corner => corner,
                    };
                    a.value_index(element).and_then(|i| a.column.vec2s().and_then(|v| v.get(i).copied())).unwrap_or([0.0; 2])
                };
                for (p, corners) in corner_of.iter().enumerate() {
                    for k in 1..corners.len().saturating_sub(1) {
                        for c in [corners[0], corners[k], corners[k + 1]] {
                            m.indices.push(indices[c]);
                            if let Some((ctx, a)) = st { m.uvs.push(uv(ctx, a, p, c)); }
                        }
                    }
                }
                m.face_count = polys.len();
                m.polys = polys;
                if let Some(n) = self.attr(Context::Point, NORMALS).and_then(|a| expand_vec3s(a, self.point_count())) {
                    m.normals = n;
                }
            }
        }
        // Everything else as primvars.
        for ctx in Context::ALL {
            for (name, attr) in self.attrs(ctx) {
                if matches!(name, POINTS | NORMALS | WIDTHS) || UV_NAMES.contains(&name) { continue; }
                if let Some(values) = float_rows(attr, self.count(ctx)) {
                    m.primvars.push(PrimVar { name: name.to_string(), interp: interp_of(ctx), values });
                }
            }
        }
        m
    }

    /// This geometry with the changes a `MeshData` operation made to its
    /// view: `before` is the view the operation was given (`to_mesh`),
    /// `after` what it returned, both in this geometry's space. Only what
    /// changed is replaced, so everything else stays shared with `self`:
    ///
    /// - points moved: new points (normals are dropped, as they no longer
    ///   match; the viewport makes its own);
    /// - UVs changed: a new `st` per polygon corner;
    /// - polygons changed: new topology from `after`, keeping the attributes
    ///   whose context still has the same elements (object ones always, point
    ///   ones while the points are the same in number). Per-polygon and
    ///   per-corner ones cannot follow a topology change and are left out,
    ///   apart from `st`, which comes from `after`.
    pub fn with_mesh_edits(&self, before: &MeshData, after: &MeshData) -> Geo {
        let Topology::Mesh { left_handed, .. } = &self.topology else {
            // Curves and points: their positions are all a MeshData edit can change.
            let mut g = self.clone();
            let moved = if after.curve_counts.is_empty() { &after.points } else { &after.curve_points };
            if moved.len() == self.point_count() && moved.as_slice() != self.points() {
                *g.points_mut() = moved.clone();
                g.remove(Context::Point, NORMALS);
            }
            return g;
        };
        let left_handed = *left_handed;
        if before.polys != after.polys {
            let mut g = Geo::from_mesh(after);
            for (name, attr) in self.attrs(Context::Object) {
                g.set(Context::Object, name, attr.clone());
            }
            if after.vertices.len() == self.point_count() {
                for (name, attr) in self.attrs(Context::Point) {
                    if name != POINTS && name != NORMALS { g.set(Context::Point, name, attr.clone()); }
                }
            }
            return g;
        }
        let mut g = self.clone();
        if before.vertices != after.vertices {
            *g.points_mut() = after.vertices.clone();
            g.remove(Context::Point, NORMALS);
        }
        if before.uvs != after.uvs && after.uvs.len() == after.indices.len() {
            // The view's polygons are wound for drawing: a left-handed mesh's
            // run backwards, so its corner UVs are turned back round.
            let mut st = corner_uvs(&after.polys, &after.uvs);
            if left_handed {
                let mut at = 0usize;
                for p in &after.polys {
                    st[at..at + p.len()].reverse();
                    at += p.len();
                }
            }
            for name in UV_NAMES { for ctx in Context::ALL { g.remove(ctx, name); } }
            g.set(Context::Corner, ST, Attr::new(Column::Vec2(Arc::new(st)), Role::TexCoord));
        }
        g
    }

    /// A `Geo` from `MeshData`. Polygons are kept when the triangles are
    /// their fan (as `MeshData` builds them), so UVs map back to polygon
    /// corners; otherwise the triangles become the polygons.
    pub fn from_mesh(m: &MeshData) -> Self {
        let mut g;
        if !m.curve_counts.is_empty() {
            g = Geo::from_points(m.curve_points.clone());
            g.topology = Topology::curves(m.curve_counts.clone(), m.curve_basis, m.curve_wrap);
        } else if m.vertices.is_empty() {
            g = Geo::from_points(m.points.clone());
            if !m.widths.is_empty() {
                let ctx = if m.widths.len() == m.points.len() { Context::Point } else { Context::Object };
                let w = if ctx == Context::Object { vec![m.widths[0]] } else { m.widths.clone() };
                g.set(ctx, WIDTHS, Attr::new(Column::Float(Arc::new(w)), Role::None));
            }
        } else {
            let fan_of_polys = !m.polys.is_empty() && is_fan(&m.polys, &m.indices);
            let polys: Vec<Vec<u32>> = if fan_of_polys { m.polys.clone() } else { m.indices.chunks_exact(3).map(|t| t.to_vec()).collect() };
            let counts: Vec<u32> = polys.iter().map(|p| p.len() as u32).collect();
            let indices: Vec<u32> = polys.iter().flatten().copied().collect();
            // UVs come per triangle corner: back to polygon corners through the fan.
            let st: Option<Vec<[f32; 2]>> = (m.uvs.len() == m.indices.len() && !m.uvs.is_empty()).then(|| corner_uvs(&polys, &m.uvs));
            g = Geo::from_polygons(m.vertices.clone(), counts, indices);
            if let Some(st) = st {
                g.set(Context::Corner, ST, Attr::new(Column::Vec2(Arc::new(st)), Role::TexCoord));
            }
            if m.normals.len() == m.vertices.len() && !m.normals.is_empty() {
                g.set(Context::Point, NORMALS, Attr::new(Column::Vec3(Arc::new(m.normals.clone())), Role::Normal));
            }
            if !m.points.is_empty() {
                // A mesh with scattered points too keeps them as an object attribute.
                g.set(Context::Object, "scatterPoints", Attr::new(Column::Vec3(Arc::new(m.points.clone())), Role::Point));
            }
        }
        for pv in &m.primvars {
            let ctx = context_of(&pv.interp);
            if let Some(column) = column_from_rows(&pv.values) {
                g.set(ctx, &pv.name, Attr::new(column, Role::None));
            }
        }
        g
    }
}

/// UVs per triangle corner (fan triangulation of `polys`, in order) as UVs
/// per polygon corner.
fn corner_uvs(polys: &[Vec<u32>], uvs: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(polys.iter().map(|p| p.len()).sum());
    let mut tri = 0usize;   // first triangle corner of the current polygon
    for p in polys {
        let n = p.len();
        for k in 0..n {
            let at = match k {
                0 => tri,
                k if k + 1 < n => tri + 3 * (k - 1) + 1,
                _ => tri + 3 * (n - 3) + 2,
            };
            out.push(uvs.get(at).copied().unwrap_or([0.0; 2]));
        }
        tri += 3 * n.saturating_sub(2);
    }
    out
}

/// Whether `indices` is the fan triangulation of `polys`, in order.
fn is_fan(polys: &[Vec<u32>], indices: &[u32]) -> bool {
    let mut at = 0usize;
    for p in polys {
        for k in 1..p.len().saturating_sub(1) {
            if indices.get(at..at + 3) != Some(&[p[0], p[k], p[k + 1]][..]) { return false; }
            at += 3;
        }
    }
    at == indices.len()
}

fn interp_of(ctx: Context) -> PrimVarInterp {
    match ctx {
        Context::Object => PrimVarInterp::Constant,
        Context::Point => PrimVarInterp::Vertex,
        Context::Primitive => PrimVarInterp::Uniform,
        Context::Corner => PrimVarInterp::FaceVarying,
    }
}

fn context_of(interp: &PrimVarInterp) -> Context {
    match interp {
        PrimVarInterp::Constant => Context::Object,
        PrimVarInterp::Vertex => Context::Point,
        PrimVarInterp::Uniform => Context::Primitive,
        PrimVarInterp::FaceVarying => Context::Corner,
    }
}

/// An attribute's values for every element, indices resolved.
fn expand<T: Copy>(a: &Attr, values: &[T], count: usize) -> Option<Vec<T>> {
    (0..count).map(|e| a.value_index(e).and_then(|i| values.get(i).copied())).collect()
}

fn expand_floats(a: &Attr, count: usize) -> Option<Vec<f32>> {
    expand(a, a.column.floats()?, count)
}

fn expand_vec3s(a: &Attr, count: usize) -> Option<Vec<[f32; 3]>> {
    expand(a, a.column.vec3s()?, count)
}

/// Float-component values per element, for `PrimVar`.
fn float_rows(a: &Attr, count: usize) -> Option<Vec<Vec<f32>>> {
    let rows: Vec<Vec<f32>> = match &a.column {
        Column::Float(v) => v.iter().map(|x| vec![*x]).collect(),
        Column::Int(v) => v.iter().map(|x| vec![*x as f32]).collect(),
        Column::Vec2(v) => v.iter().map(|x| x.to_vec()).collect(),
        Column::Vec3(v) => v.iter().map(|x| x.to_vec()).collect(),
        Column::Vec4(v) | Column::Quat(v) => v.iter().map(|x| x.to_vec()).collect(),
        _ => return None,
    };
    expand(a, &(0..rows.len()).collect::<Vec<_>>(), count).map(|ix| ix.into_iter().map(|i| rows[i].clone()).collect())
}

/// A column from `PrimVar` rows, by their component count.
fn column_from_rows(rows: &[Vec<f32>]) -> Option<Column> {
    let width = rows.first().map_or(1, |r| r.len());
    if rows.iter().any(|r| r.len() != width) { return None; }
    let at = |r: &Vec<f32>, i: usize| r.get(i).copied().unwrap_or(0.0);
    Some(match width {
        1 => Column::Float(Arc::new(rows.iter().map(|r| at(r, 0)).collect())),
        2 => Column::Vec2(Arc::new(rows.iter().map(|r| [at(r, 0), at(r, 1)]).collect())),
        3 => Column::Vec3(Arc::new(rows.iter().map(|r| [at(r, 0), at(r, 1), at(r, 2)]).collect())),
        4 => Column::Vec4(Arc::new(rows.iter().map(|r| [at(r, 0), at(r, 1), at(r, 2), at(r, 3)]).collect())),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit quad split from a triangle: two polygons sharing an edge.
    fn quad_and_triangle() -> Geo {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0]];
        let mut g = Geo::from_polygons(points, vec![4, 3], vec![0, 1, 2, 3, 1, 4, 2]);
        let st = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.5, 0.0], [0.9, 0.0], [0.5, 0.5]];
        g.set(Context::Corner, ST, Attr::new(Column::Vec2(Arc::new(st)), Role::TexCoord));
        g
    }

    #[test]
    fn contexts_are_usd_interpolations() {
        for ctx in Context::ALL {
            assert_eq!(Context::from_usd(ctx.usd_interpolation()), Some(ctx));
        }
        assert_eq!(Context::from_usd("varying"), Some(Context::Point));
    }

    #[test]
    fn counts_follow_the_topology_and_validate_checks_them() {
        let g = quad_and_triangle();
        assert_eq!((g.count(Context::Object), g.count(Context::Point), g.count(Context::Primitive), g.count(Context::Corner)), (1, 5, 2, 7));
        assert!(g.validate().is_ok());
        let mut bad = g.clone();
        bad.set(Context::Primitive, "id", Attr::new(Column::Int(Arc::new(vec![1, 2, 3])), Role::None));
        assert!(bad.validate().is_err());
    }

    #[test]
    fn writing_copies_only_what_is_written() {
        let source = quad_and_triangle();
        let mut edited = source.clone();
        edited.points_mut()[0][2] = 1.0;
        let (a, b) = (source.attr(Context::Point, POINTS).unwrap(), edited.attr(Context::Point, POINTS).unwrap());
        assert!(!a.same(b), "the written column is a copy");
        assert_eq!(source.points()[0][2], 0.0, "the source is untouched");
        assert!(source.attr(Context::Corner, ST).unwrap().same(edited.attr(Context::Corner, ST).unwrap()), "the rest is still shared");
        assert!(source.topology.same(&edited.topology));
    }

    #[test]
    fn indexed_attributes_store_shared_values_once() {
        let mut g = quad_and_triangle();
        // Two colours for seven corners.
        g.set(Context::Corner, "displayColor", Attr::indexed(
            Column::Vec3(Arc::new(vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]])), vec![0, 0, 0, 0, 1, 1, 1], Role::Color));
        assert!(g.validate().is_ok());
        let a = g.attr(Context::Corner, "displayColor").unwrap();
        assert_eq!((a.len(), a.column.len(), a.value_index(5)), (7, 2, Some(1)));
    }

    #[test]
    fn a_mesh_goes_to_mesh_data_and_back() {
        let g = quad_and_triangle();
        let m = g.to_mesh();
        assert_eq!(m.polys, vec![vec![0, 1, 2, 3], vec![1, 4, 2]]);
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3, 1, 4, 2]);
        assert_eq!(m.uvs.len(), m.indices.len());
        assert_eq!(m.uvs[4], [1.0, 1.0]);   // second triangle, second corner: polygon corner 2

        let back = Geo::from_mesh(&m);
        assert!(back.validate().is_ok());
        assert_eq!(back.points(), g.points());
        assert_eq!(back.topology.corner_count(), 7);
        assert_eq!(back.attr(Context::Corner, ST).unwrap().column.vec2s(), g.attr(Context::Corner, ST).unwrap().column.vec2s());
    }

    #[test]
    fn left_handed_meshes_turn_their_polygons_round_in_the_view() {
        let mut g = quad_and_triangle();
        if let Topology::Mesh { left_handed, .. } = &mut g.topology { *left_handed = true; }
        assert_eq!(g.to_mesh().polys[1], vec![2, 4, 1]);
    }

    #[test]
    fn points_and_curves_keep_their_kind() {
        let mut cloud = Geo::from_points(vec![[0.0; 3], [1.0, 0.0, 0.0]]);
        cloud.set(Context::Point, WIDTHS, Attr::new(Column::Float(Arc::new(vec![0.5, 0.25])), Role::None));
        let m = cloud.to_mesh();
        assert_eq!((m.points.len(), m.widths.clone()), (2, vec![0.5, 0.25]));
        assert_eq!(Geo::from_mesh(&m).attr(Context::Point, WIDTHS).unwrap().column.floats(), Some(&[0.5, 0.25][..]));

        let mut hair = Geo::from_points(vec![[0.0; 3], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0], [0.0, 3.0, 0.0]]);
        hair.topology = Topology::curves(vec![4], CurveBasis::CatmullRom, CurveWrap::Nonperiodic);
        let m = hair.to_mesh();
        assert_eq!((m.curve_counts.clone(), m.curve_basis), (vec![4], CurveBasis::CatmullRom));
        assert!(matches!(Geo::from_mesh(&m).topology, Topology::Curves { basis: CurveBasis::CatmullRom, .. }));
    }

    #[test]
    fn an_edit_replaces_only_what_it_changed() {
        let mut g = quad_and_triangle();
        g.set(Context::Point, NORMALS, Attr::new(Column::Vec3(Arc::new(vec![[0.0, 0.0, 1.0]; 5])), Role::Normal));
        g.set(Context::Object, "displayColor", Attr::new(Column::Vec3(Arc::new(vec![[1.0, 0.0, 0.0]])), Role::Color));
        let before = g.to_mesh();

        // UVs only: points, normals and topology stay shared.
        let mut after = before.clone();
        for uv in after.uvs.iter_mut() { uv[0] += 1.0; }
        let e = g.with_mesh_edits(&before, &after);
        assert!(e.attr(Context::Point, POINTS).unwrap().same(g.attr(Context::Point, POINTS).unwrap()));
        assert!(e.attr(Context::Point, NORMALS).is_some() && e.topology.same(&g.topology));
        assert!(!e.attr(Context::Corner, ST).unwrap().same(g.attr(Context::Corner, ST).unwrap()));
        assert_eq!(e.attr(Context::Corner, ST).unwrap().column.vec2s().unwrap()[2], [2.0, 1.0]);
        assert!(e.validate().is_ok());

        // Points moved: new points, stale normals gone, UVs still shared.
        let mut after = before.clone();
        after.vertices[4][1] = 3.0;
        let e = g.with_mesh_edits(&before, &after);
        assert_eq!(e.points()[4], [2.0, 3.0, 0.0]);
        assert!(e.attr(Context::Point, NORMALS).is_none());
        assert!(e.attr(Context::Corner, ST).unwrap().same(g.attr(Context::Corner, ST).unwrap()));

        // Polygons changed: new topology, object attributes kept.
        let after = MeshData::from_polys(before.vertices.clone(), vec![vec![0, 1, 2, 3]]);
        let e = g.with_mesh_edits(&before, &after);
        assert_eq!(e.topology.primitive_count(), 1);
        assert!(e.attr(Context::Object, "displayColor").is_some());
        assert!(e.validate().is_ok());
    }

    #[test]
    fn a_left_handed_mesh_keeps_its_uv_corners_in_its_own_order() {
        let mut g = quad_and_triangle();
        if let Topology::Mesh { left_handed, .. } = &mut g.topology { *left_handed = true; }
        let before = g.to_mesh();
        let mut after = before.clone();
        for uv in after.uvs.iter_mut() { uv[1] += 0.5; }
        let e = g.with_mesh_edits(&before, &after);
        let (old, new) = (g.attr(Context::Corner, ST).unwrap().column.vec2s().unwrap(), e.attr(Context::Corner, ST).unwrap().column.vec2s().unwrap());
        for (o, n) in old.iter().zip(new) { assert_eq!([o[0], o[1] + 0.5], *n); }
    }

    #[test]
    fn other_attributes_travel_as_primvars() {
        let mut g = quad_and_triangle();
        g.set(Context::Primitive, "heat", Attr::new(Column::Float(Arc::new(vec![0.25, 0.75])), Role::None));
        let m = g.to_mesh();
        let pv = m.primvars.iter().find(|p| p.name == "heat").unwrap();
        assert_eq!((pv.interp.clone(), pv.values.clone()), (PrimVarInterp::Uniform, vec![vec![0.25], vec![0.75]]));
        let back = Geo::from_mesh(&m);
        assert_eq!(back.attr(Context::Primitive, "heat").unwrap().column.floats(), Some(&[0.25, 0.75][..]));
    }
}
