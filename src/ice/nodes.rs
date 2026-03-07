use bevy::prelude::Vec3;
use crate::types::{SubnetNodeType, SubnetValue};

pub fn evaluate_subnet_node(
    t:      &SubnetNodeType,
    inputs: &[SubnetValue],
) -> Vec<SubnetValue> {

    let v0 = || inputs.first().and_then(|v| v.as_vec3()).unwrap_or(Vec3::ZERO);
    let v1 = || inputs.get(1).and_then(|v| v.as_vec3()).unwrap_or(Vec3::ZERO);

    match t {
        SubnetNodeType::SubInput  => vec![],
        SubnetNodeType::SubOutput => vec![
            inputs.get(1).cloned()
                .unwrap_or_else(|| SubnetValue::Mesh(Default::default()))
        ],

        SubnetNodeType::AddVec3      => vec![SubnetValue::Vec3(v0() + v1())],
        SubnetNodeType::SubtractVec3 => vec![SubnetValue::Vec3(v0() - v1())],
        SubnetNodeType::CrossProduct => vec![SubnetValue::Vec3(v0().cross(v1()))],

        SubnetNodeType::MultiplyVec3 { scalar } =>
            vec![SubnetValue::Vec3(v0() * *scalar)],

        SubnetNodeType::Normalize => {
            let v = v0();
            let r = if v.length_squared() > 1e-10 { v.normalize() } else { Vec3::ZERO };
            vec![SubnetValue::Vec3(r)]
        }

        SubnetNodeType::DotProduct =>
            vec![SubnetValue::Float(v0().dot(v1()))],

        SubnetNodeType::LerpVec3 { t } =>
            vec![SubnetValue::Vec3(v0().lerp(v1(), *t))],

        // ── Constant nodes — ignore inputs, emit their stored value ──────────
        SubnetNodeType::ConstVec3  { value } => vec![SubnetValue::Vec3(*value)],
        SubnetNodeType::ConstFloat { value } => vec![SubnetValue::Float(*value)],
        // ConstInt outputs as Float so it can connect to Float sockets
        SubnetNodeType::ConstInt   { value } => vec![SubnetValue::Int(*value)],
    }
}