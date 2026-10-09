//! Native instancing, PointInstancer and prototypes (stored once, referenced by instances).

use crate::*;

impl<'s> Ctx<'s> {
    pub(crate) fn prototype_data(&mut self, path: &str, include_root: bool) -> Arc<ProtoData> {
        let key = (path.to_string(), include_root);
        if let Some(d) = self.proto_cache.get(&key) {
            return d.clone();
        }
        // The prototype walk resets the current prim's motion (prototypes don't move by
        // themselves); keep the caller's (a moving instance or instancer) for its emission
        let saved_motion = self.cur_motion.take();
        // Build the prototype in its own local space and its own node arena
        let saved_tree = std::mem::replace(
            &mut self.tree,
            vec![UsdNode {
                name: String::new(),
                path: String::new(),
                type_name: String::new(),
                parent: None,
                children: Vec::new(),
                flags: NodeFlags::default(),
            }],
        );
        let saved_cur = self.cur_node;
        let mut emit = Emit::default();
        if let Some(sc) = self.scene {
            // Through the scene layer: the prototype's subtree is in the translated scene
            // A path inside a native instance (an instance proxy, e.g. a PointInstancer
            // prototype under an instanceable asset) lives in the scene layer under the
            // instance's prototype: map it there, as USD does
            if let Some(p) = xms_scene::Path::new(path).ok().map(|p| proxy_to_prototype(sc, p)) {
                if !sc.contains(&p) {
                    // Diagnose: what the stage has there, and the nearest ancestor the
                    // scene layer does hold (an Instance = the path is inside an
                    // instance proxy; a group = under prims the walk never reached)
                    self.missing_protos += 1;
                    if self.missing_protos <= 5 {
                        let on_stage = match self.stage.prim(path) {
                            Ok(sp) => format!("on the stage as `{}`", type_of(&sp)),
                            Err(_) => "not on the stage either".to_string(),
                        };
                        let mut anc = p.parent();
                        while let Some(a) = &anc {
                            if sc.contains(a) {
                                break;
                            }
                            anc = a.parent();
                        }
                        let nearest = anc.as_ref().and_then(|a| sc.get(a)).map_or("none".to_string(), |ap| {
                            let kind = match &ap.kind {
                                xms_scene::PrimKind::Instance { prototype } => format!("Instance of {prototype}"),
                                xms_scene::PrimKind::Group => format!("group (`{}`)", ap.type_name),
                                _ => format!("`{}`", ap.type_name),
                            };
                            format!("{} ({kind})", ap.path)
                        });
                        // What the instance-proxy mapping produced, and what the nearest
                        // ancestor actually holds (to compare names)
                        let mapped = if p.as_str() == path { "unmapped".to_string() } else { format!("mapped to {p}") };
                        let held: Vec<String> = anc
                            .as_ref()
                            .map(|a| {
                                let target = match sc.get(a).map(|ap| &ap.kind) {
                                    Some(xms_scene::PrimKind::Instance { prototype }) => prototype.clone(),
                                    _ => a.clone(),
                                };
                                sc.children(&target).iter().take(4).map(|c| c.as_str().to_string()).collect()
                            })
                            .unwrap_or_default();
                        self.warnings.push(format!(
                            "Scene layer lacks prototype {path} ({mapped}): {on_stage}; nearest ancestor in the scene layer: {nearest}; \
                             it holds {}",
                            if held.is_empty() { "no children".to_string() } else { held.join(", ") }
                        ));
                    }
                }
                self.proto_depth += 1;
                let id = M4::identity();
                if include_root {
                    self.walk_scene(sc, &p, &id, &Inherit::default(), 0, &mut emit);
                } else {
                    for c in sc.children(&p) {
                        self.walk_scene(sc, c, &id, &Inherit::default(), 0, &mut emit);
                    }
                }
                self.proto_depth -= 1;
            }
        }
        let nodes = std::mem::replace(&mut self.tree, saved_tree);
        self.cur_motion = saved_motion;
        self.cur_node = saved_cur;
        // Geometry is stored once per prototype; instances only reference it
        let id = self.proto_geoms.len() as u32;
        let own = Binding {
            keys: emit.batches.iter().map(|b| b.key.clone()).collect::<Vec<_>>().into(),
            nodes: emit.batches.iter().map(|b| b.node).collect(),
        };
        self.proto_geoms.push(emit.batches.into_iter().map(|b| b.part).collect());
        self.proto_names.push(path.to_string());
        let data = Arc::new(ProtoData { id, nodes, own, nested: emit.instances });
        self.proto_cache.insert(key, data.clone());
        data
    }

