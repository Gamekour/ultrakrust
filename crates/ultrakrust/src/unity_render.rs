//! Draws the level with ULTRAKILL's own compiled shaders.
//!
//! Each renderer's material picks its variant the way Unity does (material keywords); the variant's
//! Vulkan program (SMOL-V -> SPIR-V) is translated to WGSL with naga and becomes a pipeline whose
//! bind group layouts come from the shader itself. Every frame the constant buffers are filled by
//! name from the parameter layout: Unity built-ins (matrices, `_Time`, fog, the 8 vertex lights of the
//! `Vertex` light mode) or material properties.
//!
//! The project renders in gamma space, so the scene is drawn into an offscreen `Rgba8Unorm` target
//! (blending in gamma, as Unity does) and composited onto the camera's view converted to linear, so
//! the displayed pixel equals the shader's output.
use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::Core3dSystems;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::Shader;
use std::collections::HashMap;
use std::sync::Arc;
use uk_assets::db::AssetDb;
use uk_assets::scenedef::SceneDef;
use uk_assets::shader::{MaterialProps, Params, ShaderAsset};

/// Marks the camera whose view the Unity-shaded scene is composited onto.
#[derive(Component, Clone, Copy, Default, bevy::render::extract_component::ExtractComponent)]
pub struct UnityCamera;

