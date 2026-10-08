//! uGUI: the scenes' Canvas hierarchies laid out and meshed like UnityEngine.UI does it.
//!
//! Literal ports of RectTransform layout, CanvasScaler.Handle, Graphic/Image/RawImage
//! OnPopulateMesh (Simple, Sliced, Tiled, Filled + RadialCut), Shadow/Outline.ModifyMesh,
//! Mask/MaskableGraphic stencil materials (StencilMaterial.Add), RectMask2D clipping and
//! culling (Clipping.FindCullAndClipWorldRect, MaskableGraphic.Cull) and CanvasGroup alpha.
//!
//! Everything is computed in root-canvas local space (Unity axes, y up): an overlay root maps it
//! to screen pixels (bottom-left origin) with `UiBatch::to_screen`; a world-space root is drawn
//! with its node's world matrix. No Bevy here: the frontend turns `UiFrame` into draws.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy_math::{Mat4, Quat, Vec2, Vec3, Vec4};
use uk_assets::scene::to_bevy_point;
use uk_assets::scenedef::{RectDef, SceneDef};
use uk_assets::serialized::Value;
use uk_assets::shader::MaterialProps;
use uk_assets::ui::UiAssets;

use crate::tmp::{RtMats, TmpDef, TmpMesh};

pub const CLASS_CANVAS: i32 = 223;
pub const CLASS_CANVAS_GROUP: i32 = 225;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Overlay,
    Camera,
    World,
}

