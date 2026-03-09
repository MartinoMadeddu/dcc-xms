/// USD mesh loader.
///
/// - .usda : parsed with a hand-written recursive-descent parser (no external deps)
/// - .usdc : loaded via the `openusd` crate's usdc reader
/// - .usdz : treated as a zip; the first .usda/.usdc inside is extracted and loaded
///
/// Extracts all Mesh prims, triangulates quads/ngons via fan triangulation,
/// and accumulates xformOp transforms down the prim hierarchy so meshes
/// appear at correct world-space positions.

use std::path::Path;
use bevy::math::{Mat4, Vec3, Vec4, Quat};
use crate::types::MeshData;

// ── Public result type ────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UsdError {
    Io(std::io::Error),
    Parse(String),
}
impl std::fmt::Display for UsdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsdError::Io(e)    => write!(f, "IO error: {e}"),
            UsdError::Parse(s) => write!(f, "Parse error: {s}"),
        }
    }
}
impl From<std::io::Error> for UsdError { fn from(e: std::io::Error) -> Self { Self::Io(e) } }

pub type UsdResult<T> = Result<T, UsdError>;

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn load_usd_meshes(path: &Path) -> UsdResult<Vec<(String, MeshData)>> {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "usda" => { let src = std::fs::read_to_string(path)?; parse_usda_meshes(&src) }
        "usdc" => load_usdc_meshes(path),
        "usdz" => load_usdz_meshes(path),
        other  => Err(UsdError::Parse(format!("Unsupported extension: .{other}"))),
    }
}

// ============================================================================
// TRANSFORM HELPERS
// ============================================================================

/// Decompose a flat 16-element row-major matrix from USD into a bevy Mat4.
/// USD stores matrices row-major, bevy uses column-major, so we transpose.
fn mat4_from_usd_flat(v: &[f64]) -> Mat4 {
    if v.len() < 16 { return Mat4::IDENTITY; }
    // USD row-major → transpose to get column-major for bevy
    Mat4::from_cols(
        Vec4::new(v[0]  as f32, v[4]  as f32, v[8]  as f32, v[12] as f32),
        Vec4::new(v[1]  as f32, v[5]  as f32, v[9]  as f32, v[13] as f32),
        Vec4::new(v[2]  as f32, v[6]  as f32, v[10] as f32, v[14] as f32),
        Vec4::new(v[3]  as f32, v[7]  as f32, v[11] as f32, v[15] as f32),
    )
}

fn mat4_from_translate(t: [f64; 3]) -> Mat4 {
    Mat4::from_translation(Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32))
}

fn mat4_from_orient(q: [f64; 4]) -> Mat4 {
    // USD quaternion order: (real, i, j, k) → bevy Quat (x,y,z,w)
    Mat4::from_quat(Quat::from_xyzw(q[1] as f32, q[2] as f32, q[3] as f32, q[0] as f32))
}

fn mat4_from_scale(s: [f64; 3]) -> Mat4 {
    Mat4::from_scale(Vec3::new(s[0] as f32, s[1] as f32, s[2] as f32))
}

fn mat4_from_rotate_xyz_degrees(r: [f64; 3]) -> Mat4 {
    let rx = Mat4::from_rotation_x(r[0].to_radians() as f32);
    let ry = Mat4::from_rotation_y(r[1].to_radians() as f32);
    let rz = Mat4::from_rotation_z(r[2].to_radians() as f32);
    rz * ry * rx
}

/// Apply a world-space Mat4 to all vertices of a MeshData in place.
fn apply_transform(mesh: &mut MeshData, xform: &Mat4) {
    if *xform == Mat4::IDENTITY { return; }
    for v in mesh.positions_mut() {
        *v = xform.transform_point3(*v);
    }
}

// ============================================================================
// USDA TEXT PARSER
// ============================================================================

