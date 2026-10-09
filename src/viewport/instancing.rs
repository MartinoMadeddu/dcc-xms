//! GPU instancing for packed primitives.
//!
//! An entity with a mesh and `GpuInstances` is drawn once per copy in a
//! single draw call: the copies' matrices and colours go to the GPU as a
//! second vertex buffer, stepped per instance. This is Bevy's
//! `custom_shader_instancing` example, adapted: full matrices instead of a
//! position and scale, a little shading, lines as well as surfaces, and the
//! buffer kept across frames instead of made again every frame.
//!
//! Copies drawn this way get the look's colour but not its textures: they
//! are for scenes with many copies, where seeing their shape and colour is
//! what matters. A mesh with few copies is drawn the ordinary way, textures
//! and all (see `draw_instanced` in `main.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::{
    asset::load_internal_asset,
    core_pipeline::core_3d::Transparent3d,
    ecs::{
        query::QueryItem,
        system::{lifetimeless::*, SystemParamItem},
    },
    pbr::{MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshBindGroup, SetMeshViewBindGroup},
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        mesh::{GpuBufferInfo, GpuMesh, MeshVertexBufferLayoutRef},
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand, RenderCommandResult,
            SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::*,
        renderer::RenderDevice,
        view::ExtractedView,
        Render, RenderApp, RenderSet,
    },
};

const SHADER: Handle<Shader> = Handle::weak_from_u128(0x5a1c_3e0b_9d47_4f6a_b2c8_71e4_0d93_6b21);

/// One copy: a column-major matrix, then a linear colour whose alpha says
/// whether it is lit (1) or flat (0).
pub type InstanceCopy = [f32; 20];

pub fn copy(m: &Mat4, color: [f32; 3], lit: bool) -> InstanceCopy {
    let mut c = [0.0; 20];
    c[..16].copy_from_slice(&m.to_cols_array());
    c[16..19].copy_from_slice(&color);
    c[19] = if lit { 1.0 } else { 0.0 };
    c
}

/// The copies of an entity's mesh. Shared, so handing it to the renderer
/// every frame costs nothing, and its buffer is made once.
#[derive(Component, Clone)]
pub struct GpuInstances(pub Arc<[InstanceCopy]>);

impl ExtractComponent for GpuInstances {
    type QueryData = &'static GpuInstances;
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self> {
        Some(item.clone())
    }
}

pub struct InstancingPlugin;

impl Plugin for InstancingPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "instancing.wgsl", Shader::from_wgsl);
        app.add_plugins(ExtractComponentPlugin::<GpuInstances>::default());
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawInstanced>()
            .init_resource::<SpecializedMeshPipelines<InstancedPipeline>>()
            .init_resource::<KeptBuffers>()
            .add_systems(Render, (
                queue_instanced.in_set(RenderSet::QueueMeshes),
                prepare_buffers.in_set(RenderSet::PrepareResources),
            ));
    }

    fn finish(&self, app: &mut App) {
        app.sub_app_mut(RenderApp).init_resource::<InstancedPipeline>();
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_instanced(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    pipeline:       Res<InstancedPipeline>,
    msaa:           Res<Msaa>,
    mut pipelines:  ResMut<SpecializedMeshPipelines<InstancedPipeline>>,
    cache:          Res<PipelineCache>,
    meshes:         Res<RenderAssets<GpuMesh>>,
    mesh_instances: Res<RenderMeshInstances>,
    instanced:      Query<Entity, With<GpuInstances>>,
    mut phases:     ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views:          Query<(Entity, &ExtractedView)>,
) {
    let draw = draw_functions.read().id::<DrawInstanced>();
    let msaa_key = MeshPipelineKey::from_msaa_samples(msaa.samples());
    for (view_entity, view) in &views {
        let Some(phase) = phases.get_mut(&view_entity) else { continue };
        let view_key = msaa_key | MeshPipelineKey::from_hdr(view.hdr);
        let rangefinder = view.rangefinder3d();
        for entity in &instanced {
            let Some(mesh_instance) = mesh_instances.render_mesh_queue_data(entity) else { continue };
            let Some(mesh) = meshes.get(mesh_instance.mesh_asset_id) else { continue };
            let key = view_key | MeshPipelineKey::from_primitive_topology(mesh.primitive_topology());
            let Ok(id) = pipelines.specialize(&cache, &pipeline, key, &mesh.layout) else { continue };
            phase.add(Transparent3d {
                entity,
                pipeline: id,
                draw_function: draw,
                distance: rangefinder.distance_translation(&mesh_instance.translation),
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::NONE,
            });
        }
    }
}

#[derive(Component)]
struct InstanceBuffer {
    buffer: Buffer,
    length: usize,
}

/// Buffers by the copies they were made from. The render world's entities
/// are made again every frame; the buffers are not.
#[derive(Resource, Default)]
struct KeptBuffers(HashMap<usize, (Arc<[InstanceCopy]>, Buffer)>);

fn prepare_buffers(
    mut commands: Commands,
    query:        Query<(Entity, &GpuInstances)>,
    device:       Res<RenderDevice>,
    mut kept:     ResMut<KeptBuffers>,
) {
    let mut used = std::collections::HashSet::new();
    for (entity, copies) in &query {
        let key = Arc::as_ptr(&copies.0) as *const f32 as usize;
        used.insert(key);
        let buffer = match kept.0.get(&key) {
            Some((data, buffer)) if Arc::ptr_eq(data, &copies.0) => buffer.clone(),
            _ => {
                let bytes: Vec<u8> = copies.0.iter().flat_map(|c| c.iter().flat_map(|f| f.to_ne_bytes())).collect();
                let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("imago instanced copies"),
                    contents: &bytes,
                    usage: BufferUsages::VERTEX,
                });
                kept.0.insert(key, (copies.0.clone(), buffer.clone()));
                buffer
            }
        };
        commands.entity(entity).insert(InstanceBuffer { buffer, length: copies.0.len() });
    }
    kept.0.retain(|k, _| used.contains(k));
}

