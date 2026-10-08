//! Geometry behind the Edit Poly operations that are not extrusions:
//! transform, delete, remove, weld, collapse, connect, cap, bridge, detach,
//! flip, make planar, relax, tessellate and subdivision.
//!
//! Each function edits a `PolyMesh` in place and leaves it valid: no polygon
//! with fewer than three corners, no repeated corner, no unused vertex.

use std::collections::{HashMap, HashSet};

use bevy::math::{Mat3, Quat, Vec3};

use super::poly::{edge_key, PolyMesh};

impl PolyMesh {
    // ── Housekeeping ─────────────────────────────────────────────────────────

    /// Drop repeated corners and polygons that no longer span an area, then
    /// unused vertices.
    pub(crate) fn cleanup(&mut self) {
        for poly in &mut self.polys {
            poly.dedup();
            while poly.len() > 1 && poly.first() == poly.last() { poly.pop(); }
        }
        self.polys.retain(|p| p.len() >= 3);
        self.compact();
    }

    /// Open borders as closed vertex loops. Each loop runs with the surface
    /// on its right, so it is already wound the way a polygon filling the
    /// hole has to be.
    pub fn border_loops(&self) -> Vec<Vec<u32>> {
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        for poly in &self.polys {
            for i in 0..poly.len() { directed.insert((poly[i], poly[(i + 1) % poly.len()])); }
        }
        // An edge a->b with no b->a is open. The hole sees it as b->a.
        let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
        for (a, b) in &directed {
            if !directed.contains(&(*b, *a)) { next.entry(*b).or_default().push(*a); }
        }
        let mut starts: Vec<u32> = next.keys().copied().collect();
        starts.sort();
        let mut loops = vec![];
        for start in starts {
            while next.get(&start).map(|n| !n.is_empty()).unwrap_or(false) {
                let mut lp = vec![start];
                let mut cur = start;
                loop {
                    let Some(n) = next.get_mut(&cur).and_then(|v| v.pop()) else { break };
                    if n == start { break; }
                    lp.push(n);
                    cur = n;
                    if lp.len() > self.verts.len() + 1 { break; }
                }
                if lp.len() >= 3 { loops.push(lp); }
            }
        }
        loops
    }