pub fn parse_usda_meshes(src: &str) -> UsdResult<Vec<(String, MeshData)>> {
    let mut results = Vec::new();
    let tokens = tokenize(src);
    let mut pos = 0;
    parse_block(&tokens, &mut pos, "/", Mat4::IDENTITY, &mut results);
    Ok(results)
}

// ── Tokeniser ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Eq,
    Num(f64),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            '#' => { while chars.next().map(|c| c != '\n').unwrap_or(false) {} }
            c if c.is_whitespace() => { chars.next(); }
            '{' => { chars.next(); out.push(Tok::LBrace); }
            '}' => { chars.next(); out.push(Tok::RBrace); }
            '[' => { chars.next(); out.push(Tok::LBracket); }
            ']' => { chars.next(); out.push(Tok::RBracket); }
            '(' => { chars.next(); out.push(Tok::LParen); }
            ')' => { chars.next(); out.push(Tok::RParen); }
            ',' => { chars.next(); out.push(Tok::Comma); }
            '=' => { chars.next(); out.push(Tok::Eq); }
            '"' => {
                chars.next();
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '"' { break; }
                    s.push(c);
                }
                out.push(Tok::Str(s));
            }
            c if c.is_ascii_digit() || c == '-' => {
                let mut s = String::new();
                if c == '-' {
                    chars.next();
                    if chars.peek().map(|c| c.is_ascii_digit() || *c == '.').unwrap_or(false) {
                        s.push('-');
                    } else {
                        out.push(Tok::Word("-".into()));
                        continue;
                    }
                }
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E'
                        || c == '+' || (c == '-' && s.contains('e'))
                    { s.push(c); chars.next(); } else { break; }
                }
                if let Ok(n) = s.parse::<f64>() { out.push(Tok::Num(n)); }
                else { out.push(Tok::Word(s)); }
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || "{}[](),=\"#".contains(c) { break; }
                    s.push(c); chars.next();
                }
                if !s.is_empty() { out.push(Tok::Word(s)); }
            }
        }
    }
    out
}

// ── Recursive block parser ────────────────────────────────────────────────────

fn parse_block(
    toks:         &[Tok],
    pos:          &mut usize,
    current_path: &str,
    parent_xform: Mat4,
    results:      &mut Vec<(String, MeshData)>,
) {
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::RBrace => { *pos += 1; return; }
            Tok::Word(w) if w == "def" || w == "over" || w == "class" => {
                *pos += 1;
                let type_name = if let Some(Tok::Word(t)) = toks.get(*pos) {
                    if t != "\"" { let t = t.clone(); *pos += 1; t } else { String::new() }
                } else { String::new() };
                let prim_name = match toks.get(*pos) {
                    Some(Tok::Str(s)) => { let s = s.clone(); *pos += 1; s }
                    Some(Tok::Word(s)) => { let s = s.clone(); *pos += 1; s }
                    _ => String::new(),
                };
                if toks.get(*pos) == Some(&Tok::LParen) { skip_parens(toks, pos); }
                let child_path = if current_path == "/" {
                    format!("/{prim_name}")
                } else {
                    format!("{current_path}/{prim_name}")
                };
                if toks.get(*pos) == Some(&Tok::LBrace) {
                    *pos += 1;
                    if type_name == "Mesh" {
                        if let Some(mut mesh) = parse_mesh_prim(toks, pos, &child_path, results) {
                            apply_transform(&mut mesh, &parent_xform);
                            results.push((child_path.clone(), mesh));
                        }
                    } else {
                        // Collect xformOps for this non-mesh prim, then recurse
                        let local_xform = parse_xform_block(toks, pos, &child_path,
                                                             parent_xform, results);
                        let _ = local_xform; // recurse already handled inside
                    }
                }
            }
            _ => { *pos += 1; }
        }
    }
}

