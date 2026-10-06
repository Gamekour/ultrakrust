//! Loads a campaign level from the user's ULTRAKILL install into a `uk_game::Game`,
//! spawns one entity per renderer, and keeps visibility / moving objects in sync
//! with the game state every frame.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::ecs::query::QueryFilter;
use bevy::math::{Affine2, Affine3A};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;
use std::sync::Arc;
use uk_assets::db::AssetDb;
use uk_assets::scene::MaterialKey;
use uk_assets::scenedef;
use uk_assets::texture;
use uk_game::Game;

/// Render bookkeeping for a loaded level.
#[derive(Resource, Default)]
pub struct LevelView {
    /// node -> (entity, renderer enabled)
    pub node_entities: HashMap<u32, Vec<(Entity, bool)>>,
    /// mover index -> entities moved by it
    pub mover_entities: Vec<Vec<Entity>>,
    pub mover_last: Vec<Affine3A>,
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_mat: Handle<StandardMaterial>,
    pub friendly_mat: Handle<StandardMaterial>,
    pub projectiles: Vec<Entity>,
}

#[derive(Component)]
pub struct ProjectileVis;

pub struct Room {
    pub name: String,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct Loaded {
    pub game: Game,
    pub view: LevelView,
    pub summary: String,
    pub rooms: Vec<Room>,
    /// The level drawn with ULTRAKILL's own shaders (`--unity-shaders`).
    pub unity: Option<crate::unity_render::SceneData>,
}

pub fn load(
    level: &str,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    unity_shaders: Option<&mut Assets<bevy::shader::Shader>>,
) -> Result<Loaded, String> {
    let t0 = std::time::Instant::now();
    let install = uk_assets::find_install().ok_or("ULTRAKILL install not found (set ULTRAKILL_DIR)")?;
    let mut db = AssetDb::open(&install).map_err(|e| e.to_string())?;
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).map_err(|e| e.to_string())?);
    let game = Game::new(def.clone());

    let mut mat_cache: HashMap<MaterialKey, Handle<StandardMaterial>> = HashMap::new();
    let mut tex_cache: HashMap<(String, i64), Option<(Handle<Image>, bool)>> = HashMap::new();
    let fallback = materials.add(StandardMaterial { base_color: Color::srgb(0.5, 0.5, 0.5), perceptual_roughness: 1.0, ..default() });
    let mut view = LevelView { mover_entities: vec![Vec::new(); game.movers.len()], mover_last: vec![Affine3A::IDENTITY; game.movers.len()], ..default() };
    let mut rooms: HashMap<u32, (Vec3, Vec3)> = HashMap::new();
    let (mut tris, mut textured, mut ents) = (0usize, 0usize, 0usize);
    let player = game.player_node;
    let unity = unity_shaders.map(|shaders| {
        let t = std::time::Instant::now();
        let (scene, summary) = crate::unity_render::build(&mut db, &def, |n| def.nodes[n as usize].layer != scenedef::VIEWMODEL_LAYER && player.is_some_and(|p| def.is_descendant(n, p)), shaders, 1);
        info!("{summary} ({:.2}s)", t.elapsed().as_secs_f32());
        scene
    });
    for r in &def.renderers {
        if r.batch.indices.is_empty() || unity.is_some() {
            continue;
        }
        if game.player_node.is_some_and(|p| def.is_descendant(r.node, p)) {
            continue;
        }
        let mat = match &r.material {
            Some(key) => mat_cache
                .entry(key.clone())
                .or_insert_with(|| {
                    material(&mut db, key, materials, images, &mut tex_cache, &mut textured).unwrap_or_else(|| fallback.clone())
                })
                .clone(),
            None => fallback.clone(),
        };
        let b = &r.batch;
        tris += b.indices.len() / 3;
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, b.positions.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, b.normals.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, b.uvs.clone())
            .with_inserted_indices(Indices::U32(b.indices.clone()));
        let visible = game.active(r.node) && r.enabled;
        let e = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(mat),
                Transform::IDENTITY,
                if visible { Visibility::Inherited } else { Visibility::Hidden },
            ))
            .id();
        ents += 1;
        view.node_entities.entry(r.node).or_default().push((e, r.enabled));
        if let Some(m) = game.node_mover[r.node as usize] {
            view.mover_entities[m as usize].push(e);
        }
        // room bounds for the screenshot tour
        let mut root = r.node;
        while let Some(p) = def.nodes[root as usize].parent {
            root = p;
        }
        let rb = rooms.entry(root).or_insert((Vec3::MAX, Vec3::MIN));
        for p in &b.positions {
            rb.0 = rb.0.min(Vec3::from(*p));
            rb.1 = rb.1.max(Vec3::from(*p));
        }
    }
    view.projectile_mesh = meshes.add(Sphere::new(0.5).mesh().ico(2).unwrap());
    view.projectile_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.6, 0.1),
        emissive: LinearRgba::rgb(12.0, 4.0, 0.5),
        unlit: true,
        ..default()
    });
    view.friendly_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.9, 1.0),
        emissive: LinearRgba::rgb(2.0, 8.0, 12.0),
        unlit: true,
        ..default()
    });
    let mut rooms: Vec<Room> = rooms
        .into_iter()
        .map(|(n, (min, max))| Room { name: def.nodes[n as usize].name.clone(), min, max })
        .collect();
    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    let summary = format!(
        "level {level}: {} entities ({} textured materials), {} tris; {} doors/enemies moving, {} triggers, {} enemies; loaded in {:.1}s",
        ents,
        textured,
        tris,
        game.movers.len(),
        game.triggers.len(),
        game.s.enemies.len(),
        t0.elapsed().as_secs_f32()
    );
    Ok(Loaded { game, view, summary, rooms, unity })
}

