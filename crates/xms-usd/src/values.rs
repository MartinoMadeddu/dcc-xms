//! `sdf::Value` → plain Rust conversions.

use crate::*;

// ---------------------------------------------------------------------------
// Value helpers (sdf::Value -> plain Rust)
// ---------------------------------------------------------------------------

pub(crate) fn tok(v: &Value) -> Option<String> {
    match v {
        Value::Token(t) => Some(t.as_str().to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

pub(crate) fn scalar(v: Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(f as f64),
        Value::Double(d) => Some(d),
        Value::Int(i) => Some(i as f64),
        Value::Int64(i) => Some(i as f64),
        Value::Uint(u) => Some(u as f64),
        Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
        Value::Vec4f(p) => Some(p.x as f64),
        other => match other.coerce_to_kind(ValueKind::Double) {
            Ok(Value::Double(d)) => Some(d),
            _ => None,
        },
    }
}

pub(crate) fn vec3(v: Value) -> Option<[f32; 3]> {
    match v {
        Value::Vec3f(p) => Some([p.x, p.y, p.z]),
        Value::Vec3d(p) => Some([p.x as f32, p.y as f32, p.z as f32]),
        Value::Vec4f(p) => Some([p.x, p.y, p.z]),
        Value::Float(f) => Some([f; 3]),
        other => match other.coerce_to_kind(ValueKind::Vec3f) {
            Ok(Value::Vec3f(p)) => Some([p.x, p.y, p.z]),
            _ => None,
        },
    }
}

pub(crate) fn vec3_f64(v: Value) -> Option<[f64; 3]> {
    match v {
        Value::Vec3d(p) => Some([p.x, p.y, p.z]),
        other => vec3(other).map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]),
    }
}

/// Arrays at least this long convert in parallel (groom-sized attributes).
pub(crate) const PAR_CONVERT: usize = 1 << 18;

pub(crate) fn vec3_array(v: Value) -> Option<Vec<[f32; 3]>> {
    use rayon::prelude::*;
    match v {
        Value::Vec3fVec(a) if a.len() >= PAR_CONVERT => Some(a.par_iter().map(|p| [p.x, p.y, p.z]).collect()),
        Value::Vec3fVec(a) => Some(a.iter().map(|p| [p.x, p.y, p.z]).collect()),
        Value::Vec3dVec(a) if a.len() >= PAR_CONVERT => Some(a.par_iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect()),
        Value::Vec3dVec(a) => Some(a.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect()),
        other => match other.coerce_to_kind(ValueKind::Vec3fVec) {
            Ok(Value::Vec3fVec(a)) => Some(a.iter().map(|p| [p.x, p.y, p.z]).collect()),
            _ => None,
        },
    }
}

pub(crate) fn vec2_array(v: Value) -> Option<Vec<[f32; 2]>> {
    match v {
        Value::Vec2fVec(a) => Some(a.iter().map(|p| [p.x, p.y]).collect()),
        other => match other.coerce_to_kind(ValueKind::Vec2fVec) {
            Ok(Value::Vec2fVec(a)) => Some(a.iter().map(|p| [p.x, p.y]).collect()),
            _ => None,
        },
    }
}

pub(crate) fn int_array(v: Value) -> Option<Vec<i64>> {
    match v {
        Value::IntVec(a) => Some(a.into_iter().map(i64::from).collect()),
        Value::Int64Vec(a) => Some(a),
        Value::UintVec(a) => Some(a.into_iter().map(i64::from).collect()),
        Value::Uint64Vec(a) => Some(a.into_iter().map(|x| x as i64).collect()),
        _ => None,
    }
}

pub(crate) fn float_array(v: Value) -> Option<Vec<f32>> {
    match v {
        Value::FloatVec(a) => Some(a),
        Value::DoubleVec(a) => Some(a.into_iter().map(|x| x as f32).collect()),
        other => match other.coerce_to_kind(ValueKind::FloatVec) {
            Ok(Value::FloatVec(a)) => Some(a),
            _ => None,
        },
    }
}
/// Any numeric USD value or array → (components, values padded to [f32; 4]).
/// Scalars give one value. `None` for non-numeric values (strings, tokens, …).
pub(crate) fn numeric_values(v: Value) -> Option<(u8, Vec<[f32; 4]>)> {
    let one = |x: f32| [x, 0.0, 0.0, 0.0];
    Some(match v {
        Value::FloatVec(a) => (1, a.into_iter().map(one).collect()),
        Value::DoubleVec(a) => (1, a.into_iter().map(|x| one(x as f32)).collect()),
        Value::IntVec(a) => (1, a.into_iter().map(|x| one(x as f32)).collect()),
        Value::Vec2fVec(a) => (2, a.iter().map(|p| [p.x, p.y, 0.0, 0.0]).collect()),
        Value::Vec3fVec(a) => (3, a.iter().map(|p| [p.x, p.y, p.z, 0.0]).collect()),
        Value::Vec3dVec(a) => (3, a.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32, 0.0]).collect()),
        Value::Float(x) => (1, vec![one(x)]),
        Value::Double(x) => (1, vec![one(x as f32)]),
        Value::Int(x) => (1, vec![one(x as f32)]),
        Value::Vec2f(p) => (2, vec![[p.x, p.y, 0.0, 0.0]]),
        Value::Vec3f(p) => (3, vec![[p.x, p.y, p.z, 0.0]]),
        Value::Vec3d(p) => (3, vec![[p.x as f32, p.y as f32, p.z as f32, 0.0]]),
        Value::Vec4f(p) => (4, vec![[p.x, p.y, p.z, p.w]]),
        // Other array kinds (half, double vec2, …) through the coercing readers
        other => {
            if let Some(a) = vec3_array(other.clone()) {
                (3, a.into_iter().map(|p| [p[0], p[1], p[2], 0.0]).collect())
            } else if let Some(a) = vec2_array(other.clone()) {
                (2, a.into_iter().map(|p| [p[0], p[1], 0.0, 0.0]).collect())
            } else if let Some(a) = float_array(other) {
                (1, a.into_iter().map(one).collect())
            } else {
                return None;
            }
        }
    })
}

pub(crate) fn type_of(prim: &Prim) -> String {
    prim.type_name().ok().flatten().map(|t| t.as_str().to_string()).unwrap_or_default()
}

pub(crate) fn binding(prim: &Prim) -> Option<String> {
    for rel in ["material:binding", "material:binding:full", "material:binding:preview"] {
        if let Ok(targets) = prim.relationship(rel).targets() {
            if let Some(t) = targets.first() {
                return Some(t.to_string());
            }
        }
    }
    None
}