/// Parse a non-Mesh prim block: collect xformOps, build local→world matrix,
/// then recurse into child prims with the accumulated transform.
fn parse_xform_block(
    toks:         &[Tok],
    pos:          &mut usize,
    current_path: &str,
    parent_xform: Mat4,
    results:      &mut Vec<(String, MeshData)>,
) -> Mat4 {
    // We do two passes over the block tokens:
    // First pass: collect xformOps and xformOpOrder, build local matrix.
    // Second pass: handled by parse_block for child prims.
    // Since we can't rewind easily, we snapshot the token range first.
    let block_start = *pos;

    // --- Pass 1: scan for xformOps without recursing into child defs ---
    let mut translate  = None::<[f64; 3]>;
    let mut scale      = None::<[f64; 3]>;
    let mut orient     = None::<[f64; 4]>;
    let mut rotate_xyz = None::<[f64; 3]>;
    let mut matrix     = None::<Vec<f64>>;

    let mut depth = 1usize;
    let mut t = block_start;
    while t < toks.len() && depth > 0 {
        match &toks[t] {
            Tok::LBrace => { depth += 1; t += 1; }
            Tok::RBrace => { depth -= 1; t += 1; }
            Tok::Word(w) => {
                // Look for lines like:
                //   double3 xformOp:translate = (x, y, z)
                //   float3  xformOp:scale     = (x, y, z)
                //   quatf   xformOp:orient    = (w, x, y, z)
                //   matrix4d xformOp:transform = ( ... )
                if w.starts_with("xformOp:") || w == "xformOp:translate"
                    || w == "xformOp:scale" || w == "xformOp:orient"
                    || w == "xformOp:transform" || w == "xformOp:rotateXYZ"
                {
                    // scan back to find the actual op name token
                }
                // More reliable: scan for the op name right before '='
                // We look for the pattern: <type> xformOp:<op> = <value>
                // The token w here might be the type keyword, op keyword, or something else.
                // Look ahead for xformOp: tokens followed by =
                if w.contains("xformOp:") {
                    let op = w.clone();
                    let mut tt = t + 1;
                    // skip to '='
                    while tt < toks.len() && toks[tt] != Tok::Eq
                          && toks[tt] != Tok::LBrace && toks[tt] != Tok::RBrace { tt += 1; }
                    if toks.get(tt) == Some(&Tok::Eq) {
                        tt += 1;
                        if op.contains("translate") {
                            translate = Some(parse_triple_inline(&toks, &mut tt));
                        } else if op.contains("scale") {
                            scale = Some(parse_triple_inline(&toks, &mut tt));
                        } else if op.contains("orient") {
                            orient = Some(parse_quad_inline(&toks, &mut tt));
                        } else if op.contains("rotateXYZ") || op.contains("rotateXyz") {
                            rotate_xyz = Some(parse_triple_inline(&toks, &mut tt));
                        } else if op.contains("transform") {
                            matrix = Some(parse_matrix16_inline(&toks, &mut tt));
                        }
                    }
                }
                t += 1;
            }
            _ => { t += 1; }
        }
    }

    // Build local transform matrix from collected ops (USD default op order)
    let local = if let Some(m) = matrix {
        mat4_from_usd_flat(&m)
    } else {
        let t_mat = translate.map(mat4_from_translate).unwrap_or(Mat4::IDENTITY);
        let r_mat = orient.map(mat4_from_orient)
            .or_else(|| rotate_xyz.map(mat4_from_rotate_xyz_degrees))
            .unwrap_or(Mat4::IDENTITY);
        let s_mat = scale.map(mat4_from_scale).unwrap_or(Mat4::IDENTITY);
        t_mat * r_mat * s_mat
    };

    let world_xform = parent_xform * local;

    // --- Pass 2: recurse with world transform (rewind to block_start) ---
    *pos = block_start;
    parse_block(toks, pos, current_path, world_xform, results);

    world_xform
}

// ── Inline value parsers (no pos mutation side-effects on the outer loop) ────