    /// Copy a prototype's local hierarchy under `under`, rewriting paths into
    /// the instance's namespace. Returns local node index -> tree node index.
    pub(crate) fn graft(&mut self, data: &ProtoData, under: usize, extra: NodeFlags) -> Vec<usize> {
        let mut map = vec![under; data.nodes.len()];
        if self.tree.len() + data.nodes.len() > MAX_NODES {
            self.nodes_capped = true;
            return map;
        }
        for (i, n) in data.nodes.iter().enumerate().skip(1) {
            let parent = map[n.parent.unwrap_or(0)];
            let path = format!("{}/{}", self.tree[parent].path.trim_end_matches('/'), n.name);
            let mut flags = n.flags;
            flags.instance_proxy |= extra.instance_proxy;
            flags.prototype |= extra.prototype;
            map[i] = self.push_node(parent, n.name.clone(), path, n.type_name.clone(), flags);
        }
        map
    }

    /// Resolve a binding into a new context: inherited materials fill `None`
    /// keys, and local nodes map through `map` (or all to `node`).
    pub(crate) fn remap_binding(&mut self, src: &Binding, inh: &Inherit, map: Option<&[usize]>, node: usize) -> u32 {
        // Keys: shared, not copied. A prototype that names all its materials keeps its
        // own list; otherwise the list for (this prototype, this inherited material)
        // is built once and reused by every instance with the same combination
        let keys = if src.keys.iter().all(Option::is_some) {
            src.keys.clone()
        } else {
            let id = Arc::as_ptr(&src.keys) as *const () as usize;
            let mat = inh.material.as_deref();
            let cached = self.remap_cache.get(&id).and_then(|v| v.iter().find(|(m, _)| m.as_deref() == mat)).map(|(_, k)| k.clone());
            match cached {
                Some(k) => k,
                None => {
                    let inherited = inh.material.clone().map(|p| self.material_key(&p));
                    let k: Arc<[Option<String>]> =
                        src.keys.iter().map(|k| k.clone().or_else(|| inherited.clone())).collect::<Vec<_>>().into();
                    self.remap_cache.entry(id).or_default().push((inh.material.clone(), k.clone()));
                    k
                }
            }
        };
        let nodes = src.nodes.iter().map(|&n| map.map_or(node, |mp| mp.get(n).copied().unwrap_or(node))).collect();
        self.bindings.push(Binding { keys, nodes });
        (self.bindings.len() - 1) as u32
    }

