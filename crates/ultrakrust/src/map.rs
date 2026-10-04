//! The movement test map: every piece is an oriented box that becomes both a
//! collider in `uk_core::collide::World` and a mesh with world-space UVs.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use uk_core::collide::{BoxCollider, ColliderId, World};

#[derive(Clone, Copy)]
pub enum Surface {
    Floor,
    Wall,
    Accent,
}

pub struct Piece {
    pub collider: BoxCollider,
    pub surface: Surface,
}

pub struct Target {
    pub collider: ColliderId,
    pub entity: Entity,
    pub hits: u32,
    pub flash: f32,
}

fn piece(center: [f32; 3], size: [f32; 3], surface: Surface) -> Piece {
    Piece { collider: BoxCollider::new(Vec3::from(center), Vec3::from(size)), surface }
}

fn ramp(center: [f32; 3], size: [f32; 3], pitch_deg: f32, yaw_deg: f32) -> Piece {
    let rot = Quat::from_rotation_y(yaw_deg.to_radians()) * Quat::from_rotation_x(pitch_deg.to_radians());
    Piece { collider: BoxCollider::new(Vec3::from(center), Vec3::from(size)).rotated(rot), surface: Surface::Accent }
}

/// Layout in ULTRAKILL units (V1 is 3.5 tall).
pub fn layout() -> (Vec<Piece>, Vec<[f32; 3]>) {
    use Surface::*;
    let mut p = vec![
        // ground + arena walls
        piece([0.0, -0.5, 0.0], [320.0, 1.0, 320.0], Floor),
        piece([0.0, 20.0, -160.0], [320.0, 40.0, 2.0], Wall),
        piece([0.0, 20.0, 160.0], [320.0, 40.0, 2.0], Wall),
        piece([-160.0, 20.0, 0.0], [2.0, 40.0, 320.0], Wall),
        piece([160.0, 20.0, 0.0], [2.0, 40.0, 320.0], Wall),
        // wall-jump shaft: two parallel walls 7 apart
        piece([-40.0, 25.0, -60.0], [2.0, 50.0, 24.0], Wall),
        piece([-31.0, 25.0, -60.0], [2.0, 50.0, 24.0], Wall),
        // tower with a long ramp up to it (slam / fall-speed slides)
        piece([60.0, 15.0, -90.0], [24.0, 30.0, 24.0], Wall),
        // slide tunnel: 2.0 clearance, V1 only fits while sliding (h 1.25)
        piece([0.0, 3.0, 40.0], [10.0, 2.0, 40.0], Wall),
        piece([-6.0, 1.0, 40.0], [2.0, 2.0, 40.0], Wall),
        piece([6.0, 1.0, 40.0], [2.0, 2.0, 40.0], Wall),
        // pillars and platforms
        piece([30.0, 3.0, 10.0], [6.0, 6.0, 6.0], Accent),
        piece([40.0, 6.0, 20.0], [6.0, 12.0, 6.0], Accent),
        piece([50.0, 9.0, 30.0], [6.0, 18.0, 6.0], Accent),
        piece([-60.0, 8.0, 40.0], [20.0, 1.0, 20.0], Accent),
        piece([-60.0, 16.0, 70.0], [20.0, 1.0, 20.0], Accent),
        piece([-90.0, 24.0, 90.0], [20.0, 1.0, 20.0], Accent),
        // steps
        piece([90.0, 0.5, 40.0], [10.0, 1.0, 4.0], Accent),
        piece([90.0, 1.0, 44.0], [10.0, 2.0, 4.0], Accent),
        piece([90.0, 1.5, 48.0], [10.0, 3.0, 4.0], Accent),
        piece([90.0, 2.0, 52.0], [10.0, 4.0, 4.0], Accent),
    ];
    // ramps: gentle, steep, and the long ramp that reaches the tower top (y = 30)
    p.push(ramp([0.0, 2.0, -40.0], [12.0, 1.0, 24.0], -15.0, 0.0));
    p.push(ramp([25.0, 4.0, -40.0], [12.0, 1.0, 24.0], -35.0, 0.0));
    let len = 70.0f32;
    let angle = (30.0f32 / len).asin();
    p.push(ramp([60.0, 15.0 - 0.4, -78.0 + len * angle.cos() * 0.5 + 0.0], [10.0, 1.0, len], angle.to_degrees(), 0.0));

    let targets = vec![
        [0.0, 4.0, -120.0],
        [-20.0, 6.0, -120.0],
        [20.0, 8.0, -120.0],
        [60.0, 34.0, -90.0],
        [-60.0, 11.0, 40.0],
        [-90.0, 27.0, 90.0],
    ];
    (p, targets)
}

pub fn checker(dark: [u8; 3], light: [u8; 3], cells: u32) -> Image {
    let size = 64u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let cell = size / cells;
    for y in 0..size {
        for x in 0..size {
            let edge = x % cell == 0 || y % cell == 0;
            let c = if ((x / cell) + (y / cell)) % 2 == 0 { dark } else { light };
            let c = if edge { c.map(|v| v.saturating_add(18)) } else { c };
            data.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut img = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::nearest()
    });
    img
}

/// Box mesh with UVs in world units (`tile` units per texture repeat), so
/// textures keep the same scale on every face of every box.
pub fn box_mesh(half: Vec3, tile: f32) -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    // (normal, u axis, v axis)
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let base = pos.len() as u32;
        let c = n * half;
        let hu = (u * half).length();
        let hv = (v * half).length();
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = c + u * hu * su + v * hv * sv;
            pos.push(p.to_array());
            nrm.push(n.to_array());
            uv.push([(hu * su) / tile, -(hv * sv) / tile]);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

pub struct Materials {
    pub floor: Handle<StandardMaterial>,
    pub wall: Handle<StandardMaterial>,
    pub accent: Handle<StandardMaterial>,
    pub target: Handle<StandardMaterial>,
}

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &Materials,
    world: &mut World,
) -> Vec<Target> {
    let (pieces, target_spots) = layout();
    for pc in pieces {
        let c = &pc.collider;
        let mat = match pc.surface {
            Surface::Floor => mats.floor.clone(),
            Surface::Wall => mats.wall.clone(),
            Surface::Accent => mats.accent.clone(),
        };
        commands.spawn((
            Mesh3d(meshes.add(box_mesh(c.half, 4.0))),
            MeshMaterial3d(mat),
            Transform::from_translation(c.center).with_rotation(c.rot),
        ));
        world.add(pc.collider);
    }
    let mut targets = Vec::new();
    for spot in target_spots {
        let center = Vec3::from(spot);
        let size = Vec3::new(2.0, 3.0, 2.0);
        let id = world.add(BoxCollider::new(center, size));
        let entity = commands
            .spawn((
                Mesh3d(meshes.add(box_mesh(size * 0.5, 1.0))),
                MeshMaterial3d(mats.target.clone()),
                Transform::from_translation(center),
            ))
            .id();
        targets.push(Target { collider: id, entity, hits: 0, flash: 0.0 });
    }
    targets
}