fn parse_triple_inline(toks: &[Tok], pos: &mut usize) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    // value can be bare `x, y, z` or `(x, y, z)`
    if toks.get(*pos) == Some(&Tok::LParen) { *pos += 1; }
    for i in 0..3 {
        out[i] = eat_num(toks, pos);
        eat_comma(toks, pos);
    }
    if toks.get(*pos) == Some(&Tok::RParen) { *pos += 1; }
    out
}

fn parse_quad_inline(toks: &[Tok], pos: &mut usize) -> [f64; 4] {
    let mut out = [0.0f64; 4];
    if toks.get(*pos) == Some(&Tok::LParen) { *pos += 1; }
    for i in 0..4 {
        out[i] = eat_num(toks, pos);
        eat_comma(toks, pos);
    }
    if toks.get(*pos) == Some(&Tok::RParen) { *pos += 1; }
    out
}

fn parse_matrix16_inline(toks: &[Tok], pos: &mut usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(16);
    // matrix4d is written as ( (r0c0,r0c1,r0c2,r0c3), (r1...), ... )
    if toks.get(*pos) == Some(&Tok::LParen) { *pos += 1; }
    for _ in 0..4 {
        if toks.get(*pos) == Some(&Tok::LParen) { *pos += 1; }
        for _ in 0..4 {
            out.push(eat_num(toks, pos));
            eat_comma(toks, pos);
        }
        if toks.get(*pos) == Some(&Tok::RParen) { *pos += 1; }
        eat_comma(toks, pos);
    }
    if toks.get(*pos) == Some(&Tok::RParen) { *pos += 1; }
    out
}

// ── Mesh prim parser ──────────────────────────────────────────────────────────

fn parse_mesh_prim(
    toks:    &[Tok],
    pos:     &mut usize,
    path:    &str,
    results: &mut Vec<(String, MeshData)>,
) -> Option<MeshData> {
    let mut points:     Option<Vec<[f32; 3]>> = None;
    let mut fv_counts:  Option<Vec<usize>>    = None;
    let mut fv_indices: Option<Vec<usize>>    = None;
    // Mesh prims can also carry their own xformOps
    let mut translate  = None::<[f64; 3]>;
    let mut scale      = None::<[f64; 3]>;
    let mut orient     = None::<[f64; 4]>;
    let mut rotate_xyz = None::<[f64; 3]>;
    let mut matrix     = None::<Vec<f64>>;

    while *pos < toks.len() && toks[*pos] != Tok::RBrace {
        if let Tok::Word(w) = &toks[*pos] {
            if w == "def" || w == "over" || w == "class" {
                parse_block(toks, pos, path, Mat4::IDENTITY, results);
                continue;
            }
        }

        let line_start = *pos;
        let mut attr_name = String::new();
        let mut temp = *pos;
        while temp < toks.len() && toks[temp] != Tok::Eq && toks[temp] != Tok::RBrace {
            if let Tok::Word(w) = &toks[temp] { attr_name = w.clone(); }
            temp += 1;
        }

        if toks.get(temp) == Some(&Tok::Eq) {
            *pos = temp + 1;
            match attr_name.as_str() {
                "points"            => { points     = Some(parse_vec3_array(toks, pos)); }
                "faceVertexCounts"  => { fv_counts  = Some(parse_int_array(toks, pos)); }
                "faceVertexIndices" => { fv_indices = Some(parse_int_array(toks, pos)); }
                n if n.contains("xformOp:translate") => {
                    translate = Some(parse_triple_inline(toks, pos));
                }
                n if n.contains("xformOp:scale") => {
                    scale = Some(parse_triple_inline(toks, pos));
                }
                n if n.contains("xformOp:orient") => {
                    orient = Some(parse_quad_inline(toks, pos));
                }
                n if n.contains("xformOp:rotateXYZ") || n.contains("xformOp:rotateXyz") => {
                    rotate_xyz = Some(parse_triple_inline(toks, pos));
                }
                n if n.contains("xformOp:transform") => {
                    matrix = Some(parse_matrix16_inline(toks, pos));
                }
                _ => { skip_value(toks, pos); }
            }
        } else {
            if *pos == line_start { *pos += 1; }
        }
    }

    if toks.get(*pos) == Some(&Tok::RBrace) { *pos += 1; }

    let pts    = points?;
    let counts = fv_counts?;
    let idxs   = fv_indices?;
    if pts.is_empty() { return None; }

    let tri_idx = triangulate(&counts, &idxs);
    let mut mesh = MeshData::from_triangles(
        pts,
        tri_idx.into_iter().map(|i| i as u32).collect(),
    );

    // Apply any xformOps baked directly on the Mesh prim itself
    let local = if let Some(m) = matrix {
        mat4_from_usd_flat(&m)
    } else {
        let t_mat = translate.map(mat4_from_translate).unwrap_or(Mat4::IDENTITY);
        let r_mat = orient.map(mat4_from_orient)
            .or_else(|| rotate_xyz.map(mat4_from_rotate_xyz_degrees))
            .unwrap_or(Mat4::IDENTITY);
        let s_mat = scale.map(mat4_from_scale).unwrap_or(Mat4::IDENTITY);
        t_mat * r_mat * s_mat
    };
    apply_transform(&mut mesh, &local);

    Some(mesh)
}

