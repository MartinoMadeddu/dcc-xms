//! The scene store: prims keyed by path, ordered children, change tracking.

use std::collections::{HashMap, HashSet};

use crate::math::Mat4d;
use crate::path::Path;
use crate::prim::{Prim, PrimKind};
use crate::time::SceneTime;

/// What changed on a prim (Hydra-style dirty bits).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Hash)]
pub struct Dirty(pub u32);

impl Dirty {
    pub const NONE: Dirty = Dirty(0);
    pub const TRANSFORM: Dirty = Dirty(1 << 0);
    pub const POINTS: Dirty = Dirty(1 << 1);
    pub const TOPOLOGY: Dirty = Dirty(1 << 2);
    pub const PRIMVARS: Dirty = Dirty(1 << 3);
    pub const MATERIAL_BINDING: Dirty = Dirty(1 << 4);
    pub const VISIBILITY: Dirty = Dirty(1 << 5);
    /// A material's network (nodes, inputs, connections)
    pub const MATERIAL: Dirty = Dirty(1 << 6);
    /// Light, camera or render-settings parameters, or other attributes
    pub const PARAMS: Dirty = Dirty(1 << 7);
    pub const ALL: Dirty = Dirty(0xff);
    /// Dirty bits that descendants inherit (their effective values change too)
    pub const INHERITED: Dirty = Dirty(Self::TRANSFORM.0 | Self::VISIBILITY.0 | Self::MATERIAL_BINDING.0);

    pub fn contains(self, other: Dirty) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn intersects(self, other: Dirty) -> bool {
        self.0 & other.0 != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Dirty {
    type Output = Dirty;
    fn bitor(self, o: Dirty) -> Dirty {
        Dirty(self.0 | o.0)
    }
}

impl std::ops::BitOrAssign for Dirty {
    fn bitor_assign(&mut self, o: Dirty) {
        self.0 |= o.0;
    }
}

impl std::ops::BitAnd for Dirty {
    type Output = Dirty;
    fn bitand(self, o: Dirty) -> Dirty {
        Dirty(self.0 & o.0)
    }
}

/// Changes since the last [`Scene::take_changes`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// New prims (their whole data is new)
    pub added: Vec<Path>,
    /// Removed prims (whole subtrees are listed prim by prim)
    pub removed: Vec<Path>,
    /// Changed prims and what changed; inherited bits are propagated to
    /// descendants. Added prims aren't repeated here.
    pub dirty: Vec<(Path, Dirty)>,
    /// The scene time changed
    pub time: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.dirty.is_empty() && !self.time
    }
}

/// Stage-level metadata a renderer needs.
#[derive(Clone, Debug, PartialEq)]
pub struct StageInfo {
    /// `upAxis` is Z (renderers working Y-up rotate the scene)
    pub z_up: bool,
    pub meters_per_unit: Option<f64>,
    /// `startTimeCode` / `endTimeCode`, if authored
    pub time_range: Option<(f64, f64)>,
    /// Source file, for resolving relative asset paths and diagnostics
    pub source: Option<String>,
}

impl Default for StageInfo {
    fn default() -> Self {
        StageInfo { z_up: false, meters_per_unit: None, time_range: None, source: None }
    }
}

