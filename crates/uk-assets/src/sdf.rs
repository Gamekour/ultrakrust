//! FontEngine glyph rendering for dynamic TMP font assets (AtlasPopulationMode.Dynamic):
//! hinted metrics at the face's point size and an SDF over the atlas padding, packed into the
//! atlas with MaxRects BestShortSideFit (`FontEngine.TryAddGlyphToTexture`).
//!
//! FontEngine is native (FreeType + its own distance transform); this computes exact distances
//! to the flattened outline instead, calibrated against the static atlases the same engine baked
//! (`examples/sdf_calibrate.rs`).
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};

/// A rendered glyph: GlyphMetrics in pixels at the point size, and its distance field
/// (`(width + 2 padding) x (height + 2 padding)`, bottom row first).
#[derive(Clone, Debug, Default)]
pub struct Rendered {
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
    pub sdf: Vec<u8>,
}

/// FontEngine.GetGlyphIndex (0: missing)
pub fn glyph_index(ttf: &[u8], unicode: u32) -> u32 {
    let Ok(f) = FontRef::new(ttf) else { return 0 };
    f.charmap().map(unicode).map_or(0, |g| g.to_u32())
}

/// Collects the outline as line segments (curves flattened).
#[derive(Default)]
struct Segs {
    segs: Vec<(Pt, Pt)>,
    start: Pt,
    cur: Pt,
}

type Pt = [f32; 2];

impl OutlinePen for Segs {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = [x, y];
        self.cur = [x, y];
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.segs.push((self.cur, [x, y]));
        self.cur = [x, y];
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let a = self.cur;
        for k in 1..=16 {
            let t = k as f32 / 16.0;
            let u = 1.0 - t;
            self.line_to(u * u * a[0] + 2.0 * u * t * cx + t * t * x, u * u * a[1] + 2.0 * u * t * cy + t * t * y);
        }
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let a = self.cur;
        for k in 1..=16 {
            let t = k as f32 / 16.0;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line_to(w0 * a[0] + w1 * c0x + w2 * c1x + w3 * x, w0 * a[1] + w1 * c0y + w2 * c1y + w3 * y);
        }
    }
    fn close(&mut self) {
        if self.cur != self.start {
            self.segs.push((self.cur, self.start));
        }
        self.cur = self.start;
    }
}

/// Signed distance (positive inside, non-zero winding) from `q` to the outline.
fn signed_distance(segs: &[(Pt, Pt)], q: Pt) -> f32 {
    let mut best = f32::MAX;
    let mut winding = 0;
    for &(a, b) in segs {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let (ex, ey) = (a[0] + t * dx - q[0], a[1] + t * dy - q[1]);
        best = best.min(ex * ex + ey * ey);
        let cross = dx * (q[1] - a[1]) - (q[0] - a[0]) * dy;
        if a[1] <= q[1] {
            if b[1] > q[1] && cross > 0.0 {
                winding += 1;
            }
        } else if b[1] <= q[1] && cross < 0.0 {
            winding -= 1;
        }
    }
    let d = best.sqrt();
    if winding != 0 {
        d
    } else {
        -d
    }
}

