//! Reader for the USD text format (`.usda`).
//!
//! Parses a layer into the same shape the binary reader gives: a table of
//! specs by path, each a set of named fields holding `sdf::Value`s. Prims
//! carry `typeName`, `primChildren` and `properties`; attributes carry
//! `default`, `timeSamples`, `connectionPaths` and their metadata;
//! relationships carry `targetPaths`. The stage reader in `usd_scene` then
//! treats both formats alike.
//!
//! Covered: layer and prim metadata, nested prims, typed attributes of every
//! scalar, tuple, array, matrix, quaternion, asset and string type,
//! connections, time samples, relationships, dictionaries. A variant set
//! contributes its selected variant, or its first one when none is selected.

use std::collections::HashMap;

use openusd::sdf::{self, Value};

#[derive(Clone, Debug, PartialEq)]
enum Tok { Id(String), Str(String), Num(f64), Asset(String), Path(String), P(char) }

fn tokenize(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() { i += 1; continue; }
        if ch == '#' { while i < c.len() && c[i] != '\n' { i += 1; } continue; }
        if ch == '"' || ch == '\'' {
            // Triple-quoted strings run over lines.
            let triple = i + 2 < c.len() && c[i + 1] == ch && c[i + 2] == ch;
            let mut s = String::new();
            i += if triple { 3 } else { 1 };
            while i < c.len() {
                if triple { if i + 2 < c.len() && c[i] == ch && c[i + 1] == ch && c[i + 2] == ch { i += 3; break; } }
                else if c[i] == ch { i += 1; break; }
                if c[i] == '\\' && i + 1 < c.len() { i += 1; s.push(match c[i] { 'n' => '\n', 't' => '\t', other => other }); }
                else { s.push(c[i]); }
                i += 1;
            }
            out.push(Tok::Str(s));
            continue;
        }
        if ch == '@' {
            let triple = i + 2 < c.len() && c[i + 1] == '@' && c[i + 2] == '@';
            i += if triple { 3 } else { 1 };
            let mut s = String::new();
            while i < c.len() {
                if triple { if i + 2 < c.len() && c[i] == '@' && c[i + 1] == '@' && c[i + 2] == '@' { i += 3; break; } }
                else if c[i] == '@' { i += 1; break; }
                s.push(c[i]);
                i += 1;
            }
            out.push(Tok::Asset(s));
            continue;
        }
        if ch == '<' {
            let mut s = String::new();
            i += 1;
            while i < c.len() && c[i] != '>' { s.push(c[i]); i += 1; }
            i += 1;
            out.push(Tok::Path(s));
            continue;
        }
        let number_start = ch.is_ascii_digit() || ((ch == '-' || ch == '+' || ch == '.') && i + 1 < c.len() && (c[i + 1].is_ascii_digit() || c[i + 1] == '.'));
        if number_start {
            let start = i;
            i += 1;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == 'e' || c[i] == 'E'
                || ((c[i] == '-' || c[i] == '+') && (c[i - 1] == 'e' || c[i - 1] == 'E'))) { i += 1; }
            let text: String = c[start..i].iter().collect();
            out.push(Tok::Num(text.parse().unwrap_or(0.0)));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' || (ch == '-' && c[i + 1..].iter().take(3).collect::<String>() == "inf") {
            let start = i;
            i += 1;
            // Names may be namespaced (a:b), dotted (a.connect) and typed as arrays (int[]).
            while i < c.len() {
                let x = c[i];
                let joins = (x == ':' || x == '.') && i + 1 < c.len() && (c[i + 1].is_alphanumeric() || c[i + 1] == '_');
                if x.is_alphanumeric() || x == '_' || joins { i += 1; }
                else if x == '[' && i + 1 < c.len() && c[i + 1] == ']' { i += 2; }
                else { break; }
            }
            let text: String = c[start..i].iter().collect();
            out.push(match text.as_str() {
                "inf" => Tok::Num(f64::INFINITY), "-inf" => Tok::Num(f64::NEG_INFINITY), "nan" => Tok::Num(f64::NAN),
                _ => Tok::Id(text),
            });
            continue;
        }
        out.push(Tok::P(ch));
        i += 1;
    }
    out
}

/// A value before its type is known.
#[derive(Clone, Debug)]
enum Any { Num(f64), Str(String), Id(String), Asset(String), Path(String), List(Vec<Any>), Tuple(Vec<Any>), Dict(Vec<(String, Any)>), None }