    /// Edges used by one polygon only.
    pub fn open_edges(&self) -> HashSet<[u32; 2]> {
        let mut uses: HashMap<[u32; 2], u32> = HashMap::new();
        for poly in &self.polys {
            for i in 0..poly.len() { *uses.entry(edge_key(poly[i], poly[(i + 1) % poly.len()])).or_default() += 1; }
        }
        uses.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e).collect()
    }

    /// Polygons joined through shared vertices, as one group index per polygon.
    pub fn elements(&self) -> Vec<usize> {
        let mut parent: Vec<usize> = (0..self.verts.len()).collect();
        fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
            while p[x] != x { p[x] = p[p[x]]; x = p[x]; }
            x
        }
        for poly in &self.polys {
            for v in poly.iter().skip(1) {
                let (a, b) = (find(&mut parent, poly[0] as usize), find(&mut parent, *v as usize));
                parent[a] = b;
            }
        }
        self.polys.iter().map(|p| p.first().map(|v| find(&mut parent, *v as usize)).unwrap_or(usize::MAX)).collect()
    }

    // ── Transform ────────────────────────────────────────────────────────────

    /// Scale, rotate and move the chosen vertices about their centre. With
    /// a `falloff` distance, vertices near the chosen ones follow part of
    /// the way (soft selection): fully at the selection, not at all at the
    /// falloff distance.
    pub fn transform_verts(&mut self, verts: &[bool], translate: Vec3, rotate: Quat, scale: Vec3, falloff: f32) {
        let picked: Vec<usize> = (0..self.verts.len()).filter(|v| verts.get(*v).copied().unwrap_or(false)).collect();
        if picked.is_empty() { return; }
        let pivot = picked.iter().map(|v| self.verts[*v]).sum::<Vec3>() / picked.len() as f32;
        let moved = |p: Vec3| pivot + rotate * ((p - pivot) * scale) + translate;
        // Weights are measured on the mesh as it was.
        let mut soft: Vec<(usize, f32)> = vec![];
        if falloff > 1e-6 && picked.len() * self.verts.len() <= 40_000_000 {
            let anchors: Vec<Vec3> = picked.iter().map(|v| self.verts[*v]).collect();
            for v in 0..self.verts.len() {
                if verts.get(v).copied().unwrap_or(false) { continue; }
                let p = self.verts[v];
                let d = anchors.iter().map(|a| a.distance_squared(p)).fold(f32::MAX, f32::min).sqrt();
                if d < falloff {
                    let t = 1.0 - d / falloff;
                    soft.push((v, t * t * (3.0 - 2.0 * t)));
                }
            }
        }
        for v in picked { self.verts[v] = moved(self.verts[v]); }
        for (v, w) in soft { let p = self.verts[v]; self.verts[v] = p.lerp(moved(p), w); }
    }

    // ── Delete / remove ──────────────────────────────────────────────────────

    /// Delete polygons, leaving holes.
    pub fn delete_polys(&mut self, mask: &[bool]) {
        let mut i = 0;
        self.polys.retain(|_| { i += 1; !mask.get(i - 1).copied().unwrap_or(false) });
        self.compact();
    }

    /// Remove edges without leaving a hole: the two polygons beside each edge
    /// become one. With `clean`, vertices left with only two edges go too.
    pub fn remove_edges(&mut self, edges: &HashSet<[u32; 2]>, clean: bool) {
        let mut touched: HashSet<u32> = HashSet::new();
        let mut order: Vec<[u32; 2]> = edges.iter().copied().collect();
        order.sort();
        for e in order {
            // The two polygons that use the edge, in opposite directions.
            let side = |polys: &Vec<Vec<u32>>, a: u32, b: u32| -> Option<(usize, usize)> {
                polys.iter().enumerate().find_map(|(p, poly)| {
                    (0..poly.len()).find(|i| poly[*i] == a && poly[(*i + 1) % poly.len()] == b).map(|i| (p, i))
                })
            };
            let (Some((pa, ia)), Some((pb, ib))) = (side(&self.polys, e[0], e[1]), side(&self.polys, e[1], e[0])) else { continue };
            if pa == pb { continue; }
            let (a, b) = (&self.polys[pa], &self.polys[pb]);
            // A from e1 round to e0, then the rest of B strictly between e0 and e1.
            let mut merged: Vec<u32> = (0..a.len()).map(|k| a[(ia + 1 + k) % a.len()]).collect();
            merged.extend((0..b.len().saturating_sub(2)).map(|k| b[(ib + 2 + k) % b.len()]));
            self.polys[pa] = merged;
            self.polys.remove(pb);
            touched.insert(e[0]);
            touched.insert(e[1]);
        }
        // Merging can leave a spur (x, v, x): fold it away.
        self.fold_spurs();
        if clean {
            let mut uses: HashMap<u32, HashSet<u32>> = HashMap::new();
            for poly in &self.polys {
                for i in 0..poly.len() {
                    let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                    uses.entry(a).or_default().insert(b);
                    uses.entry(b).or_default().insert(a);
                }
            }
            let gone: HashSet<u32> = touched.into_iter()
                .filter(|v| uses.get(v).map(|n| n.len() == 2).unwrap_or(false))
                .collect();
            for poly in &mut self.polys {
                if poly.len() - poly.iter().filter(|v| gone.contains(v)).count() >= 3 {
                    poly.retain(|v| !gone.contains(v));
                }
            }
        }
        self.cleanup();
    }

    fn fold_spurs(&mut self) {
        for poly in &mut self.polys {
            loop {
                let n = poly.len();
                if n < 3 { break; }
                let Some(i) = (0..n).find(|i| poly[*i] == poly[(*i + 2) % n]) else { break };
                // Remove the tip and one copy of the repeated vertex.
                let (tip, dup) = ((i + 1) % n, (i + 2) % n);
                let (hi, lo) = (tip.max(dup), tip.min(dup));
                poly.remove(hi);
                poly.remove(lo);
            }
        }
    }

    /// Remove vertices without leaving a hole: the polygons around each one
    /// become a single polygon.
    pub fn remove_verts(&mut self, verts: &[bool]) {
        let gone: Vec<u32> = (0..self.verts.len() as u32).filter(|v| verts.get(*v as usize).copied().unwrap_or(false)).collect();
        for v in gone {
            let around: HashSet<[u32; 2]> = self.polys.iter().flat_map(|poly| {
                (0..poly.len()).filter_map(|i| {
                    let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                    (a == v || b == v).then(|| edge_key(a, b))
                }).collect::<Vec<_>>()
            }).collect();
            self.remove_edges_keep_verts(&around);
            for poly in &mut self.polys { poly.retain(|x| *x != v); }
            for poly in &mut self.polys {
                poly.dedup();
                while poly.len() > 1 && poly.first() == poly.last() { poly.pop(); }
            }
            self.polys.retain(|p| p.len() >= 3);
        }
        self.cleanup();
    }

    /// `remove_edges` without the final renumbering, for callers that still
    /// hold vertex indices.
    fn remove_edges_keep_verts(&mut self, edges: &HashSet<[u32; 2]>) {
        let verts = std::mem::take(&mut self.verts);
        // With no vertices to drop, compaction is skipped.
        let mut tmp = PolyMesh { verts: vec![Vec3::ZERO; verts.len()], polys: std::mem::take(&mut self.polys) };
        // Keep every vertex referenced by adding a throwaway polygon.
        let keep: Vec<u32> = (0..verts.len() as u32).collect();
        tmp.polys.push(keep.clone());
        let marker = tmp.polys.len() - 1;
        let mut order: Vec<[u32; 2]> = edges.iter().copied().collect();
        order.sort();
        for e in order {
            let side = |polys: &Vec<Vec<u32>>, a: u32, b: u32| -> Option<(usize, usize)> {
                polys.iter().enumerate().take(marker).find_map(|(p, poly)| {
                    (0..poly.len()).find(|i| poly[*i] == a && poly[(*i + 1) % poly.len()] == b).map(|i| (p, i))
                })
            };
            let (Some((pa, ia)), Some((pb, ib))) = (side(&tmp.polys, e[0], e[1]), side(&tmp.polys, e[1], e[0])) else { continue };
            if pa == pb { continue; }
            let (a, b) = (&tmp.polys[pa], &tmp.polys[pb]);
            let mut merged: Vec<u32> = (0..a.len()).map(|k| a[(ia + 1 + k) % a.len()]).collect();
            merged.extend((0..b.len().saturating_sub(2)).map(|k| b[(ib + 2 + k) % b.len()]));
            tmp.polys[pa] = merged;
            // Blank instead of removing, so `marker` stays valid.
            tmp.polys[pb] = vec![];
        }
        tmp.polys.truncate(marker);
        tmp.polys.retain(|p| !p.is_empty());
        tmp.fold_spurs();
        self.polys = tmp.polys;
        self.verts = verts;
    }

    // ── Weld / collapse ──────────────────────────────────────────────────────

    /// Merge groups of vertices into one vertex each, at the group's centre.
    fn merge_groups(&mut self, group_of: &[Option<usize>]) {
        let mut sum: HashMap<usize, (Vec3, u32, u32)> = HashMap::new();   // centre sum, count, first vertex
        for (v, g) in group_of.iter().enumerate() {
            if let Some(g) = g {
                let e = sum.entry(*g).or_insert((Vec3::ZERO, 0, v as u32));
                e.0 += self.verts[v];
                e.1 += 1;
            }
        }
        let mut remap: Vec<u32> = (0..self.verts.len() as u32).collect();
        for (v, g) in group_of.iter().enumerate() {
            if let Some(g) = g {
                let (total, n, first) = sum[g];
                remap[v] = first;
                self.verts[first as usize] = total / n as f32;
            }
        }
        for poly in &mut self.polys { for v in poly.iter_mut() { *v = remap[*v as usize]; } }
        self.cleanup();
    }

    /// Weld chosen vertices that lie within `threshold` of each other.
    pub fn weld(&mut self, verts: &[bool], threshold: f32) {
        let picked: Vec<usize> = (0..self.verts.len()).filter(|v| verts.get(*v).copied().unwrap_or(false)).collect();
        let mut parent: Vec<usize> = (0..picked.len()).collect();
        fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
            while p[x] != x { p[x] = p[p[x]]; x = p[x]; }
            x
        }
        for i in 0..picked.len() {
            for j in i + 1..picked.len() {
                if self.verts[picked[i]].distance(self.verts[picked[j]]) <= threshold {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
        let mut group_of = vec![None; self.verts.len()];
        for i in 0..picked.len() { group_of[picked[i]] = Some(find(&mut parent, i)); }
        self.merge_groups(&group_of);
    }

    /// Collapse each connected group of chosen vertices to its centre.
    /// Vertices count as connected through edges whose two ends are chosen.
    pub fn collapse(&mut self, verts: &[bool]) {
        let chosen = |v: u32| verts.get(v as usize).copied().unwrap_or(false);
        let mut parent: Vec<usize> = (0..self.verts.len()).collect();
        fn find(p: &mut Vec<usize>, mut x: usize) -> usize {
            while p[x] != x { p[x] = p[p[x]]; x = p[x]; }
            x
        }
        for e in self.edges() {
            if chosen(e[0]) && chosen(e[1]) {
                let (a, b) = (find(&mut parent, e[0] as usize), find(&mut parent, e[1] as usize));
                parent[a] = b;
            }
        }
        let group_of: Vec<Option<usize>> = (0..self.verts.len())
            .map(|v| chosen(v as u32).then(|| find(&mut parent, v)))
            .collect();
        self.merge_groups(&group_of);
    }

    // ── Connect ──────────────────────────────────────────────────────────────

    /// Cut new edges across polygons, between pairs of chosen edges. Each
    /// chosen edge gets `segments` new vertices; a polygon with exactly two
    /// chosen edges is split along them. This is how an edge loop is added
    /// across a ring of edges.
    pub fn connect(&mut self, edges: &HashSet<[u32; 2]>, segments: u32) {
        let n = segments.max(1) as usize;
        // New vertices on each edge, ordered from its lower-numbered end.
        let mut on_edge: HashMap<[u32; 2], Vec<u32>> = HashMap::new();
        let mut keys: Vec<[u32; 2]> = edges.iter().copied().collect();
        keys.sort();
        for e in keys {
            let (a, b) = (self.verts[e[0] as usize], self.verts[e[1] as usize]);
            let ids = (1..=n).map(|k| {
                self.verts.push(a.lerp(b, k as f32 / (n + 1) as f32));
                self.verts.len() as u32 - 1
            }).collect();
            on_edge.insert(e, ids);
        }

        let mut out: Vec<Vec<u32>> = Vec::with_capacity(self.polys.len());
        for poly in &self.polys {
            // Loop with the new vertices in place, and where each run starts.
            let mut lp: Vec<u32> = vec![];
            let mut runs: Vec<usize> = vec![];
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                lp.push(a);
                if let Some(ids) = on_edge.get(&edge_key(a, b)) {
                    runs.push(lp.len());
                    if a < b { lp.extend(ids.iter()); } else { lp.extend(ids.iter().rev()); }
                }
            }
            if runs.len() != 2 { out.push(lp); continue; }
            let (p, q) = (runs[0], runs[1]);           // first new vertex of each run
            let len = lp.len();
            let span = |from: usize, to: usize| -> Vec<u32> {
                let mut v = vec![];
                let mut i = from;
                loop { v.push(lp[i % len]); if i % len == to % len { break; } i += 1; }
                v
            };
            // End piece from the last vertex of the second run round to the first of the first.
            out.push(span(q + n - 1, p + len));
            for k in 0..n - 1 {
                out.push(vec![lp[p + k], lp[p + k + 1], lp[q + n - 2 - k], lp[q + n - 1 - k]]);
            }
            out.push(span(p + n - 1, q));
        }
        self.polys = out;
        self.cleanup();
    }

    /// Join two chosen vertices of a polygon with a new edge, where they are
    /// not already neighbours.
    pub fn connect_verts(&mut self, verts: &[bool]) {
        let mut out = Vec::with_capacity(self.polys.len());
        for poly in &self.polys {
            let n = poly.len();
            let hit: Vec<usize> = (0..n).filter(|i| verts.get(poly[*i] as usize).copied().unwrap_or(false)).collect();
            if hit.len() != 2 || hit[1] - hit[0] < 2 || hit[1] - hit[0] > n - 2 {
                out.push(poly.clone());
                continue;
            }
            out.push(poly[hit[0]..=hit[1]].to_vec());
            out.push((hit[1]..=hit[0] + n).map(|i| poly[i % n]).collect());
        }
        self.polys = out;
    }

    /// Split the chosen vertices: every polygon around one gets its own copy.
    pub fn break_verts(&mut self, verts: &[bool]) {
        let mut seen: HashSet<u32> = HashSet::new();
        for p in 0..self.polys.len() {
            for i in 0..self.polys[p].len() {
                let v = self.polys[p][i];
                if !verts.get(v as usize).copied().unwrap_or(false) { continue; }
                if seen.insert(v) { continue; }
                self.verts.push(self.verts[v as usize]);
                self.polys[p][i] = self.verts.len() as u32 - 1;
            }
        }
    }

    // ── Cap / bridge / detach ────────────────────────────────────────────────

    /// Fill open borders with one polygon each. With `only`, just the
    /// borders that contain one of those edges.
    pub fn cap(&mut self, only: Option<&HashSet<[u32; 2]>>) {
        for lp in self.border_loops() {
            let wanted = only.map(|set| {
                (0..lp.len()).any(|i| set.contains(&edge_key(lp[i], lp[(i + 1) % lp.len()])))
            }).unwrap_or(true);
            if wanted { self.polys.push(lp); }
        }
    }

    /// Join two loops of equal length with a band of quads. Both loops must
    /// be wound like polygons facing out of the opening they close.
    fn bridge_loops(&mut self, a: &[u32], b: &[u32]) -> bool {
        let n = a.len();
        if n < 3 || n != b.len() { return false; }
        // Matching corner of b runs backwards; pick the rotation with the
        // shortest connections.
        let cost = |s: usize| -> f32 {
            (0..n).map(|i| self.verts[a[i] as usize].distance(self.verts[b[(s + n - i) % n] as usize])).sum()
        };
        let s = (0..n).min_by(|x, y| cost(*x).total_cmp(&cost(*y))).unwrap();
        for i in 0..n {
            let (bi, bj) = (b[(s + n - i) % n], b[(s + 2 * n - i - 1) % n]);
            self.polys.push(vec![a[i], a[(i + 1) % n], bj, bi]);
        }
        true
    }

    /// Bridge two groups of polygons: the polygons go and a band of quads
    /// joins the two openings. Needs exactly two openings with the same
    /// number of edges. Returns false, changing nothing, otherwise.
    pub fn bridge_polys(&mut self, mask: &[bool]) -> bool {
        let chosen = |p: usize| mask.get(p).copied().unwrap_or(false);
        let mut directed: HashMap<(u32, u32), ()> = HashMap::new();
        for (p, poly) in self.polys.iter().enumerate() {
            if !chosen(p) { continue; }
            for i in 0..poly.len() { directed.insert((poly[i], poly[(i + 1) % poly.len()]), ()); }
        }
        // Outline of the chosen polygons, wound the way they are.
        let mut next: HashMap<u32, u32> = HashMap::new();
        for (a, b) in directed.keys() {
            if !directed.contains_key(&(*b, *a)) { next.insert(*a, *b); }
        }
        let mut loops: Vec<Vec<u32>> = vec![];
        let mut starts: Vec<u32> = next.keys().copied().collect();
        starts.sort();
        for start in starts {
            if !next.contains_key(&start) { continue; }
            let mut lp = vec![start];
            let mut cur = start;
            while let Some(n) = next.remove(&cur) {
                if n == start { break; }
                lp.push(n);
                cur = n;
            }
            loops.push(lp);
        }
        if loops.len() != 2 || loops[0].len() != loops[1].len() { return false; }
        let mut i = 0;
        self.polys.retain(|_| { i += 1; !chosen(i - 1) });
        let ok = self.bridge_loops(&loops[0], &loops[1]);
        self.compact();
        ok
    }

    /// Bridge two open borders. `only` limits it to borders containing one
    /// of those edges. Needs exactly two such borders of equal length.
    pub fn bridge_borders(&mut self, only: Option<&HashSet<[u32; 2]>>) -> bool {
        let loops: Vec<Vec<u32>> = self.border_loops().into_iter().filter(|lp| {
            only.map(|set| (0..lp.len()).any(|i| set.contains(&edge_key(lp[i], lp[(i + 1) % lp.len()])))).unwrap_or(true)
        }).collect();
        if loops.len() != 2 || loops[0].len() != loops[1].len() { return false; }
        self.bridge_loops(&loops[0], &loops[1])
    }

    /// Give the chosen polygons their own vertices, making them a separate element.
    pub fn detach(&mut self, mask: &[bool]) {
        let mut copy: HashMap<u32, u32> = HashMap::new();
        for p in 0..self.polys.len() {
            if !mask.get(p).copied().unwrap_or(false) { continue; }
            for i in 0..self.polys[p].len() {
                let v = self.polys[p][i];
                let c = *copy.entry(v).or_insert_with(|| { self.verts.push(self.verts[v as usize]); self.verts.len() as u32 - 1 });
                self.polys[p][i] = c;
            }
        }
        self.compact();
    }

    // ── Flip / planar / relax ────────────────────────────────────────────────

    pub fn flip(&mut self, mask: &[bool]) {
        for (p, poly) in self.polys.iter_mut().enumerate() {
            if mask.get(p).copied().unwrap_or(false) { poly.reverse(); }
        }
    }

    /// Flatten the chosen vertices onto a plane through their centre.
    /// `axis` 0, 1, 2 gives a plane across X, Y or Z; None fits the plane to
    /// the vertices.
    pub fn make_planar(&mut self, verts: &[bool], axis: Option<usize>) {
        let picked: Vec<usize> = (0..self.verts.len()).filter(|v| verts.get(*v).copied().unwrap_or(false)).collect();
        if picked.len() < 2 { return; }
        let centre = picked.iter().map(|v| self.verts[*v]).sum::<Vec3>() / picked.len() as f32;
        let normal = match axis {
            Some(a) => Vec3::AXES[a.min(2)],
            None => {
                // Direction of least spread: power iteration on the inverse
                // of the covariance, i.e. its smallest eigenvector.
                let mut cov = Mat3::ZERO;
                for v in &picked {
                    let d = self.verts[*v] - centre;
                    cov += Mat3::from_cols(d * d.x, d * d.y, d * d.z);
                }
                let shifted = cov + Mat3::from_diagonal(Vec3::splat(1e-9 + cov.x_axis.x + cov.y_axis.y + cov.z_axis.z) * 1e-6);
                let inv = shifted.inverse();
                let mut n = Vec3::new(0.577, 0.577, 0.577);
                for _ in 0..40 { n = (inv * n).normalize_or_zero(); }
                if n == Vec3::ZERO { Vec3::Y } else { n }
            }
        };
        for v in picked {
            let d = self.verts[v] - centre;
            self.verts[v] -= normal * d.dot(normal);
        }
    }

    /// Move each chosen vertex towards the average of its edge neighbours.
    pub fn relax(&mut self, verts: &[bool], amount: f32, iterations: u32, hold_border: bool) {
        let edges = self.edges();
        let mut near: Vec<Vec<u32>> = vec![vec![]; self.verts.len()];
        for e in &edges { near[e[0] as usize].push(e[1]); near[e[1] as usize].push(e[0]); }
        let mut on_border = vec![false; self.verts.len()];
        for lp in self.border_loops() { for v in lp { on_border[v as usize] = true; } }
        for _ in 0..iterations {
            let prev = self.verts.clone();
            for v in 0..prev.len() {
                if !verts.get(v).copied().unwrap_or(false) || near[v].is_empty() { continue; }
                if hold_border && on_border[v] { continue; }
                let avg = near[v].iter().map(|n| prev[*n as usize]).sum::<Vec3>() / near[v].len() as f32;
                self.verts[v] = prev[v].lerp(avg, amount);
            }
        }
    }

    // ── Tessellate / subdivide ───────────────────────────────────────────────

    /// Split each chosen polygon into quads around a centre vertex. Edge
    /// midpoints are shared with neighbours, which gain a corner.
    pub fn tessellate(&mut self, mask: &[bool]) {
        let chosen = |p: usize| mask.get(p).copied().unwrap_or(false);
        let mut mid: HashMap<[u32; 2], u32> = HashMap::new();
        for p in 0..self.polys.len() {
            if !chosen(p) { continue; }
            let poly = self.polys[p].clone();
            for i in 0..poly.len() {
                let e = edge_key(poly[i], poly[(i + 1) % poly.len()]);
                if !mid.contains_key(&e) {
                    self.verts.push((self.verts[e[0] as usize] + self.verts[e[1] as usize]) * 0.5);
                    mid.insert(e, self.verts.len() as u32 - 1);
                }
            }
        }
        let mut out = vec![];
        for (p, poly) in self.polys.clone().iter().enumerate() {
            let n = poly.len();
            if chosen(p) {
                self.verts.push(poly.iter().map(|v| self.verts[*v as usize]).sum::<Vec3>() / n as f32);
                let c = self.verts.len() as u32 - 1;
                for i in 0..n {
                    let before = mid[&edge_key(poly[(i + n - 1) % n], poly[i])];
                    let after  = mid[&edge_key(poly[i], poly[(i + 1) % n])];
                    out.push(vec![poly[i], after, c, before]);
                }
            } else {
                let mut lp = vec![];
                for i in 0..n {
                    lp.push(poly[i]);
                    if let Some(m) = mid.get(&edge_key(poly[i], poly[(i + 1) % n])) { lp.push(*m); }
                }
                out.push(lp);
            }
        }
        self.polys = out;
    }

    /// One step of Catmull-Clark subdivision over the whole mesh. Open
    /// borders are smoothed as curves.
    pub fn subdivide(&mut self) {
        let nv = self.verts.len();
        let face_pt: Vec<Vec3> = (0..self.polys.len()).map(|p| self.centroid(p)).collect();

        // Faces beside each edge.
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for (p, poly) in self.polys.iter().enumerate() {
            for i in 0..poly.len() { edge_faces.entry(edge_key(poly[i], poly[(i + 1) % poly.len()])).or_default().push(p); }
        }
        let mut keys: Vec<[u32; 2]> = edge_faces.keys().copied().collect();
        keys.sort();

        let mut verts = self.verts.clone();
        let face_id: Vec<u32> = face_pt.iter().map(|p| { verts.push(*p); verts.len() as u32 - 1 }).collect();
        let mut edge_id: HashMap<[u32; 2], u32> = HashMap::new();
        for e in &keys {
            let (a, b) = (self.verts[e[0] as usize], self.verts[e[1] as usize]);
            let f = &edge_faces[e];
            let pt = if f.len() == 2 { (a + b + face_pt[f[0]] + face_pt[f[1]]) * 0.25 } else { (a + b) * 0.5 };
            verts.push(pt);
            edge_id.insert(*e, verts.len() as u32 - 1);
        }

        // Original vertices.
        let mut faces_at: Vec<Vec<usize>> = vec![vec![]; nv];
        for (p, poly) in self.polys.iter().enumerate() { for v in poly { faces_at[*v as usize].push(p); } }
        let mut edges_at: Vec<Vec<[u32; 2]>> = vec![vec![]; nv];
        for e in &keys { edges_at[e[0] as usize].push(*e); edges_at[e[1] as usize].push(*e); }
        for v in 0..nv {
            let p = self.verts[v];
            let open: Vec<&[u32; 2]> = edges_at[v].iter().filter(|e| edge_faces[*e].len() != 2).collect();
            if !open.is_empty() {
                if open.len() == 2 {
                    let other = |e: &[u32; 2]| self.verts[if e[0] as usize == v { e[1] } else { e[0] } as usize];
                    verts[v] = p * 0.75 + (other(open[0]) + other(open[1])) * 0.125;
                }
                continue;
            }
            let n = faces_at[v].len() as f32;
            if n < 3.0 || edges_at[v].is_empty() { continue; }
            let f = faces_at[v].iter().map(|q| face_pt[*q]).sum::<Vec3>() / n;
            let r = edges_at[v].iter()
                .map(|e| (self.verts[e[0] as usize] + self.verts[e[1] as usize]) * 0.5)
                .sum::<Vec3>() / edges_at[v].len() as f32;
            verts[v] = (f + r * 2.0 + p * (n - 3.0)) / n;
        }

        let mut polys = Vec::with_capacity(self.polys.iter().map(|p| p.len()).sum());
        for (p, poly) in self.polys.iter().enumerate() {
            let n = poly.len();
            for i in 0..n {
                let before = edge_id[&edge_key(poly[(i + n - 1) % n], poly[i])];
                let after  = edge_id[&edge_key(poly[i], poly[(i + 1) % n])];
                polys.push(vec![poly[i], after, face_id[p], before]);
            }
        }
        self.verts = verts;
        self.polys = polys;
    }

    // ── Edge loops and rings ─────────────────────────────────────────────────

    /// Extend edges into loops: at each end, continue through vertices where
    /// exactly four edges meet, along the edge that shares no polygon.
    pub fn edge_loop(&self, start: &HashSet<[u32; 2]>) -> HashSet<[u32; 2]> {
        let mut polys_of: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for (p, poly) in self.polys.iter().enumerate() {
            for i in 0..poly.len() { polys_of.entry(edge_key(poly[i], poly[(i + 1) % poly.len()])).or_default().push(p); }
        }
        let mut at: HashMap<u32, Vec<[u32; 2]>> = HashMap::new();
        for e in polys_of.keys() { at.entry(e[0]).or_default().push(*e); at.entry(e[1]).or_default().push(*e); }
        let mut out = start.clone();
        let mut todo: Vec<([u32; 2], u32)> = start.iter().flat_map(|e| [(*e, e[0]), (*e, e[1])]).collect();
        while let Some((e, v)) = todo.pop() {
            let around = &at[&v];
            if around.len() != 4 { continue; }
            let mine = &polys_of[&e];
            let Some(next) = around.iter().find(|o| **o != e && !polys_of[*o].iter().any(|p| mine.contains(p))) else { continue };
            if out.insert(*next) {
                todo.push((*next, if next[0] == v { next[1] } else { next[0] }));
            }
        }
        out
    }

    /// Extend edges into rings: across each neighbouring quad to the edge opposite.
    pub fn edge_ring(&self, start: &HashSet<[u32; 2]>) -> HashSet<[u32; 2]> {
        let mut polys_of: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for (p, poly) in self.polys.iter().enumerate() {
            for i in 0..poly.len() { polys_of.entry(edge_key(poly[i], poly[(i + 1) % poly.len()])).or_default().push(p); }
        }
        let mut out = start.clone();
        let mut todo: Vec<[u32; 2]> = start.iter().copied().collect();
        while let Some(e) = todo.pop() {
            for p in polys_of.get(&e).into_iter().flatten() {
                let poly = &self.polys[*p];
                if poly.len() != 4 { continue; }
                let Some(i) = (0..4).find(|i| edge_key(poly[*i], poly[(*i + 1) % 4]) == e) else { continue };
                let opposite = edge_key(poly[(i + 2) % 4], poly[(i + 3) % 4]);
                if out.insert(opposite) { todo.push(opposite); }
            }
        }
        out
    }
}

