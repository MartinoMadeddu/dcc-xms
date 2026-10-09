//! `.mtlx` documents referenced from the root layer.
//!
//! Only the reference scan is kept from rray's materialx.rs: compiling MaterialX
//! graphs into shading programs is renderer work and stays in rray. The scene
//! layer just records which document (and prim in it) a material points at.

use crate::HashMap;
use std::path::Path;

/// `.mtlx` references authored in a `.usda` layer, found by reading its text:
/// prim path → (MaterialX file, referenced prim path).
///
/// USD composition can't read `.mtlx` layers (that needs USD's MaterialX file
/// format plugin), so such references are picked up here and the documents are
/// read by the renderer. Only text layers are scanned; binary `.usdc` layers
/// return nothing.
pub(crate) fn scan_mtlx_references(layer: &Path) -> HashMap<String, (String, String)> {
    let mut out = HashMap::default();
    let Ok(text) = std::fs::read_to_string(layer) else { return out };
    if !text.trim_start().starts_with("#usda") {
        return out;
    }
    let dir = layer.parent().map(Path::to_path_buf).unwrap_or_default();
    // Prim stack: (name, brace depth of its body)
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    // Parenthesis depth: braces inside `( … )` metadata are dictionaries, not bodies
    let mut paren = 0usize;
    // A prim header seen, its body `{` not yet: (name, collected metadata text)
    let mut pending: Option<(String, String)> = None;
    for line in text.lines() {
        let t = line.trim_start();
        let mut rest = t;
        for kw in ["def ", "over ", "class "] {
            if let Some(r) = t.strip_prefix(kw) {
                if let Some(q0) = r.find('"') {
                    if let Some(q1) = r[q0 + 1..].find('"') {
                        pending = Some((r[q0 + 1..q0 + 1 + q1].to_string(), String::new()));
                        rest = &r[q0 + 1 + q1 + 1..];
                    }
                }
                break;
            }
        }
        // Walk the line: collect metadata for a pending prim, track braces
        let mut in_str = false;
        for ch in rest.chars() {
            match ch {
                '"' => in_str = !in_str,
                '(' if !in_str => paren += 1,
                ')' if !in_str => paren = paren.saturating_sub(1),
                '{' if !in_str && paren == 0 => {
                    depth += 1;
                    if let Some((name, meta)) = pending.take() {
                        let mut path: String = stack.iter().map(|(n, _)| format!("/{n}")).collect();
                        path.push('/');
                        path.push_str(&name);
                        if let Some((file, target)) = mtlx_ref(&meta) {
                            let file = if Path::new(&file).is_absolute() { file } else { dir.join(file).to_string_lossy().into_owned() };
                            out.insert(path, (file, target));
                        }
                        stack.push((name, depth));
                    }
                }
                '}' if !in_str && paren == 0 => {
                    while stack.last().is_some_and(|(_, d)| *d >= depth) {
                        stack.pop();
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            if let Some((_, meta)) = pending.as_mut() {
                meta.push(ch);
            }
        }
        if let Some((_, meta)) = pending.as_mut() {
            meta.push('\n');
        }
    }
    out
}

/// First `@….mtlx@<target>` in a prim's metadata text.
fn mtlx_ref(meta: &str) -> Option<(String, String)> {
    let mut s = meta;
    while let Some(a) = s.find('@') {
        let rest = &s[a + 1..];
        let b = rest.find('@')?;
        let asset = &rest[..b];
        let after = &rest[b + 1..];
        if asset.to_ascii_lowercase().ends_with(".mtlx") {
            let target = after
                .trim_start()
                .strip_prefix('<')
                .and_then(|t| t.find('>').map(|e| t[..e].to_string()))
                .unwrap_or_default();
            return Some((asset.to_string(), target));
        }
        s = after;
    }
    None
}