// ── Array parsers ─────────────────────────────────────────────────────────────

fn parse_vec3_array(toks: &[Tok], pos: &mut usize) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    if toks.get(*pos) != Some(&Tok::LBracket) { return out; }
    *pos += 1;
    while *pos < toks.len() && toks[*pos] != Tok::RBracket {
        if toks.get(*pos) == Some(&Tok::LParen) { *pos += 1; }
        let x = eat_num(toks, pos);
        eat_comma(toks, pos);
        let y = eat_num(toks, pos);
        eat_comma(toks, pos);
        let z = eat_num(toks, pos);
        if toks.get(*pos) == Some(&Tok::RParen) { *pos += 1; }
        eat_comma(toks, pos);
        out.push([x as f32, y as f32, z as f32]);
    }
    if toks.get(*pos) == Some(&Tok::RBracket) { *pos += 1; }
    if toks.get(*pos) == Some(&Tok::LParen) { skip_parens(toks, pos); }
    out
}

fn parse_int_array(toks: &[Tok], pos: &mut usize) -> Vec<usize> {
    let mut out = Vec::new();
    if toks.get(*pos) != Some(&Tok::LBracket) { return out; }
    *pos += 1;
    while *pos < toks.len() && toks[*pos] != Tok::RBracket {
        if let Some(Tok::Num(n)) = toks.get(*pos) { out.push(*n as usize); *pos += 1; }
        else { *pos += 1; }
        eat_comma(toks, pos);
    }
    if toks.get(*pos) == Some(&Tok::RBracket) { *pos += 1; }
    if toks.get(*pos) == Some(&Tok::LParen) { skip_parens(toks, pos); }
    out
}

// ── Token helpers ─────────────────────────────────────────────────────────────

fn eat_num(toks: &[Tok], pos: &mut usize) -> f64 {
    if let Some(Tok::Num(n)) = toks.get(*pos) { let v = *n; *pos += 1; v } else { 0.0 }
}
fn eat_comma(toks: &[Tok], pos: &mut usize) {
    if toks.get(*pos) == Some(&Tok::Comma) { *pos += 1; }
}
fn skip_parens(toks: &[Tok], pos: &mut usize) {
    if toks.get(*pos) != Some(&Tok::LParen) { return; }
    *pos += 1;
    let mut depth = 1usize;
    while *pos < toks.len() && depth > 0 {
        match &toks[*pos] {
            Tok::LParen => depth += 1,
            Tok::RParen => depth -= 1,
            _ => {}
        }
        *pos += 1;
    }
}
fn skip_value(toks: &[Tok], pos: &mut usize) {
    match toks.get(*pos) {
        Some(Tok::LBracket) => {
            *pos += 1;
            let mut depth = 1;
            while *pos < toks.len() && depth > 0 {
                match &toks[*pos] {
                    Tok::LBracket => depth += 1,
                    Tok::RBracket => depth -= 1,
                    _ => {}
                }
                *pos += 1;
            }
            if toks.get(*pos) == Some(&Tok::LParen) { skip_parens(toks, pos); }
        }
        Some(Tok::LParen) => skip_parens(toks, pos),
        _ => { *pos += 1; }
    }
}