/// Counts for diagnostics (`Scene::summary`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub prims: usize,
    pub groups: usize,
    pub meshes: usize,
    pub mesh_faces: usize,
    pub mesh_points: usize,
    pub subdivision_meshes: usize,
    pub curves: usize,
    pub curve_segments_hint: usize,
    pub points: usize,
    pub gprims: usize,
    pub instancers: usize,
    pub instancer_instances: usize,
    pub instances: usize,
    pub materials: usize,
    /// Materials whose network lives in a referenced .mtlx document
    pub mtlx_materials: usize,
    pub shader_nodes: usize,
    pub lights: usize,
    pub cameras: usize,
    pub render_settings: usize,
    /// Prims with an animated transform, points, or primvar
    pub animated_xforms: usize,
    pub animated_points: usize,
    pub animated_primvars: usize,
    /// Total time samples stored across animated values
    pub samples: usize,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "prims {} ({} groups)", self.prims, self.groups)?;
        writeln!(f, "meshes {} · {} faces · {} points · {} subdivision", self.meshes, self.mesh_faces, self.mesh_points, self.subdivision_meshes)?;
        writeln!(f, "curves {} ({} vertices) · points prims {} · gprims {}", self.curves, self.curve_segments_hint, self.points, self.gprims)?;
        writeln!(f, "instancers {} ({} instances) · native instances {}", self.instancers, self.instancer_instances, self.instances)?;
        writeln!(
            f,
            "materials {} ({} shader nodes, {} from .mtlx documents) · lights {} · cameras {} · render settings {}",
            self.materials, self.shader_nodes, self.mtlx_materials, self.lights, self.cameras, self.render_settings
        )?;
        write!(
            f,
            "animated: transforms {} · points {} · primvars {} · {} time samples",
            self.animated_xforms, self.animated_points, self.animated_primvars, self.samples
        )
    }
}

/// A USD-shaped, path-keyed scene description with change tracking.
#[derive(Clone, Debug)]
pub struct Scene {
    /// Stage metadata (up axis, units, time range)
    pub info: StageInfo,
    prims: HashMap<Path, Prim>,
    children: HashMap<Path, Vec<Path>>,
    time: SceneTime,
    // Change tracking (added: in order, plus a set for fast lookups)
    added: Vec<Path>,
    added_set: HashSet<Path>,
    removed: Vec<Path>,
    dirty: HashMap<Path, Dirty>,
    time_changed: bool,
}

impl Default for Scene {
    fn default() -> Self {
        Scene::new()
    }
}

impl Scene {
    /// An empty scene: just the root (`/`).
    pub fn new() -> Scene {
        let root = Path::root();
        let mut prims = HashMap::new();
        prims.insert(root.clone(), Prim::new(root.clone(), PrimKind::Group));
        let mut children = HashMap::new();
        children.insert(root, Vec::new());
        Scene {
            info: StageInfo::default(),
            prims,
            children,
            time: SceneTime::default(),
            added: Vec::new(),
            added_set: HashSet::new(),
            removed: Vec::new(),
            dirty: HashMap::new(),
            time_changed: false,
        }
    }

    pub fn len(&self) -> usize {
        self.prims.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prims.len() <= 1
    }

    pub fn get(&self, path: &Path) -> Option<&Prim> {
        self.prims.get(path)
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.prims.contains_key(path)
    }

    /// Children of `path`, in insertion order.
    pub fn children(&self, path: &Path) -> &[Path] {
        self.children.get(path).map_or(&[], |c| c.as_slice())
    }

    pub fn time(&self) -> SceneTime {
        self.time
    }

    pub fn set_time(&mut self, time: SceneTime) {
        if time != self.time {
            self.time = time;
            self.time_changed = true;
        }
    }

    /// Insert a prim, creating missing ancestors as groups. Replacing an
    /// existing prim marks it entirely dirty (its children are kept).
    pub fn insert(&mut self, prim: Prim) {
        let path = prim.path.clone();
        if path.is_root() {
            self.prims.insert(path.clone(), prim);
            self.mark(&path, Dirty::ALL);
            return;
        }
        if let Some(parent) = path.parent() {
            if !self.prims.contains_key(&parent) {
                self.insert(Prim::new(parent.clone(), PrimKind::Group));
            }
            if self.prims.insert(path.clone(), prim).is_some() {
                self.mark(&path, Dirty::ALL);
            } else {
                self.children.entry(parent).or_default().push(path.clone());
                self.children.entry(path.clone()).or_default();
                self.added_set.insert(path.clone());
                self.added.push(path);
            }
        }
    }

