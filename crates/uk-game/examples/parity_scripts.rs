//! Parity probe: load every scene bundle in the install, list the MonoBehaviour classes each uses,
//! and report which are unported. Writes `parity/scripts.tsv` (class, scenes, instances, status).
//! Usage: parity_scripts [filter]   e.g. `parity_scripts level0-` for layer 0 only.
use std::collections::{BTreeMap, BTreeSet};
use uk_assets::{db::AssetDb, scenedef};
use uk_game::parity::{status, Port};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| { let n = p.file_name().unwrap().to_string_lossy(); n.contains("_scenes_") && n.contains(&*filter) })
        .collect();
    bundles.sort();
    // class -> (scenes, instances)
    let mut uses: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
    let mut per_scene = Vec::new();
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "").replace("specialscenes_scenes_", "");
        let name = name.split(".bundle").next().unwrap().split('_').next().unwrap().to_string();
        let t = std::time::Instant::now();
        let def = match scenedef::load_scene(&mut db, b) { Ok(d) => d, Err(e) => { println!("{name:24} LOAD FAILED: {e}"); continue } };
        let (mut inst, mut covered) = (0usize, 0usize);
        let mut classes = BTreeSet::new();
        for s in &def.scripts {
            if s.class.is_empty() { continue }
            inst += 1;
            if status(&s.class).is_some() { covered += 1 }
            classes.insert(s.class.clone());
            let e = uses.entry(s.class.clone()).or_default();
            e.0.insert(name.clone()); e.1 += 1;
        }
        let cov_classes = classes.iter().filter(|c| status(c).is_some()).count();
        println!("{name:24} {:>6} scripts, {:>4} classes | ported: {:>5.1}% instances, {:>3}/{} classes  ({:.1}s)",
            inst, classes.len(), 100.0 * covered as f64 / inst.max(1) as f64, cov_classes, classes.len(), t.elapsed().as_secs_f32());
        per_scene.push((name, inst, covered, classes.len(), cov_classes));
    }
    std::fs::create_dir_all("parity").unwrap();
    let mut out = String::from("class\tscenes\tinstances\tstatus\n");
    let mut rows: Vec<_> = uses.iter().collect();
    rows.sort_by_key(|(_, (s, n))| (std::cmp::Reverse(s.len()), std::cmp::Reverse(*n)));
    for (c, (s, n)) in &rows {
        let st = match status(c) { Some(Port::Full) => "full".into(), Some(Port::Partial(w)) => format!("partial: {w}"), Some(Port::DataOnly) => "data-only".into(), None => "MISSING".into() };
        out += &format!("{c}\t{}\t{n}\t{st}\n", s.len());
    }
    std::fs::write("parity/scripts.tsv", &out).unwrap();
    let mut sc = String::from("scene\tinstances\tported_instances\tclasses\tported_classes\n");
    for (n, i, c, k, kc) in &per_scene { sc += &format!("{n}\t{i}\t{c}\t{k}\t{kc}\n") }
    std::fs::write("parity/scenes.tsv", &sc).unwrap();
    let total: usize = rows.iter().map(|(_, (_, n))| n).sum();
    let ported: usize = rows.iter().filter(|(c, _)| status(c).is_some()).map(|(_, (_, n))| n).sum();
    let missing = rows.iter().filter(|(c, _)| status(c).is_none()).count();
    println!("\nTOTAL: {} distinct classes across {} scenes; {missing} unported; {:.1}% of script instances have a port",
        rows.len(), per_scene.len(), 100.0 * ported as f64 / total.max(1) as f64);
    println!("Top unported by scene reach:");
    for (c, (s, n)) in rows.iter().filter(|(c, _)| status(c).is_none()).take(40) { println!("  {c:36} {:>3} scenes {n:>7} instances", s.len()) }
}
