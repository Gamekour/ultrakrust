//! Numeric summary of `ui::load_ui_assets` for a level.
//! cargo run --release -p uk-assets --example ui_assets -- level0-1
use uk_assets::{db::AssetDb, scenedef, ui};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let t = std::time::Instant::now();
    let a = ui::load_ui_assets(&mut db, &def);
    println!("loaded in {:?}", t.elapsed());
    println!("textures {} sprites {} fonts {} materials {} legacy_fonts {} refs {}", a.textures.len(), a.sprites.len(), a.fonts.len(), a.materials.len(), a.legacy_fonts.len(), a.refs.len());
    for w in &a.warnings { println!("WARN {w}"); }
    if let Some(d) = a.default_material {
        let m = &a.materials[d as usize];
        let mut f: Vec<_> = m.props.floats.iter().collect(); f.sort_by(|a, b| a.0.cmp(b.0));
        let mut c: Vec<_> = m.props.colors.iter().collect(); c.sort_by(|a, b| a.0.cmp(b.0));
        println!("default material {:?} floats {:?} colors {:?} textures {:?}", m.shader, f, c, m.props.textures.keys().collect::<Vec<_>>());
    }
    for f in &a.fonts {
        let missing = f.characters.values().filter(|c| !f.glyphs.contains_key(&c.glyph)).count();
        println!("font {} dyn={} pt={} scale={} lh={} asc={} desc={} glyphs={} chars={} kern={} fallbacks={:?} mat={:?} atlas={:?} {}x{} pad={} rm={} bold={}/{} italic={} weights={}",
            f.name, f.dynamic, f.face.point_size, f.face.scale, f.face.line_height, f.face.ascent_line, f.face.descent_line, f.glyphs.len(), f.characters.len(), f.kerning.len(),
            f.fallbacks, f.material.map(|m| &a.materials[m as usize].name), f.atlas_textures, f.atlas_width, f.atlas_height, f.atlas_padding, f.atlas_render_mode,
            f.bold_style, f.bold_spacing, f.italic_style, f.weights.iter().filter(|w| w.0.is_some() || w.1.is_some()).count());
        if missing > 0 { println!("  chars without glyph: {missing}"); }
    }
    for m in &a.materials { println!("material {} shader {:?} kw {:?} tex {:?}", m.name, m.shader, m.props.keywords, m.props.textures.iter().map(|(k, t)| (k, t.texture)).collect::<Vec<_>>()); }
    let mut sliced = 0;
    for s in &a.sprites { if s.border.iter().any(|b| *b > 0.0) { sliced += 1; } if s.padding.iter().any(|p| *p != 0.0) { println!("sprite {} padded {:?}", s.name, s.padding); } }
    println!("sprites with border: {sliced}");
    for s in a.sprites.iter().take(8) { println!("sprite {} tex {:?} rect {:?} border {:?} ppu {} outer {:?} inner {:?}", s.name, s.texture, s.rect, s.border, s.pixels_per_unit, s.outer_uv, s.inner_uv); }
    let mut kinds = std::collections::BTreeMap::new();
    for ((si, path), r) in &a.refs { let k = format!("{}.{} {:?}", def.scripts[*si as usize].class, path.split('/').next().unwrap(), std::mem::discriminant(r)); *kinds.entry(k).or_insert(0) += 1; }
    for (k, n) in kinds { println!("ref {n:5} {k}"); }
}