fn material(
    db: &mut AssetDb,
    key: &MaterialKey,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    tex_cache: &mut HashMap<(String, i64), Option<(Handle<Image>, bool)>>,
    textured: &mut usize,
) -> Option<Handle<StandardMaterial>> {
    let f = db.file(&key.file).ok()?;
    let v = f.read_id(key.path_id).ok()?;
    let m = texture::decode_material(db, &f, &v).ok()?;
    let tex = m.main_tex.as_ref().and_then(|(f, id)| {
        tex_cache
            .entry((f.name.clone(), *id))
            .or_insert_with(|| {
                let v = f.read_id(*id).ok()?;
                let t = texture::decode_texture(db, &v).ok()?;
                let has_alpha = t.rgba.chunks_exact(4).any(|p| p[3] < 128);
                Some((images.add(to_image(&t)), has_alpha))
            })
            .clone()
    });
    // Light shafts / additive effects glow instead of covering what's behind them.
    let additive = m.name.contains("LightPillar") || m.name.contains("Additive") || m.name.contains("Glow");
    let alpha_mode = match &tex {
        _ if additive => AlphaMode::Add,
        _ if m.transparent => AlphaMode::Blend,
        Some((_, true)) => AlphaMode::Mask(0.5),
        _ => AlphaMode::Opaque,
    };
    if tex.is_some() {
        *textured += 1;
    }
    let c = m.color;
    Some(materials.add(StandardMaterial {
        base_color: Color::srgba(c[0], c[1], c[2], c[3]),
        base_color_texture: tex.map(|t| t.0),
        uv_transform: Affine2::from_scale_angle_translation(Vec2::from(m.tex_scale), 0.0, Vec2::from(m.tex_offset)),
        alpha_mode,
        unlit: additive,
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        ..default()
    }))
}

fn to_image(t: &texture::TextureData) -> Image {
    let mut img = Image::new(
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        t.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let filter = if t.filter == 0 { ImageFilterMode::Nearest } else { ImageFilterMode::Linear };
    let wrap = if t.wrap == 1 { ImageAddressMode::ClampToEdge } else { ImageAddressMode::Repeat };
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: wrap,
        address_mode_v: wrap,
        mag_filter: filter,
        min_filter: filter,
        ..ImageSamplerDescriptor::nearest()
    });
    img
}

/// Applies game state to the render entities: visibility, moving objects, projectiles.
pub fn sync<F: QueryFilter>(
    game: &mut Game,
    view: &mut LevelView,
    commands: &mut Commands,
    vis: &mut Query<&mut Visibility>,
    tfs: &mut Query<&mut Transform, F>,
) {
    let (full, changed) = game.drain_changed();
    let mut apply = |node: u32, view: &LevelView| {
        if let Some(list) = view.node_entities.get(&node) {
            let on = game.active(node);
            for &(e, enabled) in list {
                if let Ok(mut v) = vis.get_mut(e) {
                    *v = if on && enabled { Visibility::Inherited } else { Visibility::Hidden };
                }
            }
        }
    };
    if full {
        let nodes: Vec<u32> = view.node_entities.keys().copied().collect();
        for n in nodes {
            apply(n, view);
        }
    } else {
        for n in changed {
            apply(n, view);
        }
    }
    for m in 0..game.movers.len() {
        let d = game.mover_delta(m as u32);
        if d == view.mover_last[m] && !full {
            continue;
        }
        view.mover_last[m] = d;
        let t = Transform::from_matrix(Mat4::from(d));
        for &e in &view.mover_entities[m] {
            if let Ok(mut tf) = tfs.get_mut(e) {
                *tf = t;
            }
        }
    }
    // projectiles: grow/shrink a pool of spheres
    while view.projectiles.len() < game.s.projectiles.len() {
        let e = commands.spawn((ProjectileVis, Mesh3d(view.projectile_mesh.clone()), MeshMaterial3d(view.projectile_mat.clone()), Transform::default())).id();
        view.projectiles.push(e);
    }
    for (i, &e) in view.projectiles.iter().enumerate() {
        match game.s.projectiles.get(i) {
            Some(p) => {
                if let Ok(mut tf) = tfs.get_mut(e) {
                    tf.translation = p.pos;
                }
                if let Ok(mut v) = vis.get_mut(e) {
                    *v = Visibility::Inherited;
                }
                if p.friendly {
                    commands.entity(e).insert(MeshMaterial3d(view.friendly_mat.clone()));
                }
            }
            None => {
                if let Ok(mut v) = vis.get_mut(e) {
                    *v = Visibility::Hidden;
                }
            }
        }
    }
}