impl Any {
    fn numbers(&self, out: &mut Vec<f64>) {
        match self {
            Any::Num(n) => out.push(*n),
            Any::List(a) | Any::Tuple(a) => for x in a { x.numbers(out); },
            _ => {}
        }
    }
    fn strings(&self, out: &mut Vec<String>) {
        match self {
            Any::Str(s) | Any::Id(s) | Any::Asset(s) | Any::Path(s) => out.push(s.clone()),
            Any::List(a) | Any::Tuple(a) => for x in a { x.strings(out); },
            _ => {}
        }
    }
    fn text(&self) -> Option<String> {
        match self { Any::Str(s) | Any::Id(s) | Any::Asset(s) | Any::Path(s) => Some(s.clone()), _ => None }
    }
}

/// A typed value, from the type written in front of the attribute.
fn typed(kind: &str, any: &Any) -> Value {
    if matches!(any, Any::None) { return Value::ValueBlock; }
    let array = kind.ends_with("[]");
    let base = kind.trim_end_matches("[]");
    let text = |any: &Any| { let mut s = vec![]; any.strings(&mut s); s };
    match base {
        "token" | "string" => {
            let s = text(any);
            return if array { Value::TokenVec(s) } else { Value::Token(s.into_iter().next().unwrap_or_default()) };
        }
        "asset" => return Value::AssetPath(text(any).into_iter().next().unwrap_or_default()),
        "bool" => {
            let truth = |a: &Any| match a { Any::Num(n) => *n != 0.0, Any::Id(s) => s == "true", _ => false };
            return match any { Any::List(a) => Value::BoolVec(a.iter().map(truth).collect()), other => Value::Bool(truth(other)) };
        }
        _ => {}
    }
    let mut n = vec![];
    any.numbers(&mut n);
    let whole = matches!(base, "int" | "uint" | "int64" | "uint64" | "uchar");
    if whole {
        return if array { Value::IntVec(n.iter().map(|x| *x as i32).collect()) } else { Value::Int(n.first().copied().unwrap_or(0.0) as i32) };
    }
    if base.starts_with("matrix4") { return Value::Matrix4d(n); }
    if base.starts_with("matrix3") { return Value::Matrix3d(n); }
    if base.starts_with("matrix2") { return Value::Matrix2d(n); }
    if base.starts_with("quat") { return Value::Quatd(n); }
    // Tuples: the width is the digit in the type name (float3, point3f, texCoord2f, color4f).
    match base.chars().rev().find(|c| c.is_ascii_digit()) {
        Some('2') => Value::Vec2d(n),
        Some('3') => Value::Vec3d(n),
        Some('4') => Value::Vec4d(n),
        _ if array => Value::DoubleVec(n),
        _ => Value::Double(n.first().copied().unwrap_or(0.0)),
    }
}

fn path_list(any: &Any) -> Value {
    let mut s = vec![];
    any.strings(&mut s);
    let mut op = sdf::PathListOp::default();
    op.explicit = true;
    op.explicit_items = s.iter().filter_map(|p| sdf::path(p).ok()).collect();
    Value::PathListOp(op)
}

type Fields = HashMap<String, Value>;

/// A parsed text layer: fields by spec path. The layer itself is at "/".
#[derive(Default)]
pub struct TextLayer { specs: HashMap<String, Fields> }

impl TextLayer {
    pub fn get(&self, path: &str, field: &str) -> Option<Value> { self.specs.get(path)?.get(field).cloned() }
}

struct Parser { t: Vec<Tok>, i: usize, layer: TextLayer }

const LIST_OPS: [&str; 5] = ["prepend", "append", "add", "delete", "reorder"];

impl Parser {
    fn peek(&self) -> Option<&Tok> { self.t.get(self.i) }
    fn is(&self, ch: char) -> bool { self.peek() == Some(&Tok::P(ch)) }
    fn eat(&mut self, ch: char) -> bool { if self.is(ch) { self.i += 1; true } else { false } }
    fn id(&self) -> Option<&str> { match self.peek() { Some(Tok::Id(s)) => Some(s.as_str()), _ => None } }
    fn set(&mut self, path: &str, field: &str, value: Value) {
        self.layer.specs.entry(path.to_string()).or_default().insert(field.to_string(), value);
    }
    fn push(&mut self, path: &str, field: &str, item: String) {
        let fields = self.layer.specs.entry(path.to_string()).or_default();
        match fields.entry(field.to_string()).or_insert_with(|| Value::TokenVec(vec![])) {
            Value::TokenVec(v) => if !v.contains(&item) { v.push(item) },
            _ => {}
        }
    }

