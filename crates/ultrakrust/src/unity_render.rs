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

/// The player's Main Camera (FirstRoom/Player/Main Camera in every level): clip planes, and the
/// clear color when a scene has no camera to read (clear flags 1 = skybox: RenderSettings' skybox
/// material, else the camera's background color).
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
    /// 1, or 6 for a cubemap (faces +X -X +Y -Y +Z -Z).
    pub layers: u32,
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
    /// Fragment output locations (ULTRAKILL/Master writes 0 color, 1 outline RG, 2 view normal).
    pub outputs: Vec<u32>,
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
    /// Render target 1 (the outline buffer): src, dst, op, mask.
    pub rt1: [u8; 4],
    /// Stencil: read mask, write mask, pass op, fail op, zfail op, comparison (the reference is
    /// dynamic). STENCIL_OFF for every non-UI draw.
    pub stencil: [u8; 6],
}

pub const STENCIL_OFF: [u8; 6] = [255, 255, 0, 0, 0, 8];

#[derive(Clone)]
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
    /// SkinnedMeshRenderer: re-skinned on the CPU each frame its bones are animated.
    pub skin: Option<Arc<SkinDraw>>,
    /// Viewmodel layer (13): drawn by the HUD Camera after a depth clear.
    pub hud: bool,
    /// RenderSettings' skybox: a sphere around the camera, drawn first behind everything.
    pub sky: bool,
    /// A particle slot: world-space geometry rebuilt every frame (`UnityFrame::particles`).
    pub particle: bool,
}

pub struct SkinDraw {
    pub def: Arc<uk_assets::scenedef::SkinDef>,
    /// source mesh vertex of each draw vertex
    pub src: Vec<u32>,
    pub stride: usize,
    /// byte offsets of the position / normal inputs inside a vertex, when the shader reads them
    pub pos: Option<usize>,
    pub nrm: Option<usize>,
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
    /// HUD Camera (renders layer 13 only): load-time Bevy-space transform and vertical fov (deg).
    pub hud_cam: Option<(Mat4, f32)>,
    /// PostProcessV2_Handler.outlinePx pass 3 ("Composite1Px"): the default 1 px outline blit.
    pub outline: Option<OutlineDef>,
    pub prefs: GraphicsPrefs,
    pub ui: Option<UiScene>,
    /// particle / trail material -> its draw slots (one ParticleSystemRenderer each per frame)
    pub particle_slots: HashMap<(String, i64), Vec<usize>>,
}

/// Draw slots per particle material: the most renderers using one material drawn separately in a
/// frame (more share the last slot).
const PARTICLE_SLOTS: usize = 64;

/// One ParticleSystemRenderer's geometry this frame (Unity space), expanded against the camera in
/// prepare.
#[derive(Clone, Default)]
pub struct ParticleDraw {
    pub slot: usize,
    /// the renderer's bounds centre (transparent sorting, culling) and radius
    pub center: Vec3,
    pub radius: f32,
    /// ParticleSystemRenderer min / maxParticleSize (fractions of the viewport height)
    pub min_size: f32,
    pub max_size: f32,
    /// billboards: position, size, rotation (radians), color
    pub quads: Vec<(Vec3, f32, f32, [f32; 4])>,
    /// trails, head first: position, width, color, u
    pub strips: Vec<Vec<(Vec3, f32, [f32; 4], f32)>>,
}

/// The shader globals GraphicsSettings / PostProcessV2_Handler derive from the player's prefs.
#[derive(Clone, Copy, Debug)]
pub struct GraphicsPrefs {
    /// `_ResY` / downscaleResolution: the virtual resolution's short side, 0 = native
    pub res_y: f32,
    pub color_precision: f32,
    pub dither: f32,
    pub gamma: f32,
    pub texture_warping: f32,
    /// `_VertexWarping` (VERTEX_WARPING keyword on when nonzero)
    pub vertex_warping: f32,
    /// BloodsplatterManager: `_StainWarping` = the raw vertexWarping option
    pub stain_warping: f32,
}

impl GraphicsPrefs {
    pub fn from_prefs(p: &uk_assets::prefs::Prefs) -> Self {
        use uk_assets::prefs::*;
        let vw = p.int("vertexWarping");
        GraphicsPrefs {
            res_y: pixelization_value(p.int("pixelization")),
            // a saved colorPalette (PALETTIZE) forces 2048; palettes are not supported yet
            color_precision: color_compression_value(p.int("colorCompression")),
            dither: p.float("dithering"),
            gamma: p.float("gamma"),
            texture_warping: p.float("textureWarping").clamp(0.0, 1.0) * 0.5,
            vertex_warping: vertex_warping_value(vw),
            stain_warping: vw as f32,
        }
    }

    /// PostProcessV2_Handler.SetupRTs: the render size for a screen size
    pub fn virtual_size(&self, screen: UVec2) -> UVec2 {
        if self.res_y == 0.0 {
            return screen;
        }
        let (w, h) = (screen.x as f32, screen.y as f32);
        let m = w.min(h);
        UVec2::new(((w / m) * self.res_y) as u32, ((h / m) * self.res_y) as u32).max(UVec2::ONE)
    }
}

impl Default for GraphicsPrefs {
    fn default() -> Self {
        Self::from_prefs(&uk_assets::prefs::Prefs::default())
    }
}

pub struct OutlineDef {
    pub variant: u32,
    pub state: DrawState,
}

/// The keywords scripts toggle on PostProcessV2 at runtime, as bits of a variant index:
/// DeathSequence (DEAD), UnderwaterController (UNDERWATER, global), PowerUpMeter (VIGNETTE),
/// ScreenDistortionController (WICKED).
pub const POST_KEYWORDS: [&str; 4] = ["DEAD", "UNDERWATER", "VIGNETTE", "WICKED"];

pub struct PostDef {
    /// the pass's variant for each combination of POST_KEYWORDS bits
    pub variants: [Option<u32>; 16],
    /// NewMovement.hurtScreen's Image color: the RGB of `_HurtScreenColor`
    pub hurt_rgb: [f32; 3],
    pub material: Arc<MaterialProps>,
    /// sampled textures by shader name (scene texture index): the material's, with the handler's
    /// ditherTexture / vignetteTexture set in Start
    pub textures: Vec<(String, u32)>,
}

/// A UI material's shader: the four UNITY_UI_CLIP_RECT / UNITY_UI_ALPHACLIP variants and its
/// fixed state. CanvasRenderer.SetTexture overrides `_MainTex`; MaskUtilities' StencilMaterial
/// copies override the stencil properties and `_ColorMask` per draw.
pub struct UiMat {
    pub props: Arc<MaterialProps>,
    /// bit 0 UNITY_UI_CLIP_RECT, bit 1 UNITY_UI_ALPHACLIP
    pub variants: [Option<u32>; 4],
    /// the pass state with `unity_GUIZTestMode` resolved for [overlay, world/camera] canvases
    pub state: [DrawState; 2],
    /// the material's stencil reference
    pub stencil_ref: u8,
    /// texture binding name -> scene texture index (the material's own)
    pub textures: HashMap<String, u32>,
}

pub struct UiScene {
    pub def: Arc<uk_game::ugui::UiDef>,
    pub assets: Arc<uk_assets::ui::UiAssets>,
    /// UiAssets texture -> scene texture
    pub tex: Vec<Option<u32>>,
    /// UiAssets material -> its shader
    pub mats: Vec<Option<UiMat>>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct UnityScene(pub Option<Arc<SceneData>>);

/// Per-frame dynamic state from the game.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct UnityFrame {
    /// this frame's uGUI meshes, and each batch's object-to-world (overlay: root to screen pixels)
    pub ui: Option<(Arc<uk_game::ugui::UiFrame>, Arc<Vec<Mat4>>)>,
    pub time: f32,
    pub visible: Arc<Vec<bool>>,
    /// Unity-space object-to-world per draw (identity unless moved by a mover).
    pub object_to_world: Arc<Vec<Mat4>>,
    /// Re-skinned vertex buffers (draw index, full vertex bytes) for this frame.
    pub skinned: Arc<Vec<(usize, Vec<u8>)>>,
    /// Unity-space light world positions / forward directions, and whether each is active.
    pub lights: Arc<Vec<(Vec3, Vec3, bool)>>,
    /// NewMovement.currentColor.a (the hurt flash)
    pub hurt: f32,
    /// DeathSequence's `_Deathness`/`_Sharpness` while the player is dead (DEAD keyword on)
    pub dead: Option<f32>,
    /// `_UnderwaterOverlay` while UNDERWATER is on
    pub underwater: Option<[f32; 4]>,
    /// `_VignetteColor` while VIGNETTE is on
    pub vignette: Option<[f32; 4]>,
    /// `_RandomNoiseStrength` while WICKED is on
    pub noise: Option<f32>,
    /// the HUD Camera's current world matrix (Bevy space; NewMovement's HUD sway moves it)
    pub hud_cam: Option<Mat4>,
    /// this frame's particle and trail geometry per slot
    pub particles: Arc<Vec<ParticleDraw>>,
}

