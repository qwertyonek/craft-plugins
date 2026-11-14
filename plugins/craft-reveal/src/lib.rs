//! CraftReveal: animated reveal / wipe / typewriter-style build for a layer.
//!
//! The plug-in API gives an effect a single RGBA frame, so this cannot animate characters
//! one by one — it has no glyph data to cut with. What it does instead is drive an alpha
//! mask over whatever the layer rendered, with a direction, a soft edge, optional line- or
//! block-quantised stepping (which reads as a build rather than a wipe) and a cursor.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.reveal",
  "name": "Craft Reveal",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Animated reveal, wipe and typewriter-style build with soft edges and cursor.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "reveal", "name": "Reveal Amount (%)", "type": "slider", "default": 100.0, "min": 0.0, "max": 100.0, "sliderMax": 100.0, "decimals": 0},
    {"id": "direction", "name": "Direction", "type": "popup", "options": ["Left to Right", "Right to Left", "Top to Bottom", "Bottom to Top", "Diagonal", "Radial (Center Out)"], "default": 0},
    {"id": "softness", "name": "Edge Softness (px)", "type": "slider", "default": 24.0, "min": 0.0, "max": 300.0, "sliderMax": 120.0, "decimals": 0},
    {"id": "step_mode", "name": "Build Style", "type": "popup", "options": ["Smooth (continuous wipe)", "Lines (stepped rows / columns)", "Blocks (pixel grid)"], "default": 0},
    {"id": "block_size", "name": "Step / Block Size (px)", "type": "slider", "default": 16.0, "min": 2.0, "max": 200.0, "sliderMax": 64.0, "decimals": 0},
    {"id": "cursor", "name": "Show Cursor", "type": "checkbox", "default": true},
    {"id": "cursor_width", "name": "Cursor Width (px)", "type": "slider", "default": 3.0, "min": 1.0, "max": 40.0, "sliderMax": 20.0, "decimals": 0},
    {"id": "cursor_glow", "name": "Cursor Glow", "type": "slider", "default": 1.0, "min": 0.0, "max": 3.0, "sliderMax": 2.0, "decimals": 2},
    {"id": "invert", "name": "Invert (hide instead)", "type": "checkbox", "default": false}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn ec_api_version() -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn ec_manifest_ptr() -> i32 {
    MANIFEST.as_ptr() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn ec_manifest_len() -> i32 {
    MANIFEST.len() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn ec_alloc(bytes: i32) -> i32 {
    BUF.with(|b| {
        let mut b = b.borrow_mut();
        let words = (bytes.max(0) as usize).div_ceil(8);
        if b.len() < words {
            b.resize(words, 0);
        }
        b.as_mut_ptr() as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ec_render(
    pixels: i32,
    width: i32,
    height: i32,
    params: i32,
    nparams: i32,
    time: f64,
    scale: f64,
) -> i32 {
    if pixels == 0 || width <= 0 || height <= 0 || params == 0 || nparams < 10 {
        return 1;
    }

    let px_slice: &mut [[f32; 4]] = unsafe {
        std::slice::from_raw_parts_mut(pixels as *mut [f32; 4], (width * height) as usize)
    };

    let p_slice: &[f64] = unsafe {
        std::slice::from_raw_parts(params as *const f64, nparams as usize)
    };

    render_frame(
        px_slice,
        width as usize,
        height as usize,
        p_slice,
        time,
        scale,
    )
}

pub fn render_frame(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    p: &[f64],
    _time: f64,
    scale: f64,
) -> i32 {
    if p.len() < 10 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };

    // Params layout:
    // p[0]: about popup
    // p[1]: reveal amount (0..100)
    // p[2]: direction popup (0: L>R, 1: R>L, 2: T>B, 3: B>T, 4: Diagonal, 5: Radial)
    // p[3]: edge softness (px)
    // p[4]: build style popup (0: Smooth, 1: Lines, 2: Blocks)
    // p[5]: step / block size (px)
    // p[6]: show cursor (0.0 / 1.0)
    // p[7]: cursor width (px)
    // p[8]: cursor glow
    // p[9]: invert (0.0 / 1.0)
    let reveal = (p[1] as f32 / 100.0).clamp(0.0, 1.0);
    let direction = p[2].round() as u32;
    let softness = (p[3] as f32).max(0.0) * effective_scale;
    let style = p[4].round() as u32;
    let block = ((p[5] as f32) * effective_scale).max(1.0);
    let show_cursor = p[6] > 0.5;
    let cursor_width = ((p[7] as f32) * effective_scale).max(1.0);
    let cursor_glow = (p[8] as f32).max(0.0);
    let invert = p[9] > 0.5;

    let max_dim = w.max(h) as f32;
    let soft = (softness / max_dim).max(1e-5);
    let inv_w = 1.0 / (w.max(2) - 1) as f32;
    let inv_h = 1.0 / (h.max(2) - 1) as f32;

    // The reveal front is offset so a full amount clears the frame and an empty amount
    // hides it even with a wide soft edge.
    let edge = reveal * (1.0 + 2.0 * soft) - soft;

    let cursor_sigma = (cursor_width / max_dim).max(1e-5) * 0.5;
    let (cx, cy) = ((w - 1) as f32 * 0.5, (h - 1) as f32 * 0.5);
    let max_radius = (cx * cx + cy * cy).sqrt().max(1.0);

    for y in 0..h {
        let row_idx = y * w;
        let y_f = y as f32;
        // Rows for the stepped "lines" style — quantised along the reveal axis only.
        let y_step = (y_f / block).floor() * block;
        let y_step_norm = (y_step * inv_h).clamp(0.0, 1.0);

        for x in 0..w {
            let c = px[row_idx + x];
            if c[3] <= 1e-5 {
                continue;
            }
            let x_f = x as f32;

            let d = match direction {
                0 => x_f * inv_w,
                1 => 1.0 - x_f * inv_w,
                2 => y_f * inv_h,
                3 => 1.0 - y_f * inv_h,
                4 => (x_f * inv_w + y_f * inv_h) * 0.5,
                _ => {
                    let dx = x_f - cx;
                    let dy = y_f - cy;
                    (dx * dx + dy * dy).sqrt() / max_radius
                }
            };

            // Staircase the reveal order. For "lines" the quantisation runs along the
            // axis the wipe travels; for "blocks" both axes are quantised, which gives a
            // pixel-grid build.
            let d_eff = match style {
                1 => match direction {
                    0 => (x_f / block).floor() * block * inv_w,
                    1 => 1.0 - (x_f / block).floor() * block * inv_w,
                    2 => y_step_norm,
                    3 => 1.0 - y_step_norm,
                    4 => {
                        let xs = (x_f / block).floor() * block * inv_w;
                        (xs + y_step_norm) * 0.5
                    }
                    _ => {
                        let dx = (x_f / block).floor() * block - cx;
                        let dy = (y_f / block).floor() * block - cy;
                        (dx * dx + dy * dy).sqrt() / max_radius
                    }
                },
                2 => {
                    let bx = (x_f / block).floor() * block * inv_w;
                    let by = (y_f / block).floor() * block * inv_h;
                    match direction {
                        0 => bx,
                        1 => 1.0 - bx,
                        2 => by,
                        3 => 1.0 - by,
                        4 => (bx + by) * 0.5,
                        _ => {
                            let dx = (x_f / block).floor() * block - cx;
                            let dy = (y_f / block).floor() * block - cy;
                            (dx * dx + dy * dy).sqrt() / max_radius
                        }
                    }
                }
                _ => d,
            };

            // 1.0 where the pixel is already revealed: d_eff below the front.
            let mut mask = 1.0 - smoothstep(edge - soft, edge + soft, d_eff);

            if show_cursor && cursor_glow > 0.0 {
                // A bar (or ring, for radial) riding the reveal front.
                let dist = (d_eff - edge).abs() / cursor_sigma;
                let spark = (-dist * dist).exp() * cursor_glow;
                let base = if invert { 1.0 - mask } else { mask };
                let boost = spark * 0.5 * (0.15 + 0.85 * base);
                px[row_idx + x] = [
                    (c[0] * mask + boost).min(f32::MAX),
                    (c[1] * mask + boost).min(f32::MAX),
                    (c[2] * mask + boost).min(f32::MAX),
                    (c[3] * mask + boost * 0.25).clamp(0.0, 1.0),
                ];
                mask = -1.0; // already written
            }

            if mask >= 0.0 {
                if invert {
                    mask = 1.0 - mask;
                }
                px[row_idx + x] = [c[0] * mask, c[1] * mask, c[2] * mask, c[3] * mask];
            }
        }
    }

    0
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let denom = (edge1 - edge0).max(1e-6);
    let t = ((x - edge0) / denom).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 48;
    const H: usize = 48;

    fn params(reveal: f64, direction: f64, softness: f64, style: f64) -> Vec<f64> {
        // about, reveal, direction, softness, style, block, cursor, cursor_width, glow, invert
        vec![0.0, reveal, direction, softness, style, 8.0, 0.0, 3.0, 1.0, 0.0]
    }

    fn solid() -> Vec<[f32; 4]> {
        vec![[0.6, 0.4, 0.2, 1.0]; W * H]
    }

    fn coverage(px: &[[f32; 4]]) -> f32 {
        px.iter().map(|c| c[3]).sum::<f32>() / (W * H) as f32
    }

    #[test]
    fn full_reveal_is_identity() {
        for direction in 0..6 {
            for style in 0..3 {
                let src = solid();
                let mut px = solid();
                let p = params(100.0, direction as f64, 0.0, style as f64);
                assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
                for (a, b) in px.iter().zip(src.iter()) {
                    assert!(
                        (a[3] - b[3]).abs() < 1e-3 && (a[0] - b[0]).abs() < 1e-3,
                        "direction {direction} style {style}: {a:?} != {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn empty_reveal_hides_everything() {
        for direction in 0..6 {
            let mut px = solid();
            let p = params(0.0, direction as f64, 0.0, 0.0);
            assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
            assert!(
                coverage(&px) < 1e-3,
                "direction {direction}: {:.4} still visible",
                coverage(&px)
            );
        }
    }

    #[test]
    fn coverage_grows_with_amount() {
        let mut last = -1.0;
        for amount in [0.0, 20.0, 40.0, 60.0, 80.0, 100.0] {
            let mut px = solid();
            let p = params(amount, 0.0, 0.0, 0.0);
            assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
            let c = coverage(&px);
            assert!(c > last, "amount {amount}: coverage {c:.3} did not grow from {last:.3}");
            last = c;
        }
        assert!((last - 1.0).abs() < 1e-3);
    }

    #[test]
    fn direction_changes_which_edge_stays() {
        let mut left = solid();
        let mut right = solid();
        let p_l = params(25.0, 0.0, 0.0, 0.0); // Left to Right
        let p_r = params(25.0, 1.0, 0.0, 0.0); // Right to Left
        assert_eq!(render_frame(&mut left, W, H, &p_l, 0.0, 1.0), 0);
        assert_eq!(render_frame(&mut right, W, H, &p_r, 0.0, 1.0), 0);
        assert!(left[(H / 2) * W][3] > 0.9, "Left to Right should keep the left edge");
        assert!(left[(H / 2) * W + W - 1][3] < 0.1, "Left to Right should clear the right edge");
        assert!(right[(H / 2) * W + W - 1][3] > 0.9, "Right to Left should keep the right edge");
        assert!(right[(H / 2) * W][3] < 0.1, "Right to Left should clear the left edge");
    }

    /// The "lines" build has to quantise: within one 8 px band the mask is identical,
    /// while a smooth wipe changes row by row.
    #[test]
    fn lines_style_is_stepped() {
        let mut stepped = solid();
        let mut smooth = solid();
        let p_stepped = params(45.0, 2.0, 0.0, 1.0); // Top to Bottom, Lines
        let p_smooth = params(45.0, 2.0, 6.0, 0.0); // Top to Bottom, Smooth
        assert_eq!(render_frame(&mut stepped, W, H, &p_stepped, 0.0, 1.0), 0);
        assert_eq!(render_frame(&mut smooth, W, H, &p_smooth, 0.0, 1.0), 0);

        let col = |px: &[[f32; 4]], y: usize| px[y * W][3];
        let mut stepped_changes = 0;
        let mut smooth_changes = 0;
        for y in 1..H {
            if (col(&stepped, y) - col(&stepped, y - 1)).abs() > 1e-4 {
                stepped_changes += 1;
            }
            if (col(&smooth, y) - col(&smooth, y - 1)).abs() > 1e-4 {
                smooth_changes += 1;
            }
        }
        assert_eq!(stepped_changes, 1, "one band edge expected, got {stepped_changes}");
        assert!(
            smooth_changes > 3,
            "a soft smooth wipe should ramp over several rows, changed {smooth_changes}"
        );
        // Inside the first band every row carries the same mask.
        assert!((col(&stepped, 0) - col(&stepped, 7)).abs() < 1e-4);
    }

    #[test]
    fn invert_flips_the_mask() {
        let mut normal = solid();
        let mut inverted = solid();
        let p = params(30.0, 0.0, 0.0, 0.0);
        let mut p_inv = p.clone();
        p_inv[9] = 1.0;
        assert_eq!(render_frame(&mut normal, W, H, &p, 0.0, 1.0), 0);
        assert_eq!(render_frame(&mut inverted, W, H, &p_inv, 0.0, 1.0), 0);
        let (a, b) = (coverage(&normal), coverage(&inverted));
        assert!((a + b - 1.0).abs() < 0.02, "coverage {a:.3} + {b:.3} should be 1");
    }

    #[test]
    fn soft_edge_only_blurs_the_front() {
        let mut px = solid();
        let p = params(50.0, 0.0, 8.0, 0.0);
        assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
        let mid = H / 2;
        // Something partially transparent must exist, and the extremes stay opaque/clear.
        let partial = (0..W)
            .filter(|&x| {
                let a = px[mid * W + x][3];
                a > 0.05 && a < 0.95
            })
            .count();
        assert!(partial >= 2, "soft edge produced no partial pixels: {partial}");
        assert!(px[mid * W][3] > 0.99);
        assert!(px[mid * W + W - 1][3] < 0.01);
    }
}