    fn value(&mut self) -> Any {
        let Some(tok) = self.peek().cloned() else { return Any::None };
        self.i += 1;
        match tok {
            Tok::Num(n) => Any::Num(n),
            Tok::Str(s) => Any::Str(s),
            Tok::Asset(s) => Any::Asset(s),
            Tok::Path(s) => Any::Path(s),
            Tok::Id(s) if s == "None" => Any::None,
            Tok::Id(s) => Any::Id(s),
            Tok::P(open) if open == '[' || open == '(' => {
                let close = if open == '[' { ']' } else { ')' };
                let mut items = vec![];
                while self.i < self.t.len() && !self.is(close) {
                    items.push(self.value());
                    self.eat(',');
                }
                self.eat(close);
                if open == '[' { Any::List(items) } else { Any::Tuple(items) }
            }
            Tok::P('{') => {
                // A dictionary, or time samples: `type name = value` or `time: value`.
                let mut entries = vec![];
                while self.i < self.t.len() && !self.is('}') {
                    let mut key = String::new();
                    while self.i < self.t.len() && !self.is('=') && !self.is(':') && !self.is('}') {
                        match self.peek().cloned() {
                            Some(Tok::Id(s)) | Some(Tok::Str(s)) => key = s,
                            Some(Tok::Num(n)) => key = n.to_string(),
                            _ => {}
                        }
                        self.i += 1;
                    }
                    if !self.eat('=') && !self.eat(':') { break; }
                    entries.push((key, self.value()));
                    self.eat(',');
                    self.eat(';');
                }
                self.eat('}');
                Any::Dict(entries)
            }
            _ => Any::None,
        }
    }

    /// `( key = value ... )` after a layer, prim or property. Bare strings
    /// are documentation and are dropped.
    fn metadata(&mut self) -> Vec<(String, Any)> {
        let mut out = vec![];
        if !self.eat('(') { return out; }
        while self.i < self.t.len() && !self.is(')') {
            if matches!(self.peek(), Some(Tok::Str(_))) { self.i += 1; continue; }
            if self.id().map(|s| LIST_OPS.contains(&s)).unwrap_or(false) { self.i += 1; }
            // The key is the last name before the equals sign: a type may come first.
            let mut key = String::new();
            while let Some(s) = self.id() { key = s.to_string(); self.i += 1; }
            if self.eat('=') { out.push((key, self.value())); } else if key.is_empty() { self.i += 1; }
            self.eat(';');
        }
        self.eat(')');
        out
    }

    fn prim(&mut self, parent: &str) {
        self.i += 1;   // def, over or class
        let mut kind = String::new();
        if let Some(s) = self.id() { kind = s.to_string(); self.i += 1; }
        let name = match self.peek().cloned() { Some(Tok::Str(s)) => { self.i += 1; s } _ => return };
        let path = if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") };
        self.push(parent, "primChildren", name);
        self.set(&path, "typeName", Value::Token(kind));
        let meta = self.metadata();
        let mut chosen: HashMap<String, String> = HashMap::new();
        for (key, value) in &meta {
            match (key.as_str(), value) {
                ("instanceable", v) => self.set(&path, key, Value::Bool(matches!(v, Any::Id(s) if s == "true"))),
                ("variants", Any::Dict(sel)) => for (set, v) in sel { if let Some(v) = v.text() { chosen.insert(set.clone(), v); } },
                (_, v) => self.set(&path, key, Value::Token(v.text().unwrap_or_default())),
            }
        }
        if self.eat('{') { self.body(&path, &chosen); }
    }

    /// The contents of a prim, up to its closing brace.
    fn body(&mut self, path: &str, chosen: &HashMap<String, String>) {
        while self.i < self.t.len() && !self.is('}') {
            let Some(word) = self.id().map(str::to_string) else { self.i += 1; continue };
            match word.as_str() {
                "def" | "over" | "class" => self.prim(path),
                "variantSet" => self.variant_set(path, chosen),
                _ => self.property(path),
            }
        }
        self.eat('}');
    }