    /// Place a prototype with transform `m`: one instance reference for its own
    /// geometry, plus its nested instances with composed transforms (so the
    /// renderer's top level stays a single level). No geometry is copied.
    pub(crate) fn emit_instance(&mut self, data: &ProtoData, m: &M4, inh: &Inherit, map: Option<&[usize]>, node: usize, out: &mut Emit) {
        if !data.own.keys.is_empty() {
            let binding = self.remap_binding(&data.own, inh, map, node);
            out.instances.push(InstRef { proto: data.id, binding, xf: to_rows(m) });
            if let Some(k) = &self.cur_motion {
                out.inst_motion.push((out.instances.len() - 1, k.clone()));
            }
        }
        for ni in &data.nested {
            let src = self.bindings[ni.binding as usize].clone();
            let binding = self.remap_binding(&src, inh, map, node);
            let local = from_rows(&ni.xf);
            out.instances.push(InstRef { proto: ni.proto, binding, xf: to_rows(&(m * local)) });
            if let Some(k) = &self.cur_motion {
                out.inst_motion.push((out.instances.len() - 1, Arc::new(compose_keys(k, &local))));
            }
        }
    }
    /// Expand a PointInstancer into instances (shared by both import paths).
    pub(crate) fn emit_point_instancer(&mut self, input: InstancerInput, world: &M4, inh: &Inherit, node: usize, out: &mut Emit) {
        let InstancerInput { protos, idx, positions, scales, orients, invisible } = input;
        if invisible {
            self.warnings.push("PointInstancer invisibleIds are ignored".into());
        }

        let proto_data: Vec<Arc<ProtoData>> = protos.iter().map(|p| self.prototype_data(p, true)).collect();
        // Show the prototypes in the hierarchy (their geometry is attributed to the instancer)
        for d in &proto_data {
            self.graft(d, node, NodeFlags { prototype: true, ..NodeFlags::default() });
        }
        // Bindings are the same for every point of a prototype: resolve them once
        let mut plans: Vec<Vec<(u32, u32, Option<[f32; 12]>)>> = Vec::with_capacity(proto_data.len());
        for d in &proto_data {
            let mut plan = Vec::new();
            if !d.own.keys.is_empty() {
                plan.push((d.id, self.remap_binding(&d.own, inh, None, node), None));
            }
            for ni in &d.nested {
                let src = self.bindings[ni.binding as usize].clone();
                plan.push((ni.proto, self.remap_binding(&src, inh, None, node), Some(ni.xf)));
            }
            plans.push(plan);
        }
        for (i, &pi) in idx.iter().enumerate() {
            let Some(plan) = plans.get(pi as usize) else { continue };
            let t = positions.get(i).map_or(M4::identity(), |p| {
                M4::new_translation(&Vector3::new(p[0] as f64, p[1] as f64, p[2] as f64))
            });
            let r = orients.get(i).copied().unwrap_or_else(M4::identity);
            let s = scales.get(i).map_or(M4::identity(), |s| {
                M4::new_nonuniform_scaling(&Vector3::new(s[0] as f64, s[1] as f64, s[2] as f64))
            });
            // USD applies scale, then orientation, then translation
            let m = world * t * r * s;
            for (proto, binding, local) in plan {
                // (computed exactly as before motion blur: static renders stay bit-identical)
                let xf = match local {
                    Some(l) => m * from_rows(l),
                    None => m,
                };
                out.instances.push(InstRef { proto: *proto, binding: *binding, xf: to_rows(&xf) });
                // A moving instancer moves every point with it
                if let Some(k) = &self.cur_motion {
                    let point = match local {
                        Some(l) => t * r * s * from_rows(l),
                        None => t * r * s,
                    };
                    out.inst_motion.push((out.instances.len() - 1, Arc::new(compose_keys(k, &point))));
                }
            }
        }
    }
}

/// A PointInstancer as read from USD or the scene layer, before
/// [`Ctx::emit_point_instancer`].
pub(crate) struct InstancerInput {
    pub protos: Vec<String>,
    pub idx: Vec<i64>,
    pub positions: Vec<[f32; 3]>,
    pub scales: Vec<[f32; 3]>,
    /// Orientations as rotation matrices
    pub orients: Vec<M4>,
    pub invisible: bool,
}

/// Map an instance-proxy path to where the scene layer holds that prim: the nearest
/// native-instance ancestor's prefix is replaced by its prototype path, repeatedly
/// (the remainder may cross further instances inside the prototype). Paths the
/// scene layer already holds, or that aren't under an instance, come back unchanged.
pub(crate) fn proxy_to_prototype(sc: &xms_scene::Scene, path: xms_scene::Path) -> xms_scene::Path {
    let mut cur = path.clone();
    for _ in 0..16 {
        if sc.contains(&cur) {
            return cur;
        }
        let mut anc = cur.parent();
        while let Some(a) = &anc {
            if sc.contains(a) {
                break;
            }
            anc = a.parent();
        }
        let Some(a) = anc else { return path };
        let Some(xms_scene::PrimKind::Instance { prototype }) = sc.get(&a).map(|p| &p.kind) else { return path };
        let rest = &cur.as_str()[a.as_str().len()..];
        match xms_scene::Path::new(&format!("{}{rest}", prototype.as_str())) {
            Ok(next) => cur = next,
            Err(_) => return path,
        }
    }
    path
}

/// World motion keys followed by a fixed local transform (an instance inside a moving
/// prim): each key times `local`.
fn compose_keys(k: &MotionKeys, local: &M4) -> MotionKeys {
    MotionKeys { times: k.times.clone(), xforms: k.xforms.iter().map(|x| to_rows(&(from_rows(x) * local))).collect() }
}

