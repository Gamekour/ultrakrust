//! Loads a campaign level from the user's ULTRAKILL install and turns it into
//! Bevy meshes/materials plus a `uk_core` collision world.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;
use uk_assets::db::AssetDb;
use uk_assets::scene::{self, LevelOptions};
use uk_assets::texture;
use uk_core::collide::{BoxCollider, World};

pub struct Loaded {
    pub world: World,
    pub spawn: Vec3,
    pub yaw: f32,
    pub summary: String,
    pub rooms: Vec<scene::Room>,
}

#[derive(Component)]
pub struct LevelMesh;

pub fn load(
    level: &str,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<Loaded, String> {
    let t0 = std::time::Instant::now();
    let install = uk_assets::find_install().ok_or("ULTRAKILL install not found (set ULTRAKILL_DIR)")?;
    let mut db = AssetDb::open(&install).map_err(|e| e.to_string())?;
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{level}.bundle"));
    let lvl = scene::load_level(&mut db, &path, LevelOptions::default()).map_err(|e| e.to_string())?;

    let mut tex_cache: HashMap<(String, i64), Option<(Handle<Image>, bool)>> = HashMap::new();
    let fallback = materials.add(StandardMaterial { base_color: Color::srgb(0.5, 0.5, 0.5), perceptual_roughness: 1.0, ..default() });
    let (mut textured, mut tris) = (0usize, 0usize);
    for (key, batch) in &lvl.batches {
        if batch.indices.is_empty() {
            continue;
        }
        let mut mat_handle = fallback.clone();
        if let Some(key) = key {
            if let Some(m) = db
                .file(&key.file)
                .ok()
                .and_then(|f| f.read_id(key.path_id).ok().map(|v| (f, v)))
                .and_then(|(f, v)| texture::decode_material(&mut db, &f, &v).ok())
            {
                let tex = m.main_tex.as_ref().and_then(|(f, id)| {
                    tex_cache
                        .entry((f.name.clone(), *id))
                        .or_insert_with(|| {
                            let v = f.read_id(*id).ok()?;
                            let t = texture::decode_texture(&mut db, &v).ok()?;
                            let has_alpha = t.rgba.chunks_exact(4).any(|p| p[3] < 128);
                            Some((images.add(to_image(&t)), has_alpha))
                        })
                        .clone()
                });
                let c = m.color;
                let alpha_mode = match &tex {
                    _ if m.transparent => AlphaMode::Blend,
                    Some((_, true)) => AlphaMode::Mask(0.5),
                    _ => AlphaMode::Opaque,
                };
                if tex.is_some() {
                    textured += 1;
                }
                mat_handle = materials.add(StandardMaterial {
                    base_color: Color::srgba(c[0], c[1], c[2], c[3]),
                    base_color_texture: tex.map(|t| t.0),
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::from(m.tex_scale),
                        0.0,
                        Vec2::from(m.tex_offset),
                    ),
                    alpha_mode,
                    perceptual_roughness: 1.0,
                    reflectance: 0.1,
                    ..default()
                });
            }
        }
        tris += batch.indices.len() / 3;
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, batch.positions.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, batch.normals.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs.clone())
            .with_inserted_indices(Indices::U32(batch.indices.clone()));
        commands.spawn((LevelMesh, Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat_handle)));
    }

    let mut world = World::default();
    for t in &lvl.collision_tris {
        world.add_triangle(t[0], t[1], t[2]);
    }
    for b in &lvl.boxes {
        world.add(BoxCollider { center: b.center, half: b.half, rot: b.rot, slippery: false });
    }
    world.build();

    let (spawn, yaw) = lvl.spawn.unwrap_or((Vec3::new(0.0, 10.0, 0.0), 0.0));
    let s = &lvl.stats;
    let summary = format!(
        "level {level}: {} renderers, {} batches ({} textured), {} tris; collision {} tris + {} boxes; loaded in {:.1}s",
        s.active_renderers,
        lvl.batches.len(),
        textured,
        tris,
        lvl.collision_tris.len(),
        lvl.boxes.len(),
        t0.elapsed().as_secs_f32()
    );
    Ok(Loaded { world, spawn, yaw, summary, rooms: lvl.rooms })
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