    fn variant_set(&mut self, path: &str, chosen: &HashMap<String, String>) {
        self.i += 1;
        let set = match self.peek().cloned() { Some(Tok::Str(s)) => { self.i += 1; s } _ => String::new() };
        self.push(path, "variantSetNames", set.clone());
        self.eat('=');
        if !self.eat('{') { return; }
        let mut first = true;
        while self.i < self.t.len() && !self.is('}') {
            let name = match self.peek().cloned() { Some(Tok::Str(s)) => { self.i += 1; s } _ => { self.i += 1; continue } };
            self.metadata();
            if !self.eat('{') { continue; }
            let wanted = match chosen.get(&set) { Some(c) => *c == name, None => first };
            first = false;
            if wanted { self.body(path, chosen); } else { self.skip_block(); }
        }
        self.eat('}');
    }

    /// Skip to the brace that closes the block already opened.
    fn skip_block(&mut self) {
        let mut depth = 1;
        while self.i < self.t.len() && depth > 0 {
            if self.is('{') { depth += 1; } else if self.is('}') { depth -= 1; }
            self.i += 1;
        }
    }

    fn property(&mut self, prim: &str) {
        // Qualifiers, then for an attribute its type, then the name.
        let mut words: Vec<String> = vec![];
        while let Some(s) = self.id() { words.push(s.to_string()); self.i += 1; }
        if words.is_empty() { self.i += 1; return; }
        let full = words.pop().unwrap_or_default();
        let is_rel = words.iter().any(|w| w == "rel");
        let kind = words.iter().rev().find(|w| !matches!(w.as_str(), "custom" | "uniform" | "varying" | "config" | "rel") && !LIST_OPS.contains(&w.as_str()))
            .cloned().unwrap_or_default();
        if kind.is_empty() && !is_rel && !self.is('=') {
            // Not a property: a stray statement such as `reorder`. Drop its value.
            return;
        }
        let (name, suffix) = match full.rsplit_once('.') {
            Some((n, s)) if s == "connect" || s == "timeSamples" => (n.to_string(), s.to_string()),
            _ => (full.clone(), String::new()),
        };
        let spec = format!("{prim}.{name}");
        self.push(prim, "properties", name.clone());
        if !kind.is_empty() { self.set(&spec, "typeName", Value::Token(kind.clone())); }
        if self.eat('=') {
            let any = self.value();
            if is_rel { self.set(&spec, "targetPaths", path_list(&any)); }
            else if suffix == "connect" { self.set(&spec, "connectionPaths", path_list(&any)); }
            else if suffix == "timeSamples" {
                if let Any::Dict(entries) = &any {
                    let mut samples: Vec<(f64, Value)> = entries.iter()
                        .map(|(time, v)| (time.parse().unwrap_or(0.0), typed(&kind, v))).collect();
                    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
                    self.set(&spec, "timeSamples", Value::TimeSamples(samples));
                }
            } else {
                let value = typed(&kind, &any);
                self.set(&spec, "default", value);
            }
        }
        for (key, value) in self.metadata() {
            if let Some(text) = value.text() { self.set(&spec, &key, Value::Token(text)); }
        }
    }
}

