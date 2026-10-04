//! Census of Unity native object types across every bundle in the install (scenes + asset bundles).
//! Writes `parity/native.tsv`: class, class id, count, bundles. This is the denominator for
//! engine-feature parity (animation, audio, particles, lights, UI, navmesh...).
use std::collections::BTreeMap;
use uk_assets::db::AssetDb;

fn name(id: i32) -> &'static str {
    match id {
        1 => "GameObject", 4 => "Transform", 20 => "Camera", 21 => "Material", 23 => "MeshRenderer", 28 => "Texture2D",
        33 => "MeshFilter", 43 => "Mesh", 48 => "Shader", 49 => "TextAsset", 54 => "Rigidbody", 64 => "MeshCollider",
        65 => "BoxCollider", 74 => "AnimationClip", 81 => "AudioListener", 82 => "AudioSource", 83 => "AudioClip",
        84 => "RenderTexture", 89 => "Cubemap", 90 => "Avatar", 91 => "AnimatorController", 95 => "Animator",
        96 => "TrailRenderer", 104 => "RenderSettings", 108 => "Light", 111 => "Animation", 114 => "MonoBehaviour",
        115 => "MonoScript", 117 => "Texture3D", 119 => "Projector", 120 => "LineRenderer", 135 => "SphereCollider",
        136 => "CapsuleCollider", 137 => "SkinnedMeshRenderer", 142 => "AssetBundle", 143 => "CharacterController",
        153 => "ConfigurableJoint", 157 => "LightmapSettings", 164 => "AudioReverbFilter", 165 => "AudioHighPassFilter",
        166 => "AudioChorusFilter", 167 => "AudioReverbZone", 168 => "AudioEchoFilter", 169 => "AudioLowPassFilter",
        170 => "AudioDistortionFilter", 180 => "OcclusionPortal", 182 => "WindZone", 187 => "AudioMixerGroupController"/*approx*/,
        192 => "OcclusionArea", 195 => "NavMeshAgent", 196 => "NavMeshSettings", 198 => "ParticleSystem",
        199 => "ParticleSystemRenderer", 205 => "LODGroup", 208 => "NavMeshObstacle", 212 => "SpriteRenderer",
        213 => "Sprite", 215 => "ReflectionProbe", 218 => "Terrain", 220 => "LightProbeGroup", 221 => "AnimatorOverrideController",
        222 => "CanvasRenderer", 223 => "Canvas", 224 => "RectTransform", 225 => "CanvasGroup", 226 => "BillboardAsset",
        228 => "SpringJoint", 238 => "NavMeshData", 240 => "AudioMixer", 241 => "AudioMixerController", 243 => "AudioMixerGroupController",
        244 => "AudioMixerEffectController", 245 => "AudioMixerSnapshotController", 258 => "LightProbes", 290 => "AssetBundleManifest",
        319 => "AvatarMask", 328 => "VideoPlayer", 329 => "VideoClip", 331 => "SpriteMask", 1953259897 => "TerrainData",
        _ => "",
    }
}

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p) } else if p.extension().is_some_and(|x| x == "bundle") { bundles.push(p) }
        }
    }
    bundles.sort();
    let mut counts: BTreeMap<i32, (usize, usize)> = BTreeMap::new();
    for b in &bundles {
        let Ok(files) = db.load_bundle_files(b) else { println!("skip {}", b.display()); continue };
        let mut seen = std::collections::HashSet::new();
        for f in files {
            for o in &f.objects {
                let e = counts.entry(o.class_id).or_default();
                e.0 += 1;
                if seen.insert(o.class_id) { e.1 += 1 }
            }
        }
    }
    std::fs::create_dir_all("parity").unwrap();
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    let mut out = String::from("class\tid\tcount\tbundles\n");
    for (id, (n, b)) in &rows {
        let nm = name(*id);
        out += &format!("{}\t{id}\t{n}\t{b}\n", if nm.is_empty() { "?" } else { nm });
        println!("{:28} {id:>6} {n:>9} objs in {b:>3} bundles", if nm.is_empty() { "?" } else { nm });
    }
    std::fs::write("parity/native.tsv", out).unwrap();
    println!("{} bundles scanned", bundles.len());
}
