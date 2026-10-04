//! ULTRAKRUST harness: every headless probe as a check, plus coverage and performance metrics,
//! compared against `parity/baseline.tsv`. Exits non-zero on any regression.
//!
//!   cargo run --release -p uk-harness              # quick: level 0-1 checks + perf
//!   cargo run --release -p uk-harness -- --full    # + every scene: load, script & native coverage
//!   cargo run --release -p uk-harness -- --bless   # accept current results as the new baseline
//!
//! Output: `parity/report.tsv` (all metrics) and a printed list of failures/regressions only.
mod checks;
mod coverage;

use std::collections::BTreeMap;

/// Which way is better for a metric, and how much drift is tolerated before it counts as a regression.
#[derive(Clone, Copy)]
pub enum Dir {
    Higher,
    /// Lower is better; tolerance is relative (0.25 = 25% slower allowed — timing noise).
    Lower(f64),
}

#[derive(Default)]
pub struct Report {
    pub metrics: BTreeMap<String, (f64, Option<Dir>)>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn pass(&mut self, id: &str, ok: bool, why: impl Into<String>) {
        self.metrics.insert(format!("pass.{id}"), (ok as i32 as f64, Some(Dir::Higher)));
        if !ok { self.notes.push(format!("FAIL {id}: {}", why.into())) }
    }
    pub fn higher(&mut self, k: &str, v: f64) { self.metrics.insert(k.into(), (v, Some(Dir::Higher))); }
    pub fn lower(&mut self, k: &str, v: f64, tol: f64) { self.metrics.insert(k.into(), (v, Some(Dir::Lower(tol)))); }
    pub fn info(&mut self, k: &str, v: f64) { self.metrics.insert(k.into(), (v, None)); }
    pub fn note(&mut self, s: impl Into<String>) { self.notes.push(s.into()) }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let full = args.iter().any(|a| a == "--full");
    let bless = args.iter().any(|a| a == "--bless");
    let only: Option<String> = args.iter().position(|a| a == "--only").and_then(|i| args.get(i + 1).cloned());
    let t0 = std::time::Instant::now();
    let mut r = Report::default();
    let install = uk_assets::find_install().expect("ULTRAKILL install not found (set ULTRAKILL_DIR)");

    let run = |name: &str| only.as_deref().is_none_or(|o| name.contains(o) || o.contains(name));
    if run("level0-1") { checks::level_0_1(&install, &mut r) }
    if full && run("coverage") { coverage::all_scenes(&install, &mut r) }

    std::fs::create_dir_all("parity").unwrap();
    let mut out = String::from("metric\tvalue\n");
    for (k, (v, _)) in &r.metrics { out += &format!("{k}\t{v}\n") }
    std::fs::write("parity/report.tsv", &out).unwrap();

    // compare with baseline
    let base_path = "parity/baseline.tsv";
    let base: BTreeMap<String, f64> = std::fs::read_to_string(base_path).unwrap_or_default().lines().skip(1)
        .filter_map(|l| { let (k, v) = l.split_once('\t')?; Some((k.to_string(), v.parse().ok()?)) }).collect();
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();
    for (k, (v, dir)) in &r.metrics {
        let (Some(b), Some(dir)) = (base.get(k), dir) else { continue };
        match dir {
            Dir::Higher if v + 1e-9 < *b => regressions.push(format!("{k}: {b} -> {v}")),
            Dir::Higher if *v > b + 1e-9 => improvements.push(format!("{k}: {b} -> {v}")),
            Dir::Lower(tol) if *v > b * (1.0 + tol) + 1e-6 => regressions.push(format!("{k}: {b:.3} -> {v:.3} (> {:.0}% worse)", tol * 100.0)),
            Dir::Lower(tol) if *v < b * (1.0 - tol) => improvements.push(format!("{k}: {b:.3} -> {v:.3}")),
            _ => {}
        }
    }
    // metrics that vanished from a run that covered them
    for k in base.keys() {
        if !r.metrics.contains_key(k) && (full || !k.starts_with("cov.") && !k.starts_with("load.")) && only.is_none() {
            regressions.push(format!("{k}: missing from this run"));
        }
    }
    println!("\n== {} metrics in {:.1}s ==", r.metrics.len(), t0.elapsed().as_secs_f32());
    for n in &r.notes { println!("{n}") }
    for i in &improvements { println!("improved  {i}") }
    for g in &regressions { println!("REGRESSED {g}") }
    let fails = r.metrics.iter().filter(|(k, (v, _))| k.starts_with("pass.") && *v < 0.5).count();
    println!("checks failing: {fails}, regressions: {}", regressions.len());
    if bless {
        let mut merged = base.clone();
        for (k, (v, d)) in &r.metrics { if d.is_some() { merged.insert(k.clone(), *v); } }
        let mut s = String::from("metric\tvalue\n");
        for (k, v) in &merged { s += &format!("{k}\t{v}\n") }
        std::fs::write(base_path, s).unwrap();
        println!("baseline updated ({} metrics)", merged.len());
    } else if !regressions.is_empty() {
        std::process::exit(1);
    }
}