/// Parse the text of a `.usda` layer.
pub fn parse(src: &str) -> TextLayer {
    let mut p = Parser { t: tokenize(src), i: 0, layer: TextLayer::default() };
    p.layer.specs.entry("/".into()).or_default();
    for (key, value) in p.metadata() {
        let v = match &value {
            Any::Num(n) => Value::Double(*n),
            other => Value::Token(other.text().unwrap_or_default()),
        };
        p.set("/", &key, v);
    }
    while p.i < p.t.len() {
        match p.id() {
            Some("def") | Some("over") | Some("class") => p.prim("/"),
            _ => p.i += 1,
        }
    }
    p.layer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_prims_and_typed_attributes() {
        let layer = parse(r#"#usda 1.0
(
    "a comment string"
    defaultPrim = "root"
    metersPerUnit = 0.01
    upAxis = "Z"
)
def Xform "root" (
    kind = "component"
    prepend references = @other.usda@</thing>
)
{
    custom double3 xformOp:translate = (1, 2.5, -3e1)
    uniform token[] xformOpOrder = ["xformOp:translate", "!invert!xformOp:translate:pivot"]
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (4, 5, 6, 1) )
    def Mesh "geo"
    {
        int[] faceVertexCounts = [3]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0)] (
            interpolation = "faceVarying"
        )
        float3 xformOp:rotateXYZ.timeSamples = {
            0: (0, 0, 0),
            24: (0, 90, 0),
        }
        rel material:binding = </root/mat>
        color3f inputs:diffuseColor.connect = </root/mat/tex.outputs:rgb>
        asset inputs:file = @tex/a.png@
        bool doubleSided = true
        string note = """two
lines"""
    }
}
"#);
        assert!(matches!(layer.get("/", "upAxis"), Some(Value::Token(s)) if s == "Z"));
        assert!(matches!(layer.get("/", "metersPerUnit"), Some(Value::Double(v)) if v == 0.01));
        assert!(matches!(layer.get("/", "primChildren"), Some(Value::TokenVec(v)) if v == vec!["root".to_string()]));
        assert!(matches!(layer.get("/root", "typeName"), Some(Value::Token(s)) if s == "Xform"));
        assert!(layer.get("/root", "references").is_some());
        assert!(matches!(layer.get("/root.xformOp:translate", "default"), Some(Value::Vec3d(v)) if v == vec![1.0, 2.5, -30.0]));
        assert!(matches!(layer.get("/root.xformOpOrder", "default"), Some(Value::TokenVec(v)) if v.len() == 2 && v[1].starts_with("!invert!")));
        assert!(matches!(layer.get("/root.xformOp:transform", "default"), Some(Value::Matrix4d(v)) if v.len() == 16 && v[12] == 4.0));
        assert!(matches!(layer.get("/root/geo.faceVertexCounts", "default"), Some(Value::IntVec(v)) if v == vec![3]));
        assert!(matches!(layer.get("/root/geo.points", "default"), Some(Value::Vec3d(v)) if v.len() == 9));
        assert!(matches!(layer.get("/root/geo.primvars:st", "default"), Some(Value::Vec2d(v)) if v.len() == 4));
        assert!(matches!(layer.get("/root/geo.primvars:st", "interpolation"), Some(Value::Token(s)) if s == "faceVarying"));
        assert!(matches!(layer.get("/root/geo.xformOp:rotateXYZ", "timeSamples"), Some(Value::TimeSamples(s)) if s.len() == 2 && s[1].0 == 24.0));
        assert!(matches!(layer.get("/root/geo.material:binding", "targetPaths"), Some(Value::PathListOp(op)) if op.explicit_items.len() == 1));
        assert!(matches!(layer.get("/root/geo.inputs:diffuseColor", "connectionPaths"), Some(Value::PathListOp(_))));
        assert!(matches!(layer.get("/root/geo.inputs:file", "default"), Some(Value::AssetPath(s)) if s == "tex/a.png"));
        assert!(matches!(layer.get("/root/geo.doubleSided", "default"), Some(Value::Bool(true))));
        assert!(matches!(layer.get("/root/geo.note", "default"), Some(Value::Token(s)) if s.contains('\n')));
        let props = match layer.get("/root/geo", "properties") { Some(Value::TokenVec(v)) => v, _ => vec![] };
        assert!(props.contains(&"points".to_string()) && props.contains(&"xformOp:rotateXYZ".to_string()) && props.contains(&"material:binding".to_string()));
    }

    #[test]
    fn a_variant_set_gives_its_selected_variant() {
        let text = r#"#usda 1.0
def Xform "prop" (
    variants = {
        string size = "big"
    }
    prepend variantSets = "size"
)
{
    variantSet "size" = {
        "small" {
            def Mesh "small_geo" { }
        }
        "big" {
            float radius = 5
            def Mesh "big_geo" { }
        }
    }
    def Mesh "always" { }
}
"#;
        let layer = parse(text);
        let kids = match layer.get("/prop", "primChildren") { Some(Value::TokenVec(v)) => v, _ => vec![] };
        assert_eq!(kids, vec!["big_geo".to_string(), "always".to_string()]);
        assert!(matches!(layer.get("/prop.radius", "default"), Some(Value::Double(v)) if v == 5.0));
        // No selection: the first variant.
        let layer = parse(&text.replace("string size = \"big\"", ""));
        let kids = match layer.get("/prop", "primChildren") { Some(Value::TokenVec(v)) => v, _ => vec![] };
        assert_eq!(kids, vec!["small_geo".to_string(), "always".to_string()]);
    }
}