// ── Triangulation ─────────────────────────────────────────────────────────────

fn triangulate(counts: &[usize], indices: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut offset = 0;
    for &n in counts {
        if n < 3 || offset + n > indices.len() { offset += n; continue; }
        let face = &indices[offset..offset + n];
        for i in 1..(n - 1) {
            out.push(face[0]);
            out.push(face[i]);
            out.push(face[i + 1]);
        }
        offset += n;
    }
    out
}

// ============================================================================
// USDC BINARY LOADER
// ============================================================================

fn load_usdc_meshes(path: &Path) -> UsdResult<Vec<(String, MeshData)>> {
    use openusd::usdc::read_file;
    use openusd::sdf::{self, Value};

    let mut data = read_file(path)
        .map_err(|e| UsdError::Parse(e.to_string()))?;

    let mut queue: Vec<(sdf::Path, Mat4)> = vec![(sdf::Path::abs_root(), Mat4::IDENTITY)];
    let mut mesh_paths: Vec<(sdf::Path, Mat4)> = Vec::new();

    while let Some((parent, parent_xform)) = queue.pop() {
        let children: Vec<String> = data
            .get(&parent, "primChildren")
            .ok()
            .and_then(|v| v.into_owned().try_as_token_vec())
            .unwrap_or_default();

        for child_name in children {
            let child_str = if parent == sdf::Path::abs_root() {
                format!("/{child_name}")
            } else {
                format!("{parent}/{child_name}")
            };
            let Ok(child_path) = sdf::path(&child_str) else { continue };

            // Collect xformOps for this prim
            let local = usdc_collect_xform(&mut data, &child_path);
            let world = parent_xform * local;

            let type_name = data
                .get(&child_path, "typeName")
                .ok()
                .and_then(|v| v.into_owned().try_as_token());

            if type_name.as_deref() == Some("Mesh") {
                mesh_paths.push((child_path.clone(), world));
            }
            queue.push((child_path, world));
        }
    }

    let mut results = Vec::new();

    for (mesh_path, world_xform) in mesh_paths {
        let prop_path = |prop: &str| -> Option<sdf::Path> {
            sdf::path(&format!("{mesh_path}.{prop}")).ok()
        };
        let mut get_default = |prop: &str| -> Option<Value> {
            let pp = prop_path(prop)?;
            data.get(&pp, "default").ok().map(|v| v.into_owned())
        };

        let points: Vec<[f32; 3]> = match get_default("points") {
            Some(Value::Vec3f(flat)) => flat.chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
            _ => continue,
        };
        let fv_counts: Vec<usize> = match get_default("faceVertexCounts") {
            Some(Value::IntVec(arr))   => arr.into_iter().map(|x| x as usize).collect(),
            Some(Value::Int64Vec(arr)) => arr.into_iter().map(|x| x as usize).collect(),
            _ => continue,
        };
        let fv_indices: Vec<usize> = match get_default("faceVertexIndices") {
            Some(Value::IntVec(arr))   => arr.into_iter().map(|x| x as usize).collect(),
            Some(Value::Int64Vec(arr)) => arr.into_iter().map(|x| x as usize).collect(),
            _ => continue,
        };

        if points.is_empty() { continue; }

        let tri_idx = triangulate(&fv_counts, &fv_indices);
        let mut mesh = MeshData::from_triangles(
            points,
            tri_idx.into_iter().map(|i| i as u32).collect(),
        );
        apply_transform(&mut mesh, &world_xform);
        results.push((mesh_path.to_string(), mesh));
    }

    Ok(results)
}

