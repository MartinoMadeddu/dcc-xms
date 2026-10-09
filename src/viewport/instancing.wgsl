// Instanced packed primitives: one mesh, drawn once per copy. Each copy
// brings its own matrix and colour in a second vertex buffer, stepped per
// instance. Shading is a simple key light and sky, two-sided; a colour with
// alpha 0 is drawn flat (lines, hidden-line surfaces).

#import bevy_pbr::view_transformations::position_world_to_clip

struct Vertex {
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
    // Bevy's mesh attributes use locations 0 to 7: the copies start at 8.
    @location(8)  m0: vec4<f32>,
    @location(9)  m1: vec4<f32>,
    @location(10) m2: vec4<f32>,
    @location(11) m3: vec4<f32>,
    @location(12) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    let m = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let world = m * vec4<f32>(v.position, 1.0);
    var out: VertexOutput;
    out.clip_position = position_world_to_clip(world.xyz);
#ifdef INSTANCED_LINES
    // Reversed depth: larger is nearer.
    out.clip_position.z += 2e-5 * out.clip_position.w;
#endif
    out.color = v.color;
#ifdef VERTEX_NORMALS
    out.normal = (m * vec4<f32>(v.normal, 0.0)).xyz;
#else
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if in.color.a < 0.5 {
        return vec4<f32>(in.color.rgb, 1.0);
    }
    let len = length(in.normal);
    let n = select(vec3<f32>(0.0, 1.0, 0.0), in.normal / len, len > 1e-8);
    let key = abs(dot(n, normalize(vec3<f32>(0.4, 0.8, 0.45))));
    let sky = 0.5 + 0.5 * abs(n.y);
    return vec4<f32>(in.color.rgb * (0.22 + 0.18 * sky + 0.6 * key), 1.0);
}