// ============================================================================
// SECOND SET: chamfer, vertex and edge extrude, outline, hinge, slice,
// insert vertex, poke, triangulate, turn
// ============================================================================

impl PolyMesh {
    fn chosen(flags: &[bool], i: u32) -> bool { flags.get(i as usize).copied().unwrap_or(false) }

    /// Average of the normals of the polygons around each vertex.
    fn vertex_normals(&self) -> Vec<Vec3> {
        let mut n = vec![Vec3::ZERO; self.verts.len()];
        for p in 0..self.polys.len() {
            let a = self.area_normal(p);
            for v in &self.polys[p] { n[*v as usize] += a; }
        }
        n.into_iter().map(|v| v.normalize_or_zero()).collect()
    }

    /// Cut the corner off each chosen vertex: the vertex is replaced by a
    /// polygon with one corner on each edge that met there, `amount` along
    /// the edge. With `apex`, the vertex stays, moved that far along its
    /// normal, and the polygon becomes a pyramid: a vertex extrude.
    pub fn chamfer_verts(&mut self, verts: &[bool], amount: f32, apex: Option<f32>) {
        let base = self.verts.clone();
        let normals = self.vertex_normals();
        // Point on the edge from v towards w.
        let mut on_edge: HashMap<(u32, u32), u32> = HashMap::new();
        let mut point = |mesh: &mut PolyMesh, v: u32, w: u32| -> u32 {
            *on_edge.entry((v, w)).or_insert_with(|| {
                let (a, b) = (base[v as usize], base[w as usize]);
                let len = a.distance(b);
                // Leave room when both ends are cut.
                let limit = if Self::chosen(verts, w) { len * 0.49 } else { len * 0.98 };
                let d = amount.abs().min(limit);
                mesh.verts.push(a + (b - a).normalize_or_zero() * d);
                mesh.verts.len() as u32 - 1
            })
        };
        // Around each cut vertex: from the point on the next edge of a
        // polygon back to the point on its previous edge.
        let mut links: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for p in 0..self.polys.len() {
            let poly = self.polys[p].clone();
            let n = poly.len();
            if !poly.iter().any(|v| Self::chosen(verts, *v)) { continue; }
            let mut out = Vec::with_capacity(n + 2);
            for i in 0..n {
                let v = poly[i];
                if !Self::chosen(verts, v) { out.push(v); continue; }
                let before = point(self, v, poly[(i + n - 1) % n]);
                let after  = point(self, v, poly[(i + 1) % n]);
                out.push(before);
                out.push(after);
                links.entry(v).or_default().push((after, before));
            }
            self.polys[p] = out;
        }
        let mut cut: Vec<u32> = links.keys().copied().collect();
        cut.sort();
        for v in cut {
            let pairs = &links[&v];
            match apex {
                Some(height) => {
                    self.verts[v as usize] = base[v as usize] + normals[v as usize] * height;
                    for (a, b) in pairs { self.polys.push(vec![*a, *b, v]); }
                }
                None => {
                    // Chain the links into the polygon that closes the cut.
                    let next: HashMap<u32, u32> = pairs.iter().copied().collect();
                    let start = pairs[0].0;
                    let mut lp = vec![start];
                    let mut cur = start;
                    while let Some(n) = next.get(&cur) {
                        if *n == start || lp.len() > pairs.len() { break; }
                        lp.push(*n);
                        cur = *n;
                    }
                    // An open vertex (on a border) leaves no closed loop.
                    if lp.len() == pairs.len() && next.get(&cur) == Some(&start) && lp.len() >= 3 { self.polys.push(lp); }
                }
            }
        }
        self.cleanup();
    }