/// The player's Main Camera (FirstRoom/Player/Main Camera in every level): clip planes and clear.
/// Clear flags 2 = solid color, background black; skies are level geometry.
pub const NEAR: f32 = 0.1;
pub const FAR: f32 = 4000.0;
pub const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub struct TexCpu {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Unity FilterMode: 0 point, 1 bilinear, 2 trilinear.
    pub filter: i64,
    /// Unity TextureWrapMode: 0 repeat, 1 clamp, 2 mirror, 3 mirror once.
    pub wrap: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexDim {
    D2,
    D3,
    Cube,
}

/// One bind group entry the shader declares.
#[derive(Clone, Debug)]
pub enum Slot {
    Uniform { binding: u32, cb: usize },
    Texture { binding: u32, dim: TexDim, name: String },
    Sampler { binding: u32, comparison: bool },
    /// Read-only storage buffer (e.g. ULTRAKILL's `_CausticVolumeData`); bound zeroed until its producer is ported.
    Storage { binding: u32 },
}

pub struct Variant {
    pub label: String,
    pub vs: Handle<Shader>,
    pub fs: Handle<Shader>,
    /// group 0 (textures, samplers) and group 1 (constant buffers).
    pub groups: [Vec<Slot>; 2],
    pub params: Params,
    /// (location, Unity channel, component count)
    pub inputs: Vec<(u32, u32, u32)>,
    pub stride: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DrawState {
    pub cull: u8,
    pub zwrite: bool,
    pub ztest: u8,
    pub src: u8,
    pub dst: u8,
    pub src_a: u8,
    pub dst_a: u8,
    pub op: u8,
    pub mask: u8,
}

pub struct Draw {
    pub node: u32,
    pub enabled: bool,
    pub variant: u32,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub state: DrawState,
    pub queue: i64,
    pub material: Arc<MaterialProps>,
    /// texture binding name -> scene texture index
    pub textures: HashMap<String, u32>,
    /// Unity-space bounds centre and radius (light selection, sorting).
    pub center: Vec3,
    pub radius: f32,
}

pub struct LightCpu {
    pub node: u32,
    pub kind: u8,
    pub color: [f32; 4],
    pub intensity: f32,
    pub range: f32,
    pub spot_angle: f32,
    pub enabled: bool,
}

/// Everything static about the level's Unity-shaded scene.
pub struct SceneData {
    pub generation: u64,
    pub variants: Vec<Variant>,
    pub draws: Vec<Draw>,
    pub textures: Vec<TexCpu>,
    pub lights: Vec<LightCpu>,
    pub fog: (bool, [f32; 4], f32, f32),
    pub ambient: [f32; 4],
    pub clear: [f32; 4],
    pub composite_shader: Handle<Shader>,
    /// ULTRAKILL's final composite (PostProcessV2_Handler.postProcessV2_VSRM on the Virtual Camera quad).
    pub post: Option<PostDef>,
}

pub struct PostDef {
    pub variant: u32,
    pub material: Arc<MaterialProps>,
    /// PostProcessV2_Handler.ditherTexture (scene texture index).
    pub dither: Option<u32>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct UnityScene(pub Option<Arc<SceneData>>);

/// Per-frame dynamic state from the game.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct UnityFrame {
    pub time: f32,
    pub visible: Arc<Vec<bool>>,
    /// Unity-space object-to-world per draw (identity unless moved by a mover).
    pub object_to_world: Arc<Vec<Mat4>>,
    /// Unity-space light world positions / forward directions, and whether each is active.
    pub lights: Arc<Vec<(Vec3, Vec3, bool)>>,
}

pub struct UnityRenderPlugin;

impl Plugin for UnityRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnityScene>()
            .init_resource::<UnityFrame>()
            .add_plugins((
                ExtractResourcePlugin::<UnityScene>::default(),
                ExtractResourcePlugin::<UnityFrame>::default(),
                bevy::render::extract_component::ExtractComponentPlugin::<UnityCamera>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_resource::<UnityGpu>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_systems(Render, frame_stats.in_set(RenderSystems::Cleanup))
            .add_systems(Core3d, draw.before(main_opaque_pass_3d).in_set(Core3dSystems::MainPass));
    }
}

// ---------------------------------------------------------------------------------------------
// Building the scene (main world, at level load)

fn unity_point(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

fn parse_spirv(words: &[u32]) -> Result<naga::Module, String> {
    let words = uk_assets::spirv::prepare(words);
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    naga::front::spv::parse_u8_slice(&bytes, &naga::front::spv::Options::default()).map_err(|e| format!("parse: {e}"))
}

fn to_wgsl(m: &naga::Module) -> Result<String, String> {
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(m)
        .map_err(|e| format!("validate: {}", e.into_inner()))?;
    naga::back::wgsl::write_string(m, &info, naga::back::wgsl::WriterFlags::empty()).map_err(|e| format!("wgsl: {e}"))
}

/// Unity's Vulkan stage pairs may leave fragment inputs unwritten (undefined values); WebGPU requires
/// every fragment input to be a vertex output. Add the missing outputs to the vertex entry point,
/// with the fragment's exact type and interpolation, written as zero.
fn link_stages(vm: &mut naga::Module, fm: &naga::Module) -> Result<usize, String> {
    use naga::{Binding, Expression, Statement, StructMember, Type, TypeInner};
    let fe = fm.entry_points.iter().find(|e| e.stage == naga::ShaderStage::Fragment).ok_or("no fragment entry")?;
    let wanted: Vec<(Binding, TypeInner)> = fe
        .function
        .arguments
        .iter()
        .filter(|a| matches!(a.binding, Some(Binding::Location { .. })))
        .map(|a| (a.binding.clone().unwrap(), fm.types[a.ty].inner.clone()))
        .collect();
    let vi = vm.entry_points.iter().position(|e| e.stage == naga::ShaderStage::Vertex).ok_or("no vertex entry")?;
    let Some(result) = vm.entry_points[vi].function.result.clone() else { return Ok(0) };
    let TypeInner::Struct { members, span } = vm.types[result.ty].inner.clone() else { return Ok(0) };
    let loc = |b: &Binding| match b {
        Binding::Location { location, .. } => Some(*location),
        _ => None,
    };
    let missing: Vec<&(Binding, TypeInner)> = wanted
        .iter()
        .filter(|(b, _)| !members.iter().any(|m| m.binding.as_ref().and_then(loc) == loc(b)))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let mut new_members = members.clone();
    let mut new_span = span;
    let mut add_types = Vec::new();
    for (b, inner) in &missing {
        let ty = vm.types.insert(Type { name: None, inner: inner.clone() }, naga::Span::UNDEFINED);
        let size = inner.size(vm.to_ctx());
        new_span = new_span.next_multiple_of(16);
        new_members.push(StructMember { name: None, ty, binding: Some(b.clone()), offset: new_span });
        new_span += size;
        add_types.push(ty);
    }
    let new_ty = vm.types.insert(
        Type { name: vm.types[result.ty].name.clone(), inner: TypeInner::Struct { members: new_members, span: new_span.next_multiple_of(16) } },
        naga::Span::UNDEFINED,
    );
    let f = &mut vm.entry_points[vi].function;
    f.result = Some(naga::FunctionResult { ty: new_ty, binding: None });
    // every `return Compose(old struct, ...)` gets zero components for the new members
    let mut returns = Vec::new();
    fn collect(block: &naga::Block, out: &mut Vec<naga::Handle<Expression>>) {
        for s in block.iter() {
            match s {
                Statement::Return { value: Some(v) } => out.push(*v),
                Statement::Block(b) => collect(b, out),
                Statement::If { accept, reject, .. } => {
                    collect(accept, out);
                    collect(reject, out);
                }
                Statement::Loop { body, continuing, .. } => {
                    collect(body, out);
                    collect(continuing, out);
                }
                Statement::Switch { cases, .. } => cases.iter().for_each(|c| collect(&c.body, out)),
                _ => {}
            }
        }
    }
    collect(&f.body, &mut returns);
    for r in returns {
        let zeros: Vec<_> = add_types.iter().map(|&ty| f.expressions.append(Expression::ZeroValue(ty), naga::Span::UNDEFINED)).collect();
        if let Expression::Compose { ty, components } = &mut f.expressions[r] {
            *ty = new_ty;
            components.extend(zeros);
        } else {
            return Err("vertex entry does not return a composed struct".into());
        }
    }
    Ok(missing.len())
}

/// Bind slots of one module, from naga's view of its globals.
fn module_slots(m: &naga::Module, params: &Params, slots: &mut [Vec<Slot>; 2]) {
    for (_, g) in m.global_variables.iter() {
        let Some(b) = &g.binding else { continue };
        let group = b.group as usize;
        if group > 1 {
            continue;
        }
        let already = slots[group].iter().any(|s| match s {
            Slot::Uniform { binding, .. } | Slot::Texture { binding, .. } | Slot::Sampler { binding, .. } | Slot::Storage { binding } => *binding == b.binding,
        });
        if already {
            continue;
        }
        let packed_matches = |p: u32| ((p >> 16) & 0xff) == b.group && (p & 0xffff) == b.binding;
        let slot = match &m.types[g.ty].inner {
            naga::TypeInner::Image { dim, .. } => {
                let name = params.bindings.iter().find(|x| x.kind == 0 && packed_matches(x.packed)).map(|x| x.name.clone()).unwrap_or_default();
                let dim = match dim {
                    naga::ImageDimension::Cube => TexDim::Cube,
                    naga::ImageDimension::D3 => TexDim::D3,
                    _ => TexDim::D2,
                };
                Slot::Texture { binding: b.binding, dim, name }
            }
            naga::TypeInner::Sampler { comparison } => Slot::Sampler { binding: b.binding, comparison: *comparison },
            _ if matches!(g.space, naga::AddressSpace::Storage { .. }) => Slot::Storage { binding: b.binding },
            _ if g.space == naga::AddressSpace::Uniform => {
                let name = params.bindings.iter().find(|x| x.kind == 1 && packed_matches(x.packed)).map(|x| x.name.clone()).unwrap_or_default();
                let cb = params.constant_buffers.iter().position(|c| c.name == name).unwrap_or(usize::MAX);
                Slot::Uniform { binding: b.binding, cb }
            }
            _ => continue,
        };
        slots[group].push(slot);
    }
}

/// Translates one Vulkan variant (vertex entry holding both stages) into WGSL pipelines' parts.
fn make_variant(sh: &ShaderAsset, vsub: &uk_assets::shader::SubProgram, shaders: &mut Assets<Shader>, index: usize) -> Result<Variant, String> {
        let prog = sh.program(vsub.blob).map_err(|e| e.0)?;
        let params = sh.params(vsub.params).map_err(|e| e.0)?;
        let vw = prog.stages.first().cloned().flatten().ok_or("no vertex stage")?;
        let fw = prog.stages.get(1).cloned().flatten().ok_or("no fragment stage")?;
        let mut vm = parse_spirv(&vw)?;
        let fm = parse_spirv(&fw)?;
        link_stages(&mut vm, &fm)?;
        let vsrc = to_wgsl(&vm)?;
        let mut fsrc = to_wgsl(&fm)?;
        // Diagnostics: UNITY_DEBUG_FS="<wgsl expr over the fragment's inputs>" replaces the
        // color output, so frame statistics can show which input is wrong (no screenshots).
        let debug_this = std::env::var("UNITY_DEBUG_VARIANT").ok().and_then(|s| s.parse::<u32>().ok()) == Some(vsub.blob);
        if debug_this {
            for cb in &params.constant_buffers {
                eprintln!("CB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}{}", p.name, p.offset, if p.array_size > 0 { format!("[{}]", p.array_size) } else { String::new() })).collect::<Vec<_>>().join(" "));
            }
            eprintln!("keywords {:?}", prog.keywords);
        }
        if let (Ok(expr), true) = (std::env::var("UNITY_DEBUG_FS"), debug_this) {
            if let Some(i) = fsrc.rfind("return FragmentOutput(") {
                let start = i + "return FragmentOutput(".len();
                let end = fsrc[start..].find([',', ')']).map(|e| start + e).unwrap_or(start);
                fsrc.replace_range(start..end, &format!("vec4<f32>(({expr}).xyz, 1f)"));
            }
        }
        let mut groups: [Vec<Slot>; 2] = [Vec::new(), Vec::new()];
        module_slots(&vm, &params, &mut groups);
        module_slots(&fm, &params, &mut groups);
        if std::env::var_os("UNITY_DEBUG").is_some() && std::env::var("UNITY_DEBUG_VARIANT").ok().and_then(|s| s.parse::<u32>().ok()).is_none_or(|b| b == vsub.blob) {
            eprintln!("variant {}#{}: group0 {:?}\n   group1 {:?}", sh.name, vsub.blob, groups[0], groups[1]);
            for (_, g) in fm.global_variables.iter().filter(|(_, g)| g.binding.is_some()) {
                eprintln!("   fs global {:?} {:?} {:?}", g.binding, g.space, fm.types[g.ty].inner);
            }
        }
        // vertex inputs: component count from the entry point argument types
        let ep = vm.entry_points.iter().find(|e| e.stage == naga::ShaderStage::Vertex).ok_or("no vertex entry")?;
        let mut inputs = Vec::new();
        for a in &ep.function.arguments {
            let Some(naga::Binding::Location { location, .. }) = a.binding else { continue };
            let n = match vm.types[a.ty].inner {
                naga::TypeInner::Vector { size, .. } => size as u32,
                _ => 1,
            };
            let ch = prog.channels.iter().find(|c| c.1 == location).map(|c| c.0).unwrap_or(0);
            inputs.push((location, ch, n));
        }
        inputs.sort();
        let stride = inputs.iter().map(|i| i.2 as u64 * 4).sum();
        // unique per variant: levels can carry several copies of one shader (same name and
        // blob index, different programs), and pipeline/layout caches must not mix them up
        let label = format!("{}#{}@{}", sh.name, vsub.blob, index);
        Ok(Variant {
            vs: shaders.add(Shader::from_wgsl(vsrc, format!("unity/{label}/vs.wgsl"))),
            fs: shaders.add(Shader::from_wgsl(fsrc, format!("unity/{label}/fs.wgsl"))),
            label,
            groups,
            params,
            inputs,
            stride,
        })
}

fn state_u8(v: &uk_assets::shader::StateValue, m: &MaterialProps) -> u8 {
    v.resolve(&m.floats).round().clamp(0.0, 255.0) as u8
}

/// Builds the Unity-shaded scene for a loaded level. Renderers whose shader cannot be translated
/// yet are skipped and counted in the returned summary.
pub fn build(
    db: &mut AssetDb,
    def: &SceneDef,
    skip_node: impl Fn(u32) -> bool,
    shaders: &mut Assets<Shader>,
    generation: u64,
) -> (SceneData, String) {
    let mut shader_assets: HashMap<(String, i64), Option<Arc<ShaderAsset>>> = HashMap::new();
    let mut variants: Vec<Variant> = Vec::new();
    let mut variant_ids: HashMap<(String, i64, u32), Option<u32>> = HashMap::new();
    let mut materials: HashMap<(String, i64), Option<(Arc<MaterialProps>, (String, i64))>> = HashMap::new();
    let mut textures: Vec<TexCpu> = Vec::new();
    let mut texture_ids: HashMap<(String, i64), Option<u32>> = HashMap::new();
    let mut draws = Vec::new();
    let mut skipped: HashMap<String, usize> = HashMap::new();
    let mut chosen: HashMap<(String, i64), Option<uk_assets::shader::SubProgram>> = HashMap::new();
    for r in &def.renderers {
        if r.batch.indices.is_empty() || skip_node(r.node) {
            continue;
        }
        let Some(key) = &r.material else { continue };
        let mat = materials
            .entry((key.file.clone(), key.path_id))
            .or_insert_with(|| {
                let f = db.file(&key.file).ok()?;
                let v = f.read_id(key.path_id).ok()?;
                let props = MaterialProps::from_value(&v);
                let (sf, sid) = db.resolve(&f, props.shader).ok().flatten()?;
                Some((Arc::new(props), (sf.name.clone(), sid)))
            })
            .clone();
        let Some((mat, skey)) = mat else {
            *skipped.entry("material unreadable".into()).or_default() += 1;
            continue;
        };
        let sh = shader_assets
            .entry(skey.clone())
            .or_insert_with(|| {
                let f = db.file(&skey.0).ok()?;
                let v = f.read_id(skey.1).ok()?;
                ShaderAsset::from_value(&v).ok().map(Arc::new)
            })
            .clone();
        let Some(sh) = sh else {
            *skipped.entry("shader without Vulkan programs".into()).or_default() += 1;
            continue;
        };
        let Some(pass) = sh.passes.first() else { continue };
        // variant choice is per material (keywords), not per renderer
        let vsub = match chosen.get(&(key.file.clone(), key.path_id)) {
            Some(s) => s.clone(),
            None => {
                let enabled: Vec<&str> = mat.keywords.iter().map(|s| s.as_str()).collect();
                let s = sh.select(&pass.vertex, &enabled).cloned();
                chosen.insert((key.file.clone(), key.path_id), s.clone());
                s
            }
        };
        let Some(vsub) = vsub else {
            *skipped.entry(format!("{}: no variant", sh.name)).or_default() += 1;
            continue;
        };
        let vid = *variant_ids.entry((skey.0.clone(), skey.1, vsub.blob)).or_insert_with(|| {
            let res = make_variant(&sh, &vsub, shaders, variants.len());
            match res {
                Ok(v) => {
                    variants.push(v);
                    Some(variants.len() as u32 - 1)
                }
                Err(e) => {
                    *skipped.entry(format!("{}: {}", sh.name, e.chars().take(60).collect::<String>())).or_default() += 1;
                    None
                }
            }
        });
        let Some(vid) = vid else { continue };
        if let Some(only) = std::env::var("UNITY_DEBUG_VARIANT").ok().and_then(|s| s.parse::<u32>().ok()) {
            if !variants[vid as usize].label.contains(&format!("#{only}@")) {
                continue;
            }
        }
        let var = &variants[vid as usize];
        // vertex data packed in location order, matching the shader's input types
        let b = &r.batch;
        if var.stride == 0 || b.positions.is_empty() {
            *skipped.entry(format!("{}: no vertex inputs", var.label)).or_default() += 1;
            continue;
        }
        let mut vertices = Vec::with_capacity(b.positions.len() * var.stride as usize);
        for i in 0..b.positions.len() {
            for &(_, ch, n) in &var.inputs {
                let p = unity_point(b.positions[i]);
                let nrm = unity_point(b.normals[i]);
                let v: [f32; 4] = match ch {
                    0 => [p[0], p[1], p[2], 1.0],
                    1 => [nrm[0], nrm[1], nrm[2], 0.0],
                    2 => [1.0, 0.0, 0.0, 1.0],
                    3 => b.colors.get(i).copied().unwrap_or([1.0; 4]),
                    _ => {
                        let uv = b.uvs[i];
                        [uv[0], uv[1], 0.0, 0.0]
                    }
                };
                for c in v.iter().take(n as usize) {
                    vertices.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        // textures the variant samples
        let mut tex = HashMap::new();
        for s in var.groups[0].iter() {
            let Slot::Texture { name, dim: TexDim::D2, .. } = s else { continue };
            let Some(env) = mat.textures.get(name) else { continue };
            let f = db.file(&key.file).ok();
            let tid = f.and_then(|f| db.resolve(&f, env.texture).ok().flatten()).and_then(|(tf, tid)| {
                *texture_ids.entry((tf.name.clone(), tid)).or_insert_with(|| {
                    let v = tf.read_id(tid).ok()?;
                    let t = uk_assets::texture::decode_texture(db, &v).ok()?;
                    textures.push(TexCpu { width: t.width, height: t.height, rgba: t.rgba, filter: t.filter, wrap: t.wrap });
                    Some(textures.len() as u32 - 1)
                })
            });
            if let Some(t) = tid {
                tex.insert(name.clone(), t);
            }
        }
        let st = &pass.state;
        let state = DrawState {
            cull: state_u8(&st.cull, &mat),
            zwrite: st.zwrite.resolve(&mat.floats) >= 0.5,
            ztest: state_u8(&st.ztest, &mat),
            src: state_u8(&st.src_blend, &mat),
            dst: state_u8(&st.dst_blend, &mat),
            src_a: state_u8(&st.src_blend_alpha, &mat),
            dst_a: state_u8(&st.dst_blend_alpha, &mat),
            op: state_u8(&st.blend_op, &mat),
            mask: state_u8(&st.color_mask, &mat),
        };
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &b.positions {
            let p = Vec3::from(unity_point(*p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let queue = if mat.render_queue >= 0 {
            mat.render_queue
        } else if state.src != 1 || state.dst != 0 {
            3000
        } else {
            2000
        };
        draws.push(Draw {
            node: r.node,
            enabled: r.enabled,
            variant: vid,
            vertices,
            indices: b.indices.clone(),
            state,
            queue,
            material: mat.clone(),
            textures: tex,
            center: (lo + hi) * 0.5,
            radius: (hi - lo).length() * 0.5,
        });
    }
    // the final composite: GameController's PostProcessV2_Handler names the material and dither texture
    let post = (|| {
        let h = def.scripts.iter().find(|s| s.class == "PostProcessV2_Handler")?;
        let f = db.file(&def.scene_file).ok()?;
        let (mf, mid) = db.resolve(&f, h.data.get("postProcessV2_VSRM").pptr()).ok().flatten()?;
        let props = MaterialProps::from_value(&mf.read_id(mid).ok()?);
        let (sf, sid) = db.resolve(&mf, props.shader).ok().flatten()?;
        let sh = ShaderAsset::from_value(&sf.read_id(sid).ok()?).ok()?;
        let kw: Vec<&str> = props.keywords.iter().map(|s| s.as_str()).collect();
        let vsub = sh.select(&sh.passes.first()?.vertex, &kw)?.clone();
        let var = make_variant(&sh, &vsub, shaders, variants.len()).map_err(|e| warn!("post-process variant: {e}")).ok()?;
        variants.push(var);
        let dither = db.resolve(&f, h.data.get("ditherTexture").pptr()).ok().flatten().and_then(|(tf, tid)| {
            let t = uk_assets::texture::decode_texture(db, &tf.read_id(tid).ok()?).ok()?;
            textures.push(TexCpu { width: t.width, height: t.height, rgba: t.rgba, filter: t.filter, wrap: t.wrap });
            Some(textures.len() as u32 - 1)
        });
        Some(PostDef { variant: variants.len() as u32 - 1, material: Arc::new(props), dither })
    })();
    let rs = &def.render_settings;
    let lights = def
        .lights
        .iter()
        .map(|l| LightCpu { node: l.node, kind: l.kind, color: l.color, intensity: l.intensity, range: l.range, spot_angle: l.spot_angle, enabled: l.enabled })
        .collect();
    let total: usize = draws.len() + skipped.values().sum::<usize>();
    let mut sk: Vec<_> = skipped.into_iter().collect();
    sk.sort_by(|a, b| b.1.cmp(&a.1));
    let summary = format!(
        "unity shaders: post-process {}, {} variants, {}/{} renderers drawn, {} textures, {} lights; skipped: {:?}",
        post.as_ref().map_or("missing".to_string(), |p| format!("{} dither {:?}", variants[p.variant as usize].label, p.dither.map(|t| (textures[t as usize].width, textures[t as usize].height)))),
        variants.len(),
        draws.len(),
        total,
        textures.len(),
        def.lights.len(),
        sk.iter().take(6).collect::<Vec<_>>()
    );
    let scene = SceneData {
        generation,
        variants,
        draws,
        textures,
        lights,
        fog: (rs.fog, rs.fog_color, rs.fog_start, rs.fog_end),
        ambient: rs.ambient_sky,
        clear: CLEAR,
        composite_shader: shaders.add(Shader::from_wgsl(COMPOSITE_WGSL, "unity/composite.wgsl")),
        post,
    };
    (scene, summary)
}

/// Per-frame state from the game: what is visible, where movers have moved things, lights.
pub fn frame(game: &uk_game::Game, scene: &SceneData, time: f32) -> UnityFrame {
    let m = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    let visible = scene.draws.iter().map(|d| d.enabled && game.active(d.node)).collect();
    let object_to_world = scene
        .draws
        .iter()
        .map(|d| match game.node_mover[d.node as usize] {
            Some(mv) => m * Mat4::from(game.mover_delta(mv)) * m,
            None => Mat4::IDENTITY,
        })
        .collect();
    let lights = scene
        .lights
        .iter()
        .map(|l| {
            let w = game.def.nodes[l.node as usize].world0;
            let (_, rot, pos) = w.to_scale_rotation_translation();
            // world0 is Bevy space; Unity forward (+z) is Bevy -z
            let fwd = rot * Vec3::NEG_Z;
            (Vec3::new(pos.x, pos.y, -pos.z), Vec3::new(fwd.x, fwd.y, -fwd.z), l.enabled && game.active(l.node))
        })
        .collect();
    UnityFrame { time, visible: Arc::new(visible), object_to_world: Arc::new(object_to_world), lights: Arc::new(lights) }
}

// ---------------------------------------------------------------------------------------------
// Render world

struct GpuDraw {
    vbuf: Buffer,
    ibuf: Buffer,
    count: u32,
    /// (binding, byte offset in the shared uniform buffer, size, constant buffer index)
    ubufs: Vec<(u32, u64, u64, usize)>,
    bg0: BindGroup,
    bg1: BindGroup,
    pipeline: CachedRenderPipelineId,
}

#[derive(Resource, Default)]
struct UnityGpu {
    generation: u64,
    draws: Vec<Option<GpuDraw>>,
    layouts: Vec<[BindGroupLayoutDescriptor; 2]>,
    pipelines: HashMap<(u32, DrawState), CachedRenderPipelineId>,
    target: Option<(UVec2, Texture, TextureView, TextureView)>,
    /// `UNITY_FRAME_STATS`: one frame of the scene target copied back for numeric checks.
    readback: std::sync::Mutex<Option<(Buffer, u32, UVec2, bool)>>,
    frames: std::sync::atomic::AtomicU32,
    started: std::sync::Mutex<Option<std::time::Instant>>,
    /// Cached light choice per draw, valid for `light_key` (which lights are on).
    light_pick: Vec<Vec<usize>>,
    light_key: u64,
    /// Draws inside the view frustum this frame.
    in_view: Vec<bool>,
    /// One uniform buffer for every draw's constant buffers (256-byte aligned slices) and its CPU copy.
    ubo: Option<Buffer>,
    staging: Vec<u8>,
    composite: Option<(CachedRenderPipelineId, BindGroupLayoutDescriptor, Sampler)>,
    post: Option<PostGpu>,
    /// PostProcessV2 output (what the Virtual Camera puts on screen), same size as the scene target.
    post_target: Option<(Texture, TextureView)>,
    post_readback: std::sync::Mutex<Option<Buffer>>,
}

struct PostGpu {
    pipeline: CachedRenderPipelineId,
    layouts: [BindGroupLayoutDescriptor; 2],
    ubufs: Vec<(u32, Buffer, usize)>,
    vbuf: Buffer,
    dither: (TextureView, Sampler),
    main_sampler: Sampler,
}

/// PostProcessV2's constant buffers: the handler's globals at default settings (pixelization off ->
/// full resolution, colorCompression -> 32 levels, dithering 0.2, gamma 1, no hurt flash). The quad
/// is a clip-space triangle (identity matrices); `_ProjectionParams.x = -1` is Unity's flipped
/// render-texture convention, since the scene target is stored top row first.
fn fill_post_cb(out: &mut [u8], cb: &uk_assets::shader::ConstantBuffer, mat: &MaterialProps, size: UVec2) {
    out.fill(0);
    for p in &cb.params {
        let v: Vec<f32> = if p.is_matrix {
            Mat4::IDENTITY.to_cols_array().to_vec()
        } else {
            let v = match p.name.as_str() {
                "_ProjectionParams" => [-1.0, NEAR, FAR, 1.0 / FAR],
                "_VirtualRes" => [size.x as f32, size.y as f32, 0.0, 0.0],
                "_ColorPrecision" => [32.0, 0.0, 0.0, 0.0],
                "_DitherStrength" => [0.2, 0.0, 0.0, 0.0],
                "_Gamma" => [1.0, 0.0, 0.0, 0.0],
                "_HurtScreenColor" => [0.0; 4],
                n => mat.vector(n).unwrap_or([0.0; 4]),
            };
            v[..p.cols.clamp(1, 4) as usize].to_vec()
        };
        for (k, f) in v.iter().enumerate() {
            let o = p.offset as usize + k * 4;
            if o + 4 <= out.len() {
                let bytes = if p.ty == 1 { (*f as i32).to_le_bytes() } else { f.to_le_bytes() };
                out[o..o + 4].copy_from_slice(&bytes);
            }
        }
    }
}

const COLOR_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

fn blend_factor(f: u8) -> BlendFactor {
    match f {
        0 => BlendFactor::Zero,
        1 => BlendFactor::One,
        2 => BlendFactor::Dst,
        3 => BlendFactor::Src,
        4 => BlendFactor::OneMinusDst,
        5 => BlendFactor::SrcAlpha,
        6 => BlendFactor::OneMinusSrc,
        7 => BlendFactor::DstAlpha,
        8 => BlendFactor::OneMinusDstAlpha,
        9 => BlendFactor::SrcAlphaSaturated,
        _ => BlendFactor::OneMinusSrcAlpha,
    }
}

fn blend_op(o: u8) -> BlendOperation {
    match o {
        1 => BlendOperation::Subtract,
        2 => BlendOperation::ReverseSubtract,
        3 => BlendOperation::Min,
        4 => BlendOperation::Max,
        _ => BlendOperation::Add,
    }
}

/// Unity CompareFunction under reversed depth (near = 1).
fn depth_compare(z: u8) -> CompareFunction {
    match z {
        0 | 8 => CompareFunction::Always,
        1 => CompareFunction::Never,
        2 => CompareFunction::Greater,
        3 => CompareFunction::Equal,
        4 => CompareFunction::GreaterEqual,
        5 => CompareFunction::Less,
        6 => CompareFunction::NotEqual,
        7 => CompareFunction::LessEqual,
        _ => CompareFunction::GreaterEqual,
    }
}

fn layout_entries(slots: &[Slot]) -> Vec<BindGroupLayoutEntry> {
    slots
        .iter()
        .map(|s| {
            let (binding, ty) = match s {
                Slot::Uniform { binding, .. } => (*binding, BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                Slot::Texture { binding, dim, .. } => (
                    *binding,
                    BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: match dim {
                            TexDim::D2 => TextureViewDimension::D2,
                            TexDim::D3 => TextureViewDimension::D3,
                            TexDim::Cube => TextureViewDimension::Cube,
                        },
                        multisampled: false,
                    },
                ),
                Slot::Sampler { binding, comparison } => (
                    *binding,
                    BindingType::Sampler(if *comparison { SamplerBindingType::Comparison } else { SamplerBindingType::Filtering }),
                ),
                Slot::Storage { binding } => (*binding, BindingType::Buffer { ty: BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
            };
            BindGroupLayoutEntry { binding, visibility: ShaderStages::VERTEX_FRAGMENT, ty, count: None }
        })
        .collect()
}

fn sampler_for(dev: &RenderDevice, t: Option<&TexCpu>) -> Sampler {
    let (filter, wrap) = t.map_or((0, 0), |t| (t.filter, t.wrap));
    let mode = match wrap {
        1 => AddressMode::ClampToEdge,
        2 | 3 => AddressMode::MirrorRepeat,
        _ => AddressMode::Repeat,
    };
    let f = if filter == 0 { FilterMode::Nearest } else { FilterMode::Linear };
    dev.create_sampler(&SamplerDescriptor {
        label: Some("unity sampler"),
        address_mode_u: mode,
        address_mode_v: mode,
        address_mode_w: mode,
        mag_filter: f,
        min_filter: f,
        mipmap_filter: if filter == 2 { MipmapFilterMode::Linear } else { MipmapFilterMode::Nearest },
        ..default()
    })
}

fn upload_texture(dev: &RenderDevice, queue: &RenderQueue, t: &TexCpu) -> TextureView {
    let tex = dev.create_texture(&TextureDescriptor {
        label: Some("unity texture"),
        size: Extent3d { width: t.width.max(1), height: t.height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        &t.rgba,
        TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.width * 4), rows_per_image: Some(t.height) },
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
    );
    tex.create_view(&TextureViewDescriptor::default())
}

fn solid_view(dev: &RenderDevice, queue: &RenderQueue, rgba: [u8; 4], dim: TexDim) -> TextureView {
    let layers = if dim == TexDim::Cube { 6 } else { 1 };
    let tex = dev.create_texture(&TextureDescriptor {
        label: Some("unity default texture"),
        size: Extent3d { width: 1, height: 1, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if dim == TexDim::D3 { TextureDimension::D3 } else { TextureDimension::D2 },
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data: Vec<u8> = (0..layers).flat_map(|_| rgba).collect();
    queue.write_texture(
        tex.as_image_copy(),
        &data,
        TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        Extent3d { width: 1, height: 1, depth_or_array_layers: layers },
    );
    tex.create_view(&TextureViewDescriptor {
        dimension: Some(match dim {
            TexDim::Cube => TextureViewDimension::Cube,
            TexDim::D3 => TextureViewDimension::D3,
            TexDim::D2 => TextureViewDimension::D2,
        }),
        ..default()
    })
}

const COMPOSITE_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), uv);
}
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
    let c = textureSample(src, smp, v.uv);
    // the scene is in gamma space; the view target is linear, so decode exactly
    return vec4<f32>(to_linear(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
"#;

struct FrameCtx {
    vp: Mat4,
    v: Mat4,
    cam_pos: Vec3,
    time: f32,
    fog: (bool, [f32; 4], f32, f32),
    ambient: [f32; 4],
    near: f32,
    size: UVec2,
}

/// Unity's legacy vertex lights for one renderer: up to 8, directional first, then by brightness
/// at the renderer's bounds (Unity's exact importance ordering is engine-internal).
fn vertex_lights(scene: &SceneData, frame: &UnityFrame, d: &Draw, v: Mat4) -> [[[f32; 4]; 8]; 4] {
    light_block(scene, frame, &pick_lights(scene, frame, d), v)
}

/// Up to 8 lights for a renderer, most important first (directional, then brightness at its bounds).
/// Depends only on which lights are on and where they are, so it is cached per renderer.
fn pick_lights(scene: &SceneData, frame: &UnityFrame, d: &Draw) -> Vec<usize> {
    let mut picked: Vec<(f32, usize)> = Vec::new();
    for (i, l) in scene.lights.iter().enumerate() {
        let Some(&(pos, _, on)) = frame.lights.get(i) else { continue };
        if !on || l.intensity <= 0.0 {
            continue;
        }
        let score = if l.kind == 1 {
            f32::MAX
        } else {
            let dist = (pos.distance(d.center) - d.radius).max(0.0);
            if dist > l.range {
                continue;
            }
            let lum = l.color[0] * 0.3 + l.color[1] * 0.59 + l.color[2] * 0.11;
            lum * l.intensity / (1.0 + 25.0 * dist * dist / (l.range * l.range).max(1e-4))
        };
        picked.push((score, i));
    }
    picked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    picked.into_iter().take(8).map(|p| p.1).collect()
}

/// Unity's `Vertex`-mode light block (view space) for already-picked lights.
fn light_block(scene: &SceneData, frame: &UnityFrame, picked: &[usize], v: Mat4) -> [[[f32; 4]; 8]; 4] {
    let mut out = [[[0.0; 4]; 8]; 4]; // position, color, atten, spot direction
    for k in 0..8 {
        out[0][k] = [0.0, 0.0, 1.0, 0.0];
        out[2][k] = [-1.0, 1.0, 0.0, 1.0];
        out[3][k] = [0.0, 0.0, 1.0, 0.0];
    }
    for (k, &i) in picked.iter().take(8).enumerate() {
        let l = &scene.lights[i];
        let (pos, fwd, _) = frame.lights[i];
        out[1][k] = [l.color[0] * l.intensity, l.color[1] * l.intensity, l.color[2] * l.intensity, 1.0];
        if l.kind == 1 {
            let dir = v.transform_vector3(-fwd);
            out[0][k] = [dir.x, dir.y, dir.z, 0.0];
            out[2][k] = [-1.0, 1.0, 0.0, 1.0];
        } else {
            let p = v.transform_point3(pos);
            out[0][k] = [p.x, p.y, p.z, 1.0];
            let r2 = (l.range * l.range).max(1e-6);
            if l.kind == 0 {
                let cos_phi = (l.spot_angle.to_radians() * 0.5).cos();
                let cos_theta = (l.spot_angle.to_radians() * 0.25).cos();
                out[2][k] = [cos_phi, 1.0 / (cos_theta - cos_phi).max(1e-4), 25.0 / r2, r2];
                let sd = v.transform_vector3(-fwd).normalize_or_zero();
                out[3][k] = [sd.x, sd.y, sd.z, 0.0];
            } else {
                out[2][k] = [-1.0, 1.0, 25.0 / r2, r2];
            }
        }
    }
    out
}

/// Fills one constant buffer from built-ins and material properties.
fn fill_cb(out: &mut [u8], cb: &uk_assets::shader::ConstantBuffer, scene: &SceneData, frame: &UnityFrame, di: usize, ctx: &FrameCtx, picked: &[usize]) {
    let d = &scene.draws[di];
    out.fill(0);
    let o2w = frame.object_to_world.get(di).copied().unwrap_or(Mat4::IDENTITY);
    let mut lights: Option<[[[f32; 4]; 8]; 4]> = None;
    for p in &cb.params {
        let at = p.offset as usize;
        let mut put = |vals: &[f32]| {
            for (k, f) in vals.iter().enumerate() {
                let o = at + k * 4;
                if o + 4 <= out.len() {
                    out[o..o + 4].copy_from_slice(&f.to_le_bytes());
                }
            }
        };
        let mat = |m: Mat4| m.to_cols_array();
        let n = p.name.as_str();
        if p.is_matrix {
            let m = match n {
                "unity_MatrixVP" => ctx.vp,
                "unity_MatrixV" => ctx.v,
                "unity_MatrixInvV" => ctx.v.inverse(),
                "unity_ObjectToWorld" => o2w,
                "unity_WorldToObject" => o2w.inverse(),
                "glstate_matrix_projection" | "unity_CameraProjection" => ctx.vp * ctx.v.inverse(),
                _ => Mat4::IDENTITY,
            };
            put(&mat(m));
            continue;
        }
        if p.array_size > 0 {
            let which = match n {
                "unity_LightPosition" => Some(0),
                "unity_LightColor" => Some(1),
                "unity_LightAtten" => Some(2),
                "unity_SpotDirection" => Some(3),
                _ => None,
            };
            if let Some(w) = which {
                let l = lights.get_or_insert_with(|| light_block(scene, frame, picked, ctx.v));
                let flat: Vec<f32> = l[w].iter().take(p.array_size as usize).flat_map(|x| *x).collect();
                put(&flat);
            }
            continue;
        }
        let t = ctx.time;
        let v: [f32; 4] = match n {
            "_Time" => [t / 20.0, t, t * 2.0, t * 3.0],
            "_SinTime" => [(t / 8.0).sin(), (t / 4.0).sin(), (t / 2.0).sin(), t.sin()],
            "_CosTime" => [(t / 8.0).cos(), (t / 4.0).cos(), (t / 2.0).cos(), t.cos()],
            "_WorldSpaceCameraPos" => [ctx.cam_pos.x, ctx.cam_pos.y, ctx.cam_pos.z, 0.0],
            "_ProjectionParams" => [1.0, ctx.near, FAR, 1.0 / FAR],
            "_ScreenParams" => {
                let (w, h) = (ctx.size.x as f32, ctx.size.y as f32);
                [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h]
            }
            "unity_FogColor" => ctx.fog.1,
            // ULTRAKILL's fog reads RenderSettings' linear fog distances
            "unity_FogStart" => [ctx.fog.2, 0.0, 0.0, 0.0],
            "unity_FogEnd" => [ctx.fog.3, 0.0, 0.0, 0.0],
            "glstate_lightmodel_ambient" => ctx.ambient,
            "_PortalClipPlane" | "_WorldSpaceLightPos0" | "_LightColor0" => [0.0; 4],
            // global options at their defaults (PrefsManager: vertexWarping 0, textureWarping 0)
            "_VertexWarping" | "_TextureWarping" | "_HeightFog" => [0.0; 4],
            // PostProcessV2_Handler: (width, height) / max(width, height)
            "_ScreenRatio" => {
                let m = ctx.size.x.max(ctx.size.y) as f32;
                [ctx.size.x as f32 / m, ctx.size.y as f32 / m, 0.0, 0.0]
            }
            _ => {
                if let Some(base) = n.strip_suffix("_TexelSize") {
                    match d.textures.get(base).and_then(|&t| scene.textures.get(t as usize)) {
                        Some(tx) => [1.0 / tx.width as f32, 1.0 / tx.height as f32, tx.width as f32, tx.height as f32],
                        None => [1.0, 1.0, 1.0, 1.0],
                    }
                } else {
                    d.material.vector(n).unwrap_or(if n.ends_with("_ST") { [1.0, 1.0, 0.0, 0.0] } else { [0.0; 4] })
                }
            }
        };
        if p.ty == 1 {
            // integer parameter
            for k in 0..p.cols.max(1) as usize {
                let o = at + k * 4;
                if o + 4 <= out.len() {
                    out[o..o + 4].copy_from_slice(&(v[k] as i32).to_le_bytes());
                }
            }
        } else {
            put(&v[..p.cols.clamp(1, 4) as usize]);
        }
    }
}

fn prepare(
    mut gpu: ResMut<UnityGpu>,
    scene: Res<UnityScene>,
    frame: Res<UnityFrame>,
    dev: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    views: Query<(&ExtractedView, &ViewTarget), With<UnityCamera>>,
) {
    let t_prepare = std::time::Instant::now();
    let Some(scene) = scene.0.clone() else { return };
    let gpu = gpu.as_mut();
    if gpu.generation != scene.generation {
        gpu.generation = scene.generation;
        gpu.pipelines.clear();
        gpu.layouts = scene
            .variants
            .iter()
            .map(|v| {
                [
                    BindGroupLayoutDescriptor::new(format!("{} g0", v.label), &layout_entries(&v.groups[0])),
                    BindGroupLayoutDescriptor::new(format!("{} g1", v.label), &layout_entries(&v.groups[1])),
                ]
            })
            .collect();
        let tex_views: Vec<TextureView> = scene.textures.iter().map(|t| upload_texture(&dev, &queue, t)).collect();
        let white = solid_view(&dev, &queue, [255; 4], TexDim::D2);
        let black_cube = solid_view(&dev, &queue, [0, 0, 0, 255], TexDim::Cube);
        let black_3d = solid_view(&dev, &queue, [0, 0, 0, 0], TexDim::D3);
        let zero_storage = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity zero storage"), contents: &[0u8; 4096], usage: BufferUsages::STORAGE });
        // constant-buffer slices for every draw in one shared uniform buffer (256-byte aligned)
        let align = dev.limits().min_uniform_buffer_offset_alignment.max(16) as u64;
        let mut total = 0u64;
        let slices: Vec<Vec<(u32, u64, u64, usize)>> = scene
            .draws
            .iter()
            .map(|d| {
                let var = &scene.variants[d.variant as usize];
                var.groups[1]
                    .iter()
                    .filter_map(|s| match s {
                        Slot::Uniform { binding, cb } => {
                            let size = var.params.constant_buffers.get(*cb).map_or(16, |c| c.size.next_multiple_of(16).max(16)) as u64;
                            let off = total;
                            total = (total + size).next_multiple_of(align);
                            Some((*binding, off, size, *cb))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let ubo = Some(dev.create_buffer(&BufferDescriptor { label: Some("unity constant buffers"), size: total.max(256), usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false }));
        gpu.staging = vec![0; total.max(256) as usize];
        let mut draws = Vec::with_capacity(scene.draws.len());
        for (di, d) in scene.draws.iter().enumerate() {
            let var = &scene.variants[d.variant as usize];
            let layouts = &gpu.layouts[d.variant as usize];
            let l0 = cache.get_bind_group_layout(&layouts[0]);
            let l1 = cache.get_bind_group_layout(&layouts[1]);
            // group 0: textures and samplers (the sampler takes the first texture's settings)
            let first_tex = var.groups[0].iter().find_map(|s| match s {
                Slot::Texture { name, .. } => d.textures.get(name).and_then(|&t| scene.textures.get(t as usize)),
                _ => None,
            });
            let sampler = sampler_for(&dev, first_tex);
            // samplers split out of combined image-samplers pair with the texture 16 bindings below
            let mut split_samplers = Vec::new();
            for s in &var.groups[0] {
                if let Slot::Sampler { binding, .. } = s {
                    if *binding >= uk_assets::spirv::SAMPLER_BINDING_OFFSET {
                        let tex = var.groups[0].iter().find_map(|t| match t {
                            Slot::Texture { binding: tb, name, .. } if *tb + uk_assets::spirv::SAMPLER_BINDING_OFFSET == *binding => {
                                d.textures.get(name).and_then(|&i| scene.textures.get(i as usize))
                            }
                            _ => None,
                        });
                        split_samplers.push((*binding, sampler_for(&dev, tex)));
                    }
                }
            }
            let mut e0 = Vec::new();
            for s in &var.groups[0] {
                match s {
                    Slot::Texture { binding, dim, name } => {
                        let view = match dim {
                            TexDim::Cube => &black_cube,
                            TexDim::D3 => &black_3d,
                            TexDim::D2 => d.textures.get(name).map(|&t| &tex_views[t as usize]).unwrap_or(&white),
                        };
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::TextureView(view) });
                    }
                    Slot::Sampler { binding, .. } => {
                        let s = split_samplers.iter().find(|(b, _)| b == binding).map(|(_, s)| s).unwrap_or(&sampler);
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::Sampler(s) })
                    }
                    Slot::Storage { binding } => e0.push(BindGroupEntry { binding: *binding, resource: zero_storage.as_entire_binding() }),
                    Slot::Uniform { .. } => {}
                }
            }
            let bg0 = dev.create_bind_group("unity g0", &l0, &e0);
            let ubufs = &slices[di];
            let ubo = ubo.as_ref().unwrap();
            let mut e1: Vec<BindGroupEntry> = ubufs
                .iter()
                .map(|&(b, off, size, _)| BindGroupEntry {
                    binding: b,
                    resource: BindingResource::Buffer(BufferBinding { buffer: ubo, offset: off, size: std::num::NonZeroU64::new(size) }),
                })
                .collect();
            for s in &var.groups[1] {
                if let Slot::Storage { binding } = s {
                    e1.push(BindGroupEntry { binding: *binding, resource: zero_storage.as_entire_binding() });
                }
            }
            let bg1 = dev.create_bind_group("unity g1", &l1, &e1);
            let vbuf = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity vb"), contents: &d.vertices, usage: BufferUsages::VERTEX });
            let ibuf = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity ib"), contents: bytemuck::cast_slice(&d.indices), usage: BufferUsages::INDEX });
            let key = (d.variant, d.state);
            let pipeline = *gpu.pipelines.entry(key).or_insert_with(|| cache.queue_render_pipeline(pipeline_descriptor(var, layouts, d.state)));
            draws.push(Some(GpuDraw { vbuf, ibuf, count: d.indices.len() as u32, ubufs: ubufs.clone(), bg0, bg1, pipeline }));
        }
        gpu.draws = draws;
        gpu.ubo = ubo;
        gpu.post = scene.post.as_ref().map(|p| {
            let var = &scene.variants[p.variant as usize];
            let layouts = [
                BindGroupLayoutDescriptor::new("unity post g0", &layout_entries(&var.groups[0])),
                BindGroupLayoutDescriptor::new("unity post g1", &layout_entries(&var.groups[1])),
            ];
            let ubufs = var.groups[1]
                .iter()
                .filter_map(|s| match s {
                    Slot::Uniform { binding, cb } => {
                        let size = var.params.constant_buffers.get(*cb).map_or(16, |c| c.size.next_multiple_of(16).max(16)) as u64;
                        let b = dev.create_buffer(&BufferDescriptor { label: Some("unity post cb"), size, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false });
                        Some((*binding, b, *cb))
                    }
                    _ => None,
                })
                .collect();
            // one clip-space triangle covering the screen
            let mut verts: Vec<u8> = Vec::new();
            for c in [[-1.0f32, -1.0], [3.0, -1.0], [-1.0, 3.0]] {
                for &(_, _, n) in &var.inputs {
                    for f in [c[0], c[1], 0.5, 1.0].iter().take(n as usize) {
                        verts.extend_from_slice(&f.to_le_bytes());
                    }
                }
            }
            let vbuf = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity post vb"), contents: &verts, usage: BufferUsages::VERTEX });
            // PostProcessV2's pass: Cull Off, ZWrite Off, ZTest Always, Blend One Zero
            let state = DrawState { cull: 0, zwrite: false, ztest: 8, src: 1, dst: 0, src_a: 1, dst_a: 0, op: 0, mask: 15 };
            let mut desc = pipeline_descriptor(var, &layouts, state);
            desc.depth_stencil = None;
            let dither = match p.dither {
                Some(t) => (tex_views[t as usize].clone(), sampler_for(&dev, scene.textures.get(t as usize))),
                None => (white.clone(), sampler_for(&dev, None)),
            };
            let main_sampler = dev.create_sampler(&SamplerDescriptor { label: Some("unity post main"), ..default() });
            PostGpu { pipeline: cache.queue_render_pipeline(desc), layouts, ubufs, vbuf, dither, main_sampler }
        });
    }
    // composite pipeline
    if gpu.composite.is_none() {
        let layout = BindGroupLayoutDescriptor::new(
            "unity composite",
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true }, view_dimension: TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::FRAGMENT, ty: BindingType::Sampler(SamplerBindingType::Filtering), count: None },
            ],
        );
        if let Some((view, target)) = views.iter().next() {
            let _ = view;
            let shader = scene.composite_shader.clone();
            let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("unity composite".into()),
                layout: vec![layout.clone()],
                vertex: VertexState { shader: shader.clone(), entry_point: Some("vs".into()), ..default() },
                fragment: Some(FragmentState {
                    shader,
                    entry_point: Some("fs".into()),
                    targets: vec![Some(ColorTargetState { format: target.main_texture_format(), blend: None, write_mask: ColorWrites::ALL })],
                    ..default()
                }),
                ..default()
            });
            let sampler = dev.create_sampler(&SamplerDescriptor { label: Some("unity composite"), ..default() });
            gpu.composite = Some((id, layout, sampler));
        }
    }
    // per-frame: offscreen target, constant buffers
    let Some((view, _)) = views.iter().next() else { return };
    let size = UVec2::new(view.viewport.z.max(1), view.viewport.w.max(1));
    if gpu.target.as_ref().is_none_or(|t| t.0 != size) {
        let color = dev.create_texture(&TextureDescriptor {
            label: Some("unity scene color"),
            size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = dev.create_texture(&TextureDescriptor {
            label: Some("unity scene depth"),
            size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let cv = color.create_view(&TextureViewDescriptor::default());
        let dv = depth.create_view(&TextureViewDescriptor::default());
        gpu.target = Some((size, color, cv, dv));
        let post = dev.create_texture(&TextureDescriptor {
            label: Some("unity post output"),
            size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pv = post.create_view(&TextureViewDescriptor::default());
        gpu.post_target = Some((post, pv));
    }
    // camera: Bevy view -> Unity space (Unity world = Bevy world mirrored in z)
    let mirror = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    let world_from_view = view.world_from_view.to_matrix();
    let v = world_from_view.inverse() * mirror;
    let near = NEAR;
    let fy = view.clip_from_view.y_axis.y;
    let aspect = size.x as f32 / size.y as f32;
    let proj = Mat4::from_cols(
        Vec4::new(fy / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, fy, 0.0, 0.0),
        Vec4::new(0.0, 0.0, near / (FAR - near), -1.0),
        Vec4::new(0.0, 0.0, near * FAR / (FAR - near), 0.0),
    );
    let cam = world_from_view.w_axis.truncate();
    let ctx = FrameCtx {
        vp: proj * v,
        v,
        cam_pos: Vec3::new(cam.x, cam.y, -cam.z),
        time: frame.time,
        fog: scene.fog,
        ambient: scene.ambient,
        near,
        size,
    };
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && gpu.frames.load(std::sync::atomic::Ordering::Relaxed) == 240 {
        // numeric view of what the shaders get: frustum coverage and the nearest draw's lights
        let (mut inside, mut total) = (0usize, 0usize);
        let mut nearest: Option<(f32, usize)> = None;
        for (di, d) in scene.draws.iter().enumerate() {
            if !frame.visible.get(di).copied().unwrap_or(true) {
                continue;
            }
            let var = &scene.variants[d.variant as usize];
            let stride = var.stride as usize;
            for k in (0..d.vertices.len() / stride).step_by(7) {
                let f = |o: usize| f32::from_le_bytes(d.vertices[k * stride + o..k * stride + o + 4].try_into().unwrap());
                let c = ctx.vp * Vec4::new(f(0), f(4), f(8), 1.0);
                total += 1;
                if c.w > 0.0 && c.x.abs() <= c.w && c.y.abs() <= c.w && c.z >= 0.0 && c.z <= c.w {
                    inside += 1;
                }
            }
            let dist = d.center.distance(ctx.cam_pos);
            if nearest.is_none_or(|n| dist < n.0) {
                nearest = Some((dist, di));
            }
        }
        info!("unity camera (unity space) {:?}; sampled vertices inside frustum {inside}/{total}", ctx.cam_pos);
        // which draws could cover the whole view: bounds containing the camera, or huge on screen
        let mut big: Vec<(f32, usize)> = scene
            .draws
            .iter()
            .enumerate()
            .filter(|(di, _)| frame.visible.get(*di).copied().unwrap_or(true))
            .map(|(di, d)| (d.radius / d.center.distance(ctx.cam_pos).max(0.01), di))
            .filter(|(r, _)| *r > 1.0)
            .collect();
        big.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (r, di) in big.iter().take(8) {
            let d = &scene.draws[*di];
            info!("  covers view ({r:.1}): draw {di} node {} {} queue {} state {:?} material {}", d.node, scene.variants[d.variant as usize].label, d.queue, d.state, d.material.name);
        }
        if let Some((dist, di)) = nearest {
            let d = &scene.draws[di];
            let l = vertex_lights(&scene, &frame, d, ctx.v);
            info!(
                "nearest draw {di} ({}) at {dist:.1}: queue {} state {:?} keywords {:?}\n  light pos {:?}\n  light color {:?}\n  light atten {:?}",
                scene.variants[d.variant as usize].label, d.queue, d.state, d.material.keywords, &l[0][..3], &l[1][..3], &l[2][..3]
            );
        }
    }
    // light choice only changes when lights switch on/off (or move): re-rank then, not per frame
    let mut key: u64 = 0xcbf29ce484222325;
    for (i, (p, _, on)) in frame.lights.iter().enumerate() {
        if *on {
            key = (key ^ i as u64).wrapping_mul(0x100000001b3);
            key = (key ^ p.x.to_bits() as u64 ^ (p.z.to_bits() as u64) << 32).wrapping_mul(0x100000001b3);
        }
    }
    if gpu.light_key != key || gpu.light_pick.len() != scene.draws.len() {
        gpu.light_key = key;
        gpu.light_pick = scene.draws.iter().map(|d| pick_lights(&scene, &frame, d)).collect();
    }
    // frustum planes (Unity space) from the view-projection: x, y in [-w, w], z in [0, w]
    let m = ctx.vp;
    let row = |i: usize| Vec4::new(m.x_axis[i], m.y_axis[i], m.z_axis[i], m.w_axis[i]);
    let planes: Vec<Vec4> = [row(3) + row(0), row(3) - row(0), row(3) + row(1), row(3) - row(1), row(2), row(3) - row(2)]
        .iter()
        .map(|p| *p / p.truncate().length().max(1e-6))
        .collect();
    let mut in_view = std::mem::take(&mut gpu.in_view);
    in_view.clear();
    in_view.extend(scene.draws.iter().enumerate().map(|(di, d)| {
        if !frame.visible.get(di).copied().unwrap_or(true) {
            return false;
        }
        let c = frame.object_to_world.get(di).map_or(d.center, |o| o.transform_point3(d.center));
        planes.iter().all(|p| p.truncate().dot(c) + p.w >= -d.radius)
    }));
    // fill visible draws into the CPU copy, then upload each contiguous run with one write
    let mut staging = std::mem::take(&mut gpu.staging);
    let mut run: Option<(u64, u64)> = None;
    let mut runs = Vec::new();
    for (di, d) in gpu.draws.iter().enumerate() {
        let Some(d) = d else { continue };
        if !in_view[di] {
            continue;
        }
        let var = &scene.variants[scene.draws[di].variant as usize];
        for &(_, off, size, cb) in &d.ubufs {
            let Some(cbd) = var.params.constant_buffers.get(cb) else { continue };
            fill_cb(&mut staging[off as usize..(off + size) as usize], cbd, &scene, &frame, di, &ctx, &gpu.light_pick[di]);
            run = match run {
                Some((s, e)) if off <= e.next_multiple_of(256) + 256 => Some((s, off + size)),
                Some(r) => {
                    runs.push(r);
                    Some((off, off + size))
                }
                None => Some((off, off + size)),
            };
        }
    }
    runs.extend(run);
    if let Some(ubo) = &gpu.ubo {
        for (s, e) in runs {
            let e = e.next_multiple_of(4);
            queue.write_buffer(ubo, s, &staging[s as usize..e as usize]);
        }
    }
    gpu.staging = staging;
    gpu.in_view = in_view;
    if let (Some(post), Some(pd)) = (&gpu.post, scene.post.as_ref()) {
        let var = &scene.variants[pd.variant as usize];
        for (_, buf, cb) in &post.ubufs {
            let Some(cbd) = var.params.constant_buffers.get(*cb) else { continue };
            let mut bytes = vec![0u8; buf.size() as usize];
            fill_post_cb(&mut bytes, cbd, &pd.material, size);
            queue.write_buffer(buf, 0, &bytes);
        }
    }
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && gpu.frames.load(std::sync::atomic::Ordering::Relaxed) == 240 {
        info!("unity prepare: {:.2} ms; draws in view {}/{}", t_prepare.elapsed().as_secs_f64() * 1e3, gpu.in_view.iter().filter(|v| **v).count(), gpu.in_view.len());
    }
}

fn pipeline_descriptor(var: &Variant, layouts: &[BindGroupLayoutDescriptor; 2], s: DrawState) -> RenderPipelineDescriptor {
    let mut offset = 0;
    let attributes = var
        .inputs
        .iter()
        .map(|&(location, _, n)| {
            let format = match n {
                1 => VertexFormat::Float32,
                2 => VertexFormat::Float32x2,
                3 => VertexFormat::Float32x3,
                _ => VertexFormat::Float32x4,
            };
            let a = VertexAttribute { format, offset, shader_location: location };
            offset += n as u64 * 4;
            a
        })
        .collect();
    let opaque = s.src == 1 && s.dst == 0 && s.src_a <= 1 && s.dst_a == 0;
    let blend = (!opaque).then(|| BlendState {
        color: BlendComponent { src_factor: blend_factor(s.src), dst_factor: blend_factor(s.dst), operation: blend_op(s.op) },
        alpha: BlendComponent { src_factor: blend_factor(s.src_a.max(s.src)), dst_factor: blend_factor(s.dst_a.max(s.dst)), operation: blend_op(s.op) },
    });
    // Unity ColorWriteMask: A = 1, B = 2, G = 4, R = 8
    let mut mask = ColorWrites::empty();
    for (bit, w) in [(8, ColorWrites::RED), (4, ColorWrites::GREEN), (2, ColorWrites::BLUE), (1, ColorWrites::ALPHA)] {
        if s.mask & bit != 0 {
            mask |= w;
        }
    }
    RenderPipelineDescriptor {
        label: Some(var.label.clone().into()),
        layout: layouts.to_vec(),
        vertex: VertexState {
            shader: var.vs.clone(),
            entry_point: Some("main".into()),
            buffers: vec![VertexBufferLayout { array_stride: var.stride, step_mode: VertexStepMode::Vertex, attributes }],
            ..default()
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            front_face: FrontFace::Ccw,
            cull_mode: match s.cull {
                1 => Some(Face::Front),
                2 => Some(Face::Back),
                _ => None,
            },
            ..default()
        },
        depth_stencil: Some(DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(s.zwrite),
            depth_compare: Some(depth_compare(s.ztest)),
            stencil: default(),
            bias: default(),
        }),
        fragment: Some(FragmentState {
            shader: var.fs.clone(),
            entry_point: Some("main".into()),
            targets: vec![Some(ColorTargetState { format: COLOR_FORMAT, blend, write_mask: mask })],
            ..default()
        }),
        ..default()
    }
}


fn draw(
    world: &World,
    view: ViewQuery<(&ExtractedView, &ViewTarget), With<UnityCamera>>,
    mut ctx: RenderContext,
) {
    let (extracted, target) = view.into_inner();
    let gpu = world.resource::<UnityGpu>();
    let cache = world.resource::<PipelineCache>();
    let scene = world.resource::<UnityScene>();
    let Some(scene) = scene.0.as_ref() else { return };
    let Some((_, _, color, depth)) = gpu.target.as_ref() else { return };
    // opaque by queue, then transparent back to front
    let cam = extracted.world_from_view.translation();
    let cam_u = Vec3::new(cam.x, cam.y, -cam.z);
    let mut order: Vec<(i64, f32, usize)> = (0..gpu.draws.len())
        .filter(|&i| gpu.draws[i].is_some() && gpu.in_view.get(i).copied().unwrap_or(false))
        .map(|i| {
            let d = &scene.draws[i];
            let dist = d.center.distance(cam_u);
            (d.queue, if d.queue >= 2500 { -dist } else { dist }, i)
        })
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let t_draw = std::time::Instant::now();
    let c = scene.clear;
    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("unity scene"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(wgpu_types::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }), store: StoreOp::Store },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(Operations { load: LoadOp::Clear(0.0), store: StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for (_, _, i) in order {
            let Some(d) = &gpu.draws[i] else { continue };
            let Some(p) = cache.get_render_pipeline(d.pipeline) else { continue };
            pass.set_render_pipeline(p);
            pass.set_bind_group(0, &d.bg0, &[]);
            pass.set_bind_group(1, &d.bg1, &[]);
            pass.set_vertex_buffer(0, d.vbuf.slice(..));
            pass.set_index_buffer(d.ibuf.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..d.count, 0, 0..1);
        }
    }
    // ULTRAKILL's final composite (PostProcessV2) into the post target
    let mut shown = color;
    if let (Some(post), Some(pd), Some((_, pv))) = (&gpu.post, scene.post.as_ref(), gpu.post_target.as_ref()) {
        if let Some(p) = cache.get_render_pipeline(post.pipeline) {
            let dev = world.resource::<RenderDevice>();
            let var = &scene.variants[pd.variant as usize];
            let mut e0 = Vec::new();
            for s in &var.groups[0] {
                match s {
                    Slot::Texture { binding, name, .. } => {
                        let view = if name == "_MainTex" { color } else { &post.dither.0 };
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::TextureView(view) });
                    }
                    Slot::Sampler { binding, .. } => {
                        // split samplers sit SAMPLER_BINDING_OFFSET above their texture
                        let tex = var.groups[0].iter().find_map(|t| match t {
                            Slot::Texture { binding: tb, name, .. } if *tb + uk_assets::spirv::SAMPLER_BINDING_OFFSET == *binding => Some(name.as_str()),
                            _ => None,
                        });
                        let smp = if tex == Some("_MainTex") { &post.main_sampler } else { &post.dither.1 };
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::Sampler(smp) });
                    }
                    _ => {}
                }
            }
            let e1: Vec<BindGroupEntry> = post.ubufs.iter().map(|(b, buf, _)| BindGroupEntry { binding: *b, resource: buf.as_entire_binding() }).collect();
            let bg0 = dev.create_bind_group("unity post g0", &cache.get_bind_group_layout(&post.layouts[0]), &e0);
            let bg1 = dev.create_bind_group("unity post g1", &cache.get_bind_group_layout(&post.layouts[1]), &e1);
            let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("unity post"),
                color_attachments: &[Some(RenderPassColorAttachment { view: pv, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Clear(wgpu_types::Color::BLACK), store: StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_render_pipeline(p);
            pass.set_bind_group(0, &bg0, &[]);
            pass.set_bind_group(1, &bg1, &[]);
            pass.set_vertex_buffer(0, post.vbuf.slice(..));
            pass.draw(0..3, 0..1);
            shown = pv;
        }
    }
    // numeric frame check: copy one frame of the scene target back to the CPU
    let n = gpu.frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n == 240 && std::env::var_os("UNITY_FRAME_STATS").is_some() {
        info!("unity scene pass encode: {:.2} ms", t_draw.elapsed().as_secs_f64() * 1e3);
    }
    // steady-state timing: frames 120..240 (start-up compiles pipelines asynchronously)
    if n == 120 {
        *gpu.started.lock().unwrap() = Some(std::time::Instant::now());
    }
    if n == 240 && std::env::var_os("UNITY_FRAME_STATS").is_some() {
        let (size, tex, _, _) = gpu.target.as_ref().unwrap();
        let row = (size.x * 4).next_multiple_of(256);
        let dev = world.resource::<RenderDevice>();
        let buf = dev.create_buffer(&BufferDescriptor { label: Some("unity readback"), size: (row * size.y) as u64, usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
        ctx.command_encoder().copy_texture_to_buffer(
            tex.as_image_copy(),
            TexelCopyBufferInfo { buffer: &buf, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size.y) } },
            Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        );
        *gpu.readback.lock().unwrap() = Some((buf, row, *size, false));
        if let (Some((ptex, _)), false) = (gpu.post_target.as_ref(), std::ptr::eq(shown, color)) {
            let pbuf = dev.create_buffer(&BufferDescriptor { label: Some("unity post readback"), size: (row * size.y) as u64, usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
            ctx.command_encoder().copy_texture_to_buffer(
                ptex.as_image_copy(),
                TexelCopyBufferInfo { buffer: &pbuf, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size.y) } },
                Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            );
            *gpu.post_readback.lock().unwrap() = Some(pbuf);
        }
    }
    // composite onto the camera's view (gamma -> linear): PostProcessV2's output, else the raw scene
    let Some((pid, layout, sampler)) = gpu.composite.as_ref() else { return };
    let Some(p) = cache.get_render_pipeline(*pid) else { return };
    let dev = world.resource::<RenderDevice>();
    let bg = dev.create_bind_group(
        "unity composite",
        &cache.get_bind_group_layout(layout),
        &[BindGroupEntry { binding: 0, resource: BindingResource::TextureView(shown) }, BindGroupEntry { binding: 1, resource: BindingResource::Sampler(sampler) }],
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("unity composite"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(p);
    pass.set_bind_group(0, &bg, &[]);
    pass.draw(0..3, 0..1);
}

/// Logs statistics of the read-back frame: share of pixels not equal to the clear color, mean color,
/// distinct colors. A frame that is all clear color means nothing was drawn.
fn frame_stats(gpu: Res<UnityGpu>, dev: Res<RenderDevice>, scene: Res<UnityScene>) {
    let mut rb = gpu.readback.lock().unwrap();
    let Some((buf, row, size, done)) = rb.as_mut() else { return };
    if *done {
        return;
    }
    *done = true;
    let slice = buf.slice(..);
    slice.map_async(MapMode::Read, |_| {});
    let _ = dev.poll(wgpu_types::PollType::wait_indefinitely());
    let data = slice.get_mapped_range();
    let clear = scene.0.as_ref().map(|s| s.clear).unwrap_or([0.0; 4]);
    let clear8 = clear.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as i32);
    let (mut drawn, mut sum) = (0u64, [0u64; 3]);
    let mut distinct = std::collections::HashSet::new();
    for y in 0..size.y {
        for x in 0..size.x {
            let i = (y * *row + x * 4) as usize;
            let p = [data[i] as i32, data[i + 1] as i32, data[i + 2] as i32];
            if (0..3).any(|k| (p[k] - clear8[k]).abs() > 1) {
                drawn += 1;
            }
            for k in 0..3 {
                sum[k] += p[k] as u64;
            }
            distinct.insert((p[0] >> 2, p[1] >> 2, p[2] >> 2));
        }
    }
    let n = (size.x * size.y) as f64;
    let ms = gpu.started.lock().unwrap().map(|s| s.elapsed().as_secs_f64() * 1e3 / 120.0).unwrap_or(0.0);
    info!("unity frame time: {ms:.2} ms average over frames 120..240 ({:.0} fps)", 1000.0 / ms.max(1e-3));
    info!(
        "unity frame stats: {}x{} drawn {:.1}% mean rgb ({:.1}, {:.1}, {:.1}) distinct colors {}",
        size.x,
        size.y,
        100.0 * drawn as f64 / n,
        sum[0] as f64 / n,
        sum[1] as f64 / n,
        sum[2] as f64 / n,
        distinct.len()
    );
    // PostProcessV2 output against the scene it read: mean |post - scene| as stored and with rows
    // flipped (orientation check), and its distinct colors (dither + 32-level quantization)
    if let Some(pbuf) = gpu.post_readback.lock().unwrap().take() {
        let ps = pbuf.slice(..);
        ps.map_async(MapMode::Read, |_| {});
        let _ = dev.poll(wgpu_types::PollType::wait_indefinitely());
        let post = ps.get_mapped_range();
        let (mut same, mut flip) = (0u64, 0u64);
        let mut pd = std::collections::HashSet::new();
        for y in 0..size.y {
            for x in 0..size.x {
                let i = (y * *row + x * 4) as usize;
                let j = ((size.y - 1 - y) * *row + x * 4) as usize;
                for k in 0..3 {
                    same += (post[i + k] as i32 - data[i + k] as i32).unsigned_abs() as u64;
                    flip += (post[i + k] as i32 - data[j + k] as i32).unsigned_abs() as u64;
                }
                pd.insert((post[i] >> 2, post[i + 1] >> 2, post[i + 2] >> 2));
            }
        }
        info!("unity post stats: mean abs diff vs scene {:.2} (rows flipped {:.2}) distinct colors {}", same as f64 / (3.0 * n), flip as f64 / (3.0 * n), pd.len());
    }
}
