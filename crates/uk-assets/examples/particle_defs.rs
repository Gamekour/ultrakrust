//! The effect prefabs a scene loads (SceneDef::particle_prefabs) as parsed: per node its
//! transform, scripts, and ParticleSystem / renderer values the simulation uses.
//! cargo run --release -p uk-assets --example particle_defs -- [level0-1] [prefab name filter]
use uk_assets::particles::{MinMaxCurve, MinMaxGradient};
use uk_assets::{db::AssetDb, scenedef};

fn c(m: &MinMaxCurve) -> String {
    match m.mode {
        0 => format!("{}", m.scalar),
        3 => format!("{}..{}", m.min_scalar, m.scalar),
        1 => format!("curve x{} {:?}", m.scalar, m.max.0.iter().map(|k| (k.time, k.value)).collect::<Vec<_>>()),
        _ => format!("curves x{} {:?} / {:?}", m.scalar, m.min.0.iter().map(|k| (k.time, k.value)).collect::<Vec<_>>(), m.max.0.iter().map(|k| (k.time, k.value)).collect::<Vec<_>>()),
    }
}

fn g(m: &MinMaxGradient) -> String {
    match m.mode {
        0 => format!("{:?}", m.max_color),
        2 => format!("{:?}..{:?}", m.min_color, m.max_color),
        _ => format!("mode {} colors {:?} alphas {:?}", m.mode, m.max.colors, m.max.alphas),
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let only = std::env::args().nth(2);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let mut refs: Vec<_> = def.script_prefabs.iter().collect();
    refs.sort();
    for ((s, f), i) in refs {
        println!("{}.{f} (script {s}) -> prefab {i}", def.scripts[*s as usize].class);
    }
    for (i, p) in def.particle_prefabs.iter().enumerate() {
        if only.as_ref().is_some_and(|o| !p.name.contains(o.as_str())) {
            continue;
        }
        println!("=== prefab {i} {}", p.name);
        for (ni, n) in p.nodes.iter().enumerate() {
            println!("  [{ni}] {} parent {:?} active {} layer {} pos {:?} rot {:?} scale {:?}", n.name, n.parent, n.active_self, n.layer, n.local_pos, n.local_rot, n.local_scale);
            for s in &n.scripts {
                println!("      script {}", s.class);
            }
            if let Some(sp) = n.sphere {
                println!("      sphere center {:?} radius {} trigger {} enabled {}", sp.0, sp.1, sp.2, sp.3);
            }
            if let Some(s) = &n.system {
                println!("      system duration {} looping {} playOnAwake {} space {} scaling {} culling {} stopAction {} maxParticles {}", s.duration, s.looping, s.play_on_awake, s.simulation_space, s.scaling_mode, s.culling_mode, s.stop_action, s.max_particles);
                println!("        lifetime {} speed {} size {} rotation {} gravity {} delay {}", c(&s.start_lifetime), c(&s.start_speed), c(&s.start_size), c(&s.start_rotation), c(&s.gravity_modifier), c(&s.start_delay));
                println!("        color {}", g(&s.start_color));
                if let Some(sh) = &s.shape {
                    println!("        shape kind {} radius {} thickness {} angle {} arc {} (mode {}) length {} pos {:?} rot {:?} scale {:?} randDir {} sphDir {} randPos {} align {}", sh.kind, sh.radius, sh.radius_thickness, sh.angle, sh.arc, sh.arc_mode, sh.length, sh.position, sh.rotation, sh.scale, sh.random_direction, sh.spherical_direction, sh.random_position, sh.align_to_direction);
                }
                println!("        emission {} rate {} perDistance {} bursts {:?}", s.emission_enabled, c(&s.rate_over_time), c(&s.rate_over_distance), s.bursts.iter().map(|b| format!("t{} n{} cycles {} every {} p{}", b.time, c(&b.count), b.cycles, b.interval, b.probability)).collect::<Vec<_>>());
                if let Some(m) = &s.color_over_lifetime {
                    println!("        colorOverLifetime {}", g(m));
                }
                if let Some(m) = &s.size_over_lifetime {
                    println!("        sizeOverLifetime {}", c(m));
                }
                if let Some(m) = &s.rotation_over_lifetime {
                    println!("        rotationOverLifetime {}", c(m));
                }
                if let Some(v) = &s.velocity_over_lifetime {
                    println!("        velocityOverLifetime x {} y {} z {} radial {} speedModifier {} world {}", c(&v.x), c(&v.y), c(&v.z), c(&v.radial), c(&v.speed_modifier), v.world_space);
                }
                if let Some(t) = &s.trail {
                    println!("        trail mode {} ratio {} lifetime {} minVertexDistance {} texMode {} world {} dieWithParticles {} sizeAffectsWidth {} sizeAffectsLifetime {} inheritColor {}", t.mode, t.ratio, c(&t.lifetime), t.min_vertex_distance, t.texture_mode, t.world_space, t.die_with_particles, t.size_affects_width, t.size_affects_lifetime, t.inherit_particle_color);
                    println!("          colorOverLifetime {} widthOverTrail {} colorOverTrail {}", g(&t.color_over_lifetime), c(&t.width_over_trail), g(&t.color_over_trail));
                }
                if !s.unsupported.is_empty() {
                    println!("        UNSUPPORTED {:?}", s.unsupported);
                }
            }
            if let Some(r) = &n.renderer {
                println!("      renderer enabled {} mode {} sort {} size {}..{} align {} pivot {:?} streams {:?} mats {:?}", r.enabled, r.render_mode, r.sort_mode, r.min_particle_size, r.max_particle_size, r.render_alignment, r.pivot, r.vertex_streams, r.materials.iter().map(|m| m.as_ref().map(|m| m.path_id)).collect::<Vec<_>>());
            }
        }
    }
    for w in &def.warnings {
        if w.contains("prefab") || w.contains("Bloodsplatter") || w.contains("MaliciousFace") {
            println!("warning {w}");
        }
    }
}