    /// Replace each chosen edge by a strip of polygons `amount` wide on
    /// each side. Corners where several chosen edges meet are closed with a
    /// polygon.
    pub fn chamfer_edges(&mut self, edges: &HashSet<[u32; 2]>, amount: f32) {
        if edges.is_empty() || amount.abs() < 1e-9 { return; }
        let amount = amount.abs();
        let base = self.verts.clone();
        let old_count = self.verts.len() as u32;
        let touched: HashSet<u32> = edges.iter().flat_map(|e| [e[0], e[1]]).collect();
        let picked = |a: u32, b: u32| edges.contains(&edge_key(a, b));
        let old_polys = self.polys.clone();

        // Point sliding from v along an edge that is not chosen.
        let mut slide: HashMap<(u32, u32), u32> = HashMap::new();
        let mut slide_point = |mesh: &mut PolyMesh, v: u32, w: u32| -> u32 {
            *slide.entry((v, w)).or_insert_with(|| {
                let (a, b) = (base[v as usize], base[w as usize]);
                let len = a.distance(b);
                let limit = if touched.contains(&w) { len * 0.49 } else { len * 0.98 };
                mesh.verts.push(a + (b - a).normalize_or_zero() * amount.min(limit));
                mesh.verts.len() as u32 - 1
            })
        };
        // The corner of polygon p at v that lies next to a chosen edge.
        let mut beside: HashMap<(usize, u32), u32> = HashMap::new();

        for (p, poly) in old_polys.iter().enumerate() {
            let n = poly.len();
            if !poly.iter().any(|v| touched.contains(v)) { continue; }
            let mut out = Vec::with_capacity(n + 2);
            for i in 0..n {
                let v = poly[i];
                if !touched.contains(&v) { out.push(v); continue; }
                let (prev, next) = (poly[(i + n - 1) % n], poly[(i + 1) % n]);
                match (picked(prev, v), picked(v, next)) {
                    (true, true) => {
                        // Inset corner: `amount` away from both edges.
                        let (dp, dn) = ((base[prev as usize] - base[v as usize]).normalize_or_zero(),
                                        (base[next as usize] - base[v as usize]).normalize_or_zero());
                        let sin = dp.cross(dn).length().max(0.2);
                        let reach = (amount / sin).min(base[prev as usize].distance(base[v as usize]) * 0.49)
                            .min(base[next as usize].distance(base[v as usize]) * 0.49);
                        self.verts.push(base[v as usize] + (dp + dn) * reach);
                        let id = self.verts.len() as u32 - 1;
                        beside.insert((p, v), id);
                        out.push(id);
                    }
                    (true, false) => { let id = slide_point(self, v, next); beside.insert((p, v), id); out.push(id); }
                    (false, true) => { let id = slide_point(self, v, prev); beside.insert((p, v), id); out.push(id); }
                    (false, false) => {
                        // Not beside a chosen edge, but at its end: the corner opens up.
                        out.push(slide_point(self, v, prev));
                        out.push(slide_point(self, v, next));
                    }
                }
            }
            self.polys[p] = out;
        }

        // A strip for each chosen edge, between the two polygons beside it.
        let mut side: HashMap<(u32, u32), usize> = HashMap::new();
        for (p, poly) in old_polys.iter().enumerate() {
            for i in 0..poly.len() { side.insert((poly[i], poly[(i + 1) % poly.len()]), p); }
        }
        let mut keys: Vec<[u32; 2]> = edges.iter().copied().collect();
        keys.sort();
        for e in keys {
            let (a, b) = (e[0], e[1]);
            let (Some(f), Some(g)) = (side.get(&(a, b)), side.get(&(b, a))) else { continue };
            let (Some(fa), Some(fb), Some(ga), Some(gb)) =
                (beside.get(&(*f, a)), beside.get(&(*f, b)), beside.get(&(*g, a)), beside.get(&(*g, b))) else { continue };
            self.polys.push(vec![*fb, *fa, *ga, *gb]);
        }

        // Close the holes left where chosen edges meet or end: borders made
        // of new points only.
        for poly in &mut self.polys {
            poly.dedup();
            while poly.len() > 1 && poly.first() == poly.last() { poly.pop(); }
        }
        self.polys.retain(|p| p.len() >= 3);
        for lp in self.border_loops() {
            if lp.iter().all(|v| *v >= old_count) { self.polys.push(lp); }
        }
        self.cleanup();
    }