    /// Remove a prim and its whole subtree. Returns false if it didn't exist.
    pub fn remove(&mut self, path: &Path) -> bool {
        if path.is_root() || !self.prims.contains_key(path) {
            return false;
        }
        if let Some(parent) = path.parent() {
            if let Some(c) = self.children.get_mut(&parent) {
                c.retain(|p| p != path);
            }
        }
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            if let Some(c) = self.children.remove(&p) {
                stack.extend(c);
            }
            self.prims.remove(&p);
            self.dirty.remove(&p);
            // Added and removed since the last sync: nothing to report
            if !self.added_set.remove(&p) {
                self.removed.push(p);
            }
        }
        true
    }

    /// Edit a prim in place, recording what changed. Returns false if the prim
    /// doesn't exist.
    pub fn edit(&mut self, path: &Path, dirty: Dirty, f: impl FnOnce(&mut Prim)) -> bool {
        match self.prims.get_mut(path) {
            Some(p) => {
                f(p);
                self.mark(path, dirty);
                true
            }
            None => false,
        }
    }

    fn mark(&mut self, path: &Path, dirty: Dirty) {
        if dirty.is_empty() || self.added_set.contains(path) {
            return;
        }
        *self.dirty.entry(path.clone()).or_default() |= dirty;
    }

    /// Collect and clear the changes since the last call. Inherited dirty bits
    /// (transform, visibility, material binding) are propagated to descendants.
    pub fn take_changes(&mut self) -> Changes {
        let mut dirty: HashMap<Path, Dirty> = std::mem::take(&mut self.dirty);
        let seeds: Vec<(Path, Dirty)> =
            dirty.iter().filter(|(_, d)| d.intersects(Dirty::INHERITED)).map(|(p, d)| (p.clone(), *d & Dirty::INHERITED)).collect();
        for (p, d) in seeds {
            let mut stack: Vec<Path> = self.children(&p).to_vec();
            while let Some(c) = stack.pop() {
                stack.extend(self.children(&c).iter().cloned());
                *dirty.entry(c).or_default() |= d;
            }
        }
        // Still-present additions, each once (a prim re-added after a removal appears twice in the list)
        let mut set = std::mem::take(&mut self.added_set);
        let added: Vec<Path> = std::mem::take(&mut self.added).into_iter().filter(|p| set.remove(p)).collect();
        for a in &added {
            dirty.remove(a);
        }
        let mut dirty: Vec<(Path, Dirty)> = dirty.into_iter().collect();
        dirty.sort_by(|a, b| a.0.cmp(&b.0));
        Changes { added, removed: std::mem::take(&mut self.removed), dirty, time: std::mem::take(&mut self.time_changed) }
    }

    /// Counts per prim kind, geometry sizes and animated values, for diagnostics.
    pub fn summary(&self) -> Summary {
        use crate::prim::PrimKind as K;
        let mut s = Summary { prims: self.prims.len(), ..Summary::default() };
        fn primvar_anim(pvs: &[crate::prim::Primvar], s: &mut Summary) {
            for pv in pvs {
                if pv.values.is_animated() {
                    s.animated_primvars += 1;
                    s.samples += pv.values.samples().len();
                }
            }
        }
        for prim in self.prims.values() {
            if prim.local_xform.is_animated() {
                s.animated_xforms += 1;
                s.samples += prim.local_xform.samples().len();
            }
            match &prim.kind {
                K::Group => s.groups += 1,
                K::Mesh(m) => {
                    s.meshes += 1;
                    s.mesh_faces += m.face_vertex_counts.len();
                    s.mesh_points += m.points.first().map_or(0, |p| p.len());
                    if m.subdivision.is_tagged() {
                        s.subdivision_meshes += 1;
                    }
                    if m.points.is_animated() {
                        s.animated_points += 1;
                        s.samples += m.points.samples().len();
                    }
                    primvar_anim(&m.primvars, &mut s);
                }
                K::Curves(c) => {
                    s.curves += 1;
                    s.curve_segments_hint += c.curve_vertex_counts.iter().map(|&n| n as usize).sum::<usize>();
                    if c.points.is_animated() {
                        s.animated_points += 1;
                        s.samples += c.points.samples().len();
                    }
                    primvar_anim(&c.primvars, &mut s);
                }
                K::Points(p) => {
                    s.points += 1;
                    if p.points.is_animated() {
                        s.animated_points += 1;
                        s.samples += p.points.samples().len();
                    }
                }
                K::Gprim(_) => s.gprims += 1,
                K::Instancer(i) => {
                    s.instancers += 1;
                    s.instancer_instances += i.proto_indices.len();
                    if i.positions.is_animated() {
                        s.animated_points += 1;
                        s.samples += i.positions.samples().len();
                    }
                }
                K::Instance { .. } => s.instances += 1,
                K::Material(m) => {
                    s.materials += 1;
                    s.shader_nodes += m.nodes.len();
                    if m.mtlx_document.is_some() {
                        s.mtlx_materials += 1;
                    }
                }
                K::Light(_) => s.lights += 1,
                K::Camera(_) => s.cameras += 1,
                K::RenderSettings(_) => s.render_settings += 1,
            }
        }
        s
    }

    /// Depth-first traversal from `path` (included), children in order.
    pub fn traverse(&self, path: &Path) -> Vec<Path> {
        let mut out = Vec::new();
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            if !self.prims.contains_key(&p) {
                continue;
            }
            stack.extend(self.children(&p).iter().rev().cloned());
            out.push(p);
        }
        out
    }

    /// World transform at time `t`: local transforms composed up the ancestors,
    /// stopping at a `resetXformStack`.
    pub fn world_xform_at(&self, path: &Path, t: f64) -> Mat4d {
        let mut m = Mat4d::IDENTITY;
        let mut cur = Some(path.clone());
        while let Some(p) = cur {
            let Some(prim) = self.prims.get(&p) else { break };
            m = m.mul(&prim.local_xform_at(t));
            if prim.reset_xform_stack {
                break;
            }
            cur = p.parent();
        }
        m
    }

    /// World transforms sampled across [open, close] for motion blur: at every
    /// time sample of the prim and its ancestors inside the interval, plus the
    /// interval's ends. A single entry means no motion.
    pub fn world_xform_samples(&self, path: &Path, open: f64, close: f64) -> Vec<(f64, Mat4d)> {
        let mut times = vec![open];
        let mut animated = false;
        let mut cur = Some(path.clone());
        while let Some(p) = cur {
            let Some(prim) = self.prims.get(&p) else { break };
            if prim.local_xform.is_animated() {
                animated = true;
                times.extend(prim.local_xform.times().filter(|&s| s > open && s < close));
            }
            if prim.reset_xform_stack {
                break;
            }
            cur = p.parent();
        }
        if !animated || close <= open {
            return vec![(open, self.world_xform_at(path, open))];
        }
        times.push(close);
        times.sort_by(|a, b| a.total_cmp(b));
        times.dedup();
        times.into_iter().map(|t| (t, self.world_xform_at(path, t))).collect()
    }

    /// Effective visibility at time `t`: invisible if the prim or any ancestor is.
    pub fn visible_at(&self, path: &Path, t: f64) -> bool {
        let mut cur = Some(path.clone());
        while let Some(p) = cur {
            match self.prims.get(&p) {
                Some(prim) if !prim.visible_at(t) => return false,
                Some(_) => cur = p.parent(),
                None => break,
            }
        }
        true
    }

    /// The material bound to `path`, inherited from the nearest ancestor with a binding.
    pub fn resolved_material(&self, path: &Path) -> Option<Path> {
        let mut cur = Some(path.clone());
        while let Some(p) = cur {
            if let Some(m) = self.prims.get(&p).and_then(|prim| prim.material_binding.clone()) {
                return Some(m);
            }
            cur = p.parent();
        }
        None
    }
}