#[derive(Clone, Debug)]
pub struct CanvasDef {
    pub node: u32,
    pub enabled: bool,
    pub mode: RenderMode,
    pub sorting_order: i32,
    pub override_sorting: bool,
    pub pixel_perfect: bool,
    /// m_Camera (the render camera of ScreenSpaceCamera / the event camera of WorldSpace)
    pub camera: Option<u32>,
    pub plane_distance: f32,
    /// nearest ancestor canvas (None: a root canvas)
    pub parent: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct GroupDef {
    pub node: u32,
    pub enabled: bool,
    pub alpha: f32,
    pub ignore_parent: bool,
}

#[derive(Clone, Debug)]
pub struct ScalerDef {
    pub script: u32,
    pub node: u32,
    pub mode: i64,
    pub reference_ppu: f32,
    pub scale_factor: f32,
    pub reference_resolution: [f32; 2],
    pub match_mode: i64,
    pub match_width_or_height: f32,
    pub physical_unit: i64,
    pub fallback_dpi: f32,
    pub default_sprite_dpi: f32,
    pub dynamic_ppu: f32,
}

#[derive(Clone, Debug)]
pub struct ImageDef {
    pub sprite: Option<u32>,
    /// 0 Simple, 1 Sliced, 2 Tiled, 3 Filled
    pub ty: i64,
    pub preserve_aspect: bool,
    pub fill_center: bool,
    /// 0 Horizontal, 1 Vertical, 2 Radial90, 3 Radial180, 4 Radial360
    pub fill_method: i64,
    pub fill_amount: f32,
    pub fill_clockwise: bool,
    pub fill_origin: i64,
    pub use_sprite_mesh: bool,
    pub ppu_multiplier: f32,
}

#[derive(Clone, Debug)]
pub enum GraphicKind {
    Image(ImageDef),
    RawImage { texture: Option<u32>, uv_rect: [f32; 4] },
    Text,
    Tmp(Box<TmpDef>),
}

#[derive(Clone, Debug)]
pub struct GraphicDef {
    pub script: u32,
    pub node: u32,
    pub kind: GraphicKind,
    pub color: [f32; 4],
    /// m_Material (None: Canvas.GetDefaultCanvasMaterial)
    pub material: Option<u32>,
    pub maskable: bool,
}

#[derive(Clone, Debug)]
pub struct MaskDef {
    pub script: u32,
    pub node: u32,
    pub show_graphic: bool,
}

#[derive(Clone, Debug)]
pub struct RectMaskDef {
    pub script: u32,
    pub node: u32,
    /// (left, bottom, right, top)
    pub padding: [f32; 4],
    pub softness: [i32; 2],
}

#[derive(Clone, Debug)]
pub struct EffectDef {
    pub script: u32,
    pub node: u32,
    pub outline: bool,
    pub color: [f32; 4],
    pub distance: [f32; 2],
    pub use_graphic_alpha: bool,
}

/// Static UI components of a scene.
#[derive(Default)]
pub struct UiDef {
    pub canvases: Vec<CanvasDef>,
    pub groups: Vec<GroupDef>,
    pub scalers: Vec<ScalerDef>,
    pub graphics: Vec<GraphicDef>,
    pub masks: Vec<MaskDef>,
    pub rect_masks: Vec<RectMaskDef>,
    pub effects: Vec<EffectDef>,
    pub node_canvas: HashMap<u32, u32>,
    pub node_group: HashMap<u32, u32>,
    pub node_scaler: HashMap<u32, u32>,
    pub node_graphic: HashMap<u32, u32>,
    pub node_mask: HashMap<u32, u32>,
    pub node_rect_mask: HashMap<u32, u32>,
    /// mesh modifiers in component order
    pub node_effects: HashMap<u32, Vec<u32>>,
    pub script_graphic: HashMap<u32, u32>,
    /// root canvases, in scene order
    pub roots: Vec<u32>,
    pub warnings: Vec<String>,
    /// TMP's runtime material instances (shared + fallback materials)
    pub tmp_mats: Mutex<RtMats>,
    /// graphic -> (inputs, mesh) of its last TMP mesh
    pub tmp_cache: Mutex<HashMap<u32, (TmpKey, Arc<TmpMesh>)>>,
}

/// What a TMP mesh depends on beyond its serialized fields.
#[derive(Clone, PartialEq, Debug)]
pub struct TmpKey {
    text: Option<Arc<str>>,
    rect: [u32; 4],
    uv2: u32,
    color: [u32; 4],
}

fn color4(v: &Value) -> [f32; 4] {
    [v.get("r").f32(), v.get("g").f32(), v.get("b").f32(), v.get("a").f32()]
}

fn vec2(v: &Value) -> [f32; 2] {
    [v.get("x").f32(), v.get("y").f32()]
}

impl UiDef {
    pub fn build(def: &SceneDef, assets: &UiAssets) -> UiDef {
        let mut ui = UiDef::default();
        for nat in &def.ui_natives {
            let d = &nat.data;
            match nat.class_id {
                CLASS_CANVAS => {
                    let mode = match d.get("m_RenderMode").i64() {
                        0 => RenderMode::Overlay,
                        1 => RenderMode::Camera,
                        _ => RenderMode::World,
                    };
                    ui.node_canvas.insert(nat.node, ui.canvases.len() as u32);
                    ui.canvases.push(CanvasDef {
                        node: nat.node,
                        enabled: d.get("m_Enabled").bool(),
                        mode,
                        sorting_order: d.get("m_SortingOrder").i64() as i32,
                        override_sorting: d.get("m_OverrideSorting").bool(),
                        pixel_perfect: d.get("m_PixelPerfect").bool(),
                        camera: def.node_ref(d.get("m_Camera")),
                        plane_distance: d.get("m_PlaneDistance").f32(),
                        parent: None,
                    });
                }
                CLASS_CANVAS_GROUP => {
                    ui.node_group.insert(nat.node, ui.groups.len() as u32);
                    ui.groups.push(GroupDef {
                        node: nat.node,
                        enabled: d.get("m_Enabled").bool(),
                        alpha: d.get("m_Alpha").f32(),
                        ignore_parent: d.get("m_IgnoreParentGroups").bool(),
                    });
                }
                _ => {}
            }
        }
        for ci in 0..ui.canvases.len() {
            let mut p = def.nodes[ui.canvases[ci].node as usize].parent;
            while let Some(n) = p {
                if let Some(&c) = ui.node_canvas.get(&n) {
                    ui.canvases[ci].parent = Some(c);
                    break;
                }
                p = def.nodes[n as usize].parent;
            }
            if ui.canvases[ci].parent.is_none() {
                ui.roots.push(ci as u32);
            }
        }
        for (si, s) in def.scripts.iter().enumerate() {
            let si = si as u32;
            let d = &s.data;
            let graphic = |kind: GraphicKind| GraphicDef {
                script: si,
                node: s.node,
                kind,
                color: color4(d.get("m_Color")),
                material: assets.material(si, "m_Material"),
                maskable: d.get("m_Maskable").bool(),
            };
            let g = match s.class.as_str() {
                "Image" => Some(graphic(GraphicKind::Image(ImageDef {
                    sprite: assets.sprite(si, "m_Sprite"),
                    ty: d.get("m_Type").i64(),
                    preserve_aspect: d.get("m_PreserveAspect").bool(),
                    fill_center: d.get("m_FillCenter").bool(),
                    fill_method: d.get("m_FillMethod").i64(),
                    fill_amount: d.get("m_FillAmount").f32(),
                    fill_clockwise: d.get("m_FillClockwise").bool(),
                    fill_origin: d.get("m_FillOrigin").i64(),
                    use_sprite_mesh: d.get("m_UseSpriteMesh").bool(),
                    ppu_multiplier: if d.has("m_PixelsPerUnitMultiplier") { d.get("m_PixelsPerUnitMultiplier").f32().max(0.01) } else { 1.0 },
                }))),
                "RawImage" => {
                    let r = d.get("m_UVRect");
                    Some(graphic(GraphicKind::RawImage {
                        texture: assets.texture(si, "m_Texture"),
                        uv_rect: [r.get("x").f32(), r.get("y").f32(), r.get("width").f32(), r.get("height").f32()],
                    }))
                }
                "Text" => Some(graphic(GraphicKind::Text)),
                "TextMeshProUGUI" => {
                    let td = TmpDef::from_script(d, assets, si, &mut ui.warnings, &def.path(s.node));
                    let mut g = graphic(GraphicKind::Tmp(Box::new(td.clone())));
                    // TMP_Text.color is m_fontColor; materialForRendering is m_sharedMaterial
                    g.color = color4(d.get("m_fontColor"));
                    g.material = td.material;
                    Some(g)
                }
                "Mask" => {
                    ui.node_mask.insert(s.node, ui.masks.len() as u32);
                    ui.masks.push(MaskDef { script: si, node: s.node, show_graphic: d.get("m_ShowMaskGraphic").bool() });
                    None
                }
                "RectMask2D" => {
                    let p = d.get("m_Padding");
                    let sf = d.get("m_Softness");
                    ui.node_rect_mask.insert(s.node, ui.rect_masks.len() as u32);
                    ui.rect_masks.push(RectMaskDef {
                        script: si,
                        node: s.node,
                        padding: [p.get("x").f32(), p.get("y").f32(), p.get("z").f32(), p.get("w").f32()],
                        softness: [sf.get("x").i64() as i32, sf.get("y").i64() as i32],
                    });
                    None
                }
                "Shadow" | "Outline" => {
                    ui.node_effects.entry(s.node).or_default().push(ui.effects.len() as u32);
                    ui.effects.push(EffectDef {
                        script: si,
                        node: s.node,
                        outline: s.class == "Outline",
                        color: color4(d.get("m_EffectColor")),
                        distance: vec2(d.get("m_EffectDistance")),
                        use_graphic_alpha: d.get("m_UseGraphicAlpha").bool(),
                    });
                    None
                }
                "CanvasScaler" => {
                    ui.node_scaler.insert(s.node, ui.scalers.len() as u32);
                    ui.scalers.push(ScalerDef {
                        script: si,
                        node: s.node,
                        mode: d.get("m_UiScaleMode").i64(),
                        reference_ppu: d.get("m_ReferencePixelsPerUnit").f32(),
                        scale_factor: d.get("m_ScaleFactor").f32(),
                        reference_resolution: vec2(d.get("m_ReferenceResolution")),
                        match_mode: d.get("m_ScreenMatchMode").i64(),
                        match_width_or_height: d.get("m_MatchWidthOrHeight").f32(),
                        physical_unit: d.get("m_PhysicalUnit").i64(),
                        fallback_dpi: d.get("m_FallbackScreenDPI").f32(),
                        default_sprite_dpi: d.get("m_DefaultSpriteDPI").f32(),
                        dynamic_ppu: d.get("m_DynamicPixelsPerUnit").f32(),
                    });
                    None
                }
                _ => None,
            };
            if let Some(g) = g {
                if let Some(&prev) = ui.node_graphic.get(&s.node) {
                    ui.warnings.push(format!("{}: second Graphic (script {} after {})", def.path(s.node), si, ui.graphics[prev as usize].script));
                    continue;
                }
                if let GraphicKind::Image(im) = &g.kind {
                    if im.use_sprite_mesh && im.ty == 0 && im.sprite.is_some() {
                        ui.warnings.push(format!("{}: useSpriteMesh (sprite mesh not loaded; drawn as a Simple quad)", def.path(s.node)));
                    }
                }
                ui.node_graphic.insert(s.node, ui.graphics.len() as u32);
                ui.script_graphic.insert(si, ui.graphics.len() as u32);
                ui.graphics.push(g);
            }
        }
        ui
    }
}

/// Runtime UI values scripts change (cloned with the game state for checkpoints).
#[derive(Clone, Default)]
pub struct UiState {
    /// RectTransform overrides (anchoredPosition, sizeDelta, ...)
    pub rects: HashMap<u32, RectDef>,
    /// localRotation / localScale overrides (Unity space)
    pub rot: HashMap<u32, Quat>,
    pub scale: HashMap<u32, Vec3>,
    /// Graphic.color per graphic
    pub color: Vec<[f32; 4]>,
    /// CanvasRenderer.GetColor per graphic (CrossFadeColor / SetColor / SetAlpha)
    pub cr_color: Vec<[f32; 4]>,
    /// Image.sprite (overrideSprite folded in) per graphic
    pub sprite: Vec<Option<u32>>,
    /// Image.fillAmount per graphic
    pub fill: Vec<f32>,
    pub canvas_enabled: Vec<bool>,
    pub group_alpha: Vec<f32>,
    pub group_enabled: Vec<bool>,
    /// TMP_Text.text set by scripts per graphic (None: the serialized text)
    pub text: Vec<Option<Arc<str>>>,
}

impl UiState {
    pub fn new(ui: &UiDef) -> UiState {
        UiState {
            rects: HashMap::new(),
            rot: HashMap::new(),
            scale: HashMap::new(),
            color: ui.graphics.iter().map(|g| g.color).collect(),
            cr_color: vec![[1.0; 4]; ui.graphics.len()],
            sprite: ui.graphics.iter().map(|g| if let GraphicKind::Image(i) = &g.kind { i.sprite } else { None }).collect(),
            fill: ui.graphics.iter().map(|g| if let GraphicKind::Image(i) = &g.kind { i.fill_amount.clamp(0.0, 1.0) } else { 1.0 }).collect(),
            canvas_enabled: ui.canvases.iter().map(|c| c.enabled).collect(),
            group_alpha: ui.groups.iter().map(|g| g.alpha).collect(),
            group_enabled: ui.groups.iter().map(|g| g.enabled).collect(),
            text: vec![None; ui.graphics.len()],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiVertex {
    pub pos: Vec3,
    pub color: [u8; 4],
    pub uv0: Vec4,
    pub uv1: Vec4,
}

/// Material stencil / color-mask state (UI/Default's _Stencil* / _ColorMask / UNITY_UI_ALPHACLIP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stencil {
    pub id: u8,
    /// StencilOp: 0 Keep, 1 Zero, 2 Replace, ...
    pub op: u8,
    /// CompareFunction: 3 Equal, 8 Always, ...
    pub comp: u8,
    pub read: u8,
    pub write: u8,
    /// ColorWriteMask (15 All)
    pub color_mask: u8,
    pub alpha_clip: bool,
}

impl Default for Stencil {
    fn default() -> Self {
        Stencil { id: 0, op: 0, comp: 8, read: 255, write: 255, color_mask: 15, alpha_clip: false }
    }
}

pub const OP_KEEP: u8 = 0;
pub const OP_ZERO: u8 = 1;
pub const OP_REPLACE: u8 = 2;
pub const CMP_EQUAL: u8 = 3;
pub const CMP_ALWAYS: u8 = 8;

/// StencilMaterial.Add: None when the base material is returned unchanged.
fn stencil_add(id: i32, op: u8, comp: u8, color_mask: u8, read: i32, write: i32) -> Option<Stencil> {
    if id <= 0 && color_mask == 15 {
        return None;
    }
    Some(Stencil { id: id as u8, op, comp, read: read as u8, write: write as u8, color_mask, alpha_clip: op != OP_KEEP && write > 0 })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn xmax(&self) -> f32 {
        self.x + self.w
    }
    pub fn ymax(&self) -> f32 {
        self.y + self.h
    }
    fn norm(&self) -> Rect {
        let (x0, x1) = if self.w < 0.0 { (self.xmax(), self.x) } else { (self.x, self.xmax()) };
        let (y0, y1) = if self.h < 0.0 { (self.ymax(), self.y) } else { (self.y, self.ymax()) };
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
    /// Rect.Overlaps(other, allowInverse: true)
    pub fn overlaps(&self, o: &Rect) -> bool {
        let a = self.norm();
        let b = o.norm();
        b.xmax() > a.x && b.x < a.xmax() && b.ymax() > a.y && b.y < a.ymax()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    /// root-canvas local (xmin, ymin, xmax, ymax): `_ClipRect`
    pub rect: [f32; 4],
    /// RectMask2D.softness: `_UIMaskSoftnessX/Y`
    pub softness: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct UiDraw {
    pub node: u32,
    pub graphic: u32,
    /// root-canvas local positions; colors already multiplied by the CanvasRenderer color and group alpha
    pub verts: Vec<UiVertex>,
    pub idx: Vec<u32>,
    /// index into `UiAssets::materials` (None: no material at all)
    pub material: Option<u32>,
    /// main texture (index into `UiAssets::textures`; None: Texture2D.whiteTexture)
    pub texture: Option<u32>,
    pub stencil: Stencil,
    pub clip: Option<Clip>,
    /// the Mask's pop instruction (drawn after the mask's children)
    pub pop: bool,
    /// TMP's runtime material properties (replace the asset material's)
    pub props: Option<Arc<MaterialProps>>,
    /// a TextMeshPro mesh (tangent (-1, 0, 0, 1))
    pub tmp: bool,
}

#[derive(Clone, Debug)]
pub struct UiBatch {
    /// the canvas that sorts this batch (the root or a nested override-sorting canvas)
    pub canvas: u32,
    pub root: u32,
    pub root_node: u32,
    pub mode: RenderMode,
    pub sorting_order: i32,
    pub layer: u8,
    /// overlay / camera roots: root-local -> screen pixels (bottom-left origin)
    pub to_screen: Mat4,
    pub scale_factor: f32,
    pub reference_ppu: f32,
    pub root_rect: Rect,
    pub draws: Vec<UiDraw>,
}

#[derive(Default, Debug)]
pub struct UiFrame {
    /// overlay batches sorted by sorting order, then world-space batches
    pub batches: Vec<UiBatch>,
    /// graphic -> its rect in root-local space (corners 0 and 2) when laid out this frame
    pub graphic_rects: HashMap<u32, Rect>,
    /// node -> RectTransform.rect (local) of every laid-out node
    pub node_rects: HashMap<u32, Rect>,
    /// node -> local-to-root matrix (Unity space)
    pub node_to_root: HashMap<u32, Mat4>,
}

pub struct UiInput<'a> {
    pub def: &'a SceneDef,
    pub ui: &'a UiDef,
    pub assets: &'a UiAssets,
    pub state: &'a UiState,
    /// activeInHierarchy per node
    pub active: &'a [bool],
    /// Behaviour.enabled per script
    pub script_enabled: &'a [bool],
    pub screen: [f32; 2],
    /// Screen.dpi (0: unknown)
    pub dpi: f32,
    /// a node's current world matrix (world-space canvas scale for TMP's uv2)
    pub world_of: Option<&'a dyn Fn(u32) -> Mat4>,
}

/// Vertex scratch like VertexHelper.
#[derive(Default, Clone)]
pub struct Vh {
    pub verts: Vec<UiVertex>,
    pub idx: Vec<u32>,
}

/// Color -> Color32 (Mathf.Round(Clamp01(c) * 255))
pub fn color32(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl Vh {
    fn add_vert(&mut self, p: Vec2, c: [u8; 4], uv: Vec2) {
        self.verts.push(UiVertex { pos: p.extend(0.0), color: c, uv0: Vec4::new(uv.x, uv.y, 0.0, 0.0), uv1: Vec4::ZERO });
    }
    fn add_tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend([a, b, c]);
    }
    fn add_quad(&mut self, min: Vec2, max: Vec2, c: [u8; 4], uv_min: Vec2, uv_max: Vec2) {
        let b = self.verts.len() as u32;
        self.add_vert(Vec2::new(min.x, min.y), c, Vec2::new(uv_min.x, uv_min.y));
        self.add_vert(Vec2::new(min.x, max.y), c, Vec2::new(uv_min.x, uv_max.y));
        self.add_vert(Vec2::new(max.x, max.y), c, Vec2::new(uv_max.x, uv_max.y));
        self.add_vert(Vec2::new(max.x, min.y), c, Vec2::new(uv_max.x, uv_min.y));
        self.add_tri(b, b + 1, b + 2);
        self.add_tri(b + 2, b + 3, b);
    }
    fn add_quad4(&mut self, xy: &[Vec2; 4], c: [u8; 4], uv: &[Vec2; 4]) {
        let b = self.verts.len() as u32;
        for i in 0..4 {
            self.add_vert(xy[i], c, uv[i]);
        }
        self.add_tri(b, b + 1, b + 2);
        self.add_tri(b + 2, b + 3, b);
    }
    /// VertexHelper.GetUIVertexStream
    fn stream(&self) -> Vec<UiVertex> {
        self.idx.iter().map(|&i| self.verts[i as usize]).collect()
    }
    /// VertexHelper.AddUIVertexTriangleStream after Clear
    fn set_stream(&mut self, s: Vec<UiVertex>) {
        self.idx = (0..s.len() as u32).collect();
        self.verts = s;
    }
}

/// Graphic.OnPopulateMesh: a quad over the (pixel adjusted) rect.
fn graphic_quad(vh: &mut Vh, r: Rect, c: [u8; 4]) {
    let v = Vec4::new(r.x, r.y, r.xmax(), r.ymax());
    vh.add_vert(Vec2::new(v.x, v.y), c, Vec2::new(0.0, 0.0));
    vh.add_vert(Vec2::new(v.x, v.w), c, Vec2::new(0.0, 1.0));
    vh.add_vert(Vec2::new(v.z, v.w), c, Vec2::new(1.0, 1.0));
    vh.add_vert(Vec2::new(v.z, v.y), c, Vec2::new(1.0, 0.0));
    vh.add_tri(0, 1, 2);
    vh.add_tri(2, 3, 0);
}

fn v4(a: [f32; 4]) -> Vec4 {
    Vec4::from_array(a)
}

/// Image.OnPopulateMesh for an Image with an active sprite.
struct ImageMesher<'a> {
    im: &'a ImageDef,
    sp: &'a uk_assets::ui::Sprite,
    /// sprite texture wrapMode != Repeat
    tex_wrap_not_repeat: bool,
    rect: Rect,
    /// GetPixelAdjustedRect
    adj: Rect,
    pivot: Vec2,
    mppu: f32,
    color: [u8; 4],
    fill: f32,
}

impl ImageMesher<'_> {
    fn has_border(&self) -> bool {
        v4(self.sp.border).length_squared() > 0.0
    }

    fn preserve_sprite_aspect_ratio(&self, rect: &mut Rect, sprite_size: Vec2) {
        let num = sprite_size.x / sprite_size.y;
        let num2 = rect.w / rect.h;
        if num > num2 {
            let height = rect.h;
            rect.h = rect.w * (1.0 / num);
            rect.y += (height - rect.h) * self.pivot.y;
        } else {
            let width = rect.w;
            rect.w = rect.h * num;
            rect.x += (width - rect.w) * self.pivot.x;
        }
    }

    fn drawing_dimensions(&self, preserve: bool) -> Vec4 {
        let v = v4(self.sp.padding);
        let sprite_size = Vec2::new(self.sp.rect[2], self.sp.rect[3]);
        let mut rect = self.adj;
        let num = sprite_size.x.round() as i32 as f32;
        let num2 = sprite_size.y.round() as i32 as f32;
        let v2 = Vec4::new(v.x / num, v.y / num2, (num - v.z) / num, (num2 - v.w) / num2);
        if preserve && sprite_size.length_squared() > 0.0 {
            self.preserve_sprite_aspect_ratio(&mut rect, sprite_size);
        }
        Vec4::new(rect.x + rect.w * v2.x, rect.y + rect.h * v2.y, rect.x + rect.w * v2.z, rect.y + rect.h * v2.w)
    }

    fn adjusted_borders(&self, mut border: Vec4, adjusted: Rect) -> Vec4 {
        let rect = self.rect;
        let rs = [rect.w, rect.h];
        let asz = [adjusted.w, adjusted.h];
        for i in 0..=1 {
            if rs[i] != 0.0 {
                let num = asz[i] / rs[i];
                border[i] *= num;
                border[i + 2] *= num;
            }
            let num2 = border[i] + border[i + 2];
            if asz[i] < num2 && num2 != 0.0 {
                let num = asz[i] / num2;
                border[i] *= num;
                border[i + 2] *= num;
            }
        }
        border
    }

    fn simple(&self, vh: &mut Vh, preserve: bool) {
        let d = self.drawing_dimensions(preserve);
        let u = v4(self.sp.outer_uv);
        let c = self.color;
        vh.add_vert(Vec2::new(d.x, d.y), c, Vec2::new(u.x, u.y));
        vh.add_vert(Vec2::new(d.x, d.w), c, Vec2::new(u.x, u.w));
        vh.add_vert(Vec2::new(d.z, d.w), c, Vec2::new(u.z, u.w));
        vh.add_vert(Vec2::new(d.z, d.y), c, Vec2::new(u.z, u.y));
        vh.add_tri(0, 1, 2);
        vh.add_tri(2, 3, 0);
    }

    fn sliced(&self, vh: &mut Vh) {
        if !self.has_border() {
            self.simple(vh, false);
            return;
        }
        let outer = v4(self.sp.outer_uv);
        let inner = v4(self.sp.inner_uv);
        let mut pad = v4(self.sp.padding);
        let border = v4(self.sp.border);
        let r = self.adj;
        let ab = self.adjusted_borders(border / self.mppu, r);
        pad /= self.mppu;
        let mut vs = [Vec2::ZERO; 4];
        vs[0] = Vec2::new(pad.x, pad.y);
        vs[3] = Vec2::new(r.w - pad.z, r.h - pad.w);
        vs[1] = Vec2::new(ab.x, ab.y);
        vs[2] = Vec2::new(r.w - ab.z, r.h - ab.w);
        for v in &mut vs {
            v.x += r.x;
            v.y += r.y;
        }
        let uvs = [Vec2::new(outer.x, outer.y), Vec2::new(inner.x, inner.y), Vec2::new(inner.z, inner.w), Vec2::new(outer.z, outer.w)];
        for j in 0..3 {
            let num = j + 1;
            for k in 0..3 {
                if self.im.fill_center || j != 1 || k != 1 {
                    let num2 = k + 1;
                    vh.add_quad(
                        Vec2::new(vs[j].x, vs[k].y),
                        Vec2::new(vs[num].x, vs[num2].y),
                        self.color,
                        Vec2::new(uvs[j].x, uvs[k].y),
                        Vec2::new(uvs[num].x, uvs[num2].y),
                    );
                }
            }
        }
    }

    fn tiled(&self, vh: &mut Vh) {
        let v = v4(self.sp.outer_uv);
        let v2 = v4(self.sp.inner_uv);
        let mut v3 = v4(self.sp.border);
        let v4s = Vec2::new(self.sp.rect[2], self.sp.rect[3]);
        let r = self.adj;
        let pos = Vec2::new(r.x, r.y);
        let mut num = (v4s.x - v3.x - v3.z) / self.mppu;
        let mut num2 = (v4s.y - v3.y - v3.w) / self.mppu;
        v3 = self.adjusted_borders(v3 / self.mppu, r);
        let v5 = Vec2::new(v2.x, v2.y);
        let v6 = Vec2::new(v2.z, v2.w);
        let x = v3.x;
        let num3 = r.w - v3.z;
        let y = v3.y;
        let num4 = r.h - v3.w;
        let mut uv_max = v6;
        if num <= 0.0 {
            num = num3 - x;
        }
        if num2 <= 0.0 {
            num2 = num4 - y;
        }
        let c = self.color;
        let has_border = self.has_border();
        if has_border || self.sp.packed || self.tex_wrap_not_repeat {
            let mut num5: i64;
            let mut num6: i64;
            if self.im.fill_center {
                num5 = ((num3 - x) / num).ceil() as f64 as i64;
                num6 = ((num4 - y) / num2).ceil() as f64 as i64;
                let num7 = if !has_border { (num5 * num6) as f64 * 4.0 } else { (num5 as f64 + 2.0) * (num6 as f64 + 2.0) * 4.0 };
                if num7 > 65000.0 {
                    let num8 = if !has_border { num5 as f64 / num6 as f64 } else { (num5 as f64 + 2.0) / (num6 as f64 + 2.0) };
                    let mut num9 = (16250.0 / num8).sqrt();
                    let mut num10 = num9 * num8;
                    if has_border {
                        num9 -= 2.0;
                        num10 -= 2.0;
                    }
                    num5 = num9.floor() as i64;
                    num6 = num10.floor() as i64;
                    num = (num3 - x) / num5 as f32;
                    num2 = (num4 - y) / num6 as f32;
                }
            } else if has_border {
                num5 = ((num3 - x) / num).ceil() as i64;
                num6 = ((num4 - y) / num2).ceil() as i64;
                if ((num6 + num5) as f64 + 2.0) * 2.0 * 4.0 > 65000.0 {
                    let num11 = num5 as f64 / num6 as f64;
                    let num12 = (16250.0 - 4.0) / (2.0 * (1.0 + num11));
                    let d = num12 * num11;
                    num5 = num12.floor() as i64;
                    num6 = d.floor() as i64;
                    num = (num3 - x) / num5 as f32;
                    num2 = (num4 - y) / num6 as f32;
                }
            } else {
                num5 = 0;
                num6 = 0;
            }
            if self.im.fill_center {
                for num13 in 0..num6 {
                    let num14 = y + num13 as f32 * num2;
                    let mut num15 = y + (num13 + 1) as f32 * num2;
                    if num15 > num4 {
                        uv_max.y = v5.y + (v6.y - v5.y) * (num4 - num14) / (num15 - num14);
                        num15 = num4;
                    }
                    uv_max.x = v6.x;
                    for num16 in 0..num5 {
                        let num17 = x + num16 as f32 * num;
                        let mut num18 = x + (num16 + 1) as f32 * num;
                        if num18 > num3 {
                            uv_max.x = v5.x + (v6.x - v5.x) * (num3 - num17) / (num18 - num17);
                            num18 = num3;
                        }
                        vh.add_quad(Vec2::new(num17, num14) + pos, Vec2::new(num18, num15) + pos, c, v5, uv_max);
                    }
                }
            }
            if !has_border {
                return;
            }
            uv_max = v6;
            for num19 in 0..num6 {
                let num20 = y + num19 as f32 * num2;
                let mut num21 = y + (num19 + 1) as f32 * num2;
                if num21 > num4 {
                    uv_max.y = v5.y + (v6.y - v5.y) * (num4 - num20) / (num21 - num20);
                    num21 = num4;
                }
                vh.add_quad(Vec2::new(0.0, num20) + pos, Vec2::new(x, num21) + pos, c, Vec2::new(v.x, v5.y), Vec2::new(v5.x, uv_max.y));
                vh.add_quad(Vec2::new(num3, num20) + pos, Vec2::new(r.w, num21) + pos, c, Vec2::new(v6.x, v5.y), Vec2::new(v.z, uv_max.y));
            }
            uv_max = v6;
            for num22 in 0..num5 {
                let num23 = x + num22 as f32 * num;
                let mut num24 = x + (num22 + 1) as f32 * num;
                if num24 > num3 {
                    uv_max.x = v5.x + (v6.x - v5.x) * (num3 - num23) / (num24 - num23);
                    num24 = num3;
                }
                vh.add_quad(Vec2::new(num23, 0.0) + pos, Vec2::new(num24, y) + pos, c, Vec2::new(v5.x, v.y), Vec2::new(uv_max.x, v5.y));
                vh.add_quad(Vec2::new(num23, num4) + pos, Vec2::new(num24, r.h) + pos, c, Vec2::new(v5.x, v6.y), Vec2::new(uv_max.x, v.w));
            }
            vh.add_quad(Vec2::new(0.0, 0.0) + pos, Vec2::new(x, y) + pos, c, Vec2::new(v.x, v.y), Vec2::new(v5.x, v5.y));
            vh.add_quad(Vec2::new(num3, 0.0) + pos, Vec2::new(r.w, y) + pos, c, Vec2::new(v6.x, v.y), Vec2::new(v.z, v5.y));
            vh.add_quad(Vec2::new(0.0, num4) + pos, Vec2::new(x, r.h) + pos, c, Vec2::new(v.x, v6.y), Vec2::new(v5.x, v.w));
            vh.add_quad(Vec2::new(num3, num4) + pos, Vec2::new(r.w, r.h) + pos, c, Vec2::new(v6.x, v6.y), Vec2::new(v.z, v.w));
        } else {
            let b = Vec2::new((num3 - x) / num, (num4 - y) / num2);
            if self.im.fill_center {
                vh.add_quad(Vec2::new(x, y) + pos, Vec2::new(num3, num4) + pos, c, v5 * b, v6 * b);
            }
        }
    }

    fn filled(&self, vh: &mut Vh, preserve: bool) {
        let fill_amount = self.fill;
        if fill_amount < 0.001 {
            return;
        }
        let mut dd = self.drawing_dimensions(preserve);
        let o = v4(self.sp.outer_uv);
        let (mut num, mut num2, mut num3, mut num4) = (o.x, o.y, o.z, o.w);
        let method = self.im.fill_method;
        let origin = self.im.fill_origin;
        let cw = self.im.fill_clockwise;
        if method == 0 {
            let num5 = (num3 - num) * fill_amount;
            if origin == 1 {
                dd.x = dd.z - (dd.z - dd.x) * fill_amount;
                num = num3 - num5;
            } else {
                dd.z = dd.x + (dd.z - dd.x) * fill_amount;
                num3 = num + num5;
            }
        } else if method == 1 {
            let num6 = (num4 - num2) * fill_amount;
            if origin == 1 {
                dd.y = dd.w - (dd.w - dd.y) * fill_amount;
                num2 = num4 - num6;
            } else {
                dd.w = dd.y + (dd.w - dd.y) * fill_amount;
                num4 = num2 + num6;
            }
        }
        let mut xy = [Vec2::new(dd.x, dd.y), Vec2::new(dd.x, dd.w), Vec2::new(dd.z, dd.w), Vec2::new(dd.z, dd.y)];
        let mut uv = [Vec2::new(num, num2), Vec2::new(num, num4), Vec2::new(num3, num4), Vec2::new(num3, num2)];
        let c = self.color;
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        if fill_amount < 1.0 && method != 0 && method != 1 {
            if method == 2 {
                if radial_cut(&mut xy, &mut uv, fill_amount, cw, origin as i32) {
                    vh.add_quad4(&xy, c, &uv);
                }
            } else if method == 3 {
                for i in 0..2i32 {
                    let num7 = if origin > 1 { 1 } else { 0 };
                    let (t, t2, t3, t4);
                    if origin == 0 || origin == 2 {
                        t = 0.0;
                        t2 = 1.0;
                        if i == num7 {
                            t3 = 0.0;
                            t4 = 0.5;
                        } else {
                            t3 = 0.5;
                            t4 = 1.0;
                        }
                    } else {
                        t3 = 0.0;
                        t4 = 1.0;
                        if i == num7 {
                            t = 0.5;
                            t2 = 1.0;
                        } else {
                            t = 0.0;
                            t2 = 0.5;
                        }
                    }
                    set_quad(&mut xy, &mut uv, &dd, [num, num2, num3, num4], t3, t4, t, t2, lerp);
                    let value = if cw { fill_amount * 2.0 - i as f32 } else { fill_amount * 2.0 - (1 - i) as f32 };
                    if radial_cut(&mut xy, &mut uv, value.clamp(0.0, 1.0), cw, (i + origin as i32 + 3) % 4) {
                        vh.add_quad4(&xy, c, &uv);
                    }
                }
            } else if method == 4 {
                for j in 0..4i32 {
                    let (t5, t6) = if j < 2 { (0.0, 0.5) } else { (0.5, 1.0) };
                    let (t7, t8) = if j == 0 || j == 3 { (0.0, 0.5) } else { (0.5, 1.0) };
                    set_quad(&mut xy, &mut uv, &dd, [num, num2, num3, num4], t5, t6, t7, t8, lerp);
                    let value2 = if cw {
                        fill_amount * 4.0 - ((j + origin as i32) % 4) as f32
                    } else {
                        fill_amount * 4.0 - (3 - (j + origin as i32) % 4) as f32
                    };
                    if radial_cut(&mut xy, &mut uv, value2.clamp(0.0, 1.0), cw, (j + 2) % 4) {
                        vh.add_quad4(&xy, c, &uv);
                    }
                }
            }
        } else {
            vh.add_quad4(&xy, c, &uv);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn set_quad(xy: &mut [Vec2; 4], uv: &mut [Vec2; 4], dd: &Vec4, u: [f32; 4], tx0: f32, tx1: f32, ty0: f32, ty1: f32, lerp: impl Fn(f32, f32, f32) -> f32) {
    xy[0].x = lerp(dd.x, dd.z, tx0);
    xy[1].x = xy[0].x;
    xy[2].x = lerp(dd.x, dd.z, tx1);
    xy[3].x = xy[2].x;
    xy[0].y = lerp(dd.y, dd.w, ty0);
    xy[1].y = lerp(dd.y, dd.w, ty1);
    xy[2].y = xy[1].y;
    xy[3].y = xy[0].y;
    uv[0].x = lerp(u[0], u[2], tx0);
    uv[1].x = uv[0].x;
    uv[2].x = lerp(u[0], u[2], tx1);
    uv[3].x = uv[2].x;
    uv[0].y = lerp(u[1], u[3], ty0);
    uv[1].y = lerp(u[1], u[3], ty1);
    uv[2].y = uv[1].y;
    uv[3].y = uv[0].y;
}

/// Image.RadialCut(xy, uv, fill, invert, corner)
fn radial_cut(xy: &mut [Vec2; 4], uv: &mut [Vec2; 4], fill: f32, mut invert: bool, corner: i32) -> bool {
    if fill < 0.001 {
        return false;
    }
    if (corner & 1) == 1 {
        invert = !invert;
    }
    if !invert && fill > 0.999 {
        return true;
    }
    let mut num = fill.clamp(0.0, 1.0);
    if invert {
        num = 1.0 - num;
    }
    num *= std::f32::consts::PI / 2.0;
    let cos = num.cos();
    let sin = num.sin();
    radial_cut_pts(xy, cos, sin, invert, corner);
    radial_cut_pts(uv, cos, sin, invert, corner);
    true
}

fn radial_cut_pts(xy: &mut [Vec2; 4], mut cos: f32, mut sin: f32, invert: bool, corner: i32) {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t.clamp(0.0, 1.0);
    let c = corner as usize;
    let num = ((corner + 1) % 4) as usize;
    let num2 = ((corner + 2) % 4) as usize;
    let num3 = ((corner + 3) % 4) as usize;
    if (corner & 1) == 1 {
        if sin > cos {
            cos /= sin;
            sin = 1.0;
            if invert {
                xy[num].x = lerp(xy[c].x, xy[num2].x, cos);
                xy[num2].x = xy[num].x;
            }
        } else if cos > sin {
            sin /= cos;
            cos = 1.0;
            if !invert {
                xy[num2].y = lerp(xy[c].y, xy[num2].y, sin);
                xy[num3].y = xy[num2].y;
            }
        } else {
            cos = 1.0;
            sin = 1.0;
        }
        if !invert {
            xy[num3].x = lerp(xy[c].x, xy[num2].x, cos);
        } else {
            xy[num].y = lerp(xy[c].y, xy[num2].y, sin);
        }
        return;
    }
    if cos > sin {
        sin /= cos;
        cos = 1.0;
        if !invert {
            xy[num].y = lerp(xy[c].y, xy[num2].y, sin);
            xy[num2].y = xy[num].y;
        }
    } else if sin > cos {
        cos /= sin;
        sin = 1.0;
        if invert {
            xy[num2].x = lerp(xy[c].x, xy[num2].x, cos);
            xy[num3].x = xy[num2].x;
        }
    } else {
        cos = 1.0;
        sin = 1.0;
    }
    if invert {
        xy[num3].y = lerp(xy[c].y, xy[num2].y, sin);
    } else {
        xy[num].x = lerp(xy[c].x, xy[num2].x, cos);
    }
}

/// Shadow.ApplyShadowZeroAlloc
fn apply_shadow(verts: &mut Vec<UiVertex>, color: [u8; 4], start: usize, end: usize, x: f32, y: f32, use_graphic_alpha: bool) {
    for i in start..end {
        let mut v = verts[i];
        verts.push(v);
        v.pos.x += x;
        v.pos.y += y;
        let mut c2 = color;
        if use_graphic_alpha {
            c2[3] = (c2[3] as u32 * verts[i].color[3] as u32 / 255) as u8;
        }
        v.color = c2;
        verts[i] = v;
    }
}

fn apply_effect(vh: &mut Vh, e: &EffectDef) {
    let mut list = vh.stream();
    let c = color32(e.color);
    let (x, y) = (e.distance[0], e.distance[1]);
    if e.outline {
        let mut start = 0;
        let count = list.len();
        apply_shadow(&mut list, c, start, count, x, y, e.use_graphic_alpha);
        start = count;
        let count2 = list.len();
        apply_shadow(&mut list, c, start, count2, x, -y, e.use_graphic_alpha);
        start = count2;
        let count3 = list.len();
        apply_shadow(&mut list, c, start, count3, -x, y, e.use_graphic_alpha);
        start = count3;
        let end = list.len();
        apply_shadow(&mut list, c, start, end, -x, -y, e.use_graphic_alpha);
    } else {
        let end = list.len();
        apply_shadow(&mut list, c, 0, end, x, y, e.use_graphic_alpha);
    }
    vh.set_stream(list);
}

/// root canvas data for a layout pass
#[derive(Clone, Copy)]
struct RootCtx {
    root: u32,
    mode: RenderMode,
    scale: f32,
    ref_ppu: f32,
    pixel_perfect: bool,
    to_screen: Mat4,
}

struct Layout<'a> {
    inp: &'a UiInput<'a>,
    frame: UiFrame,
    /// per node script indices (component order)
    scripts_by_node: HashMap<u32, Vec<u32>>,
}

/// RectTransform world corners (0: bottom-left, 1: top-left, 2: top-right, 3: bottom-right) in root space
fn corners(m: &Mat4, r: &Rect) -> [Vec3; 4] {
    [
        m.transform_point3(Vec3::new(r.x, r.y, 0.0)),
        m.transform_point3(Vec3::new(r.x, r.ymax(), 0.0)),
        m.transform_point3(Vec3::new(r.xmax(), r.ymax(), 0.0)),
        m.transform_point3(Vec3::new(r.xmax(), r.y, 0.0)),
    ]
}

impl<'a> Layout<'a> {
    fn script_on(&self, ok: bool, s: u32) -> bool {
        ok && self.inp.script_enabled.get(s as usize).copied().unwrap_or(true)
    }

    fn node_active(&self, n: u32) -> bool {
        self.inp.active.get(n as usize).copied().unwrap_or(false)
    }

    fn graphic_active(&self, g: u32) -> bool {
        let gd = &self.inp.ui.graphics[g as usize];
        self.node_active(gd.node) && self.script_on(true, gd.script)
    }

    /// Mask.MaskEnabled() && graphic.IsActive()
    fn mask_on(&self, n: u32) -> bool {
        let ui = self.inp.ui;
        let Some(&m) = ui.node_mask.get(&n) else { return false };
        let md = &ui.masks[m as usize];
        if !(self.node_active(n) && self.script_on(true, md.script)) {
            return false;
        }
        match ui.node_graphic.get(&n) {
            Some(&g) => self.graphic_active(g),
            None => false,
        }
    }

    fn canvas_on(&self, c: u32) -> bool {
        self.inp.state.canvas_enabled[c as usize] && self.node_active(self.inp.ui.canvases[c as usize].node)
    }

    /// MaskUtilities.FindRootSortOverrideCanvas
    fn root_sort_override_canvas(&self, start: u32) -> Option<u32> {
        let def = self.inp.def;
        let mut found = None;
        let mut n = Some(start);
        while let Some(x) = n {
            if let Some(&c) = self.inp.ui.node_canvas.get(&x) {
                if self.node_active(x) {
                    found = Some(x);
                    if self.inp.ui.canvases[c as usize].override_sorting {
                        break;
                    }
                }
            }
            n = def.nodes[x as usize].parent;
        }
        found
    }

    /// MaskUtilities.GetStencilDepth
    fn stencil_depth(&self, t: u32, stop_after: Option<u32>) -> i32 {
        let def = self.inp.def;
        let mut num = 0;
        if Some(t) == stop_after {
            return 0;
        }
        let mut parent = def.nodes[t as usize].parent;
        while let Some(p) = parent {
            if self.mask_on(p) {
                num += 1;
            }
            if Some(p) == stop_after {
                break;
            }
            parent = def.nodes[p as usize].parent;
        }
        num
    }

    fn is_descendant_or_self(&self, father: u32, child: u32) -> bool {
        self.inp.def.is_descendant(child, father)
    }

    /// active canvases of `n` and its parents (GetComponentsInParent order: self first)
    fn canvases_in_parent(&self, n: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut x = Some(n);
        while let Some(p) = x {
            if let Some(&c) = self.inp.ui.node_canvas.get(&p) {
                if self.node_active(p) {
                    out.push(c);
                }
            }
            x = self.inp.def.nodes[p as usize].parent;
        }
        out
    }

    fn rect_masks_in_parent(&self, n: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut x = Some(n);
        while let Some(p) = x {
            if let Some(&m) = self.inp.ui.node_rect_mask.get(&p) {
                if self.node_active(p) {
                    out.push(m);
                }
            }
            x = self.inp.def.nodes[p as usize].parent;
        }
        out
    }

    fn rect_mask_active(&self, m: u32) -> bool {
        let rm = &self.inp.ui.rect_masks[m as usize];
        self.node_active(rm.node) && self.script_on(true, rm.script)
    }

    /// MaskUtilities.GetRectMaskForClippable
    fn rect_mask_for(&self, n: u32) -> Option<u32> {
        let ui = self.inp.ui;
        let list = self.rect_masks_in_parent(n);
        for &m in &list {
            let mnode = ui.rect_masks[m as usize].node;
            if mnode == n || !self.rect_mask_active(m) {
                continue;
            }
            for &c in self.canvases_in_parent(n).iter().rev() {
                let cd = &ui.canvases[c as usize];
                if !self.is_descendant_or_self(cd.node, mnode) && cd.override_sorting {
                    return None;
                }
            }
            return Some(m);
        }
        None
    }

    /// MaskUtilities.GetRectMasksForClip
    fn rect_masks_for_clip(&self, clipper: u32) -> Vec<u32> {
        let ui = self.inp.ui;
        let cnode = ui.rect_masks[clipper as usize].node;
        let list2 = self.rect_masks_in_parent(cnode);
        let list = self.canvases_in_parent(cnode);
        let mut masks = Vec::new();
        for &m in list2.iter().rev() {
            if self.rect_mask_active(m) {
                let mnode = ui.rect_masks[m as usize].node;
                let ok = !list.iter().rev().any(|&c| {
                    let cd = &ui.canvases[c as usize];
                    !self.is_descendant_or_self(cd.node, mnode) && cd.override_sorting
                });
                if ok {
                    masks.push(m);
                }
            }
        }
        masks
    }

    fn canvas_rect_of(&self, n: u32) -> Option<Rect> {
        let m = self.frame.node_to_root.get(&n)?;
        let r = self.frame.node_rects.get(&n)?;
        let c = corners(m, r);
        Some(Rect { x: c[0].x, y: c[0].y, w: c[2].x - c[0].x, h: c[2].y - c[0].y })
    }

    fn bounds_rect_of(&self, n: u32) -> Option<Rect> {
        let m = self.frame.node_to_root.get(&n)?;
        let r = self.frame.node_rects.get(&n)?;
        let c = corners(m, r);
        let mut mn = Vec2::new(c[0].x, c[0].y);
        let mut mx = mn;
        for p in &c[1..] {
            mn = mn.min(Vec2::new(p.x, p.y));
            mx = mx.max(Vec2::new(p.x, p.y));
        }
        Some(Rect { x: mn.x, y: mn.y, w: mx.x - mn.x, h: mx.y - mn.y })
    }

    /// RectMask2D.PerformClipping for the clip parent `m`: (clip rect, valid)
    fn perform_clipping(&self, m: u32, rc: &RootCtx) -> (Rect, bool) {
        let ui = self.inp.ui;
        let clippers = self.rect_masks_for_clip(m);
        let mut valid;
        let rect;
        if clippers.is_empty() {
            valid = false;
            rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        } else {
            let mut num = f32::MIN;
            let mut num2 = f32::MAX;
            let mut num3 = f32::MIN;
            let mut num4 = f32::MAX;
            for (i, &c) in clippers.iter().enumerate() {
                let rm = &ui.rect_masks[c as usize];
                let cr = self.canvas_rect_of(rm.node).unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
                let p = rm.padding;
                let (a, b, cc, d) = (cr.x + p[0], cr.xmax() - p[2], cr.y + p[1], cr.ymax() - p[3]);
                if i == 0 {
                    num = a;
                    num2 = b;
                    num3 = cc;
                    num4 = d;
                } else {
                    if num < a {
                        num = a;
                    }
                    if num3 < cc {
                        num3 = cc;
                    }
                    if num2 > b {
                        num2 = b;
                    }
                    if num4 > d {
                        num4 = d;
                    }
                }
            }
            valid = num2 > num && num4 > num3;
            rect = if valid { Rect { x: num, y: num3, w: num2 - num, h: num4 - num3 } } else { Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 } };
        }
        let mut rect = rect;
        if rc.mode != RenderMode::World {
            let root_rect = self.bounds_rect_of(ui.rect_masks[m as usize].node);
            if !root_rect.is_some_and(|r| rect.overlaps(&r)) {
                rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
                valid = false;
            }
        }
        (rect, valid)
    }

    /// Graphic.GetPixelAdjustedRect (RectTransformUtility.PixelAdjustRect on pixel perfect screen canvases)
    fn pixel_adjusted_rect(&self, r: Rect, m: &Mat4, rc: &RootCtx) -> Rect {
        if !rc.pixel_perfect || rc.mode == RenderMode::World || rc.scale == 0.0 {
            return r;
        }
        let to_px = rc.to_screen * *m;
        let inv = to_px.inverse();
        let adj = |p: Vec2| {
            let s = to_px.transform_point3(p.extend(0.0));
            let s = Vec3::new(s.x.round(), s.y.round(), s.z);
            let l = inv.transform_point3(s);
            Vec2::new(l.x, l.y)
        };
        let mn = adj(Vec2::new(r.x, r.y));
        let mx = adj(Vec2::new(r.xmax(), r.ymax()));
        Rect { x: mn.x, y: mn.y, w: mx.x - mn.x, h: mx.y - mn.y }
    }

    /// CanvasGroup inherited alpha
    fn group_alpha(&self, n: u32) -> f32 {
        let ui = self.inp.ui;
        let st = self.inp.state;
        let mut a = 1.0;
        let mut x = Some(n);
        while let Some(p) = x {
            if let Some(&g) = ui.node_group.get(&p) {
                if st.group_enabled[g as usize] {
                    a *= st.group_alpha[g as usize];
                    if ui.groups[g as usize].ignore_parent {
                        break;
                    }
                }
            }
            x = self.inp.def.nodes[p as usize].parent;
        }
        a
    }

    fn rect_def(&self, n: u32) -> Option<RectDef> {
        self.inp.state.rects.get(&n).copied().or(self.inp.def.nodes[n as usize].rect)
    }

    fn local_trs(&self, n: u32) -> (Quat, Vec3) {
        let nd = &self.inp.def.nodes[n as usize];
        let rot = self.inp.state.rot.get(&n).copied().unwrap_or_else(|| {
            let q = nd.local_rot;
            Quat::from_xyzw(-q.x, -q.y, q.z, q.w)
        });
        let scale = self.inp.state.scale.get(&n).copied().unwrap_or(nd.local_scale);
        (rot, scale)
    }

    fn root(&mut self, ci: u32) {
        let ui = self.inp.ui;
        let cd = &ui.canvases[ci as usize];
        let r = cd.node;
        if !self.canvas_on(ci) {
            return;
        }
        let [w, h] = self.inp.screen;
        // CanvasScaler.Handle
        let mut scale = 1.0;
        let mut ref_ppu = 100.0;
        if let Some(&si) = ui.node_scaler.get(&r) {
            let s = &ui.scalers[si as usize];
            if self.script_on(true, s.script) {
                if cd.mode == RenderMode::World {
                    scale = s.dynamic_ppu;
                    ref_ppu = s.reference_ppu;
                } else {
                    match s.mode {
                        0 => {
                            scale = s.scale_factor;
                            ref_ppu = s.reference_ppu;
                        }
                        1 => {
                            let [rx, ry] = s.reference_resolution;
                            scale = match s.match_mode {
                                0 => {
                                    let a = (w / rx).log2();
                                    let b = (h / ry).log2();
                                    2f32.powf(a + (b - a) * s.match_width_or_height)
                                }
                                1 => (w / rx).min(h / ry),
                                2 => (w / rx).max(h / ry),
                                _ => 0.0,
                            };
                            ref_ppu = s.reference_ppu;
                        }
                        _ => {
                            let dpi = if self.inp.dpi == 0.0 { s.fallback_dpi } else { self.inp.dpi };
                            let num3 = match s.physical_unit {
                                0 => 2.54,
                                1 => 25.4,
                                2 => 1.0,
                                3 => 72.0,
                                4 => 6.0,
                                _ => 1.0,
                            };
                            scale = dpi / num3;
                            ref_ppu = s.reference_ppu * num3 / s.default_sprite_dpi;
                        }
                    }
                }
            }
        }
        let rd = self.rect_def(r).unwrap_or(RectDef { anchor_min: [0.0; 2], anchor_max: [0.0; 2], anchored_pos: [0.0; 2], size_delta: [0.0; 2], pivot: [0.5; 2] });
        let pivot = Vec2::from_array(rd.pivot);
        let (root_rect, to_screen) = if cd.mode == RenderMode::World {
            let size = Vec2::from_array(rd.size_delta);
            (Rect { x: -pivot.x * size.x, y: -pivot.y * size.y, w: size.x, h: size.y }, Mat4::IDENTITY)
        } else {
            let size = Vec2::new(w, h) / scale;
            (
                Rect { x: -pivot.x * size.x, y: -pivot.y * size.y, w: size.x, h: size.y },
                Mat4::from_translation(Vec3::new(pivot.x * w, pivot.y * h, 0.0)) * Mat4::from_scale(Vec3::new(scale, scale, scale)),
            )
        };
        let rc = RootCtx { root: ci, mode: cd.mode, scale, ref_ppu, pixel_perfect: cd.pixel_perfect, to_screen };
        let bi = self.new_batch(ci, &rc, root_rect);
        self.frame.node_rects.insert(r, root_rect);
        self.frame.node_to_root.insert(r, Mat4::IDENTITY);
        self.visit_content(r, Mat4::IDENTITY, root_rect, &rc, bi);
    }

    fn new_batch(&mut self, canvas: u32, rc: &RootCtx, root_rect: Rect) -> usize {
        let ui = self.inp.ui;
        let root_node = ui.canvases[rc.root as usize].node;
        self.frame.batches.push(UiBatch {
            canvas,
            root: rc.root,
            root_node,
            mode: rc.mode,
            sorting_order: ui.canvases[canvas as usize].sorting_order,
            layer: self.inp.def.nodes[root_node as usize].layer,
            to_screen: rc.to_screen,
            scale_factor: rc.scale,
            reference_ppu: rc.ref_ppu,
            root_rect,
            draws: Vec::new(),
        });
        self.frame.batches.len() - 1
    }

    fn visit(&mut self, n: u32, parent_m: Mat4, parent_rect: Rect, rc: &RootCtx, mut bi: usize) {
        if !self.node_active(n) {
            return;
        }
        let ui = self.inp.ui;
        if let Some(&c) = ui.node_canvas.get(&n) {
            if !self.inp.state.canvas_enabled[c as usize] {
                return;
            }
            if ui.canvases[c as usize].override_sorting {
                let rr = self.frame.batches[bi].root_rect;
                bi = self.new_batch(c, rc, rr);
            }
        }
        let nd = &self.inp.def.nodes[n as usize];
        let (rot, scale) = self.local_trs(n);
        let (pos, rect) = match self.rect_def(n) {
            Some(rd) => {
                let amin = Vec2::from_array(rd.anchor_min);
                let amax = Vec2::from_array(rd.anchor_max);
                let pivot = Vec2::from_array(rd.pivot);
                let psize = Vec2::new(parent_rect.w, parent_rect.h);
                let size = psize * (amax - amin) + Vec2::from_array(rd.size_delta);
                let p = Vec2::new(parent_rect.x, parent_rect.y) + psize * (amin + (amax - amin) * pivot) + Vec2::from_array(rd.anchored_pos);
                (Vec3::new(p.x, p.y, -nd.local_pos.z), Rect { x: -pivot.x * size.x, y: -pivot.y * size.y, w: size.x, h: size.y })
            }
            None => (to_bevy_point(nd.local_pos), Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }),
        };
        let m = parent_m * Mat4::from_scale_rotation_translation(scale, rot, pos);
        self.frame.node_rects.insert(n, rect);
        self.frame.node_to_root.insert(n, m);
        self.visit_content(n, m, rect, rc, bi);
    }

    fn visit_content(&mut self, n: u32, m: Mat4, rect: Rect, rc: &RootCtx, bi: usize) {
        let ui = self.inp.ui;
        let mut drawn = None;
        if let Some(&g) = ui.node_graphic.get(&n) {
            if self.graphic_active(g) {
                drawn = self.graphic(g, m, rect, rc, bi);
            }
        }
        let children = self.inp.def.nodes[n as usize].children.clone();
        for c in children {
            self.visit(c, m, rect, rc, bi);
        }
        // the Mask's pop instruction
        if let Some(mut d) = drawn {
            if self.mask_on(n) {
                let md = &ui.masks[ui.node_mask[&n] as usize];
                let _ = md;
                let depth = self.stencil_depth(n, self.root_sort_override_canvas(n));
                if depth < 8 {
                    let num = 1i32 << depth;
                    d.stencil = if num == 1 {
                        stencil_add(1, OP_ZERO, CMP_ALWAYS, 0, 255, 255).unwrap()
                    } else {
                        stencil_add(num - 1, OP_REPLACE, CMP_EQUAL, 0, num - 1, num | (num - 1)).unwrap()
                    };
                    d.pop = true;
                    self.frame.batches[bi].draws.push(d);
                }
            }
        }
    }

    /// materialForRendering's stencil: IMaterialModifier components in component order
    /// (TextMeshProUGUI.GetModifiedMaterial skips MaskableGraphic's own-Mask check)
    fn material_stencil(&self, g: u32, tmp: bool) -> Option<Stencil> {
        let ui = self.inp.ui;
        let gd = &ui.graphics[g as usize];
        let n = gd.node;
        let mut stencil = None;
        let mask = ui.node_mask.get(&n).copied().filter(|_| self.mask_on(n));
        let mut modifiers: Vec<(u32, bool)> = vec![(gd.script, false)];
        if let Some(mk) = mask {
            modifiers.push((ui.masks[mk as usize].script, true));
        }
        let order = self.scripts_by_node.get(&n);
        modifiers.sort_by_key(|(s, _)| order.and_then(|o| o.iter().position(|x| x == s)).unwrap_or(0));
        for (_, is_mask) in modifiers {
            if !is_mask {
                // MaskableGraphic.GetModifiedMaterial
                let v = if gd.maskable { self.stencil_depth(n, self.root_sort_override_canvas(n)) } else { 0 };
                if v > 0 && (tmp || mask.is_none()) {
                    let id = (1 << v) - 1;
                    stencil = stencil_add(id, OP_KEEP, CMP_EQUAL, 15, id, 0).or(stencil);
                }
            } else {
                // Mask.GetModifiedMaterial
                let md = &ui.masks[mask.unwrap() as usize];
                let depth = self.stencil_depth(n, self.root_sort_override_canvas(n));
                if depth < 8 {
                    let num = 1i32 << depth;
                    let cm = if md.show_graphic { 15 } else { 0 };
                    stencil = if num == 1 {
                        stencil_add(1, OP_REPLACE, CMP_ALWAYS, cm, 255, 255)
                    } else {
                        stencil_add(num | (num - 1), OP_REPLACE, CMP_EQUAL, cm, num - 1, num | (num - 1))
                    };
                }
            }
        }
        stencil
    }

    /// Emits the graphic's draw; returns a copy (for the Mask pop instruction).
    fn graphic(&mut self, g: u32, m: Mat4, rect: Rect, rc: &RootCtx, bi: usize) -> Option<UiDraw> {
        let inp = self.inp;
        let ui = inp.ui;
        let gd = &ui.graphics[g as usize];
        let n = gd.node;
        let st = inp.state;
        if let GraphicKind::Tmp(td) = &gd.kind {
            return self.tmp_graphic(g, td, m, rect, rc, bi);
        }
        let color = color32(st.color[g as usize]);
        let adj = self.pixel_adjusted_rect(rect, &m, rc);
        self.frame.graphic_rects.insert(g, self.canvas_rect_of(n).unwrap_or(rect));
        let mut vh = Vh::default();
        let mut texture = None;
        match &gd.kind {
            GraphicKind::Image(im) => {
                let sprite = st.sprite[g as usize].and_then(|s| inp.assets.sprites.get(s as usize));
                match sprite {
                    None => graphic_quad(&mut vh, adj, color),
                    Some(sp) => {
                        texture = sp.texture;
                        let tex = sp.texture.and_then(|t| inp.assets.textures.get(t as usize));
                        let ppu = sp.pixels_per_unit / rc.ref_ppu;
                        let mesher = ImageMesher {
                            im,
                            sp,
                            tex_wrap_not_repeat: tex.is_some_and(|t| t.wrap_u != 0),
                            rect,
                            adj,
                            pivot: Vec2::from_array(self.rect_def(n).map(|r| r.pivot).unwrap_or([0.5; 2])),
                            mppu: ppu * im.ppu_multiplier,
                            color,
                            fill: st.fill[g as usize],
                        };
                        match im.ty {
                            0 => mesher.simple(&mut vh, im.preserve_aspect),
                            1 => mesher.sliced(&mut vh),
                            2 => mesher.tiled(&mut vh),
                            3 => mesher.filled(&mut vh, im.preserve_aspect),
                            _ => {}
                        }
                    }
                }
            }
            GraphicKind::RawImage { texture: t, uv_rect } => {
                // mainTexture: m_Texture ?? material.mainTexture ?? whiteTexture; texelSize * size == 1
                texture = *t;
                let v = Vec4::new(adj.x, adj.y, adj.xmax(), adj.ymax());
                let (x0, y0, x1, y1) = (uv_rect[0], uv_rect[1], uv_rect[0] + uv_rect[2], uv_rect[1] + uv_rect[3]);
                vh.add_vert(Vec2::new(v.x, v.y), color, Vec2::new(x0, y0));
                vh.add_vert(Vec2::new(v.x, v.w), color, Vec2::new(x0, y1));
                vh.add_vert(Vec2::new(v.z, v.w), color, Vec2::new(x1, y1));
                vh.add_vert(Vec2::new(v.z, v.y), color, Vec2::new(x1, y0));
                vh.add_tri(0, 1, 2);
                vh.add_tri(2, 3, 0);
            }
            // TODO(ugui): legacy Text meshes
            GraphicKind::Text | GraphicKind::Tmp(_) => {}
        }
        // IMeshModifier components in order
        if let Some(effects) = ui.node_effects.get(&n) {
            for &e in effects {
                let ed = &ui.effects[e as usize];
                if self.script_on(true, ed.script) {
                    apply_effect(&mut vh, ed);
                }
            }
        }
        let stencil = self.material_stencil(g, false);
        // RectMask2D clip + cull
        let mut clip = None;
        if gd.maskable {
            if let Some(rm) = self.rect_mask_for(n) {
                let (cr, valid) = self.perform_clipping(rm, rc);
                let bounds = self.bounds_rect_of(n);
                let cull = !valid || !bounds.is_some_and(|b| cr.overlaps(&b));
                if cull {
                    return None;
                }
                let sf = ui.rect_masks[rm as usize].softness;
                clip = Some(Clip { rect: [cr.x, cr.y, cr.xmax(), cr.ymax()], softness: [sf[0] as f32, sf[1] as f32] });
            }
        }
        // CanvasRenderer color and inherited alpha
        let crc = st.cr_color[g as usize];
        let alpha = self.group_alpha(n);
        let tint = [crc[0], crc[1], crc[2], crc[3] * alpha];
        let mut verts = vh.verts;
        for v in &mut verts {
            v.pos = m.transform_point3(v.pos);
            for k in 0..4 {
                v.color[k] = (v.color[k] as f32 * tint[k]).round().clamp(0.0, 255.0) as u8;
            }
        }
        let d = UiDraw {
            node: n,
            graphic: g,
            verts,
            idx: vh.idx,
            material: gd.material.or(inp.assets.default_material),
            texture,
            stencil: stencil.unwrap_or_default(),
            clip,
            pop: false,
            props: None,
            tmp: false,
        };
        self.frame.batches[bi].draws.push(d.clone());
        Some(d)
    }

    /// TextMeshProUGUI: its mesh, then its TMP_SubMeshUI children's (last first, as they are
    /// created as children and the main mesh's sub objects sort before the text's own children).
    fn tmp_graphic(&mut self, g: u32, td: &TmpDef, m: Mat4, rect: Rect, rc: &RootCtx, bi: usize) -> Option<UiDraw> {
        let inp = self.inp;
        let ui = inp.ui;
        let gd = &ui.graphics[g as usize];
        let n = gd.node;
        let st = inp.state;
        self.frame.graphic_rects.insert(g, self.canvas_rect_of(n).unwrap_or(rect));
        // uv2.y *= lossyScale.y (/ scaleFactor on overlay canvases, whose root is scaled by it)
        let ly = m.y_axis.truncate().length();
        let root_node = ui.canvases[rc.root as usize].node;
        let root_lossy = || inp.world_of.map_or(1.0, |f| f(root_node).y_axis.truncate().length());
        let uv2 = match rc.mode {
            RenderMode::Overlay => ly,
            RenderMode::Camera => {
                if ui.canvases[rc.root as usize].camera.is_some() {
                    root_lossy() * ly
                } else {
                    1.0
                }
            }
            RenderMode::World => root_lossy() * ly,
        };
        let text = st.text.get(g as usize).cloned().flatten();
        let color = st.color[g as usize];
        let key = TmpKey { text: text.clone(), rect: [rect.x, rect.y, rect.w, rect.h].map(f32::to_bits), uv2: uv2.to_bits(), color: color.map(f32::to_bits) };
        let cached = ui.tmp_cache.lock().unwrap().get(&g).filter(|(k, _)| *k == key).map(|(_, m)| m.clone());
        let mesh = match cached {
            Some(mesh) => mesh,
            None => {
                let mut rt = ui.tmp_mats.lock().unwrap();
                let tin = crate::tmp::TmpInput { def: td, assets: inp.assets, text: text.as_deref(), color, rect: [rect.x, rect.y, rect.w, rect.h], uv2_scale: uv2 };
                let mesh = Arc::new(crate::tmp::generate(&tin, &mut rt));
                drop(rt);
                ui.tmp_cache.lock().unwrap().insert(g, (key, mesh.clone()));
                mesh
            }
        };
        let stencil = self.material_stencil(g, true);
        // TMP_SubMeshUI.GetModifiedMaterial: its stencil depth counts the text's own Mask
        let sub_stencil = if gd.maskable {
            let v = self.stencil_depth(n, self.root_sort_override_canvas(n)) + self.mask_on(n) as i32;
            if v > 0 {
                let id = (1 << v) - 1;
                stencil_add(id, OP_KEEP, CMP_EQUAL, 15, id, 0)
            } else {
                None
            }
        } else {
            None
        };
        // RectMask2D clip; TextMeshProUGUI.Cull tests the compound mesh bounds
        let mut clip = None;
        if gd.maskable {
            if let Some(rm) = self.rect_mask_for(n) {
                let (cr, valid) = self.perform_clipping(rm, rc);
                if let Some((mn, mx)) = mesh.compound_bounds() {
                    let ratio = Vec2::new(m.x_axis.truncate().length(), m.y_axis.truncate().length());
                    let pos = m.w_axis.truncate().truncate();
                    let r = Rect { x: pos.x + mn.x * ratio.x, y: pos.y + mn.y * ratio.y, w: (mx.x - mn.x) * ratio.x, h: (mx.y - mn.y) * ratio.y };
                    if r.w != 0.0 && r.h != 0.0 && (!valid || !cr.overlaps(&r)) {
                        return None;
                    }
                }
                let sf = ui.rect_masks[rm as usize].softness;
                clip = Some(Clip { rect: [cr.x, cr.y, cr.xmax(), cr.ymax()], softness: [sf[0] as f32, sf[1] as f32] });
            }
        }
        let crc = st.cr_color[g as usize];
        let alpha = self.group_alpha(n);
        let tint = [crc[0], crc[1], crc[2], crc[3] * alpha];
        let rts = ui.tmp_mats.lock().unwrap();
        let draw_of = |sub: Option<&crate::tmp::TmpSub>, stencil: Option<Stencil>| {
            let (mut verts, idx, material, texture, props) = match sub {
                Some(s) => {
                    let r = &rts.list[s.rt as usize];
                    (s.verts.clone(), s.idx.clone(), Some(r.base), r.tex, Some(r.props.clone()))
                }
                None => (Vec::new(), Vec::new(), gd.material.or(inp.assets.default_material), None, None),
            };
            for v in &mut verts {
                v.pos = m.transform_point3(v.pos);
                for k in 0..4 {
                    v.color[k] = (v.color[k] as f32 * tint[k]).round().clamp(0.0, 255.0) as u8;
                }
            }
            UiDraw { node: n, graphic: g, verts, idx, material, texture, stencil: stencil.unwrap_or_default(), clip, pop: false, props, tmp: true }
        };
        let main = draw_of(mesh.subs.first(), stencil);
        let subs: Vec<UiDraw> = mesh.subs.iter().skip(1).rev().map(|s| draw_of(Some(s), sub_stencil)).collect();
        drop(rts);
        let draws = &mut self.frame.batches[bi].draws;
        draws.push(main.clone());
        draws.extend(subs);
        Some(main)
    }
}

/// Lays out and meshes every active canvas.
pub fn build_frame(inp: &UiInput) -> UiFrame {
    let mut scripts_by_node: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, s) in inp.def.scripts.iter().enumerate() {
        scripts_by_node.entry(s.node).or_default().push(i as u32);
    }
    let mut l = Layout { inp, frame: UiFrame::default(), scripts_by_node };
    for &r in &inp.ui.roots {
        l.root(r);
    }
    let mut f = l.frame;
    // overlay / camera canvases sort by sorting order (stable: scene order breaks ties); world canvases after
    let key = |b: &UiBatch| (b.mode == RenderMode::World, b.sorting_order);
    f.batches.sort_by_key(key);
    f
}