    /// Extrude open edges: a new polygon grows from each chosen border edge,
    /// `width` outwards in the plane of its polygon and `height` along its
    /// normal.
    pub fn extrude_border_edges(&mut self, edges: &HashSet<[u32; 2]>, height: f32, width: f32) {
        let open = self.open_edges();
        // Chosen border edges as the polygon winds them, with its normal.
        let mut found: Vec<(u32, u32, Vec3)> = vec![];
        for p in 0..self.polys.len() {
            let poly = &self.polys[p];
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let k = edge_key(a, b);
                if edges.contains(&k) && open.contains(&k) { found.push((a, b, self.normal(p))); }
            }
        }
        // Per vertex: the outward directions of the edges arriving and leaving, and the normal.
        let mut push: HashMap<u32, (Vec<Vec3>, Vec3)> = HashMap::new();
        for (a, b, n) in &found {
            let d = (self.verts[*b as usize] - self.verts[*a as usize]).normalize_or_zero();
            let outward = d.cross(*n).normalize_or_zero();
            for v in [a, b] {
                let e = push.entry(*v).or_insert((vec![], Vec3::ZERO));
                e.0.push(outward);
                e.1 += *n;
            }
        }
        let mut copy: HashMap<u32, u32> = HashMap::new();
        let mut ids: Vec<u32> = push.keys().copied().collect();
        ids.sort();
        for v in ids {
            let (o, n) = &push[&v];
            // Between two edges: the shift that moves both of them out by `width`.
            let out = if o.len() >= 2 { (o[0] + o[1]) / (1.0 + o[0].dot(o[1])).max(0.25) } else { o[0] };
            self.verts.push(self.verts[v as usize] + out * width + n.normalize_or_zero() * height);
            copy.insert(v, self.verts.len() as u32 - 1);
        }
        for (a, b, _) in found { self.polys.push(vec![b, a, copy[&a], copy[&b]]); }
        self.cleanup();
    }

    /// Grow (positive) or shrink the outline of the chosen polygons in
    /// their own plane. No polygons are added: the neighbours follow.
    pub fn outline(&mut self, mask: &[bool], amount: f32) {
        let chosen = |p: usize| mask.get(p).copied().unwrap_or(false);
        let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
        for (p, poly) in self.polys.iter().enumerate() {
            if !chosen(p) { continue; }
            for i in 0..poly.len() { directed.insert((poly[i], poly[(i + 1) % poly.len()]), p); }
        }
        let (mut incoming, mut outgoing): (HashMap<u32, Vec3>, HashMap<u32, Vec3>) = (HashMap::new(), HashMap::new());
        for ((a, b), p) in &directed {
            if directed.contains_key(&(*b, *a)) { continue; }
            let d = (self.verts[*b as usize] - self.verts[*a as usize]).normalize_or_zero();
            let outward = d.cross(self.normal(*p)).normalize_or_zero();
            outgoing.insert(*a, outward);
            incoming.insert(*b, outward);
        }
        let mut ids: Vec<u32> = incoming.keys().copied().collect();
        ids.sort();
        for v in ids {
            if let (Some(p1), Some(p2)) = (incoming.get(&v), outgoing.get(&v)) {
                self.verts[v as usize] += (*p1 + *p2) / (1.0 + p1.dot(*p2)).max(0.25) * amount;
            }
        }
    }

    /// Swing the chosen polygons about one edge of their outline, like a
    /// door on its hinge, leaving a wedge of new polygons behind. `edge`
    /// counts along the outline to pick the hinge.
    pub fn hinge(&mut self, mask: &[bool], angle_deg: f32, segments: u32, edge: u32) {
        let chosen = |p: usize| mask.get(p).copied().unwrap_or(false);
        let region: Vec<usize> = (0..self.polys.len()).filter(|p| chosen(*p)).collect();
        if region.is_empty() { return; }
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        for p in &region {
            let poly = &self.polys[*p];
            for i in 0..poly.len() { directed.insert((poly[i], poly[(i + 1) % poly.len()])); }
        }
        let mut outline: Vec<(u32, u32)> = directed.iter().copied().filter(|(a, b)| !directed.contains(&(*b, *a))).collect();
        if outline.is_empty() { return; }
        outline.sort();
        let (a, b) = outline[edge as usize % outline.len()];
        let (origin, axis) = (self.verts[a as usize], (self.verts[b as usize] - self.verts[a as usize]).normalize_or_zero());
        let segments = segments.clamp(1, 64);
        let step = Quat::from_axis_angle(axis, angle_deg.to_radians() / segments as f32);
        for _ in 0..segments {
            let mut flags = vec![false; self.polys.len()];
            for p in &region { flags[*p] = true; }
            super::poly::offset_faces(self, &flags, 0.0, 0.0, super::poly::ExtrudeMode::Group);
            // The region's polygons keep their indices: read its vertices back from them.
            let moved: HashSet<u32> = region.iter().flat_map(|p| self.polys[*p].iter().copied()).collect();
            for v in &moved {
                let p = self.verts[*v as usize];
                self.verts[*v as usize] = origin + step * (p - origin);
            }
        }
        // The copies on the hinge never left it: join them back.
        let on_axis: Vec<bool> = self.verts.iter().map(|p| {
            let d = *p - origin;
            (d - axis * d.dot(axis)).length() < 1e-5
        }).collect();
        self.weld(&on_axis, 1e-5);
    }

    /// Cut through polygons with a plane across X, Y or Z at `offset`. With
    /// a mask, only those polygons are cut.
    pub fn slice(&mut self, axis: usize, offset: f32, mask: Option<&[bool]>) {
        let axis = axis.min(2);
        let wanted = |p: usize| mask.map(|m| m.get(p).copied().unwrap_or(false)).unwrap_or(true);
        let side: Vec<i8> = self.verts.iter().map(|v| {
            let d = v[axis] - offset;
            if d.abs() < 1e-6 { 0 } else if d > 0.0 { 1 } else { -1 }
        }).collect();
        // Edges of the wanted polygons that cross the plane get a vertex on it.
        let mut cut: HashMap<[u32; 2], u32> = HashMap::new();
        for p in 0..self.polys.len() {
            if !wanted(p) { continue; }
            let poly = self.polys[p].clone();
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if side[a as usize] as i32 * side[b as usize] as i32 >= 0 { continue; }
                cut.entry(edge_key(a, b)).or_insert_with(|| {
                    let (pa, pb) = (self.verts[a as usize], self.verts[b as usize]);
                    let t = (offset - pa[axis]) / (pb[axis] - pa[axis]);
                    self.verts.push(pa.lerp(pb, t));
                    self.verts.len() as u32 - 1
                });
            }
        }
        let on_plane = |v: u32| v as usize >= side.len() || side[v as usize] == 0;
        let mut out = Vec::with_capacity(self.polys.len());
        for (p, poly) in self.polys.iter().enumerate() {
            let mut lp = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                lp.push(a);
                if let Some(c) = cut.get(&edge_key(a, b)) { lp.push(*c); }
            }
            let hits: Vec<usize> = (0..lp.len()).filter(|i| on_plane(lp[*i])).collect();
            let n = lp.len();
            // Two points on the plane, not already neighbours, with the
            // polygon on both sides of it: split.
            let crosses = lp.iter().any(|v| !on_plane(*v) && side[*v as usize] > 0)
                && lp.iter().any(|v| !on_plane(*v) && side[*v as usize] < 0);
            if wanted(p) && crosses && hits.len() == 2 && hits[1] - hits[0] >= 2 && hits[1] - hits[0] <= n - 2 {
                out.push(lp[hits[0]..=hits[1]].to_vec());
                out.push((hits[1]..=hits[0] + n).map(|i| lp[i % n]).collect());
            } else {
                out.push(lp);
            }
        }
        self.polys = out;
        self.cleanup();
    }

    /// Insert `segments` new vertices along each chosen edge.
    pub fn split_edges(&mut self, edges: &HashSet<[u32; 2]>, segments: u32) {
        let n = segments.clamp(1, 64) as usize;
        let mut on_edge: HashMap<[u32; 2], Vec<u32>> = HashMap::new();
        let mut keys: Vec<[u32; 2]> = edges.iter().copied().collect();
        keys.sort();
        for e in keys {
            let (a, b) = (self.verts[e[0] as usize], self.verts[e[1] as usize]);
            let ids = (1..=n).map(|k| { self.verts.push(a.lerp(b, k as f32 / (n + 1) as f32)); self.verts.len() as u32 - 1 }).collect();
            on_edge.insert(e, ids);
        }
        for poly in &mut self.polys {
            let mut lp = Vec::with_capacity(poly.len() + n);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                lp.push(a);
                if let Some(ids) = on_edge.get(&edge_key(a, b)) {
                    if a < b { lp.extend(ids.iter()); } else { lp.extend(ids.iter().rev()); }
                }
            }
            *poly = lp;
        }
        self.cleanup();
    }

    /// Insert a vertex in the middle of each chosen polygon, joined to its
    /// corners with triangles.
    pub fn poke(&mut self, mask: &[bool]) {
        let mut out = Vec::with_capacity(self.polys.len());
        for p in 0..self.polys.len() {
            let poly = self.polys[p].clone();
            if !mask.get(p).copied().unwrap_or(false) { out.push(poly); continue; }
            self.verts.push(self.centroid(p));
            let c = self.verts.len() as u32 - 1;
            for i in 0..poly.len() { out.push(vec![poly[i], poly[(i + 1) % poly.len()], c]); }
        }
        self.polys = out;
    }

    /// Split the chosen polygons into triangles. A quad is cut along its
    /// shorter diagonal.
    pub fn triangulate(&mut self, mask: &[bool]) {
        let mut out = Vec::with_capacity(self.polys.len());
        for (p, poly) in self.polys.iter().enumerate() {
            if !mask.get(p).copied().unwrap_or(false) || poly.len() <= 3 { out.push(poly.clone()); continue; }
            let pos = |i: usize| self.verts[poly[i] as usize];
            if poly.len() == 4 && pos(1).distance(pos(3)) < pos(0).distance(pos(2)) {
                out.push(vec![poly[0], poly[1], poly[3]]);
                out.push(vec![poly[1], poly[2], poly[3]]);
            } else {
                for i in 1..poly.len() - 1 { out.push(vec![poly[0], poly[i], poly[i + 1]]); }
            }
        }
        self.polys = out;
    }

    /// Turn each chosen edge that lies between two triangles: the diagonal
    /// of the quad they form flips to the other pair of corners.
    pub fn turn_edges(&mut self, edges: &HashSet<[u32; 2]>) {
        let mut keys: Vec<[u32; 2]> = edges.iter().copied().collect();
        keys.sort();
        for e in keys {
            let find = |polys: &Vec<Vec<u32>>, a: u32, b: u32| -> Option<(usize, u32)> {
                polys.iter().enumerate().find_map(|(p, poly)| {
                    if poly.len() != 3 { return None; }
                    (0..3).find(|i| poly[*i] == a && poly[(*i + 1) % 3] == b).map(|i| (p, poly[(i + 2) % 3]))
                })
            };
            let (a, b) = (e[0], e[1]);
            let (Some((p, c)), Some((q, d))) = (find(&self.polys, a, b), find(&self.polys, b, a)) else { continue };
            if p == q || c == d { continue; }
            self.polys[p] = vec![a, d, c];
            self.polys[q] = vec![d, b, c];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::poly::*;

    fn cube() -> PolyMesh { PolyMesh::from_mesh(&crate::node_graph::nodes::create_cube(1.0)) }
    fn grid(n: u32) -> PolyMesh { PolyMesh::from_mesh(&crate::node_graph::nodes::create_grid(n, n, n as f32)) }
    /// Polygon of the cube facing `dir`.
    fn face(m: &PolyMesh, dir: Vec3) -> usize { (0..m.polys.len()).find(|p| m.normal(*p).dot(dir) > 0.9).unwrap() }
    fn only(n: usize, i: usize) -> Vec<bool> { (0..n).map(|k| k == i).collect() }
    fn verts_of(m: &PolyMesh, p: usize) -> Vec<bool> { (0..m.verts.len() as u32).map(|v| m.polys[p].contains(&v)).collect() }
    fn area(m: &PolyMesh) -> f32 { (0..m.polys.len()).map(|p| m.area_normal(p).length() * 0.5).sum() }
    fn valid(m: &PolyMesh) {
        for poly in &m.polys {
            assert!(poly.len() >= 3);
            let set: HashSet<u32> = poly.iter().copied().collect();
            assert_eq!(set.len(), poly.len(), "repeated corner in {poly:?}");
            assert!(poly.iter().all(|v| (*v as usize) < m.verts.len()));
        }
    }
    /// Two unit cubes, the second 3 units above the first.
    fn two_cubes() -> PolyMesh {
        let mut m = cube();
        let n = m.verts.len() as u32;
        let other = cube();
        m.verts.extend(other.verts.iter().map(|v| *v + Vec3::Y * 3.0));
        m.polys.extend(other.polys.iter().map(|p| p.iter().map(|v| v + n).collect()));
        m
    }

    #[test]
    fn transform_moves_turns_and_scales_about_the_centre() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        let sel = verts_of(&m, top);
        m.transform_verts(&sel, Vec3::Y * 0.5, Quat::IDENTITY, Vec3::ONE, 0.0);
        assert!((m.volume() - 1.5).abs() < 1e-5);
        // A quarter turn about Y maps the square top onto itself.
        let before: Vec<Vec3> = m.polys[top].iter().map(|v| m.verts[*v as usize]).collect();
        m.transform_verts(&sel, Vec3::ZERO, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::ONE, 0.0);
        for v in &m.polys[top] { assert!(before.iter().any(|b| b.distance(m.verts[*v as usize]) < 1e-5)); }
        m.transform_verts(&sel, Vec3::ZERO, Quat::IDENTITY, Vec3::new(2.0, 1.0, 2.0), 0.0);
        assert!((m.area_normal(top).length() * 0.5 - 4.0).abs() < 1e-4);
        assert!((m.centroid(top) - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn delete_leaves_a_hole_and_cap_fills_it() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.delete_polys(&only(6, top));
        assert_eq!((m.verts.len(), m.polys.len()), (8, 5));
        assert!(!m.is_closed());
        let loops = m.border_loops();
        assert_eq!((loops.len(), loops[0].len()), (1, 4));
        assert_eq!(m.open_edges().len(), 4);
        m.cap(None);
        assert!(m.is_closed());
        assert!((m.volume() - 1.0).abs() < 1e-5);
        // Deleting a whole element drops its vertices.
        let mut t = two_cubes();
        let mask: Vec<bool> = (0..12).map(|p| p >= 6).collect();
        t.delete_polys(&mask);
        assert_eq!((t.verts.len(), t.polys.len()), (8, 6));
    }

    #[test]
    fn remove_edge_merges_two_polygons() {
        let g = grid(2);
        assert_eq!((g.verts.len(), g.polys.len()), (9, 4));
        // An edge between two cells: used by two polygons.
        let open = g.open_edges();
        let inner: Vec<[u32; 2]> = g.edges().into_iter().filter(|e| !open.contains(e)).collect();
        assert_eq!(inner.len(), 4);
        let one: HashSet<[u32; 2]> = [inner[0]].into_iter().collect();
        let mut a = g.clone();
        a.remove_edges(&one, false);
        valid(&a);
        assert_eq!((a.verts.len(), a.polys.len()), (9, 3));
        assert!((area(&a) - 4.0).abs() < 1e-4);
        // Cleaning also takes the vertex left on the outline with two edges.
        let mut b = g.clone();
        b.remove_edges(&one, true);
        valid(&b);
        assert_eq!((b.verts.len(), b.polys.len()), (8, 3));
        assert!((area(&b) - 4.0).abs() < 1e-4);
        // All four inner edges: one polygon, the centre vertex goes.
        let mut c = g.clone();
        c.remove_edges(&inner.iter().copied().collect(), true);
        valid(&c);
        assert_eq!(c.polys.len(), 1);
        assert!((area(&c) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn remove_vertex_merges_the_polygons_around_it() {
        let mut g = grid(2);
        let centre = (0..9).find(|v| g.verts[*v].length() < 1e-5).unwrap();
        g.remove_verts(&only(9, centre));
        valid(&g);
        assert_eq!((g.verts.len(), g.polys.len()), (8, 1));
        assert!((area(&g) - 4.0).abs() < 1e-4);
        // On a cube: three faces become one.
        let mut c = cube();
        c.remove_verts(&only(8, 0));
        valid(&c);
        assert_eq!((c.verts.len(), c.polys.len()), (7, 4));
        assert!(c.is_closed());
    }

    #[test]
    fn detach_then_weld_gives_the_cube_back() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.detach(&only(6, top));
        assert_eq!(m.verts.len(), 12);
        assert!(!m.is_closed());
        let el = m.elements();
        assert_eq!(el.iter().collect::<HashSet<_>>().len(), 2);
        m.weld(&vec![true; 12], 0.001);
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (8, 6));
        assert!(m.is_closed());
        // Far apart: nothing welds.
        let mut far = cube();
        far.weld(&vec![true; 8], 0.5);
        assert_eq!(far.verts.len(), 8);
    }

    #[test]
    fn collapse_turns_a_face_into_a_point() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.collapse(&verts_of(&m, top));
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (5, 5));
        assert!(m.is_closed());
        assert!((m.volume() - 1.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn rings_loops_and_connect() {
        let c = cube();
        // A vertical edge: its ring is the four vertical edges.
        let up = c.edges().into_iter().find(|e| (c.verts[e[0] as usize] - c.verts[e[1] as usize]).y.abs() > 0.9).unwrap();
        let ring = c.edge_ring(&[up].into_iter().collect());
        assert_eq!(ring.len(), 4);
        assert!(ring.iter().all(|e| (c.verts[e[0] as usize] - c.verts[e[1] as usize]).y.abs() > 0.9));
        for (segments, verts, polys) in [(1, 12, 10), (2, 16, 14), (3, 20, 18)] {
            let mut m = c.clone();
            m.connect(&ring, segments);
            valid(&m);
            assert_eq!((m.verts.len(), m.polys.len()), (verts, polys), "{segments} segments");
            assert!(m.is_closed());
            assert!((m.volume() - 1.0).abs() < 1e-5);
            assert!(m.polys.iter().all(|p| p.len() == 4));
        }
        // The new edges form a loop: from one of them, the loop is all four.
        let mut m = c.clone();
        m.connect(&ring, 1);
        let new: Vec<[u32; 2]> = m.edges().into_iter().filter(|e| e[0] >= 8 && e[1] >= 8).collect();
        assert_eq!(new.len(), 4);
        assert_eq!(m.edge_loop(&[new[0]].into_iter().collect()), new.iter().copied().collect());
        // On a grid a loop runs straight across and stops at the outline.
        let g = grid(3);
        let open = g.open_edges();
        let inner = g.edges().into_iter().find(|e| !open.contains(e)).unwrap();
        let lp = g.edge_loop(&[inner].into_iter().collect());
        assert_eq!(lp.len(), 3);
        let dir = (g.verts[inner[0] as usize] - g.verts[inner[1] as usize]).normalize();
        assert!(lp.iter().all(|e| (g.verts[e[0] as usize] - g.verts[e[1] as usize]).normalize().dot(dir).abs() > 0.99));
    }

    #[test]
    fn connect_two_corners_splits_a_polygon() {
        let mut g = grid(1);
        assert_eq!(g.polys.len(), 1);
        let (a, c) = (g.polys[0][0], g.polys[0][2]);
        let sel: Vec<bool> = (0..4).map(|v| v == a || v == c).collect();
        g.connect_verts(&sel);
        valid(&g);
        assert_eq!(g.polys.len(), 2);
        assert!(g.polys.iter().all(|p| p.len() == 3));
        assert!((area(&g) - 1.0).abs() < 1e-5);
        // Neighbouring corners: nothing to do.
        let mut h = grid(1);
        let sel: Vec<bool> = (0..4).map(|v| v == h.polys[0][0] || v == h.polys[0][1]).collect();
        h.connect_verts(&sel);
        assert_eq!(h.polys.len(), 1);
    }

    #[test]
    fn bridge_joins_two_cubes() {
        let mut m = two_cubes();
        let mask: Vec<bool> = (0..12).map(|p| {
            let n = m.normal(p);
            (p < 6 && n.y > 0.9) || (p >= 6 && n.y < -0.9)
        }).collect();
        assert!(m.bridge_polys(&mask));
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (16, 14));
        assert!(m.is_closed());
        assert!((m.volume() - 4.0).abs() < 1e-4, "{}", m.volume());
        // Same through borders: delete the two faces, then bridge the holes.
        let mut b = two_cubes();
        b.delete_polys(&mask);
        assert_eq!(b.border_loops().len(), 2);
        assert!(b.bridge_borders(None));
        assert!(b.is_closed());
        assert!((b.volume() - 4.0).abs() < 1e-4);
        // One opening only: refused, nothing changes.
        let mut c = cube();
        let before = c.clone();
        assert!(!c.bridge_polys(&only(6, 0)));
        assert_eq!(c, before);
    }

    #[test]
    fn flip_break_and_planar() {
        let mut m = cube();
        m.flip(&vec![true; 6]);
        assert!((m.volume() + 1.0).abs() < 1e-5);

        let mut b = cube();
        b.break_verts(&only(8, 0));
        assert_eq!(b.verts.len(), 10);
        assert!(!b.is_closed());

        // Tilt the top, then flatten it across Y.
        let mut p = cube();
        let top = face(&p, Vec3::Y);
        let v = p.polys[top][0] as usize;
        p.verts[v].y += 0.4;
        let sel = verts_of(&p, top);
        let mut flat = p.clone();
        flat.make_planar(&sel, Some(1));
        let ys: Vec<f32> = flat.polys[top].iter().map(|v| flat.verts[*v as usize].y).collect();
        assert!(ys.iter().all(|y| (y - ys[0]).abs() < 1e-5));
        assert!((ys[0] - 0.6).abs() < 1e-5);
        // Best fit: the four corners end up in one plane.
        p.make_planar(&sel, None);
        let q: Vec<Vec3> = p.polys[top].iter().map(|v| p.verts[*v as usize]).collect();
        assert!((q[1] - q[0]).cross(q[2] - q[0]).normalize().dot(q[3] - q[0]).abs() < 1e-4);
    }

    #[test]
    fn relax_smooths_a_spike() {
        let mut g = grid(4);
        let centre = (0..g.verts.len()).find(|v| g.verts[*v].length() < 1e-5).unwrap();
        g.verts[centre].y = 1.0;
        let outline = g.verts.clone();
        let n = g.verts.len();
        g.relax(&vec![true; n], 0.5, 1, true);
        assert!((g.verts[centre].y - 0.5).abs() < 1e-5);
        for lp in g.border_loops() { for v in lp { assert_eq!(g.verts[v as usize], outline[v as usize]); } }
    }

    #[test]
    fn tessellate_and_subdivide() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.tessellate(&only(6, top));
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (13, 9));
        assert!(m.is_closed());
        assert!((m.volume() - 1.0).abs() < 1e-5);

        let mut s = cube();
        s.subdivide();
        valid(&s);
        assert_eq!((s.verts.len(), s.polys.len()), (26, 24));
        assert!(s.is_closed());
        assert!(s.volume() > 0.3 && s.volume() < 1.0, "{}", s.volume());
        s.subdivide();
        assert_eq!(s.polys.len(), 96);
        assert!(s.is_closed());

        let mut g = grid(2);
        g.subdivide();
        valid(&g);
        assert_eq!((g.verts.len(), g.polys.len()), (25, 16));
        assert!(g.verts.iter().all(|v| v.y.abs() < 1e-6));
    }

    #[test]
    fn chamfer_vertex_cuts_a_corner() {
        let mut m = cube();
        m.chamfer_verts(&only(8, 0), 0.25, None);
        valid(&m);
        // One corner: three new points, a triangle, three faces gain a corner.
        assert_eq!((m.verts.len(), m.polys.len()), (10, 7));
        assert!(m.is_closed());
        // The corner taken off is a tetrahedron with three legs of 0.25.
        assert!((m.volume() - (1.0 - 0.25f32.powi(3) / 6.0)).abs() < 1e-5, "{}", m.volume());
        // All eight corners.
        let mut all = cube();
        all.chamfer_verts(&vec![true; 8], 0.2, None);
        valid(&all);
        assert_eq!((all.verts.len(), all.polys.len()), (24, 14));
        assert!(all.is_closed());
        assert!((all.volume() - (1.0 - 8.0 * 0.2f32.powi(3) / 6.0)).abs() < 1e-5);
        // Vertex extrude: a spike on the corner.
        let mut spike = cube();
        spike.chamfer_verts(&only(8, 0), 0.25, Some(0.5));
        valid(&spike);
        assert_eq!((spike.verts.len(), spike.polys.len()), (11, 9));
        assert!(spike.is_closed());
        assert!(spike.volume() > 1.0);
    }

    #[test]
    fn chamfer_edge_makes_a_strip() {
        let c = cube();
        // One edge of the top face.
        let top = face(&c, Vec3::Y);
        let e = edge_key(c.polys[top][0], c.polys[top][1]);
        let mut m = c.clone();
        m.chamfer_edges(&[e].into_iter().collect(), 0.2);
        valid(&m);
        assert!(m.is_closed());
        // A prism with a right triangle of legs 0.2 comes off, along the whole edge.
        assert!((m.volume() - (1.0 - 0.2 * 0.2 / 2.0)).abs() < 1e-5, "{}", m.volume());
        assert_eq!((m.verts.len(), m.polys.len()), (10, 7));
        // All four edges of the top: a bevelled lid.
        let ring: HashSet<[u32; 2]> = (0..4).map(|i| edge_key(c.polys[top][i], c.polys[top][(i + 1) % 4])).collect();
        let mut lid = c.clone();
        lid.chamfer_edges(&ring, 0.1);
        valid(&lid);
        assert!(lid.is_closed());
        assert!(lid.volume() < 1.0 && lid.volume() > 0.95, "{}", lid.volume());
        // Every edge of the cube.
        let mut all = c.clone();
        all.chamfer_edges(&c.edges().into_iter().collect(), 0.1);
        valid(&all);
        assert!(all.is_closed());
        // 6 faces, 12 strips, 8 corner triangles.
        assert_eq!(all.polys.len(), 26);
        assert!(all.volume() < 1.0 && all.volume() > 0.9);
        for p in 0..all.polys.len() { assert!(all.normal(p).dot(all.centroid(p)) > 0.0, "polygon {p} faces in"); }
        // An edge loop on a grid: one strip per edge, still one flat sheet.
        let g = grid(3);
        let open = g.open_edges();
        let inner = g.edges().into_iter().find(|e| !open.contains(e)).unwrap();
        let lp = g.edge_loop(&[inner].into_iter().collect());
        let mut cut = g.clone();
        cut.chamfer_edges(&lp, 0.1);
        valid(&cut);
        assert!((area(&cut) - 9.0).abs() < 1e-4);
        assert_eq!(cut.polys.len(), 9 + 3);
    }

    #[test]
    fn border_edges_extrude_outwards() {
        let g = grid(1);
        let border: HashSet<[u32; 2]> = g.open_edges();
        let mut m = g.clone();
        m.extrude_border_edges(&border, 0.0, 0.5);
        valid(&m);
        // A 1 x 1 square grows a 0.5 rim all round.
        assert_eq!((m.verts.len(), m.polys.len()), (8, 5));
        assert!((area(&m) - 4.0).abs() < 1e-4, "{}", area(&m));
        for p in 0..m.polys.len() { assert!(m.normal(p).y > 0.99); }
        // Upwards: walls.
        let mut w = g.clone();
        w.extrude_border_edges(&border, 0.5, 0.0);
        assert!((area(&w) - 3.0).abs() < 1e-4);
        // Edges that are not open are left alone.
        let mut c = cube();
        c.extrude_border_edges(&cube().edges().into_iter().collect(), 1.0, 1.0);
        assert_eq!(c, cube());
    }

    #[test]
    fn outline_grows_a_face_in_its_plane() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.outline(&only(6, top), 0.25);
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (8, 6));
        assert!((m.area_normal(top).length() * 0.5 - 2.25).abs() < 1e-4);
        assert!((m.centroid(top).y - 0.5).abs() < 1e-6);
        assert!(m.is_closed());
        m.outline(&only(6, top), -0.25);
        assert!((m.volume() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn hinge_swings_a_face_open() {
        let mut m = cube();
        let top = face(&m, Vec3::Y);
        m.hinge(&only(6, top), 90.0, 3, 0);
        valid(&m);
        assert!(m.is_closed());
        // A quarter cylinder of radius 1 and length 1 is added, as three wedges.
        let wedge = 3.0 * 0.5 * (std::f32::consts::FRAC_PI_2 / 3.0).sin();
        assert!((m.volume() - (1.0 + wedge)).abs() < 1e-4, "{} vs {}", m.volume(), 1.0 + wedge);
        // The face now stands upright.
        assert!(m.normal(top).y.abs() < 1e-4);
        assert!((m.area_normal(top).length() * 0.5 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn slice_cuts_across() {
        let mut m = cube();
        m.slice(1, 0.1, None);
        valid(&m);
        // Four side faces split in two; top and bottom untouched.
        assert_eq!((m.verts.len(), m.polys.len()), (12, 10));
        assert!(m.is_closed());
        assert!((m.volume() - 1.0).abs() < 1e-5);
        assert_eq!(m.verts.iter().filter(|v| (v.y - 0.1).abs() < 1e-6).count(), 4);
        // Through existing vertices: nothing to cut.
        let mut flat = cube();
        flat.slice(1, 0.5, None);
        assert_eq!(flat, cube());
        // Only the chosen polygons: their neighbours gain a vertex but stay whole.
        let mut one = cube();
        let side = face(&one, Vec3::X);
        one.slice(1, 0.0, Some(&only(6, side)));
        valid(&one);
        assert_eq!(one.polys.len(), 7);
        assert!(one.is_closed());
        // A grid cut twice.
        let mut g = grid(2);
        g.slice(0, 0.5, None);
        g.slice(2, -0.25, None);
        valid(&g);
        assert_eq!(g.polys.len(), 9);
        assert!((area(&g) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn insert_poke_triangulate_and_turn() {
        let mut m = cube();
        let e = m.edges()[0];
        m.split_edges(&[e].into_iter().collect(), 2);
        valid(&m);
        assert_eq!((m.verts.len(), m.polys.len()), (10, 6));
        assert_eq!(m.polys.iter().filter(|p| p.len() == 6).count(), 2);
        assert!(m.is_closed() && (m.volume() - 1.0).abs() < 1e-5);

        let mut p = cube();
        p.poke(&only(6, 0));
        valid(&p);
        assert_eq!((p.verts.len(), p.polys.len()), (9, 9));
        assert!(p.is_closed() && (p.volume() - 1.0).abs() < 1e-5);

        let mut t = cube();
        t.triangulate(&vec![true; 6]);
        valid(&t);
        assert_eq!(t.polys.len(), 12);
        assert!(t.polys.iter().all(|q| q.len() == 3));
        assert!(t.is_closed() && (t.volume() - 1.0).abs() < 1e-5);

        // Turn the diagonal of one face: same surface, other diagonal.
        let diagonal = t.edges().into_iter().find(|e| !cube().edges().contains(e)).unwrap();
        let mut turned = t.clone();
        turned.turn_edges(&[diagonal].into_iter().collect());
        valid(&turned);
        assert!(turned.is_closed() && (turned.volume() - 1.0).abs() < 1e-5);
        assert!(!turned.edges().contains(&diagonal));
        assert_eq!(turned.edges().len(), t.edges().len());
        // An edge between quads cannot be turned.
        let mut q = cube();
        q.turn_edges(&[e].into_iter().collect());
        assert_eq!(q, cube());
    }

    #[test]
    fn soft_selection_drags_the_neighbours() {
        let mut g = grid(4);
        let centre = (0..g.verts.len()).find(|v| g.verts[*v].length() < 1e-5).unwrap();
        let sel = only(g.verts.len(), centre);
        let mut hard = g.clone();
        hard.transform_verts(&sel, Vec3::Y, Quat::IDENTITY, Vec3::ONE, 0.0);
        assert_eq!(hard.verts.iter().filter(|v| v.y > 0.0).count(), 1);
        g.transform_verts(&sel, Vec3::Y, Quat::IDENTITY, Vec3::ONE, 1.5);
        assert!((g.verts[centre].y - 1.0).abs() < 1e-6);
        // One cell away: part of the way. Two cells away: beyond the falloff.
        let near = (0..g.verts.len()).find(|v| (g.verts[*v].x - 1.0).abs() < 1e-5 && g.verts[*v].z.abs() < 1e-5).unwrap();
        let far = (0..g.verts.len()).find(|v| (g.verts[*v].x - 2.0).abs() < 1e-5 && g.verts[*v].z.abs() < 1e-5).unwrap();
        assert!(g.verts[near].y > 0.1 && g.verts[near].y < 0.9, "{}", g.verts[near].y);
        assert_eq!(g.verts[far].y, 0.0);
    }

    #[test]
    fn every_operation_runs_through_the_node() {
        // Open the top, select its border, cap it, then keep going.
        let top = PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 5.0 }, ..Default::default() };
        let border = PolySelection { level: SubLevel::Border, source: SelSource::All, ..Default::default() };
        let all = PolySelection { source: SelSource::All, ..Default::default() };
        let ops = vec![
            PolyOp::new(top.clone(), PolyOpKind::Delete),
            PolyOp::new(border.clone(), PolyOpKind::Transform { translate: [0.0, 0.5, 0.0], rotate: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], falloff: 0.0 }),
            PolyOp::new(border.clone(), PolyOpKind::Cap),
            PolyOp::new(top.clone(), PolyOpKind::Inset { amount: 0.1, by_polygon: false }),
            PolyOp::new(top.clone(), PolyOpKind::Tessellate),
            PolyOp::new(all.clone(), PolyOpKind::Relax { amount: 0.0, iterations: 2, hold_border: true }),
            PolyOp::new(all.clone(), PolyOpKind::Subdivide { iterations: 1 }),
        ];
        let m = apply_ops(&cube(), &ops, 3);
        assert!(m.is_closed());
        assert!((m.volume() - 1.5).abs() < 1e-5);
        let end = apply_ops(&cube(), &ops, ops.len());
        valid(&end);
        assert!(end.is_closed());
        assert!(end.polys.iter().all(|p| p.len() == 4));
        // Border level on a closed mesh selects nothing.
        assert_eq!(border.count(&cube()), 0);
        assert_eq!(border.count(&apply_ops(&cube(), &ops, 1)), 4);
        // Element level: one polygon stands for its whole element.
        let el = PolySelection { level: SubLevel::Element, polys: vec![7], ..Default::default() };
        let t = two_cubes();
        assert_eq!(el.poly_mask(&t), (0..12).map(|p| p >= 6).collect::<Vec<_>>());
        assert_eq!(widen_pick(&t, SubLevel::Element, vec![Component::Polygon(2)]).len(), 6);
        let open = apply_ops(&cube(), &ops, 1);
        let e = *open.open_edges().iter().next().unwrap();
        assert_eq!(widen_pick(&open, SubLevel::Border, vec![Component::Edge(e)]).len(), 4);
        // Selections of other levels reach the right components.
        assert_eq!(top.vertex_set(&cube()).iter().filter(|s| **s).count(), 4);
        assert_eq!(top.edge_set(&cube()).len(), 4);
        assert!((top.centre(&cube()).unwrap() - Vec3::new(0.0, 0.5, 0.0)).length() < 1e-5);
    }

    #[test]
    fn collapsing_keeps_the_result() {
        let top = PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 5.0 }, ..Default::default() };
        let ext = |h: f32| PolyOp::new(top.clone(), PolyOpKind::Extrude { height: h, mode: ExtrudeMode::Group });
        let mut ops = vec![];
        push_op(&mut ops, false, ext(0.5));
        push_op(&mut ops, false, PolyOp { enabled: false, ..ext(9.0) });
        push_op(&mut ops, false, ext(0.25));
        assert!(ops.iter().all(|op| !op.collapsed));
        let want = apply_ops(&cube(), &ops, ops.len());
        assert!((want.volume() - 1.75).abs() < 1e-5);

        // One at a time: frozen, still applied.
        collapse_op(&mut ops, 0);
        assert!(ops[0].collapsed && ops.len() == 3);
        assert_eq!(apply_ops(&cube(), &ops, ops.len()), want);
        assert_eq!(*eval_cached(&cube(), &ops, ops.len()), want);
        // A disabled operation does nothing, so collapsing drops it.
        collapse_op(&mut ops, 1);
        assert_eq!(ops.len(), 2);
        collapse_all(&mut ops);
        assert!(ops.iter().all(|op| op.collapsed));
        assert_eq!(*eval_cached(&cube(), &ops, ops.len()), want);
        // Live operation after collapsed ones, edited: the cache follows.
        push_op(&mut ops, false, ext(1.0));
        assert!((eval_cached(&cube(), &ops, ops.len()).volume() - 2.75).abs() < 1e-5);
        if let PolyOpKind::Extrude { height, .. } = &mut ops[2].kind { *height = 2.0; }
        assert!((eval_cached(&cube(), &ops, ops.len()).volume() - 3.75).abs() < 1e-5);
        assert!((eval_cached(&cube(), &ops, 2).volume() - 1.75).abs() < 1e-5);
        // A different input does not hit the same entry.
        let mut big = cube();
        for v in &mut big.verts { *v *= 2.0; }
        assert!((eval_cached(&big, &ops, 2).volume() - 8.0 - 4.0 * 0.75).abs() < 1e-4);

        restore_run(&mut ops, 0);
        assert!(ops.iter().all(|op| !op.collapsed));

        // Auto-collapse: adding one freezes everything before it.
        let mut auto = vec![];
        push_op(&mut auto, true, ext(0.5));
        push_op(&mut auto, true, ext(0.5));
        push_op(&mut auto, true, ext(0.5));
        assert_eq!(auto.iter().map(|op| op.collapsed).collect::<Vec<_>>(), vec![true, true, false]);
    }

    #[test]
    fn graphs_saved_before_collapsing_still_load() {
        let json = r#"{"enabled":true,"selection":{"level":"Polygon","source":"Picked","verts":[],"edges":[],"polys":[3],"grow":0,"invert":false},"kind":{"Inset":{"amount":0.1,"by_polygon":false}}}"#;
        let op: PolyOp = serde_json::from_str(json).unwrap();
        assert!(!op.collapsed && op.enabled);
    }
}