/// FontEngine.LoadGlyph + render at `point_size` pixels per em with GlyphRenderMode SDFAA_HINTED
/// (4169: the TrueType bytecode hints, FreeType's full-hinting interpreter; `hinted` false for
/// the unhinted modes).
pub fn render(ttf: &[u8], index: u32, point_size: f32, padding: u32, hinted: bool) -> Option<Rendered> {
    let f = FontRef::new(ttf).ok()?;
    let outlines = f.outline_glyphs();
    let size = Size::new(point_size);
    let glyph = outlines.get(GlyphId::new(index))?;
    let mut pen = Segs::default();
    let adjusted = if hinted {
        let inst = HintingInstance::new(&outlines, size, LocationRef::default(), HintingOptions { engine: Engine::Interpreter, target: Target::Mono }).ok()?;
        glyph.draw(DrawSettings::hinted(&inst, false), &mut pen).ok()?
    } else {
        glyph.draw(DrawSettings::unhinted(size, LocationRef::default()), &mut pen).ok()?
    };
    let linear = f.glyph_metrics(size, LocationRef::default()).advance_width(GlyphId::new(index)).unwrap_or(0.0);
    let advance = adjusted.advance_width.unwrap_or(linear.round());
    let segs = pen.segs;
    if segs.is_empty() {
        return Some(Rendered { advance, ..Default::default() });
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (a, b) in &segs {
        for q in [a, b] {
            x0 = x0.min(q[0]);
            y0 = y0.min(q[1]);
            x1 = x1.max(q[0]);
            y1 = y1.max(q[1]);
        }
    }
    // FT_Outline_Get_CBox grid-fitted
    let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let p = padding as f32;
    let spread = 2.0 * (p + 1.0);
    let (tw, th) = (w + 2 * padding, h + 2 * padding);
    let mut sdf = vec![0u8; (tw * th) as usize];
    for j in 0..th {
        for i in 0..tw {
            let q = [x0 - p + i as f32 + 0.5, y0 - p + j as f32 + 0.5];
            let d = signed_distance(&segs, q);
            sdf[(j * tw + i) as usize] = ((0.5 + d / spread).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    Some(Rendered { width: w as f32, height: h as f32, bearing_x: x0, bearing_y: y1, advance, sdf })
}

/// GlyphRect (x, y, width, height)
pub type Rect = [i32; 4];

/// MaxRects BestShortSideFit over FontEngine's free / used rect lists: the rect a
/// `(w + padding) x (h + padding)` glyph takes, or None when the atlas is full.
pub fn pack(free: &mut Vec<Rect>, used: &mut Vec<Rect>, w: i32, h: i32) -> Option<Rect> {
    let mut best: Option<(i32, i32, Rect)> = None;
    for r in free.iter() {
        if r[2] >= w && r[3] >= h {
            let (lx, ly) = (r[2] - w, r[3] - h);
            let (short, long) = (lx.min(ly), lx.max(ly));
            if best.is_none_or(|(s, l, _)| short < s || (short == s && long < l)) {
                best = Some((short, long, [r[0], r[1], w, h]));
            }
        }
    }
    let (_, _, n) = best?;
    // split every free rect the new one overlaps
    let mut out = Vec::new();
    for r in free.drain(..) {
        if n[0] >= r[0] + r[2] || n[0] + n[2] <= r[0] || n[1] >= r[1] + r[3] || n[1] + n[3] <= r[1] {
            out.push(r);
            continue;
        }
        if n[0] > r[0] {
            out.push([r[0], r[1], n[0] - r[0], r[3]]);
        }
        if n[0] + n[2] < r[0] + r[2] {
            out.push([n[0] + n[2], r[1], r[0] + r[2] - (n[0] + n[2]), r[3]]);
        }
        if n[1] > r[1] {
            out.push([r[0], r[1], r[2], n[1] - r[1]]);
        }
        if n[1] + n[3] < r[1] + r[3] {
            out.push([r[0], n[1] + n[3], r[2], r[1] + r[3] - (n[1] + n[3])]);
        }
    }
    // prune contained rects
    let contains = |a: &Rect, b: &Rect| b[0] >= a[0] && b[1] >= a[1] && b[0] + b[2] <= a[0] + a[2] && b[1] + b[3] <= a[1] + a[3];
    let mut keep = vec![true; out.len()];
    for i in 0..out.len() {
        for j in 0..out.len() {
            if i != j && keep[j] && contains(&out[j], &out[i]) && (out[i] != out[j] || i > j) {
                keep[i] = false;
                break;
            }
        }
    }
    *free = out.into_iter().zip(keep).filter_map(|(r, k)| k.then_some(r)).collect();
    used.push(n);
    Some(n)
}