#[derive(Resource)]
struct InstancedPipeline {
    mesh_pipeline: MeshPipeline,
}

impl FromWorld for InstancedPipeline {
    fn from_world(world: &mut World) -> Self {
        InstancedPipeline { mesh_pipeline: world.resource::<MeshPipeline>().clone() }
    }
}

impl SpecializedMeshPipeline for InstancedPipeline {
    type Key = MeshPipelineKey;

    fn specialize(&self, key: Self::Key, layout: &MeshVertexBufferLayoutRef) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;
        descriptor.vertex.shader = SHADER;
        let column = VertexFormat::Float32x4.size();
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceCopy>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: (0..5).map(|i| VertexAttribute {
                format: VertexFormat::Float32x4,
                offset: column * i as u64,
                shader_location: 8 + i,
            }).collect(),
        });
        if let Some(fragment) = descriptor.fragment.as_mut() { fragment.shader = SHADER; }
        // Lines are pulled a little towards the camera, so the edges win
        // against the surface they lie on.
        if key.primitive_topology() == PrimitiveTopology::LineList {
            descriptor.vertex.shader_defs.push("INSTANCED_LINES".into());
        }
        // Both sides: instanced copies may be mirrored, and USD surfaces are
        // often single sheets.
        descriptor.primitive.cull_mode = None;
        Ok(descriptor)
    }
}

type DrawInstanced = (SetItemPipeline, SetMeshViewBindGroup<0>, SetMeshBindGroup<1>, DrawMeshInstanced);

struct DrawMeshInstanced;

impl<P: PhaseItem> RenderCommand<P> for DrawMeshInstanced {
    type Param = (SRes<RenderAssets<GpuMesh>>, SRes<RenderMeshInstances>);
    type ViewQuery = ();
    type ItemQuery = Read<InstanceBuffer>;

    #[inline]
    fn render<'w>(
        item:            &P,
        _view:           (),
        instance_buffer: Option<&'w InstanceBuffer>,
        (meshes, mesh_instances): SystemParamItem<'w, '_, Self::Param>,
        pass:            &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(mesh_instance) = mesh_instances.render_mesh_queue_data(item.entity()) else { return RenderCommandResult::Failure };
        let Some(gpu_mesh) = meshes.into_inner().get(mesh_instance.mesh_asset_id) else { return RenderCommandResult::Failure };
        let Some(instance_buffer) = instance_buffer else { return RenderCommandResult::Failure };
        pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.buffer.slice(..));
        let copies = 0..instance_buffer.length as u32;
        match &gpu_mesh.buffer_info {
            GpuBufferInfo::Indexed { buffer, index_format, count } => {
                pass.set_index_buffer(buffer.slice(..), 0, *index_format);
                pass.draw_indexed(0..*count, 0, copies);
            }
            GpuBufferInfo::NonIndexed => pass.draw(0..gpu_mesh.vertex_count, copies),
        }
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_the_matrix_by_columns_then_the_colour() {
        let m = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let c = copy(&m, [0.1, 0.2, 0.3], true);
        assert_eq!(&c[12..15], &[1.0, 2.0, 3.0]);
        assert_eq!(&c[16..20], &[0.1, 0.2, 0.3, 1.0]);
        assert_eq!(std::mem::size_of::<InstanceCopy>(), 80);
    }
}
