//! Transforms: xformOps and quaternions. (rray's geometry-placement helpers,
//! which need renderer types, are not part of xms-usd.)

use crate::*;

/// gf matrices are row-major with row vectors; nalgebra here uses column vectors.
pub(crate) fn gf_to_col(m: gf::Matrix4d) -> M4 {
    M4::from_row_slice(&m.0).transpose()
}

pub(crate) fn quat_matrix(v: Value) -> Option<M4> {
    let v = match v {
        Value::Quatf(_) => v,
        other => other.coerce_to_kind(ValueKind::Quatf).ok()?,
    };
    match v {
        Value::Quatf(q) => Some(gf_to_col(gf::Matrix4d::from_quat(q))),
        _ => None,
    }
}

/// Authored quaternion array components, in the order openusd's `from_quat`
/// takes them (real, i, j, k). The values are `f32` (half-precision `quath`
/// converts exactly), so the `f64` widening is exact too.
pub(crate) fn quat_components(v: Value) -> Option<Vec<[f64; 4]>> {
    let v = match v {
        Value::QuatfVec(_) => v,
        other => other.coerce_to_kind(ValueKind::QuatfVec).ok()?,
    };
    match v {
        Value::QuatfVec(qs) => Some(qs.into_iter().map(<[f64; 4]>::from).collect()),
        _ => None,
    }
}

pub(crate) fn rot_axis(axis: char, deg: f64) -> M4 {
    let a = match axis {
        'X' => Vector3::x_axis(),
        'Y' => Vector3::y_axis(),
        _ => Vector3::z_axis(),
    };
    Rotation3::from_axis_angle(&a, deg.to_radians()).to_homogeneous()
}

pub(crate) fn op_matrix(kind: &str, v: Value) -> Option<M4> {
    let t = |x: f64, y: f64, z: f64| M4::new_translation(&Vector3::new(x, y, z));
    let s = |x: f64, y: f64, z: f64| M4::new_nonuniform_scaling(&Vector3::new(x, y, z));
    Some(match kind {
        "translate" => {
            let p = vec3_f64(v)?;
            t(p[0], p[1], p[2])
        }
        "translateX" => t(scalar(v)?, 0.0, 0.0),
        "translateY" => t(0.0, scalar(v)?, 0.0),
        "translateZ" => t(0.0, 0.0, scalar(v)?),
        "scale" => {
            let p = vec3_f64(v)?;
            s(p[0], p[1], p[2])
        }
        "scaleX" => s(scalar(v)?, 1.0, 1.0),
        "scaleY" => s(1.0, scalar(v)?, 1.0),
        "scaleZ" => s(1.0, 1.0, scalar(v)?),
        "rotateX" => rot_axis('X', scalar(v)?),
        "rotateY" => rot_axis('Y', scalar(v)?),
        "rotateZ" => rot_axis('Z', scalar(v)?),
        k if k.len() == 9 && k.starts_with("rotate") => {
            // rotateXYZ etc.: first letter is applied first
            let a = vec3_f64(v)?;
            let mut r = M4::identity();
            for ch in k[6..].chars() {
                let ang = match ch {
                    'X' => a[0],
                    'Y' => a[1],
                    _ => a[2],
                };
                r = rot_axis(ch, ang) * r;
            }
            r
        }
        "orient" => quat_matrix(v)?,
        "transform" => match v {
            Value::Matrix4d(m) => gf_to_col(m),
            _ => return None,
        },
        _ => return None,
    })
}