impl UnityFrame {
    /// the POST_KEYWORDS bits enabled this frame
    pub fn post_mask(&self) -> usize {
        self.dead.is_some() as usize | (self.underwater.is_some() as usize) << 1 | (self.vignette.is_some() as usize) << 2 | (self.noise.is_some() as usize) << 3
    }
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
        // UNITY_DUMP_WGSL=<dir outside the repo>: every variant's stages, for reading what a shader does
        if let Ok(dir) = std::env::var("UNITY_DUMP_WGSL") {
            let stem = format!("{dir}/{}_{}", sh.name.replace('/', "_"), vsub.blob);
            let cbs: String = params.constant_buffers.iter().map(|cb| format!("// CB {} size {}: {}
", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" "))).collect();
            let _ = std::fs::write(format!("{stem}_vs.wgsl"), &vsrc);
            let _ = std::fs::write(format!("{stem}_fs.wgsl"), format!("// keywords {:?}
{cbs}{fsrc}", prog.keywords));
        }
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
        let fep = fm.entry_points.iter().find(|e| e.stage == naga::ShaderStage::Fragment).ok_or("no fragment entry")?;
        let mut outputs = Vec::new();
        if let Some(r) = &fep.function.result {
            let loc = |b: &Option<naga::Binding>| match b {
                Some(naga::Binding::Location { location, .. }) => Some(*location),
                _ => None,
            };
            match &fm.types[r.ty].inner {
                naga::TypeInner::Struct { members, .. } => outputs.extend(members.iter().filter_map(|m| loc(&m.binding))),
                _ => outputs.extend(loc(&r.binding)),
            }
        }
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
            outputs,
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
    ui: Option<(Arc<uk_game::ugui::UiDef>, Arc<uk_assets::ui::UiAssets>)>,
    skip_node: impl Fn(u32) -> bool,
    shaders: &mut Assets<Shader>,
    generation: u64,
    prefs: GraphicsPrefs,
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
    // Unity draws the skybox material on its own sphere around the camera
    let sky_def = def.render_settings.skybox.clone().filter(|_| def.render_settings.camera_clear.is_none_or(|c| c.0 == 1)).map(|m| uk_assets::scenedef::RenderDef {
        node: 0,
        material: Some(m),
        batch: sky_sphere(),
        enabled: true,
        skin: None,
    });
    // EnemySimplifier (on the renderer's GameObject) sets its property block in Start: at default
    // prefs (simplifyEnemies off) _Outline 0 and _ForceOutline 0.5, so enemies write R = 0.5 into the
    // outline buffer, unmarked; and the forced SV_Target1 blend One/Zero/Add.
    // neverOutlineAndRemoveSimplifier zeroes both instead.
    let simplifiers: HashMap<u32, bool> =
        def.scripts.iter().filter(|s| s.class == "EnemySimplifier" && s.enabled).map(|s| (s.node, s.data.get("neverOutlineAndRemoveSimplifier").i64() != 0)).collect();
    let mut simplified: HashMap<(String, i64, bool), Arc<MaterialProps>> = HashMap::new();
    // every particle / trail material the effect prefabs use: a template draw (a quad fixing the
    // vertex layout), copied into PARTICLE_SLOTS slots below
    let mut particle_keys: Vec<uk_assets::scene::MaterialKey> = Vec::new();
    for pf in &def.particle_prefabs {
        for r in pf.nodes.iter().filter_map(|n| n.renderer.as_ref()) {
            for k in r.materials.iter().flatten() {
                if !particle_keys.iter().any(|q| q.file == k.file && q.path_id == k.path_id) {
                    particle_keys.push(k.clone());
                }
            }
        }
    }
    let particle_defs: Vec<uk_assets::scenedef::RenderDef> = particle_keys
        .iter()
        .map(|k| uk_assets::scenedef::RenderDef {
            node: 0,
            material: Some(k.clone()),
            batch: uk_assets::scene::Batch {
                positions: vec![[0.0; 3]; 4],
                normals: vec![[0.0, 0.0, 1.0]; 4],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                src: vec![0, 1, 2, 3],
            },
            enabled: false,
            skin: None,
        })
        .collect();
    let first_particle = def.renderers.len() + sky_def.is_some() as usize;
    let mut particle_slots: HashMap<(String, i64), Vec<usize>> = HashMap::new();
    for (ri, r) in def.renderers.iter().chain(sky_def.iter()).chain(particle_defs.iter()).enumerate() {
        let sky = ri == def.renderers.len() && sky_def.is_some();
        let particle = ri >= first_particle;
        if r.batch.indices.is_empty() || (!sky && !particle && skip_node(r.node)) {
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
        let Some((mut mat, skey)) = mat else {
            *skipped.entry("material unreadable".into()).or_default() += 1;
            continue;
        };
        if let Some(&never) = simplifiers.get(&r.node).filter(|_| !sky) {
            mat = simplified
                .entry((key.file.clone(), key.path_id, never))
                .or_insert_with(|| {
                    let mut m = (*mat).clone();
                    // prefs simplifyEnemies (default off; UNITY_SIMPLIFY_ENEMIES probes it, ignoring
                    // simplifiedDistance): SetOutline's shouldBeOutlined
                    let simplify_enemies = if !never && std::env::var_os("UNITY_SIMPLIFY_ENEMIES").is_some() { 1.0 } else { 0.0 };
                    for (k, v) in [("_Outline", simplify_enemies), ("_ForceOutline", if never { 0.0 } else { 0.5 }), ("_BlendOp1", 0.0), ("_SrcBlend1", 1.0), ("_DstBlend1", 0.0), ("_ForceOutlineBehind", 0.0)] {
                        m.floats.insert(k.into(), v);
                    }
                    Arc::new(m)
                })
                .clone();
        }
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
                let mut enabled: Vec<&str> = mat.keywords.iter().map(|s| s.as_str()).collect();
                // global keywords (GraphicsSettings)
                if prefs.vertex_warping != 0.0 {
                    enabled.push("VERTEX_WARPING");
                }
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
            let Slot::Texture { name, dim, .. } = s else { continue };
            let layers = match dim {
                TexDim::D2 => 1,
                TexDim::Cube => 6,
                TexDim::D3 => continue,
            };
            let Some(env) = mat.textures.get(name) else { continue };
            let f = db.file(&key.file).ok();
            let tid = f.and_then(|f| db.resolve(&f, env.texture).ok().flatten()).and_then(|(tf, tid)| {
                *texture_ids.entry((tf.name.clone(), tid)).or_insert_with(|| {
                    let v = tf.read_id(tid).ok()?;
                    let t = uk_assets::texture::decode_texture(db, &v).ok()?;
                    textures.push(TexCpu { width: t.width, height: t.height, rgba: t.rgba, filter: t.filter, wrap: t.wrap, layers: t.layers });
                    Some(textures.len() as u32 - 1)
                })
            });
            if let Some(t) = tid.filter(|&t| textures[t as usize].layers == layers) {
                tex.insert(name.clone(), t);
            }
        }
        let st = &pass.state;
        let mut state = DrawState {
            cull: state_u8(&st.cull, &mat),
            zwrite: st.zwrite.resolve(&mat.floats) >= 0.5,
            ztest: state_u8(&st.ztest, &mat),
            src: state_u8(&st.src_blend, &mat),
            dst: state_u8(&st.dst_blend, &mat),
            src_a: state_u8(&st.src_blend_alpha, &mat),
            dst_a: state_u8(&st.dst_blend_alpha, &mat),
            op: state_u8(&st.blend_op, &mat),
            mask: state_u8(&st.color_mask, &mat),
            rt1: st.rt[0].each_ref().map(|v| state_u8(v, &mat)),
            stencil: STENCIL_OFF,
        };
        if sky {
            // behind everything: drawn first, never tested or written (Unity's skybox sits at the far plane)
            state.ztest = 8;
            state.zwrite = false;
            state.cull = 0;
        }
        let skin = r.skin.as_ref().map(|def| {
            let (mut off, mut pos, mut nrm) = (0usize, None, None);
            for &(_, ch, n) in &var.inputs {
                match ch {
                    0 if n >= 3 => pos = Some(off),
                    1 if n >= 3 => nrm = Some(off),
                    _ => {}
                }
                off += n as usize * 4;
            }
            Arc::new(SkinDraw { def: def.clone(), src: b.src.clone(), stride: var.stride as usize, pos, nrm })
        });
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &b.positions {
            let p = Vec3::from(unity_point(*p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let queue = if sky {
            0
        } else if mat.render_queue >= 0 {
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
            // animated limbs leave the bind-pose bounds
            radius: (hi - lo).length() * if skin.is_some() { 1.0 } else { 0.5 } + if skin.is_some() { 1.0 } else { 0.0 },
            skin,
            hud: !sky && !particle && def.nodes[r.node as usize].layer == uk_assets::scenedef::VIEWMODEL_LAYER,
            sky,
            particle,
        });
        if particle {
            let t = draws.len() - 1;
            let slots: Vec<usize> = (0..PARTICLE_SLOTS).map(|k| if k == 0 { t } else { draws.push(draws[t].clone()); draws.len() - 1 }).collect();
            particle_slots.insert((key.file.clone(), key.path_id), slots);
        }
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
        let pass = sh.passes.first()?;
        let mut post_variants = [None; 16];
        for (mask, slot) in post_variants.iter_mut().enumerate() {
            let mut kw = kw.clone();
            kw.extend(POST_KEYWORDS.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 1).map(|(_, k)| *k));
            let Some(vsub) = sh.select(&pass.vertex, &kw).cloned() else { continue };
            match make_variant(&sh, &vsub, shaders, variants.len()) {
                Ok(var) => {
                    variants.push(var);
                    *slot = Some(variants.len() as u32 - 1);
                }
                Err(e) => warn!("post-process variant {kw:?}: {e}"),
            }
        }
        post_variants[0]?;
        // the hurt flash color: NewMovement.hurtScreen (a disabled Image the shader stands in for)
        let hurt_rgb = def
            .scripts
            .iter()
            .find(|s| s.class == "NewMovement")
            .and_then(|s| {
                let nf = db.file(s.file.as_deref().unwrap_or(&def.scene_file)).ok()?;
                let (imf, id) = db.resolve(&nf, s.data.get("hurtScreen").pptr()).ok().flatten()?;
                let c = imf.read_id(id).ok()?.get("m_Color").clone();
                Some([c.get("r").f32(), c.get("g").f32(), c.get("b").f32()])
            })
            .unwrap_or_else(|| {
                warn!("NewMovement.hurtScreen unresolved; hurt flash disabled");
                [0.0; 3]
            });
        // the material's textures, then PostProcessV2_Handler.Start's SetTexture calls on top
        let mut refs = Vec::new();
        for (name, t) in &props.textures {
            if let Some(r) = db.resolve(&mf, t.texture).ok().flatten() {
                refs.push((name.clone(), r));
            }
        }
        for (name, field) in [("_Dither", "ditherTexture"), ("_VignetteTex", "vignetteTexture")] {
            if let Some(r) = db.resolve(&f, h.data.get(field).pptr()).ok().flatten() {
                refs.retain(|(n, _)| n != name);
                refs.push((name.to_string(), r));
            }
        }
        let mut post_tex = Vec::new();
        for (name, (tf, tid)) in refs {
            let Some(t) = tf.read_id(tid).ok().and_then(|v| uk_assets::texture::decode_texture(db, &v).ok()) else {
                warn!("post-process texture {name} undecodable");
                continue;
            };
            textures.push(TexCpu { width: t.width, height: t.height, rgba: t.rgba, filter: t.filter, wrap: t.wrap, layers: t.layers });
            post_tex.push((name, textures.len() as u32 - 1));
        }
        Some(PostDef { variants: post_variants, hurt_rgb, material: Arc::new(props), textures: post_tex })
    })();
    // the outline composite: at default prefs (simplifyEnemies off) the handler blits the outline
    // buffer onto the scene with OutlinePx pass 3 at CameraEvent.AfterEverything
    let outline = (|| {
        let h = def.scripts.iter().find(|s| s.class == "PostProcessV2_Handler")?;
        let f = db.file(&def.scene_file).ok()?;
        let (sf, sid) = db.resolve(&f, h.data.get("outlinePx").pptr()).ok().flatten()?;
        let sh = ShaderAsset::from_value(&sf.read_id(sid).ok()?).ok()?;
        let pass = sh.passes.get(3)?;
        let vsub = sh.select(&pass.vertex, &[])?.clone();
        let var = make_variant(&sh, &vsub, shaders, variants.len()).map_err(|e| warn!("outline variant: {e}")).ok()?;
        variants.push(var);
        let (st, m) = (&pass.state, &MaterialProps::default());
        let state = DrawState {
            cull: state_u8(&st.cull, m),
            zwrite: false,
            ztest: 8,
            src: state_u8(&st.src_blend, m),
            dst: state_u8(&st.dst_blend, m),
            src_a: state_u8(&st.src_blend_alpha, m),
            dst_a: state_u8(&st.dst_blend_alpha, m),
            op: state_u8(&st.blend_op, m),
            mask: state_u8(&st.color_mask, m),
            rt1: [1, 0, 0, 0],
            stencil: STENCIL_OFF,
        };
        Some(OutlineDef { variant: variants.len() as u32 - 1, state })
    })();
    let rs = &def.render_settings;
    // uGUI: every texture a Graphic can show and every material a Graphic can draw with
    let mut ui_skipped: Vec<String> = Vec::new();
    let ui = ui.map(|(udef, assets)| {
        // `blank`: a dynamic font atlas (written at runtime in prepare), starts cleared
        let mut decode = |db: &mut AssetDb, file: &str, path_id: i64, blank: Option<&uk_assets::ui::UiTexture>| -> Option<u32> {
            *texture_ids.entry((file.to_string(), path_id)).or_insert_with(|| {
                if let Some(t) = blank {
                    textures.push(TexCpu { width: t.width, height: t.height, rgba: vec![0; (t.width * t.height * 4) as usize], filter: t.filter, wrap: t.wrap_u, layers: 1 });
                    return Some(textures.len() as u32 - 1);
                }
                let v = db.file(file).ok()?.read_id(path_id).ok()?;
                let t = uk_assets::texture::decode_texture(db, &v).ok()?;
                textures.push(TexCpu { width: t.width, height: t.height, rgba: t.rgba, filter: t.filter, wrap: t.wrap, layers: t.layers });
                Some(textures.len() as u32 - 1)
            })
        };
        let tex: Vec<Option<u32>> = assets
            .textures
            .iter()
            .enumerate()
            .map(|(i, t)| match &t.dynamic {
                Some(_) => decode(db, "", -1 - i as i64, Some(t)),
                None => decode(db, &t.file, t.path_id, None),
            })
            .collect();
        let mut mats = Vec::new();
        for m in &assets.materials {
            let Some(sh) = db.file(&m.shader.0).ok().and_then(|f| f.read_id(m.shader.1).ok()).and_then(|v| ShaderAsset::from_value(&v).ok()) else {
                ui_skipped.push(format!("{}: shader unreadable", m.name));
                mats.push(None);
                continue;
            };
            let Some(pass) = sh.passes.first() else {
                mats.push(None);
                continue;
            };
            let base: Vec<&str> = m.props.keywords.iter().map(|s| s.as_str()).filter(|k| *k != "UNITY_UI_CLIP_RECT" && *k != "UNITY_UI_ALPHACLIP").collect();
            let mut vs = [None; 4];
            for (bits, slot) in vs.iter_mut().enumerate() {
                let mut kw = base.clone();
                if bits & 1 != 0 {
                    kw.push("UNITY_UI_CLIP_RECT");
                }
                if bits & 2 != 0 {
                    kw.push("UNITY_UI_ALPHACLIP");
                }
                let Some(vsub) = sh.select(&pass.vertex, &kw).cloned() else { continue };
                if let Some(&v) = variant_ids.get(&(m.shader.0.clone(), m.shader.1, vsub.blob)) {
                    *slot = v;
                    continue;
                }
                let v = match make_variant(&sh, &vsub, shaders, variants.len()) {
                    Ok(var) => {
                        variants.push(var);
                        Some(variants.len() as u32 - 1)
                    }
                    Err(e) => {
                        ui_skipped.push(format!("{} {kw:?}: {}", sh.name, e.chars().take(80).collect::<String>()));
                        None
                    }
                };
                variant_ids.insert((m.shader.0.clone(), m.shader.1, vsub.blob), v);
                *slot = v;
            }
            let Some(v0) = vs.iter().flatten().next().copied() else {
                mats.push(None);
                continue;
            };
            let mut textures_m = HashMap::new();
            let mf = db.file(&m.file).ok();
            for s in variants[v0 as usize].groups[0].clone().iter() {
                let Slot::Texture { name, dim: TexDim::D2, .. } = s else { continue };
                let Some(env) = m.props.textures.get(name) else { continue };
                let Some((tf, tid)) = mf.as_ref().and_then(|f| db.resolve(f, env.texture).ok().flatten()) else { continue };
                if let Some(t) = decode(db, &tf.name.clone(), tid, None) {
                    textures_m.insert(name.clone(), t);
                }
            }
            let st = &pass.state;
            let state = [8.0, 4.0].map(|zt| {
                let mut p = (*m.props).clone();
                p.floats.insert("unity_GUIZTestMode".into(), zt);
                DrawState {
                    cull: state_u8(&st.cull, &p),
                    zwrite: st.zwrite.resolve(&p.floats) >= 0.5,
                    ztest: state_u8(&st.ztest, &p),
                    src: state_u8(&st.src_blend, &p),
                    dst: state_u8(&st.dst_blend, &p),
                    src_a: state_u8(&st.src_blend_alpha, &p),
                    dst_a: state_u8(&st.dst_blend_alpha, &p),
                    op: state_u8(&st.blend_op, &p),
                    mask: state_u8(&st.color_mask, &p),
                    rt1: st.rt[0].each_ref().map(|v| state_u8(v, &p)),
                    stencil: [1, 2, 3, 4, 5, 6].map(|i| state_u8(&st.stencil[i], &p)),
                }
            });
            mats.push(Some(UiMat { props: m.props.clone(), variants: vs, state, stencil_ref: state_u8(&st.stencil[0], &m.props), textures: textures_m }));
        }
        UiScene { def: udef, assets, tex, mats }
    });
    let lights = def
        .lights
        .iter()
        .map(|l| LightCpu { node: l.node, kind: l.kind, color: l.color, intensity: l.intensity, range: l.range, spot_angle: l.spot_angle, enabled: l.enabled })
        .collect();
    let total: usize = draws.len() + skipped.values().sum::<usize>();
    let mut sk: Vec<_> = skipped.into_iter().collect();
    sk.sort_by(|a, b| b.1.cmp(&a.1));
    let enemy_draws: Vec<&Draw> = draws.iter().filter(|d| simplifiers.contains_key(&d.node)).collect();
    let summary = format!(
        "unity shaders: post-process {}, outline {}, sky {} (camera {:?}), enemy simplifiers {} on {} draws ({} write SV_Target1: {:?}), {} variants, {}/{} renderers drawn, {} textures, {} lights; skipped: {:?}; ui: {}; particle materials {}/{}: {:?}",
        post.as_ref().map_or("missing".to_string(), |p| {
            let n = p.variants.iter().flatten().count();
            let tex: Vec<String> = p.textures.iter().map(|(n, t)| format!("{n} {}x{}", textures[*t as usize].width, textures[*t as usize].height)).collect();
            format!("{} ({n}/16 keyword variants) textures {tex:?}", p.variants[0].map_or("?", |v| &variants[v as usize].label))
        }),
        outline.as_ref().map_or("missing".to_string(), |o| format!("{} state {:?}", variants[o.variant as usize].label, o.state)),
        draws.iter().find(|d| d.sky).map_or("none".to_string(), |d| format!("{} {:?} textures {:?}", variants[d.variant as usize].label, d.material.name, d.textures.values().map(|&t| (textures[t as usize].width, textures[t as usize].layers)).collect::<Vec<_>>())),
        def.render_settings.camera_clear,
        simplifiers.len(),
        enemy_draws.len(),
        enemy_draws.iter().filter(|d| variants[d.variant as usize].outputs.contains(&1)).count(),
        enemy_draws.iter().map(|d| (variants[d.variant as usize].label.as_str(), variants[d.variant as usize].outputs.clone(), d.state.rt1)).collect::<std::collections::BTreeSet<_>>(),
        variants.len(),
        draws.len(),
        total,
        textures.len(),
        def.lights.len(),
        sk.iter().take(6).collect::<Vec<_>>(),
        ui.as_ref().map_or("none".to_string(), |u| format!(
            "{} canvases, {}/{} textures, {}/{} materials ({} variants: {:?}), skipped {:?}",
            u.def.canvases.len(),
            u.tex.iter().flatten().count(),
            u.tex.len(),
            u.mats.iter().flatten().count(),
            u.mats.len(),
            u.mats.iter().flatten().map(|m| m.variants.iter().flatten().count()).sum::<usize>(),
            u.mats.iter().zip(&u.assets.materials).filter_map(|(m, a)| m.as_ref().map(|m| (a.name.as_str(), m.state[0].ztest, m.state[0].src, m.state[0].dst, m.state[0].zwrite))).collect::<Vec<_>>(),
            ui_skipped
        )),
        particle_slots.len(),
        particle_keys.len(),
        particle_slots.values().map(|v| {
            let d = &draws[v[0]];
            (d.material.name.as_str(), variants[d.variant as usize].label.as_str(), d.queue, d.state.src, d.state.dst, d.state.zwrite, d.state.cull, variants[d.variant as usize].inputs.clone())
        }).collect::<Vec<_>>()
    );
    let scene = SceneData {
        generation,
        variants,
        draws,
        textures,
        lights,
        fog: (rs.fog, rs.fog_color, rs.fog_start, rs.fog_end),
        ambient: rs.ambient_sky,
        clear: def.render_settings.camera_clear.map_or(CLEAR, |(_, c)| [c[0], c[1], c[2], 1.0]),
        composite_shader: shaders.add(Shader::from_wgsl(COMPOSITE_WGSL, "unity/composite.wgsl")),
        post,
        // the HUD Camera component: fov 90, culling mask layer 13, depth-only clear
        hud_cam: def.find("HUD Camera").map(|h| (def.nodes[h as usize].world0, 90.0)),
        outline,
        prefs,
        ui,
        particle_slots,
    };
    (scene, summary)
}

/// The skybox mesh: a UV sphere (sky shaders take the view ray from the vertex position; the
/// procedural sky is evaluated per vertex, so it needs Unity's fine tessellation).
fn sky_sphere() -> uk_assets::scene::Batch {
    let (seg, rings) = (64u32, 32u32);
    let mut b = uk_assets::scene::Batch::default();
    for j in 0..=rings {
        let th = std::f32::consts::PI * j as f32 / rings as f32;
        for i in 0..=seg {
            let ph = std::f32::consts::TAU * i as f32 / seg as f32;
            let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            b.positions.push(d.map(|c| c * 10.0));
            b.normals.push(d.map(|c| -c));
            b.uvs.push([i as f32 / seg as f32, 1.0 - j as f32 / rings as f32]);
            b.colors.push([1.0; 4]);
            b.src.push(b.src.len() as u32);
        }
    }
    for j in 0..rings {
        for i in 0..seg {
            let a = j * (seg + 1) + i;
            let c = a + seg + 1;
            b.indices.extend_from_slice(&[a, c, a + 1, a + 1, c, c + 1]);
        }
    }
    b
}

/// Re-skins every skinned draw at the scene's rest pose and returns (draws checked, largest
/// position error vs the baked vertex bytes): proves the vertex mapping the animated path writes through.
pub fn skin_rest_error(game: &uk_game::Game, scene: &SceneData) -> (usize, f32) {
    let f = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    let rest = |n: u32| f * game.def.nodes[n as usize].world0 * f;
    let (mut n, mut err) = (0, 0.0f32);
    let (mut pos, mut nrm, mut mir) = (Vec::new(), Vec::new(), Vec::new());
    for d in &scene.draws {
        let Some(sk) = &d.skin else { continue };
        let Some(o) = sk.pos else { continue };
        let bones: Vec<Mat4> = sk.def.bones.iter().map(|b| rest(b.unwrap_or(d.node))).collect();
        uk_assets::scenedef::skin_vertices(&sk.def.mesh, &bones, &mut pos, &mut nrm, &mut mir);
        n += 1;
        for (i, &src) in sk.src.iter().enumerate() {
            let b = &d.vertices[i * sk.stride + o..i * sk.stride + o + 12];
            let baked = Vec3::from_slice(bytemuck::cast_slice::<u8, f32>(b));
            err = err.max(pos.get(src as usize).map_or(f32::MAX, |p| p.distance(baked)));
        }
    }
    (n, err)
}

/// Per-frame state from the game: what is visible, where movers have moved things, lights.
/// The live effects' ParticleSystemRenderers: billboards (render mode 0) and trails, in world
/// space, each renderer in its own slot of its material.
fn particle_draws(game: &uk_game::Game, scene: &SceneData) -> Vec<ParticleDraw> {
    let mut out: Vec<ParticleDraw> = Vec::new();
    let mut used: HashMap<(String, i64), usize> = HashMap::new();
    let mut slot_of = |out: &mut Vec<ParticleDraw>, key: &uk_assets::scene::MaterialKey, r: &uk_assets::particles::ParticleRendererDef| -> Option<usize> {
        let slots = scene.particle_slots.get(&(key.file.clone(), key.path_id))?;
        let n = used.entry((key.file.clone(), key.path_id)).or_default();
        let slot = slots[(*n).min(slots.len() - 1)];
        *n += 1;
        Some(out.iter().position(|d| d.slot == slot).unwrap_or_else(|| {
            out.push(ParticleDraw { slot, min_size: r.min_particle_size, max_size: r.max_particle_size, ..Default::default() });
            out.len() - 1
        }))
    };
    for e in game.fx.effects.iter().filter(|e| !e.destroyed && e.active) {
        let pf = &game.def.particle_prefabs[e.prefab as usize];
        for sys in &e.systems {
            if sys.particles.is_empty() || !e.node_active_in_hierarchy(pf, sys.node) {
                continue;
            }
            let Some(r) = pf.nodes[sys.node as usize].renderer.as_ref().filter(|r| r.enabled && r.render_mode == 0) else { continue };
            let def = &sys.def;
            let scale = sys.sim_to_world.x_axis.truncate().length();
            let mut quads = Vec::with_capacity(sys.particles.len());
            let mut strips = Vec::new();
            for p in &sys.particles {
                let pos = sys.sim_to_world.transform_point3(p.pos);
                let size = p.size(def) * scale;
                let color = p.color(def);
                quads.push((pos, size, p.rot, color));
                let (Some(tr), Some(td)) = (&p.trail, &def.trail) else { continue };
                // head (the particle) first, then the recorded points newest to oldest
                let pts: Vec<Vec3> = std::iter::once(pos).chain(tr.points.iter().rev().map(|(q, _)| sys.sim_to_world.transform_point3(*q))).collect();
                let mut along = vec![0.0f32; pts.len()];
                for k in 1..pts.len() {
                    along[k] = along[k - 1] + pts[k].distance(pts[k - 1]);
                }
                let total = along.last().copied().unwrap_or(0.0);
                if pts.len() < 2 || total <= 1e-5 {
                    continue;
                }
                let life = td.color_over_lifetime.eval(p.age / p.lifetime, p.rand[0]);
                let base = uk_game::particles::mul4(if td.inherit_particle_color { color } else { [1.0; 4] }, life);
                strips.push(
                    pts.iter()
                        .zip(&along)
                        .map(|(q, a)| {
                            let u = a / total;
                            let w = td.width_over_trail.eval(u, p.rand[0]) * if td.size_affects_width { size } else { 1.0 };
                            (*q, w, uk_game::particles::mul4(base, td.color_over_trail.eval(u, p.rand[0])), u)
                        })
                        .collect::<Vec<_>>(),
                );
            }
            if let Some(Some(k)) = r.materials.first() {
                if let Some(i) = slot_of(&mut out, k, r) {
                    out[i].quads.extend(quads);
                }
            }
            if let (Some(Some(k)), false) = (r.materials.get(1), strips.is_empty()) {
                if let Some(i) = slot_of(&mut out, k, r) {
                    out[i].strips.extend(strips);
                }
            }
        }
    }
    for d in &mut out {
        let (mut lo, mut hi, mut big) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 0.0f32);
        for &(p, s, _, _) in &d.quads {
            (lo, hi, big) = (lo.min(p), hi.max(p), big.max(s));
        }
        for &(p, w, _, _) in d.strips.iter().flatten() {
            (lo, hi, big) = (lo.min(p), hi.max(p), big.max(w));
        }
        d.center = (lo + hi) * 0.5;
        d.radius = (hi - lo).length() * 0.5 + big;
    }
    out
}

pub fn frame(game: &uk_game::Game, scene: &SceneData, time: f32, screen: [f32; 2]) -> UnityFrame {
    let m = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    let particles = particle_draws(game, scene);
    let mut visible: Vec<bool> = scene.draws.iter().map(|d| d.sky || (d.enabled && game.active(d.node))).collect();
    for p in &particles {
        visible[p.slot] = true;
    }
    let object_to_world = scene
        .draws
        .iter()
        .map(|d| {
            if d.particle {
                return Mat4::IDENTITY;
            }
            let mover = match game.node_mover[d.node as usize] {
                Some(mv) => m * Mat4::from(game.mover_delta(mv)) * m,
                None => Mat4::IDENTITY,
            };
            // skinned vertices already carry the animated bones; rigid ones follow their node
            if d.skin.is_some() { mover } else { mover * game.anim.delta_of(d.node) }
        })
        .collect::<Vec<_>>();
    let mut skinned = Vec::new();
    let (mut pos, mut nrm, mut mir) = (Vec::new(), Vec::new(), Vec::new());
    for (di, d) in scene.draws.iter().enumerate() {
        let Some(sk) = &d.skin else { continue };
        if !visible[di] || (sk.pos.is_none() && sk.nrm.is_none()) || !sk.def.bones.iter().flatten().any(|&b| game.anim.animated(b)) {
            continue;
        }
        let fallback = game.anim.world_of(d.node);
        let bones: Vec<Mat4> = sk.def.bones.iter().map(|b| b.map_or(fallback, |b| game.anim.world_of(b))).collect();
        uk_assets::scenedef::skin_vertices(&sk.def.mesh, &bones, &mut pos, &mut nrm, &mut mir);
        let mut bytes = d.vertices.clone();
        for (i, &src) in sk.src.iter().enumerate() {
            let base = i * sk.stride;
            for (off, v) in [(sk.pos, pos.get(src as usize)), (sk.nrm, nrm.get(src as usize))] {
                if let (Some(o), Some(v)) = (off, v) {
                    if base + o + 12 <= bytes.len() {
                        bytes[base + o..base + o + 12].copy_from_slice(bytemuck::cast_slice(&v.to_array()));
                    }
                }
            }
        }
        skinned.push((di, bytes));
    }
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
    let s = &game.s;
    // uGUI: canvases laid out and meshed at the window's pixel size (Screen.width/height)
    let ui = scene.ui.as_ref().map(|u| {
        let f = uk_game::ugui::build_frame(&uk_game::ugui::UiInput {
            def: &game.def,
            ui: &u.def,
            assets: &u.assets,
            state: &s.ui,
            active: &s.active,
            script_enabled: &s.script_enabled,
            screen,
            dpi: 96.0,
            world_of: Some(&|n| game.node_world(n)),
        });
        // a world-space root draws with its node's current (Unity-space) world matrix
        let world = f.batches.iter().map(|b| if b.mode == uk_game::ugui::RenderMode::World { game.node_world(b.root_node) } else { b.to_screen }).collect();
        (Arc::new(f), Arc::new(world))
    });
    UnityFrame {
        ui,
        time,
        visible: Arc::new(visible),
        object_to_world: Arc::new(object_to_world),
        skinned: Arc::new(skinned),
        lights: Arc::new(lights),
        hurt: s.hurt_alpha,
        // DeathSequence.Update: timeSinceDeath * 0.5 for its first 2 s, then held
        dead: s.dead.then(|| s.dead_timer.min(2.0) * 0.5),
        underwater: s.underwater_overlay,
        vignette: s.vignette,
        noise: s.screen_noise,
        hud_cam: scene.hud_cam.and(game.sway).map(|sw| game.node_world_bevy(sw.hud_cam)),
        particles: Arc::new(particles),
    }
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
    /// keyed by (variant, state, main camera): the main camera renders color + the outline buffer,
    /// the HUD Camera color only
    pipelines: HashMap<(u32, DrawState, bool), CachedRenderPipelineId>,
    target: Option<(UVec2, Texture, TextureView, TextureView)>,
    /// PostProcessV2's reusableBufferA: the main camera's second target (outline RG), cleared black each frame.
    outline_target: Option<(Texture, TextureView)>,
    outline: Option<OutlineGpu>,

    outline_readback: std::sync::Mutex<Option<Buffer>>,
    /// `UNITY_FRAME_STATS`: one frame of the scene target copied back for numeric checks.
    readback: std::sync::Mutex<Option<(Buffer, u32, UVec2, bool)>>,
    frames: std::sync::atomic::AtomicU32,
    started: std::sync::Mutex<Option<std::time::Instant>>,
    /// Cached light choice per draw, valid for `light_key` (which lights are on).
    light_pick: Vec<Vec<usize>>,
    light_key: u64,
    /// Draws inside the view frustum this frame.
    in_view: Vec<bool>,
    /// this frame's particle slots: bounds centre and radius (Unity space)
    centers: HashMap<usize, (Vec3, f32)>,
    /// One uniform buffer for every draw's constant buffers (256-byte aligned slices) and its CPU copy.
    ubo: Option<Buffer>,
    staging: Vec<u8>,
    composite: Option<(CachedRenderPipelineId, BindGroupLayoutDescriptor, Sampler)>,
    /// PostProcessV2 per POST_KEYWORDS mask (None where the variant is missing)
    post: Vec<Option<PostGpu>>,
    /// PostProcessV2 output (what the Virtual Camera puts on screen), same size as the scene target.
    post_target: Option<(Texture, TextureView)>,
    /// the post output's readback: buffer, row pitch, size (the screen's)
    post_readback: std::sync::Mutex<Option<(Buffer, u32, UVec2)>>,
    /// every scene texture, and the stand-ins for unbound slots (kept for the per-frame UI draws)
    tex_views: Vec<TextureView>,
    /// dynamic TMP font atlases: (texture, runtime atlas, uploaded version)
    dyn_atlases: Vec<(Texture, Arc<std::sync::Mutex<uk_assets::ui::DynAtlas>>, u64)>,
    fallback: Option<(TextureView, TextureView, TextureView, Buffer)>,
    /// this frame's uGUI draws, in draw order
    ui_draws: Vec<UiGpuDraw>,
    /// the overlay canvases' depth-stencil (screen size)
    ui_depth: Option<(UVec2, TextureView)>,
    /// `UNITY_FRAME_STATS`: the overlay target before the overlay canvases drew
    ui_readback: std::sync::Mutex<Option<Buffer>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UiPass {
    /// Screen Space - Overlay (and Camera, see PARITY.md): after the post pass, at screen resolution
    Overlay,
    /// World Space canvases rendered by the Main Camera / the HUD Camera (layer 13)
    Main,
    Hud,
}

struct UiGpuDraw {
    vbuf: Buffer,
    ibuf: Buffer,
    count: u32,
    bg0: BindGroup,
    bg1: BindGroup,
    pipeline: CachedRenderPipelineId,
    stencil_ref: u32,
    pass: UiPass,
}

impl UnityGpu {
    /// the post pass for a keyword mask; a missing combination falls back to the material's
    fn post_for(&self, mask: usize) -> Option<&PostGpu> {
        self.post.get(mask).and_then(Option::as_ref).or_else(|| self.post.first()?.as_ref())
    }
}

impl PostGpu {
    /// a sampled texture by shader name (every non-scene slot of the variant is bound at build)
    fn texture(&self, name: &str) -> &(TextureView, Sampler) {
        &self.textures.iter().find(|(n, _)| n == name).expect("post texture slot bound at build").1
    }
}

struct OutlineGpu {
    pipeline: CachedRenderPipelineId,
    layouts: [BindGroupLayoutDescriptor; 2],
    ubufs: Vec<(u32, Buffer, usize)>,
    sampler: Sampler,
}

struct PostGpu {
    variant: u32,
    pipeline: CachedRenderPipelineId,
    layouts: [BindGroupLayoutDescriptor; 2],
    ubufs: Vec<(u32, Buffer, usize)>,
    vbuf: Buffer,
    /// non-scene textures by shader name
    textures: Vec<(String, (TextureView, Sampler))>,
    main_sampler: Sampler,
}

/// PostProcessV2's constant buffers: the handler's globals at default settings (pixelization off ->
/// full resolution, colorCompression -> 32 levels, dithering 0.2, gamma 1, no hurt flash). The quad
/// is a clip-space triangle (identity matrices); `_ProjectionParams.x = -1` is Unity's flipped
/// render-texture convention, since the scene target is stored top row first.
/// The per-frame globals PostProcessV2 reads beyond the handler's settings.
#[derive(Default, Clone, Copy)]
struct PostGlobals {
    time: f32,
    hurt: [f32; 4],
    deathness: f32,
    underwater: [f32; 4],
    vignette: [f32; 4],
    noise: f32,
}

/// `vsize` is the virtual (pixelized) render size, `screen` the window's, `target` the size of the
/// texture the pass draws into (`_ScreenParams`).
#[allow(clippy::too_many_arguments)]
fn fill_post_cb(out: &mut [u8], cb: &uk_assets::shader::ConstantBuffer, mat: &MaterialProps, vsize: UVec2, screen: UVec2, target: UVec2, prefs: &GraphicsPrefs, g: PostGlobals) {
    let (size, t) = (vsize.as_vec2(), target.as_vec2());
    out.fill(0);
    for p in &cb.params {
        let v: Vec<f32> = if p.is_matrix {
            Mat4::IDENTITY.to_cols_array().to_vec()
        } else {
            let v = match p.name.as_str() {
                "_ProjectionParams" => [-1.0, NEAR, FAR, 1.0 / FAR],
                "_VirtualRes" => [size.x, size.y, 0.0, 0.0],
                "_ColorPrecision" => [prefs.color_precision, 0.0, 0.0, 0.0],
                "_DitherStrength" => [prefs.dither, 0.0, 0.0, 0.0],
                "_Gamma" => [prefs.gamma, 0.0, 0.0, 0.0],
                "_ResY" => [prefs.res_y, 0.0, 0.0, 0.0],
                "_HurtScreenColor" => g.hurt,
                "_Sharpness" | "_Deathness" => [g.deathness, 0.0, 0.0, 0.0],
                "_UnderwaterOverlay" => g.underwater,
                "_VignetteColor" => g.vignette,
                "_RandomNoiseStrength" => [g.noise, 0.0, 0.0, 0.0],
                "_Time" => [g.time / 20.0, g.time, g.time * 2.0, g.time * 3.0],
                "_ScreenParams" => [t.x, t.y, 1.0 + 1.0 / t.x, 1.0 + 1.0 / t.y],
                // OutlinePx: SetupOutlines (mainTex size vs the screen), forced to one pixel
                "_OutlineTex_TexelSize" | "_MainTex_TexelSize" => [1.0 / size.x, 1.0 / size.y, size.x, size.y],
                "_Resolution" => [size.x, size.y, 0.0, 0.0],
                "_ResolutionDiff" => [size.x / screen.x as f32, size.y / screen.y as f32, 0.0, 0.0],
                "_OutlineDistance" => [1.0, 0.0, 0.0, 0.0],
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
/// Unity RenderTextureFormat.RG16: two 8-bit channels.
const OUTLINE_FORMAT: TextureFormat = TextureFormat::Rg8Unorm;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32FloatStencil8;

/// Unity CompareFunction (0 Disabled, 1 Never .. 8 Always) for the stencil test (not reversed)
fn stencil_compare(c: u8) -> CompareFunction {
    match c {
        1 => CompareFunction::Never,
        2 => CompareFunction::Less,
        3 => CompareFunction::Equal,
        4 => CompareFunction::LessEqual,
        5 => CompareFunction::Greater,
        6 => CompareFunction::NotEqual,
        7 => CompareFunction::GreaterEqual,
        _ => CompareFunction::Always,
    }
}

/// Unity StencilOp
fn stencil_op(o: u8) -> StencilOperation {
    match o {
        1 => StencilOperation::Zero,
        2 => StencilOperation::Replace,
        3 => StencilOperation::IncrementClamp,
        4 => StencilOperation::DecrementClamp,
        5 => StencilOperation::Invert,
        6 => StencilOperation::IncrementWrap,
        7 => StencilOperation::DecrementWrap,
        _ => StencilOperation::Keep,
    }
}

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

/// Min/Max ignore the factors in D3D/Vulkan; WebGPU requires them to be One.
fn component(src: u8, dst: u8, op: u8) -> BlendComponent {
    let operation = blend_op(op);
    if matches!(operation, BlendOperation::Min | BlendOperation::Max) {
        return BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation };
    }
    BlendComponent { src_factor: blend_factor(src), dst_factor: blend_factor(dst), operation }
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
        size: Extent3d { width: t.width.max(1), height: t.height.max(1), depth_or_array_layers: t.layers },
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
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: t.layers },
    );
    tex.create_view(&TextureViewDescriptor { dimension: Some(if t.layers == 6 { TextureViewDimension::Cube } else { TextureViewDimension::D2 }), ..default() })
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
/// One vertex in a variant's input layout (Unity channels: 0 position, 1 normal, 2 tangent,
/// 3 color, 4+ texcoords).
fn push_vertex(out: &mut Vec<u8>, inputs: &[(u32, u32, u32)], p: Vec3, n: Vec3, c: [f32; 4], uv: [f32; 2]) {
    for &(_, ch, k) in inputs {
        let v: [f32; 4] = match ch {
            0 => [p.x, p.y, p.z, 1.0],
            1 => [n.x, n.y, n.z, 0.0],
            2 => [1.0, 0.0, 0.0, 1.0],
            3 => c,
            _ => [uv[0], uv[1], 0.0, 0.0],
        };
        for x in v.iter().take(k as usize) {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
}

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

/// RenderSettings' sun when unset: the brightest directional light that is on, as (direction
/// towards the light, color x intensity), Unity space.
fn sun(scene: &SceneData, frame: &UnityFrame) -> Option<(Vec3, [f32; 4])> {
    let lum = |l: &LightCpu| (l.color[0] * 0.3 + l.color[1] * 0.59 + l.color[2] * 0.11) * l.intensity;
    let (i, l) = scene
        .lights
        .iter()
        .enumerate()
        .filter(|(i, l)| l.kind == 1 && frame.lights.get(*i).is_some_and(|f| f.2))
        .max_by(|a, b| lum(a.1).total_cmp(&lum(b.1)))?;
    let c = l.color.map(|c| c * l.intensity);
    Some((-frame.lights[i].1, [c[0], c[1], c[2], 1.0]))
}

/// Fills one constant buffer from built-ins and material properties.
fn fill_cb(out: &mut [u8], cb: &uk_assets::shader::ConstantBuffer, scene: &SceneData, frame: &UnityFrame, di: usize, ctx: &FrameCtx, picked: &[usize]) {
    let d = &scene.draws[di];
    out.fill(0);
    let o2w = if d.sky { Mat4::from_translation(ctx.cam_pos) } else { frame.object_to_world.get(di).copied().unwrap_or(Mat4::IDENTITY) };
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
            // the skybox's sun (Procedural): the brightest directional light that is on
            "_WorldSpaceLightPos0" | "_LightColor0" if d.sky => match sun(scene, frame) {
                Some((_, col)) if n == "_LightColor0" => col,
                Some((dir, _)) => [dir.x, dir.y, dir.z, 0.0],
                None => [0.0; 4],
            },
            "_PortalClipPlane" | "_WorldSpaceLightPos0" | "_LightColor0" => [0.0; 4],
            // GraphicsSettings' globals from the player's prefs
            "_VertexWarping" => [scene.prefs.vertex_warping, 0.0, 0.0, 0.0],
            "_TextureWarping" => [scene.prefs.texture_warping, 0.0, 0.0, 0.0],
            "_StainWarping" => [scene.prefs.stain_warping, 0.0, 0.0, 0.0],
            "_ResY" => [scene.prefs.res_y, 0.0, 0.0, 0.0],
            "_HeightFog" => [0.0; 4],
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
                } else if n.ends_with("_HDR") && d.material.vector(n).is_none() {
                    // the texture's HDR decode values; LDR textures decode as rgb * 1
                    [1.0, 1.0, 0.0, 0.0]
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
        // per-draw caches belong to the previous scene
        gpu.light_pick.clear();
        gpu.in_view.clear();
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
        let mut tex_views: Vec<TextureView> = scene.textures.iter().map(|t| upload_texture(&dev, &queue, t)).collect();
        gpu.dyn_atlases.clear();
        if let Some(u) = &scene.ui {
            for (i, t) in u.assets.textures.iter().enumerate() {
                let (Some(d), Some(si)) = (&t.dynamic, u.tex[i]) else { continue };
                let tc = &scene.textures[si as usize];
                let tex = dev.create_texture(&TextureDescriptor {
                    label: Some("dynamic font atlas"),
                    size: Extent3d { width: tc.width.max(1), height: tc.height.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                tex_views[si as usize] = tex.create_view(&TextureViewDescriptor::default());
                // nothing uploaded yet: the first prepare writes the pixels as loaded
                gpu.dyn_atlases.push((tex, d.clone(), u64::MAX));
            }
        }
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
                            TexDim::Cube => d.textures.get(name).map(|&t| &tex_views[t as usize]).unwrap_or(&black_cube),
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
            let vbuf = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity vb"), contents: &d.vertices, usage: BufferUsages::VERTEX | BufferUsages::COPY_DST });
            let ibuf = dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity ib"), contents: bytemuck::cast_slice(&d.indices), usage: BufferUsages::INDEX | BufferUsages::COPY_DST });
            let key = (d.variant, d.state, !d.hud);
            let pipeline = *gpu.pipelines.entry(key).or_insert_with(|| cache.queue_render_pipeline(pipeline_descriptor(var, layouts, d.state, !d.hud)));
            draws.push(Some(GpuDraw { vbuf, ibuf, count: d.indices.len() as u32, ubufs: ubufs.clone(), bg0, bg1, pipeline }));
        }
        gpu.draws = draws;
        gpu.ubo = ubo;
        let build_post = |p: &PostDef, v: u32| {
            let var = &scene.variants[v as usize];
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
            let state = DrawState { cull: 0, zwrite: false, ztest: 8, src: 1, dst: 0, src_a: 1, dst_a: 0, op: 0, mask: 15, rt1: [1, 0, 0, 0], stencil: STENCIL_OFF };
            let mut desc = pipeline_descriptor(var, &layouts, state, false);
            desc.depth_stencil = None;
            // every sampled texture but the scene: the definition's, else white
            let textures = var.groups[0]
                .iter()
                .filter_map(|s| match s {
                    Slot::Texture { name, .. } if name != "_MainTex" => Some(name.clone()),
                    _ => None,
                })
                .map(|name| {
                    let t = p.textures.iter().find(|(n, _)| *n == name).map(|&(_, t)| t);
                    let bound = match t {
                        Some(t) => (tex_views[t as usize].clone(), sampler_for(&dev, scene.textures.get(t as usize))),
                        None => (white.clone(), sampler_for(&dev, None)),
                    };
                    (name, bound)
                })
                .collect();
            let main_sampler = dev.create_sampler(&SamplerDescriptor { label: Some("unity post main"), ..default() });
            PostGpu { variant: v, pipeline: cache.queue_render_pipeline(desc), layouts, ubufs, vbuf, textures, main_sampler }
        };
        gpu.post = scene.post.as_ref().map_or_else(Vec::new, |p| p.variants.iter().map(|v| v.map(|v| build_post(p, v))).collect());
        gpu.outline = scene.outline.as_ref().map(|o| {
            let var = &scene.variants[o.variant as usize];
            let layouts = [
                BindGroupLayoutDescriptor::new("unity outline g0", &layout_entries(&var.groups[0])),
                BindGroupLayoutDescriptor::new("unity outline g1", &layout_entries(&var.groups[1])),
            ];
            let ubufs = var.groups[1]
                .iter()
                .filter_map(|s| match s {
                    Slot::Uniform { binding, cb } => {
                        let size = var.params.constant_buffers.get(*cb).map_or(16, |c| c.size.next_multiple_of(16).max(16)) as u64;
                        let b = dev.create_buffer(&BufferDescriptor { label: Some("unity outline cb"), size, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false });
                        Some((*binding, b, *cb))
                    }
                    _ => None,
                })
                .collect();
            let mut desc = pipeline_descriptor(var, &layouts, o.state, false);
            desc.depth_stencil = None;
            // a Blit is never culled (Unity matches its winding to the flip); ours may wind either way
            desc.primitive.cull_mode = None;
            // the pass builds its triangle from the vertex index
            if var.inputs.is_empty() {
                desc.vertex.buffers.clear();
            }
            // Blit's source: a render texture at its default (bilinear) filtering
            let sampler = dev.create_sampler(&SamplerDescriptor { label: Some("unity outline"), mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
            OutlineGpu { pipeline: cache.queue_render_pipeline(desc), layouts, ubufs, sampler }
        });
        gpu.tex_views = tex_views;
        gpu.fallback = Some((white, black_cube, black_3d, zero_storage));
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
    let screen = UVec2::new(view.viewport.z.max(1), view.viewport.w.max(1));
    // pixelization: the cameras render at the virtual size, PostProcessV2 draws at the screen's
    let size = scene.prefs.virtual_size(screen);
    if gpu.target.as_ref().is_none_or(|t| t.0 != size) || gpu.post_target.as_ref().is_none_or(|t| t.0.width() != screen.x || t.0.height() != screen.y) {
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
            size: Extent3d { width: screen.x, height: screen.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pv = post.create_view(&TextureViewDescriptor::default());
        gpu.post_target = Some((post, pv));
        let ol = dev.create_texture(&TextureDescriptor {
            label: Some("unity outline buffer"),
            size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: OUTLINE_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let ov = ol.create_view(&TextureViewDescriptor::default());
        gpu.outline_target = Some((ol, ov));
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
    for (di, bytes) in frame.skinned.iter() {
        if let Some(Some(g)) = gpu.draws.get(*di) {
            queue.write_buffer(&g.vbuf, 0, bytes);
        }
    }
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
    // particles: camera-facing quads and trail strips, built for this camera
    gpu.centers.clear();
    {
        let iv = ctx.v.inverse();
        let (right, up, back) = (iv.x_axis.truncate().normalize(), iv.y_axis.truncate().normalize(), iv.z_axis.truncate().normalize());
        let (mut vb, mut ib) = (Vec::new(), Vec::new());
        for pd in frame.particles.iter() {
            let Some(Some(g)) = gpu.draws.get_mut(pd.slot) else { continue };
            let inputs = &scene.variants[scene.draws[pd.slot].variant as usize].inputs;
            vb.clear();
            ib.clear();
            let mut n = 0u32;
            for &(p, size, rot, col) in &pd.quads {
                // ParticleSystemRenderer min/maxParticleSize: fractions of the viewport height
                let depth = -ctx.v.transform_point3(p).z;
                let h = 2.0 * depth.max(0.0) / fy;
                let size = if h > 0.0 { size.clamp(pd.min_size * h, pd.max_size * h) } else { size };
                let (sn, cs) = (-rot).sin_cos();
                for (x, y, u, v) in [(-0.5f32, -0.5f32, 0.0f32, 0.0f32), (0.5, -0.5, 1.0, 0.0), (0.5, 0.5, 1.0, 1.0), (-0.5, 0.5, 0.0, 1.0)] {
                    let q = p + (right * (x * cs - y * sn) + up * (x * sn + y * cs)) * size;
                    push_vertex(&mut vb, inputs, q, back, col, [u, v]);
                }
                ib.extend_from_slice(&[n, n + 1, n + 2, n, n + 2, n + 3]);
                n += 4;
            }
            for strip in &pd.strips {
                for (k, &(q, w, col, u)) in strip.iter().enumerate() {
                    let a = strip[k.saturating_sub(1)].0;
                    let b = strip[(k + 1).min(strip.len() - 1)].0;
                    let side = (b - a).cross(ctx.cam_pos - q).normalize_or_zero() * (w * 0.5);
                    push_vertex(&mut vb, inputs, q - side, back, col, [u, 0.0]);
                    push_vertex(&mut vb, inputs, q + side, back, col, [u, 1.0]);
                    if k > 0 {
                        let o = n + 2 * k as u32;
                        ib.extend_from_slice(&[o - 2, o - 1, o + 1, o - 2, o + 1, o]);
                    }
                }
                n += 2 * strip.len() as u32;
            }
            if vb.len() as u64 > g.vbuf.size() {
                g.vbuf = dev.create_buffer(&BufferDescriptor { label: Some("unity particle vb"), size: (vb.len() as u64).next_power_of_two(), usage: BufferUsages::VERTEX | BufferUsages::COPY_DST, mapped_at_creation: false });
            }
            let ib_bytes: &[u8] = bytemuck::cast_slice(&ib);
            if ib_bytes.len() as u64 > g.ibuf.size() {
                g.ibuf = dev.create_buffer(&BufferDescriptor { label: Some("unity particle ib"), size: (ib_bytes.len() as u64).next_power_of_two(), usage: BufferUsages::INDEX | BufferUsages::COPY_DST, mapped_at_creation: false });
            }
            if !vb.is_empty() {
                queue.write_buffer(&g.vbuf, 0, &vb);
                queue.write_buffer(&g.ibuf, 0, ib_bytes);
            }
            g.count = ib.len() as u32;
            gpu.centers.insert(pd.slot, (pd.center, pd.radius));
        }
    }
    // HUD Camera: the viewmodel animates in its load-time world space, so its camera stays there too
    let hud_ctx = scene.hud_cam.map(|(w, fov)| {
        let w = frame.hud_cam.unwrap_or(w);
        let v = w.inverse() * mirror;
        let fy = 1.0 / (fov.to_radians() * 0.5).tan();
        let proj = Mat4::from_cols(proj.x_axis * (fy / proj.y_axis.y), proj.y_axis * (fy / proj.y_axis.y), proj.z_axis, proj.w_axis);
        let c = w.w_axis.truncate();
        FrameCtx { vp: proj * v, v, cam_pos: Vec3::new(c.x, c.y, -c.z), ..ctx }
    });
    let fno = gpu.frames.load(std::sync::atomic::Ordering::Relaxed);
    if let (Some(h), true) = (&hud_ctx, std::env::var_os("UNITY_FRAME_STATS").is_some() && (200..=400).contains(&fno) && fno % 10 == 0) {
        // viewmodel time series: visible (posed) layer-13 vertices on screen through the HUD Camera
        let (mut inside, mut total, mut shown) = (0usize, 0usize, 0usize);
        for (di, d) in scene.draws.iter().enumerate().filter(|(i, d)| d.hud && frame.visible.get(*i).copied().unwrap_or(true)) {
            shown += 1;
            let stride = scene.variants[d.variant as usize].stride as usize;
            let bytes = frame.skinned.iter().find(|(i, _)| *i == di).map_or(&d.vertices[..], |(_, b)| &b[..]);
            for k in 0..bytes.len() / stride {
                let f = |o: usize| f32::from_le_bytes(bytes[k * stride + o..k * stride + o + 4].try_into().unwrap());
                let c = h.vp * Vec4::new(f(0), f(4), f(8), 1.0);
                total += 1;
                inside += (c.w > 0.0 && c.x.abs() <= c.w && c.y.abs() <= c.w && c.z >= 0.0 && c.z <= c.w) as usize;
            }
        }
        info!("unity hud series f{fno}: visible draws {shown} vertices on screen {inside}/{total}");
    }
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && fno == 240 {
        let slots: Vec<String> = frame
            .particles
            .iter()
            .map(|p| format!("{}#{} in view {} quads {} strips {} count {}", scene.draws[p.slot].material.name, p.slot, gpu.in_view[p.slot], p.quads.len(), p.strips.len(), gpu.draws[p.slot].as_ref().map_or(0, |g| g.count)))
            .collect();
        info!("unity particle stats: {} slots {slots:?}", slots.len());
    }
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && fno == 240 {
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
        info!("unity camera (unity space) {:?} time {:.3}; sampled vertices inside frustum {inside}/{total}", ctx.cam_pos, ctx.time);
        if let Some(h) = &hud_ctx {
            // the viewmodel at rest through the HUD Camera: vertices on screen, and their screen bounds
            let (mut inside, mut total) = (0usize, 0usize);
            for (di, d) in scene.draws.iter().enumerate().filter(|(_, d)| d.hud) {
                let i0 = inside;
                let (lo, hi) = (&mut Vec2::splat(9.0), &mut Vec2::splat(-9.0));
                let stride = scene.variants[d.variant as usize].stride as usize;
                // the animated pose when this frame re-skinned it
                let bytes = frame.skinned.iter().find(|(i, _)| *i == di).map_or(&d.vertices[..], |(_, b)| &b[..]);
                let posed = bytes.as_ptr() != d.vertices.as_ptr();
                let mut sum = Vec3::ZERO;
                for k in 0..bytes.len() / stride {
                    let f = |o: usize| f32::from_le_bytes(bytes[k * stride + o..k * stride + o + 4].try_into().unwrap());
                    let c = h.vp * Vec4::new(f(0), f(4), f(8), 1.0);
                    sum += h.v.transform_point3(Vec3::new(f(0), f(4), f(8)));
                    total += 1;
                    if c.w > 0.0 && c.x.abs() <= c.w && c.y.abs() <= c.w && c.z >= 0.0 && c.z <= c.w {
                        inside += 1;
                        let n = Vec2::new(c.x, c.y) / c.w;
                        *lo = lo.min(n);
                        *hi = hi.max(n);
                    }
                }
                info!("  hud draw {di} node {} visible {} posed {posed} queue {} on screen {} ndc {lo:.2?}..{hi:.2?} view centroid {:.2?}", d.node, frame.visible.get(di).copied().unwrap_or(true), d.queue, inside - i0, sum / (bytes.len() / stride).max(1) as f32);
            }
            info!("unity hud camera {:?}: viewmodel vertices on screen {inside}/{total}", h.cam_pos);
        }
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
    let sky_only = std::env::var_os("UNITY_SKY_ONLY").is_some();
    let mut in_view = std::mem::take(&mut gpu.in_view);
    in_view.clear();
    in_view.extend(scene.draws.iter().enumerate().map(|(di, d)| {
        if !frame.visible.get(di).copied().unwrap_or(true) {
            return false;
        }
        if d.hud {
            return hud_ctx.is_some();
        }
        if d.sky {
            return true;
        }
        // `UNITY_SKY_ONLY`: the skybox alone, so frame stats measure what the sky draws
        if sky_only {
            return false;
        }
        let (c, r) = match gpu.centers.get(&di) {
            Some(&cr) => cr,
            None if d.particle => return false,
            None => (frame.object_to_world.get(di).map_or(d.center, |o| o.transform_point3(d.center)), d.radius),
        };
        planes.iter().all(|p| p.truncate().dot(c) + p.w >= -r)
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
            let c = if scene.draws[di].hud { hud_ctx.as_ref().unwrap_or(&ctx) } else { &ctx };
            fill_cb(&mut staging[off as usize..(off + size) as usize], cbd, &scene, &frame, di, c, &gpu.light_pick[di]);
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
    // FontEngine wrote glyphs into a dynamic atlas (Texture2D.Apply)
    for (tex, d, uploaded) in &mut gpu.dyn_atlases {
        let d = d.lock().unwrap();
        if *uploaded == d.version || d.pixels.is_empty() {
            continue;
        }
        *uploaded = d.version;
        let rgba: Vec<u8> = d.pixels.iter().flat_map(|&a| [0, 0, 0, a]).collect();
        queue.write_texture(
            tex.as_image_copy(),
            &rgba,
            TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(d.width * 4), rows_per_image: Some(d.height) },
            Extent3d { width: d.width, height: d.height, depth_or_array_layers: 1 },
        );
    }
    prepare_ui(gpu, &scene, &frame, &dev, &cache, &ctx, hud_ctx.as_ref(), screen);
    if let Some(pd) = scene.post.as_ref() {
        let g = PostGlobals {
            time: frame.time,
            // NewMovement.Update: _HurtScreenColor = hurtColor with currentColor.a
            hurt: [pd.hurt_rgb[0], pd.hurt_rgb[1], pd.hurt_rgb[2], frame.hurt],
            deathness: frame.dead.unwrap_or(0.0),
            underwater: frame.underwater.unwrap_or_default(),
            vignette: frame.vignette.unwrap_or_default(),
            noise: frame.noise.unwrap_or(0.0),
        };
        if let Some(post) = gpu.post_for(frame.post_mask()) {
            let var = &scene.variants[post.variant as usize];
            for (_, buf, cb) in &post.ubufs {
                let Some(cbd) = var.params.constant_buffers.get(*cb) else { continue };
                let mut bytes = vec![0u8; buf.size() as usize];
                fill_post_cb(&mut bytes, cbd, &pd.material, size, screen, screen, &scene.prefs, g);
                queue.write_buffer(buf, 0, &bytes);
            }
        }
    }
    if let (Some(og), Some(od)) = (&gpu.outline, scene.outline.as_ref()) {
        let var = &scene.variants[od.variant as usize];
        for (_, buf, cb) in &og.ubufs {
            let Some(cbd) = var.params.constant_buffers.get(*cb) else { continue };
            let mut bytes = vec![0u8; buf.size() as usize];
            fill_post_cb(&mut bytes, cbd, &MaterialProps::default(), size, screen, size, &scene.prefs, PostGlobals::default());
            queue.write_buffer(buf, 0, &bytes);
        }
    }
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && gpu.frames.load(std::sync::atomic::Ordering::Relaxed) == 240 {
        let hud = scene.draws.iter().enumerate().filter(|(i, d)| d.hud && gpu.in_view[*i]).count();
        info!("unity prepare: {:.2} ms; draws in view {}/{} (hud camera {hud}/{})", t_prepare.elapsed().as_secs_f64() * 1e3, gpu.in_view.iter().filter(|v| **v).count(), gpu.in_view.len(), scene.draws.iter().filter(|d| d.hud).count());
    }
}

/// uGUI: one vertex/index buffer, constant buffers and bind groups per draw, rebuilt every frame
/// (the canvases re-mesh every frame here; performance is not critical).
#[allow(clippy::too_many_arguments)]
fn prepare_ui(gpu: &mut UnityGpu, scene: &SceneData, frame: &UnityFrame, dev: &RenderDevice, cache: &PipelineCache, ctx: &FrameCtx, hud_ctx: Option<&FrameCtx>, screen: UVec2) {
    gpu.ui_draws.clear();
    let (Some(u), Some((uf, mats))) = (scene.ui.as_ref(), frame.ui.as_ref()) else { return };
    if gpu.fallback.is_none() {
        return;
    }
    if gpu.ui_depth.as_ref().is_none_or(|d| d.0 != screen) {
        let t = dev.create_texture(&TextureDescriptor {
            label: Some("unity ui depth"),
            size: Extent3d { width: screen.x, height: screen.y, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        gpu.ui_depth = Some((screen, t.create_view(&TextureViewDescriptor::default())));
    }
    let (white, black_cube, black_3d, zero_storage) = gpu.fallback.as_ref().unwrap();
    // Screen Space - Overlay: pixels (bottom-left origin) -> clip space
    let (w, h) = (screen.x as f32, screen.y as f32);
    let overlay = FrameCtx {
        vp: Mat4::from_cols(Vec4::new(2.0 / w, 0.0, 0.0, 0.0), Vec4::new(0.0, 2.0 / h, 0.0, 0.0), Vec4::ZERO, Vec4::new(-1.0, -1.0, 0.5, 1.0)),
        v: Mat4::IDENTITY,
        cam_pos: Vec3::ZERO,
        size: screen,
        ..*ctx
    };
    let mut draws = Vec::new();
    let mut new_pipelines = Vec::new();
    // skipped draws: [no indices, no material, no variant]
    let mut skipped = [0usize; 3];
    let mut skipped_mats = std::collections::BTreeSet::new();
    for (bi, b) in uf.batches.iter().enumerate() {
        let world = b.mode == uk_game::ugui::RenderMode::World;
        let (pass, c) = match (world, b.layer) {
            (false, _) => (UiPass::Overlay, &overlay),
            (true, 13) => match hud_ctx {
                Some(h) => (UiPass::Hud, h),
                None => continue,
            },
            (true, _) => (UiPass::Main, ctx),
        };
        let o2w = mats.get(bi).copied().unwrap_or(Mat4::IDENTITY);
        for d in &b.draws {
            if d.idx.is_empty() {
                skipped[0] += 1;
                continue;
            }
            let mi = d.material.or(u.assets.default_material);
            let Some(m) = mi.and_then(|m| u.mats.get(m as usize)).and_then(Option::as_ref) else {
                skipped[1] += 1;
                skipped_mats.insert(mi);
                continue;
            };
            let bits = d.clip.is_some() as usize | (d.stencil.alpha_clip as usize) << 1;
            let Some(v) = m.variants[bits].or_else(|| m.variants.iter().flatten().next().copied()) else {
                skipped[2] += 1;
                skipped_mats.insert(mi);
                continue;
            };
            let var = &scene.variants[v as usize];
            let mut state = m.state[(pass != UiPass::Overlay) as usize];
            let mut stencil_ref = m.stencil_ref as u32;
            if d.stencil != uk_game::ugui::Stencil::default() {
                // StencilMaterial.Add: the Mask's stencil state and color mask replace the material's
                let s = d.stencil;
                state.stencil = [s.read, s.write, s.op, 0, 0, s.comp];
                state.mask = s.color_mask;
                stencil_ref = s.id as u32;
            }
            // vertex streams in the variant's input order
            let mut vb: Vec<u8> = Vec::with_capacity(d.verts.len() * var.stride as usize);
            for vx in &d.verts {
                for &(_, ch, n) in &var.inputs {
                    let vals: [f32; 4] = match ch {
                        0 => [vx.pos.x, vx.pos.y, vx.pos.z, 1.0],
                        1 => [0.0, 0.0, -1.0, 0.0],
                        // TMP_Text's meshes carry tangent (-1, 0, 0, 1); VertexHelper's (1, 0, 0, -1)
                        2 => if d.tmp { [-1.0, 0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0, -1.0] },
                        3 => vx.color.map(|c| c as f32 / 255.0),
                        4 => vx.uv0.to_array(),
                        5 => vx.uv1.to_array(),
                        _ => [0.0; 4],
                    };
                    for f in &vals[..n as usize] {
                        vb.extend_from_slice(&f.to_le_bytes());
                    }
                }
            }
            // _MainTex: the Graphic's texture (CanvasRenderer.SetTexture), else Texture2D.whiteTexture
            let main = d.texture.and_then(|t| u.tex.get(t as usize).copied().flatten());
            let tex_of = |name: &str| if name == "_MainTex" { main } else { m.textures.get(name).copied() };
            let layouts = &gpu.layouts[v as usize];
            let textures: Vec<(u32, &str)> = var.groups[0]
                .iter()
                .filter_map(|t| match t {
                    Slot::Texture { binding, name, .. } => Some((*binding, name.as_str())),
                    _ => None,
                })
                .collect();
            let mut samplers = Vec::new();
            for s in &var.groups[0] {
                if let Slot::Sampler { binding, .. } = s {
                    // split samplers sit SAMPLER_BINDING_OFFSET above their texture; else the first texture's
                    let tex = textures.iter().find(|(tb, _)| tb + uk_assets::spirv::SAMPLER_BINDING_OFFSET == *binding).or(textures.first()).and_then(|(_, n)| tex_of(n));
                    samplers.push((*binding, sampler_for(dev, tex.and_then(|t| scene.textures.get(t as usize)))));
                }
            }
            let mut e0 = Vec::new();
            for s in &var.groups[0] {
                match s {
                    Slot::Texture { binding, dim, name } => {
                        let view = match dim {
                            TexDim::Cube => black_cube,
                            TexDim::D3 => black_3d,
                            TexDim::D2 => tex_of(name).map_or(white, |t| &gpu.tex_views[t as usize]),
                        };
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::TextureView(view) });
                    }
                    Slot::Sampler { binding, .. } => {
                        let smp = &samplers.iter().find(|(b, _)| b == binding).unwrap().1;
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::Sampler(smp) });
                    }
                    Slot::Storage { binding } => e0.push(BindGroupEntry { binding: *binding, resource: zero_storage.as_entire_binding() }),
                    Slot::Uniform { .. } => {}
                }
            }
            let mut ubufs = Vec::new();
            for s in &var.groups[1] {
                if let Slot::Uniform { binding, cb } = s {
                    let size = var.params.constant_buffers.get(*cb).map_or(16, |c| c.size.next_multiple_of(16).max(16)) as usize;
                    let mut bytes = vec![0u8; size];
                    if let Some(cbd) = var.params.constant_buffers.get(*cb) {
                        fill_ui_cb(&mut bytes, cbd, scene, d.props.as_deref().unwrap_or(&m.props), o2w, d.clip.as_ref(), main, c);
                    }
                    ubufs.push((*binding, dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity ui cb"), contents: &bytes, usage: BufferUsages::UNIFORM })));
                }
            }
            let mut e1: Vec<BindGroupEntry> = ubufs.iter().map(|(b, buf)| BindGroupEntry { binding: *b, resource: buf.as_entire_binding() }).collect();
            for s in &var.groups[1] {
                if let Slot::Storage { binding } = s {
                    e1.push(BindGroupEntry { binding: *binding, resource: zero_storage.as_entire_binding() });
                }
            }
            let bg0 = dev.create_bind_group("unity ui g0", &cache.get_bind_group_layout(&layouts[0]), &e0);
            let bg1 = dev.create_bind_group("unity ui g1", &cache.get_bind_group_layout(&layouts[1]), &e1);
            let mrt = pass == UiPass::Main;
            let key = (v, state, mrt);
            let pipeline = match gpu.pipelines.get(&key).or_else(|| new_pipelines.iter().find(|(k, _)| *k == key).map(|(_, p)| p)) {
                Some(p) => *p,
                None => {
                    let p = cache.queue_render_pipeline(pipeline_descriptor(var, layouts, state, mrt));
                    new_pipelines.push((key, p));
                    p
                }
            };
            draws.push(UiGpuDraw {
                vbuf: dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity ui vb"), contents: &vb, usage: BufferUsages::VERTEX }),
                ibuf: dev.create_buffer_with_data(&BufferInitDescriptor { label: Some("unity ui ib"), contents: bytemuck::cast_slice(&d.idx), usage: BufferUsages::INDEX }),
                count: d.idx.len() as u32,
                bg0,
                bg1,
                pipeline,
                stencil_ref,
                pass,
            });
        }
    }
    gpu.pipelines.extend(new_pipelines);
    let fno = gpu.frames.load(std::sync::atomic::Ordering::Relaxed);
    if std::env::var_os("UNITY_FRAME_STATS").is_some() && fno == 240 {
        // per batch: draws, vertices, and how many vertices land inside the view
        for (bi, b) in uf.batches.iter().enumerate() {
            let world = b.mode == uk_game::ugui::RenderMode::World;
            let c = if !world { &overlay } else if b.layer == 13 { hud_ctx.unwrap_or(ctx) } else { ctx };
            let o2w = mats.get(bi).copied().unwrap_or(Mat4::IDENTITY);
            let (mut inside, mut total) = (0usize, 0usize);
            let (mut lo, mut hi) = (Vec2::splat(9.0), Vec2::splat(-9.0));
            for v in b.draws.iter().flat_map(|d| &d.verts) {
                let p = c.vp * (o2w * v.pos.extend(1.0));
                total += 1;
                if p.w > 0.0 && p.x.abs() <= p.w * 1.0001 && p.y.abs() <= p.w * 1.0001 && p.z >= 0.0 && p.z <= p.w {
                    inside += 1;
                    lo = lo.min(Vec2::new(p.x, p.y) / p.w);
                    hi = hi.max(Vec2::new(p.x, p.y) / p.w);
                }
            }
            info!(
                "unity ui batch {bi} canvas node {} {:?} order {} layer {} scale {:.3}: draws {} vertices in view {inside}/{total} ndc {lo:.3?}..{hi:.3?}",
                b.root_node,
                b.mode,
                b.sorting_order,
                b.layer,
                b.scale_factor,
                b.draws.len()
            );
        }
        let by = |p: UiPass| draws.iter().filter(|d| d.pass == p).count();
        info!("unity ui gpu draws: overlay {} main {} hud {}; screen {}x{}", by(UiPass::Overlay), by(UiPass::Main), by(UiPass::Hud), screen.x, screen.y);
        let names: Vec<String> = skipped_mats.iter().map(|m| m.and_then(|m| u.assets.materials.get(m as usize)).map_or("-".into(), |m| format!("{} ({:?})", m.name, m.shader))).collect();
        info!("unity ui skipped draws: no indices {} no material {} no variant {}; materials {names:?}", skipped[0], skipped[1], skipped[2]);
    }
    gpu.ui_draws = draws;
}

/// uGUI's constant buffers: the canvas's matrix, the camera (or the overlay's pixel projection),
/// RectMask2D's clip rect and softness, and the material's properties.
#[allow(clippy::too_many_arguments)]
fn fill_ui_cb(out: &mut [u8], cb: &uk_assets::shader::ConstantBuffer, scene: &SceneData, mat: &MaterialProps, o2w: Mat4, clip: Option<&uk_game::ugui::Clip>, main: Option<u32>, ctx: &FrameCtx) {
    out.fill(0);
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
            put(&m.to_cols_array());
            continue;
        }
        if p.array_size > 0 {
            continue;
        }
        let t = ctx.time;
        let soft = clip.map_or([0.0; 2], |c| c.softness);
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
            // CanvasRenderer: RectMask2D's rect, else unclipped
            "_ClipRect" => clip.map_or([-32767.0, -32767.0, 32767.0, 32767.0], |c| c.rect),
            "_UIMaskSoftnessX" | "_MaskSoftnessX" => [soft[0], 0.0, 0.0, 0.0],
            "_UIMaskSoftnessY" | "_MaskSoftnessY" => [soft[1], 0.0, 0.0, 0.0],
            // (1,1,1,0) only for Alpha8 textures (legacy font atlases)
            "_TextureSampleAdd" => [0.0; 4],
            "_VertexWarping" => [scene.prefs.vertex_warping, 0.0, 0.0, 0.0],
            "_TextureWarping" => [scene.prefs.texture_warping, 0.0, 0.0, 0.0],
            "_ResY" => [scene.prefs.res_y, 0.0, 0.0, 0.0],
            _ => {
                if let Some(base) = n.strip_suffix("_TexelSize") {
                    let t = if base == "_MainTex" { main } else { None };
                    match t.and_then(|t| scene.textures.get(t as usize)) {
                        Some(tx) => [1.0 / tx.width as f32, 1.0 / tx.height as f32, tx.width as f32, tx.height as f32],
                        None => [1.0, 1.0, 1.0, 1.0],
                    }
                } else {
                    mat.vector(n).unwrap_or(if n.ends_with("_ST") { [1.0, 1.0, 0.0, 0.0] } else { [0.0; 4] })
                }
            }
        };
        if p.ty == 1 {
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

fn draw_ui<'a>(pass: &mut bevy::render::render_phase::TrackedRenderPass<'a>, gpu: &'a UnityGpu, cache: &'a PipelineCache, which: UiPass) {
    for d in gpu.ui_draws.iter().filter(|d| d.pass == which) {
        let Some(p) = cache.get_render_pipeline(d.pipeline) else { continue };
        pass.set_render_pipeline(p);
        pass.set_stencil_reference(d.stencil_ref);
        pass.set_bind_group(0, &d.bg0, &[]);
        pass.set_bind_group(1, &d.bg1, &[]);
        pass.set_vertex_buffer(0, d.vbuf.slice(..));
        pass.set_index_buffer(d.ibuf.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..d.count, 0, 0..1);
    }
}

/// `mrt`: the main camera's targets (color, outline RG); otherwise color only.
fn pipeline_descriptor(var: &Variant, layouts: &[BindGroupLayoutDescriptor; 2], s: DrawState, mrt: bool) -> RenderPipelineDescriptor {
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
    let blend = (!opaque).then(|| BlendState { color: component(s.src, s.dst, s.op), alpha: component(s.src_a.max(s.src), s.dst_a.max(s.dst), s.op) });
    // Unity ColorWriteMask: A = 1, B = 2, G = 4, R = 8
    let writes = |m: u8| {
        let mut mask = ColorWrites::empty();
        for (bit, w) in [(8, ColorWrites::RED), (4, ColorWrites::GREEN), (2, ColorWrites::BLUE), (1, ColorWrites::ALPHA)] {
            if m & bit != 0 {
                mask |= w;
            }
        }
        mask
    };
    let mask = writes(s.mask);
    let mut targets = vec![Some(ColorTargetState { format: COLOR_FORMAT, blend, write_mask: mask })];
    if mrt {
        // SV_Target1 -> the outline buffer; a shader without that output leaves it untouched.
        // SV_Target2 (view normal, read only by stains) has no target.
        let [src, dst, op, m] = s.rt1;
        let c = component(src, dst, op);
        let has = var.outputs.contains(&1);
        targets.push(Some(ColorTargetState {
            format: OUTLINE_FORMAT,
            blend: (src != 1 || dst != 0 || op != 0).then_some(BlendState { color: c, alpha: c }),
            write_mask: if has { writes(m) } else { ColorWrites::empty() },
        }));
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
            stencil: if s.stencil == STENCIL_OFF {
                default()
            } else {
                let [read, write, pass, fail, zfail, comp] = s.stencil;
                let face = StencilFaceState { compare: stencil_compare(comp), fail_op: stencil_op(fail), depth_fail_op: stencil_op(zfail), pass_op: stencil_op(pass) };
                StencilState { front: face, back: face, read_mask: read as u32, write_mask: write as u32 }
            },
            bias: default(),
        }),
        fragment: Some(FragmentState {
            shader: var.fs.clone(),
            entry_point: Some("main".into()),
            targets,
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
    let hud_u = scene.hud_cam.map_or(cam_u, |(w, _)| {
        let c = world.resource::<UnityFrame>().hud_cam.unwrap_or(w).w_axis.truncate();
        Vec3::new(c.x, c.y, -c.z)
    });
    let sorted = |hud: bool| {
        let eye = if hud { hud_u } else { cam_u };
        let mut order: Vec<(i64, f32, usize)> = (0..gpu.draws.len())
            .filter(|&i| gpu.draws[i].is_some() && scene.draws[i].hud == hud && gpu.in_view.get(i).copied().unwrap_or(false))
            .map(|i| {
                let d = &scene.draws[i];
                let dist = gpu.centers.get(&i).map_or(d.center, |c| c.0).distance(eye);
                (d.queue, if d.queue >= 2500 { -dist } else { dist }, i)
            })
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        order
    };
    let t_draw = std::time::Instant::now();
    let c = scene.clear;
    // Main Camera, then the HUD Camera (depth-only clear) over it
    for hud in [false, true] {
        let order = sorted(hud);
        let ui_pass = if hud { UiPass::Hud } else { UiPass::Main };
        let has_ui = gpu.ui_draws.iter().any(|d| d.pass == ui_pass);
        if hud && order.is_empty() && !has_ui {
            continue;
        }
        if hud {
            outline_pass(world, gpu, scene, cache, color, &mut ctx);
        }
        let color_load = if hud { LoadOp::Load } else { LoadOp::Clear(wgpu_types::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }) };
        let main = Some(RenderPassColorAttachment { view: color, depth_slice: None, resolve_target: None, ops: Operations { load: color_load, store: StoreOp::Store } });
        // the main camera's second target: the outline buffer, cleared black (OnPreRenderCallback)
        let outline = gpu.outline_target.as_ref().filter(|_| !hud).map(|(_, v)| RenderPassColorAttachment {
            view: v,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(wgpu_types::Color::TRANSPARENT), store: StoreOp::Store },
        });
        let attachments = if hud { vec![main] } else { vec![main, outline] };
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(if hud { "unity hud camera" } else { "unity scene" }),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(Operations { load: LoadOp::Clear(0.0), store: StoreOp::Store }),
                // Unity clears depth and stencil together
                stencil_ops: Some(Operations { load: LoadOp::Clear(0), store: StoreOp::Store }),
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
        // world-space canvases: the canvas renders after its camera's geometry (UI queue, 3000)
        draw_ui(&mut pass, gpu, cache, ui_pass);
    }
    if sorted(true).is_empty() && !gpu.ui_draws.iter().any(|d| d.pass == UiPass::Hud) {
        outline_pass(world, gpu, scene, cache, color, &mut ctx);
    }
    // ULTRAKILL's final composite (PostProcessV2) into the post target
    let mut shown = color;
    let post = gpu.post_for(world.resource::<UnityFrame>().post_mask());
    if let (Some(post), Some((_, pv))) = (post, gpu.post_target.as_ref()) {
        if let Some(p) = cache.get_render_pipeline(post.pipeline) {
            let dev = world.resource::<RenderDevice>();
            let var = &scene.variants[post.variant as usize];
            let mut e0 = Vec::new();
            for s in &var.groups[0] {
                match s {
                    Slot::Texture { binding, name, .. } => {
                        let view = if name == "_MainTex" { color } else { &post.texture(name).0 };
                        e0.push(BindGroupEntry { binding: *binding, resource: BindingResource::TextureView(view) });
                    }
                    Slot::Sampler { binding, .. } => {
                        // split samplers sit SAMPLER_BINDING_OFFSET above their texture
                        let tex = var.groups[0].iter().find_map(|t| match t {
                            Slot::Texture { binding: tb, name, .. } if *tb + uk_assets::spirv::SAMPLER_BINDING_OFFSET == *binding => Some(name.as_str()),
                            _ => None,
                        });
                        let smp = match tex {
                            Some("_MainTex") | None => &post.main_sampler,
                            Some(n) => &post.texture(n).1,
                        };
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
        let f = world.resource::<UnityFrame>();
        let mask = f.post_mask();
        let on: Vec<&str> = POST_KEYWORDS.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 1).map(|(_, k)| *k).collect();
        let built = gpu.post.get(mask).is_some_and(|p| p.is_some());
        info!(
            "unity post globals: hurt alpha {:.3} deathness {:?} underwater {:?} vignette {:?} noise {:?} keywords {on:?} pass {}",
            f.hurt,
            f.dead,
            f.underwater,
            f.vignette,
            f.noise,
            if built { "exact" } else { "fallback" }
        );
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
        if let Some((otex, _)) = gpu.outline_target.as_ref() {
            let orow = (size.x * 2).next_multiple_of(256);
            let obuf = dev.create_buffer(&BufferDescriptor { label: Some("unity outline readback"), size: (orow * size.y) as u64, usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
            ctx.command_encoder().copy_texture_to_buffer(
                otex.as_image_copy(),
                TexelCopyBufferInfo { buffer: &obuf, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(orow), rows_per_image: Some(size.y) } },
                Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            );
            *gpu.outline_readback.lock().unwrap() = Some(obuf);
        }
        if let (Some((ptex, _)), false) = (gpu.post_target.as_ref(), std::ptr::eq(shown, color)) {
            let ps = UVec2::new(ptex.width(), ptex.height());
            let prow = (ps.x * 4).next_multiple_of(256);
            let pbuf = dev.create_buffer(&BufferDescriptor { label: Some("unity post readback"), size: (prow * ps.y) as u64, usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
            ctx.command_encoder().copy_texture_to_buffer(
                ptex.as_image_copy(),
                TexelCopyBufferInfo { buffer: &pbuf, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(prow), rows_per_image: Some(ps.y) } },
                Extent3d { width: ps.x, height: ps.y, depth_or_array_layers: 1 },
            );
            *gpu.post_readback.lock().unwrap() = Some((pbuf, prow, ps));
        }
    }
    // Screen Space - Overlay canvases: drawn over the final image at screen resolution
    if gpu.ui_draws.iter().any(|d| d.pass == UiPass::Overlay) {
        let on_post = !std::ptr::eq(shown, color);
        let ds = if on_post { gpu.ui_depth.as_ref().map(|d| &d.1) } else { Some(depth) };
        if let Some(ds) = ds {
            let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("unity ui overlay"),
                color_attachments: &[Some(RenderPassColorAttachment { view: shown, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: ds,
                    depth_ops: Some(Operations { load: LoadOp::Clear(0.0), store: StoreOp::Store }),
                    stencil_ops: Some(Operations { load: LoadOp::Clear(0), store: StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            draw_ui(&mut pass, gpu, cache, UiPass::Overlay);
        }
        // numeric check: the post output after the overlay, diffed against the pre-UI copy
        if n == 240 && on_post && std::env::var_os("UNITY_FRAME_STATS").is_some() {
            if let Some((ptex, _)) = gpu.post_target.as_ref() {
                let dev = world.resource::<RenderDevice>();
                let ps = UVec2::new(ptex.width(), ptex.height());
                let prow = (ps.x * 4).next_multiple_of(256);
                let ubuf = dev.create_buffer(&BufferDescriptor { label: Some("unity ui readback"), size: (prow * ps.y) as u64, usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
                ctx.command_encoder().copy_texture_to_buffer(
                    ptex.as_image_copy(),
                    TexelCopyBufferInfo { buffer: &ubuf, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(prow), rows_per_image: Some(ps.y) } },
                    Extent3d { width: ps.x, height: ps.y, depth_or_array_layers: 1 },
                );
                *gpu.ui_readback.lock().unwrap() = Some(ubuf);
            }
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

/// PostProcessV2's outline composite (OutlinePx pass 3, CameraEvent.AfterEverything on the main
/// camera, so before the HUD Camera): multiplies color by 1 - (marked neighbour and unmarked self).
fn outline_pass(world: &World, gpu: &UnityGpu, scene: &SceneData, cache: &PipelineCache, color: &TextureView, ctx: &mut RenderContext) {
    let (Some(og), Some(od), Some((_, ov))) = (&gpu.outline, scene.outline.as_ref(), gpu.outline_target.as_ref()) else { return };
    if std::env::var_os("UNITY_NO_OUTLINE").is_some() {
        return;
    }
    let Some(p) = cache.get_render_pipeline(og.pipeline) else { return };
    let dev = world.resource::<RenderDevice>();
    let var = &scene.variants[od.variant as usize];
    let e0: Vec<BindGroupEntry> = var.groups[0]
        .iter()
        .filter_map(|s| match s {
            Slot::Texture { binding, .. } => Some(BindGroupEntry { binding: *binding, resource: BindingResource::TextureView(ov) }),
            Slot::Sampler { binding, .. } => Some(BindGroupEntry { binding: *binding, resource: BindingResource::Sampler(&og.sampler) }),
            _ => None,
        })
        .collect();
    let e1: Vec<BindGroupEntry> = og.ubufs.iter().map(|(b, buf, _)| BindGroupEntry { binding: *b, resource: buf.as_entire_binding() }).collect();
    let bg0 = dev.create_bind_group("unity outline g0", &cache.get_bind_group_layout(&og.layouts[0]), &e0);
    let bg1 = dev.create_bind_group("unity outline g1", &cache.get_bind_group_layout(&og.layouts[1]), &e1);
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("unity outline"),
        color_attachments: &[Some(RenderPassColorAttachment { view: color, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(p);
    pass.set_bind_group(0, &bg0, &[]);
    pass.set_bind_group(1, &bg1, &[]);
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
    // blood: red-dominant pixels
    let mut red = 0u64;
    for y in 0..size.y {
        for x in 0..size.x {
            let i = (y * *row + x * 4) as usize;
            let (r, g, b) = (data[i] as u32, data[i + 1] as u32, data[i + 2] as u32);
            if r > 48 && r > 2 * g && r > 2 * b {
                red += 1;
            }
        }
    }
    info!("unity blood pixels: {red} ({:.2}%)", 100.0 * red as f64 / n);
    // PostProcessV2 output against the scene it read: mean |post - scene| as stored and with rows
    // flipped (orientation check), and its distinct colors (dither + quantization). The post output
    // is screen-sized; each pixel is compared with the scene pixel it covers (pixelization)
    if let Some((pbuf, prow, psz)) = gpu.post_readback.lock().unwrap().take() {
        let ps = pbuf.slice(..);
        ps.map_async(MapMode::Read, |_| {});
        let _ = dev.poll(wgpu_types::PollType::wait_indefinitely());
        let post = ps.get_mapped_range();
        let (mut same, mut flip, mut runs) = (0u64, 0u64, 0u64);
        let mut shift = [0i64; 3];
        let mut pd = std::collections::HashSet::new();
        let mut levels = std::collections::HashSet::new();
        for py in 0..psz.y {
            for px in 0..psz.x {
                let pi = (py * prow + px * 4) as usize;
                // equal to its left neighbour: ~1 - vsize/screen when pixelized
                runs += (px > 0 && post[pi..pi + 3] == post[pi - 4..pi - 1]) as u64;
                let (x, y) = (px * size.x / psz.x, py * size.y / psz.y);
                let i = (y * *row + x * 4) as usize;
                let j = ((size.y - 1 - y) * *row + x * 4) as usize;
                for k in 0..3 {
                    same += (post[pi + k] as i32 - data[i + k] as i32).unsigned_abs() as u64;
                    flip += (post[pi + k] as i32 - data[j + k] as i32).unsigned_abs() as u64;
                    shift[k] += post[pi + k] as i64 - data[i + k] as i64;
                }
                pd.insert((post[pi] >> 2, post[pi + 1] >> 2, post[pi + 2] >> 2));
                levels.insert(post[pi]);
            }
        }
        let n = (psz.x * psz.y) as f64;
        info!("unity post stats: mean abs diff vs scene {:.2} (rows flipped {:.2}) distinct colors {}", same as f64 / (3.0 * n), flip as f64 / (3.0 * n), pd.len());
        info!("unity post size: {}x{} from scene {}x{}, pixels equal to left neighbour {:.3}, red levels {}", psz.x, psz.y, size.x, size.y, runs as f64 / n, levels.len());
        // the hurt flash lerps toward _HurtScreenColor by its alpha: a signed per-channel shift
        info!("unity post shift: mean rgb post - scene ({:.1}, {:.1}, {:.1})", shift[0] as f64 / n, shift[1] as f64 / n, shift[2] as f64 / n);
        // the overlay canvases: pixels they changed, mean change, and the changed pixels' bounds
        if let Some(ubuf) = gpu.ui_readback.lock().unwrap().take() {
            let us = ubuf.slice(..);
            us.map_async(MapMode::Read, |_| {});
            let _ = dev.poll(wgpu_types::PollType::wait_indefinitely());
            let ui = us.get_mapped_range();
            let (mut changed, mut diff) = (0u64, 0u64);
            let (mut lo, mut hi) = (UVec2::MAX, UVec2::ZERO);
            for py in 0..psz.y {
                for px in 0..psz.x {
                    let pi = (py * prow + px * 4) as usize;
                    let d: u32 = (0..3).map(|k| (ui[pi + k] as i32 - post[pi + k] as i32).unsigned_abs()).sum();
                    if d > 0 {
                        changed += 1;
                        diff += d as u64;
                        lo = lo.min(UVec2::new(px, py));
                        hi = hi.max(UVec2::new(px, py));
                    }
                }
            }
            info!(
                "unity ui pixels changed: {changed} of {} ({:.2}%), mean abs diff where changed {:.1}, bounds {lo}..{hi} (rows from the top)",
                psz.x * psz.y,
                100.0 * changed as f64 / (psz.x * psz.y) as f64,
                diff as f64 / (3.0 * changed.max(1) as f64)
            );
        }
    }
    // the outline buffer: coverage, marked pixels (pass 3's test: R > 0.999 or R + G > 1), the
    // outline pixels that marking predicts, and how many of those the scene target shows black
    if let Some(obuf) = gpu.outline_readback.lock().unwrap().take() {
        let os = obuf.slice(..);
        os.map_async(MapMode::Read, |_| {});
        let _ = dev.poll(wgpu_types::PollType::wait_indefinitely());
        let o = os.get_mapped_range();
        let orow = (size.x * 2).next_multiple_of(256);
        let rg = |x: u32, y: u32| {
            let i = (y * orow + x * 2) as usize;
            (o[i] as u32, o[i + 1] as u32)
        };
        let marked = |x: u32, y: u32| {
            let (r, g) = rg(x, y);
            r as f32 / 255.0 > 0.999 || r + g > 255
        };
        let (mut cover, mut half, mut mk, mut predicted, mut black) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for y in 0..size.y {
            for x in 0..size.x {
                let (r, g) = rg(x, y);
                cover += (r > 0 || g > 0) as u64;
                half += (r == 128 || r == 127) as u64;
                let m = marked(x, y);
                mk += m as u64;
                if !m && [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    nx >= 0 && ny >= 0 && (nx as u32) < size.x && (ny as u32) < size.y && marked(nx as u32, ny as u32)
                }) {
                    predicted += 1;
                    let i = (y * *row + x * 4) as usize;
                    black += (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0) as u64;
                }
            }
        }
        info!(
            "unity outline stats: buffer written {:.2}% (R = 0.5: {:.2}%) marked {:.3}% outline pixels {} black in scene {}",
            100.0 * cover as f64 / n,
            100.0 * half as f64 / n,
            100.0 * mk as f64 / n,
            predicted,
            black
        );
    }
}
