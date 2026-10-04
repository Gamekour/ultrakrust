//! `--render`: runs the real game binary on every campaign level with ULTRAKILL's own shaders and a
//! numeric frame read-back (no screenshots): renderer coverage, GPU validation errors, frame stats.
//! Needs a GPU and a desktop session; each level runs ~15 s.
use crate::Report;
use std::path::Path;
use std::process::Command;

fn grab<'a>(log: &'a str, key: &str) -> Option<&'a str> {
    log.lines().find(|l| l.contains(key))
}

pub fn levels(install: &Path, filter: Option<&str>, r: &mut Report) {
    let dir = uk_assets::db::AssetDb::bundle_dir(install);
    let mut levels: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix("campaign_scenes_level").and_then(|s| s.split(".bundle").next()).map(|s| s.split('_').next().unwrap().to_string())
        })
        .filter(|l| filter.is_none_or(|f| l.contains(f)))
        .collect();
    levels.sort();
    let exe = std::env::current_exe().unwrap().with_file_name(if cfg!(windows) { "ultrakrust.exe" } else { "ultrakrust" });
    let (mut ok_levels, mut ran) = (0, 0);
    for l in &levels {
        let out = Command::new(&exe)
            .args(["--level", l, "--unity-shaders", "--exit-after", "20"])
            .env("UNITY_FRAME_STATS", "1")
            .env("RUST_LOG", "info,naga=error,wgpu_hal=warn,wgpu_core=warn")
            .output();
        let Ok(out) = out else {
            r.note(format!("FAIL render {l}: could not start {}", exe.display()));
            continue;
        };
        ran += 1;
        let log = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        let log: String = log.replace("\u{1b}[", "\u{1}").split('\u{1}').map(|s| s.trim_start_matches(|c: char| c.is_ascii_digit() || c == ';' || c == 'm')).collect();
        let errors = log.matches("Validation Error").count() + log.matches("panicked").count();
        r.lower(&format!("render.{l}.gpu_errors"), errors as f64, 0.0);
        if let Some(line) = grab(&log, "renderers drawn") {
            if let Some(frac) = line.split_whitespace().find(|w| w.contains('/') && w.chars().next().is_some_and(|c| c.is_ascii_digit())) {
                let (a, b) = frac.split_once('/').unwrap();
                let (a, b): (f64, f64) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(1.0));
                r.higher(&format!("render.{l}.renderers_drawn_pct"), 100.0 * a / b.max(1.0));
            }
        }
        if let Some(ms) = grab(&log, "unity frame time:").and_then(|l| l.split("frame time: ").nth(1)).and_then(|s| s.split(' ').next()).and_then(|s| s.parse::<f64>().ok()) {
            r.lower(&format!("perf.render.{l}.frame_ms"), ms, 0.35);
        }
        if let Some(line) = grab(&log, "unity frame stats") {
            if let Some(n) = line.rsplit("distinct colors ").next().and_then(|s| s.trim().parse::<f64>().ok()) {
                r.info(&format!("render.{l}.frame_distinct_colors"), n);
            }
        } else {
            r.note(format!("gap: render {l}: no frame read back"));
        }
        if errors == 0 && out.status.success() {
            ok_levels += 1;
        } else {
            r.note(format!("FAIL render {l}: {errors} GPU errors/panics (exit {:?})", out.status.code()));
        }
    }
    r.higher("render.levels_clean", ok_levels as f64);
    r.pass("render.all_levels_clean", ok_levels == ran, format!("{ok_levels}/{ran} levels render without GPU errors"));
}