/// Read xformOp properties off a usdc prim and return its local Mat4.
fn usdc_collect_xform(
    data: &mut Box<dyn openusd::sdf::AbstractData>,
    path: &openusd::sdf::Path,
) -> Mat4 {
    use openusd::sdf::Value;

    let mut translate  = None::<[f64; 3]>;
    let mut scale      = None::<[f64; 3]>;
    let mut orient     = None::<[f64; 4]>;
    let mut matrix     = None::<Vec<f64>>;

    let prop_names = ["xformOp:translate", "xformOp:scale",
                      "xformOp:orient",    "xformOp:transform"];

    for prop in prop_names {
        let Ok(pp) = openusd::sdf::path(&format!("{path}.{prop}")) else { continue };
        let Ok(val) = data.get(&pp, "default") else { continue };
        match (prop, val.into_owned()) {
            ("xformOp:translate", Value::Vec3d(v)) =>
                { translate = Some([v[0], v[1], v[2]]); }
            ("xformOp:translate", Value::Vec3f(v)) =>
                { translate = Some([v[0] as f64, v[1] as f64, v[2] as f64]); }
            ("xformOp:scale", Value::Vec3f(v)) =>
                { scale = Some([v[0] as f64, v[1] as f64, v[2] as f64]); }
            ("xformOp:orient", Value::Quatf(v)) =>
                { orient = Some([v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64]); }
            ("xformOp:transform", Value::Matrix4d(v)) =>
                { matrix = Some(v.iter().map(|&x| x as f64).collect()); }
            _ => {}
        }
    }

    if let Some(m) = matrix { return mat4_from_usd_flat(&m); }
    let t = translate.map(mat4_from_translate).unwrap_or(Mat4::IDENTITY);
    let r = orient.map(mat4_from_orient).unwrap_or(Mat4::IDENTITY);
    let s = scale.map(mat4_from_scale).unwrap_or(Mat4::IDENTITY);
    t * r * s
}

// ============================================================================
// USDZ — zip container
// ============================================================================

fn load_usdz_meshes(path: &Path) -> UsdResult<Vec<(String, MeshData)>> {
    let bytes = std::fs::read(path)?;
    let tmp = std::env::temp_dir().join("bevy_dcc_usdz_extract");
    std::fs::create_dir_all(&tmp)?;

    let mut i = 0usize;
    while i + 30 < bytes.len() {
        if &bytes[i..i+4] == b"PK\x03\x04" {
            let name_len  = u16::from_le_bytes([bytes[i+26], bytes[i+27]]) as usize;
            let extra_len = u16::from_le_bytes([bytes[i+28], bytes[i+29]]) as usize;
            let comp_size = u32::from_le_bytes([bytes[i+18], bytes[i+19],
                                                bytes[i+20], bytes[i+21]]) as usize;
            let name_start = i + 30;
            let name_end   = name_start + name_len;
            let data_start = name_end + extra_len;
            let data_end   = data_start + comp_size;

            if name_end <= bytes.len() {
                let name = String::from_utf8_lossy(&bytes[name_start..name_end]).to_string();
                let ext  = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                if (ext == "usda" || ext == "usdc") && data_end <= bytes.len() {
                    let data = &bytes[data_start..data_end];
                    let out_path = tmp.join(&name);
                    std::fs::write(&out_path, data)?;
                    return load_usd_meshes(&out_path);
                }
            }
            i = data_end.max(i + 1);
        } else {
            i += 1;
        }
    }

    Err(UsdError::Parse("No usda/usdc found inside usdz".into()))
}