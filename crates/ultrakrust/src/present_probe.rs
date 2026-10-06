//! `UK_PROBE_PRESENT=1`: numeric read-back of the presented window (swapchain screenshots reduced
//! to numbers in the log, never saved): early-frame brightness without any resize, and whether
//! HUD text clears from the screen once it is removed.
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

/// Frames the hint text is forced on and then cleared again.
const PROBE_ON: u32 = 160;
const PROBE_OFF: u32 = 200;
const PROBE_TEXT: &str = "PRESENT PROBE PRESENT PROBE PRESENT PROBE";

pub fn enabled() -> bool {
    std::env::var_os("UK_PROBE_PRESENT").is_some()
}

/// Runs after `update_hud`: forces the hint on/off and requests captures.
pub fn run(mut commands: Commands, mut hint: Single<&mut Text, With<super::HintText>>, mut n: Local<u32>) {
    *n += 1;
    let f = *n;
    if (PROBE_ON..PROBE_OFF).contains(&f) {
        hint.0 = PROBE_TEXT.into();
    }
    let label = match f {
        3 | 10 | 30 | 60 | 90 | 105 | 120 | 135 => format!("early{f}"),
        150 => "before".into(),
        190 => "during".into(),
        240 => "after".into(),
        600 => "late".into(),
        _ => return,
    };
    let label = format!("{label} (hint text {:?})", hint.0);
    commands.spawn(Screenshot::primary_window()).observe(move |cap: On<ScreenshotCaptured>| report(&label, &cap.image));
}

fn report(label: &str, img: &Image) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let Some(data) = img.data.as_ref().filter(|d| d.len() >= w * h * 4) else {
        info!("present probe {label}: unreadable {w}x{h}");
        return;
    };
    let lum = |x: usize, y: usize| {
        let p = &data[(y * w + x) * 4..][..3];
        (p[0] as f32 + p[1] as f32 + p[2] as f32) / (3.0 * 255.0)
    };
    // whole frame: mean luminance and share of pixels above black
    let (mut sum, mut lit) = (0.0f64, 0usize);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let l = lum(x, y);
            sum += l as f64;
            lit += (l > 0.04) as usize;
        }
    }
    let samples = (h.div_ceil(2) * w.div_ceil(2)).max(1);
    // near-white "ink" in the hint band (bottom 140 px, centered) and the top-left debug corner
    let ink = |x0: usize, x1: usize, y0: usize, y1: usize| {
        let mut c = 0;
        for y in y0.min(h)..y1.min(h) {
            for x in x0.min(w)..x1.min(w) {
                c += (lum(x, y) > 0.85) as usize;
            }
        }
        c
    };
    let hint = ink(w / 6, w - w / 6, h.saturating_sub(200), h.saturating_sub(120));
    let corner = ink(0, w / 3, 0, 140);
    info!("present probe {label}: {w}x{h} mean_lum {:.4} lit_frac {:.4} hint_ink {hint} corner_ink {corner}", sum / samples as f64, lit as f64 / samples as f64);
}
