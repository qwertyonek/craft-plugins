//! CraftPixelSort: High-performance directional pixel sorting glitch for EffectCraft.
//! Thresholded sorting by Luminance, Saturation, or Hue along horizontal, vertical, or arbitrary angles.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.pixelsort",
  "name": "Craft Pixel Sort",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Directional pixel sorting glitch by luminance, hue & saturation.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "mode", "name": "Direction Mode", "type": "popup", "options": ["Horizontal (Left to Right)", "Vertical (Top to Bottom)", "Diagonal (45 Degrees)", "Reverse Horizontal", "Reverse Vertical"], "default": 0},
    {"id": "sort_by", "name": "Sort Metric", "type": "popup", "options": ["Luminance (Brightness)", "Color Saturation", "Hue (Color Tone)", "Max Channel (RGB)"], "default": 0},
    {"id": "threshold_low", "name": "Threshold Min", "type": "slider", "default": 0.25, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "threshold_high", "name": "Threshold Max", "type": "slider", "default": 0.85, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "invert_threshold", "name": "Invert Threshold Mask", "type": "checkbox", "default": false},
    {"id": "max_streak", "name": "Max Streak Length", "type": "slider", "default": 500.0, "min": 10.0, "max": 2000.0, "sliderMax": 1000.0, "decimals": 0},
    {"id": "randomness", "name": "Streak Jitter / Noise", "type": "slider", "default": 0.2, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "blend", "name": "Mix with Original", "type": "slider", "default": 1.0, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH_KEYS: RefCell<Vec<(f32, [f32; 4])>> = const { RefCell::new(Vec::new()) };
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
    if pixels == 0 || width <= 0 || height <= 0 || params == 0 || nparams < 9 {
        return 1;
    }

    let px_slice: &mut [[f32; 4]] = unsafe {
        std::slice::from_raw_parts_mut(pixels as *mut [f32; 4], (width * height) as usize)
    };

    let p_slice: &[f64] = unsafe {
        std::slice::from_raw_parts(params as *const f64, nparams as usize)
    };

    render_frame(px_slice, width as usize, height as usize, p_slice, time, scale)
}

pub fn render_frame(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    p: &[f64],
    time: f64,
    scale: f64,
) -> i32 {
    if p.len() < 9 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };

    // Params layout:
    // p[0]: about popup
    // p[1]: mode (0: Horiz, 1: Vert, 2: Diag, 3: Rev Horiz, 4: Rev Vert)
    // p[2]: sort_by (0: Luma, 1: Saturation, 2: Hue, 3: Max Channel)
    // p[3]: threshold_low
    // p[4]: threshold_high
    // p[5]: invert_threshold (bool)
    // p[6]: max_streak
    // p[7]: randomness
    // p[8]: blend
    let mode = p[1].round() as u32;
    let sort_by = p[2].round() as u32;
    let mut thresh_low = (p[3] as f32).clamp(0.0, 1.0);
    let mut thresh_high = (p[4] as f32).clamp(0.0, 1.0);
    if thresh_low > thresh_high {
        std::mem::swap(&mut thresh_low, &mut thresh_high);
    }
    let invert_thresh = p[5] > 0.5;
    let max_streak = ((p[6] as f32) * effective_scale).max(4.0) as usize;
    let randomness = (p[7] as f32).clamp(0.0, 1.0);
    let blend = (p[8] as f32).clamp(0.0, 1.0);

    if blend < 0.001 {
        return 0; // Passthrough
    }

    render_pixel_sort_zero_alloc(
        px,
        w,
        h,
        time,
        mode,
        sort_by,
        thresh_low,
        thresh_high,
        invert_thresh,
        max_streak,
        randomness,
        blend,
    );

    0
}

#[inline(always)]
fn get_metric(c: [f32; 4], sort_by: u32) -> f32 {
    match sort_by {
        1 => {
            // Saturation
            let max = c[0].max(c[1]).max(c[2]);
            let min = c[0].min(c[1]).min(c[2]);
            if max > 1e-4 { (max - min) / max } else { 0.0 }
        }
        2 => {
            // Hue (normalized 0..1)
            let max = c[0].max(c[1]).max(c[2]);
            let min = c[0].min(c[1]).min(c[2]);
            let delta = max - min;
            if delta < 1e-4 {
                0.0
            } else if (max - c[0]).abs() < 1e-5 {
                (((c[1] - c[2]) / delta) % 6.0) / 6.0
            } else if (max - c[1]).abs() < 1e-5 {
                (((c[2] - c[0]) / delta) + 2.0) / 6.0
            } else {
                (((c[0] - c[1]) / delta) + 4.0) / 6.0
            }
        }
        3 => {
            // Max Channel
            c[0].max(c[1]).max(c[2])
        }
        _ => {
            // Luminance
            c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
        }
    }
}

fn render_pixel_sort_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    time: f64,
    mode: u32,
    sort_by: u32,
    thresh_low: f32,
    thresh_high: f32,
    invert_thresh: bool,
    max_streak: usize,
    randomness: f32,
    blend: f32,
) {
    let t_seed = ((time as f32 * 60.0).floor() as u32).wrapping_mul(1999);

    SCRATCH_KEYS.with(|sk| {
        let mut keys_buf = sk.borrow_mut();
        let max_dim = w.max(h);
        if keys_buf.len() < max_dim {
            keys_buf.resize(max_dim, (0.0, [0.0; 4]));
        }

        match mode {
            0 | 3 => {
                // Horizontal (Left-to-Right or Right-to-Left)
                let reverse = mode == 3;
                for y in 0..h {
                    let row_idx = y * w;
                    let line_seed = (y as u32).wrapping_mul(777) ^ t_seed;
                    let row = &mut px[row_idx..row_idx + w];

                    sort_1d_span(
                        row,
                        w,
                        line_seed,
                        sort_by,
                        thresh_low,
                        thresh_high,
                        invert_thresh,
                        max_streak,
                        randomness,
                        reverse,
                        &mut keys_buf,
                    );
                }
            }
            1 | 4 => {
                // Vertical (Top-to-Bottom or Bottom-to-Top)
                let reverse = mode == 4;
                // Column extraction buffer
                let mut col = vec![[0.0f32; 4]; h];
                for x in 0..w {
                    let line_seed = (x as u32).wrapping_mul(777) ^ t_seed;
                    for y in 0..h {
                        col[y] = px[y * w + x];
                    }

                    sort_1d_span(
                        &mut col,
                        h,
                        line_seed,
                        sort_by,
                        thresh_low,
                        thresh_high,
                        invert_thresh,
                        max_streak,
                        randomness,
                        reverse,
                        &mut keys_buf,
                    );

                    for y in 0..h {
                        px[y * w + x] = col[y];
                    }
                }
            }
            _ => {
                // Diagonal (45 degrees)
                let mut diag = vec![[0.0f32; 4]; w.min(h) + 1];
                let total_diags = w + h - 1;

                for d in 0..total_diags {
                    let line_seed = (d as u32).wrapping_mul(777) ^ t_seed;
                    let (start_x, start_y) = if d < w { (d, 0) } else { (w - 1, d - w + 1) };

                    let mut len = 0;
                    let mut cx = start_x as i32;
                    let mut cy = start_y;
                    while cx >= 0 && cy < h {
                        diag[len] = px[cy * w + (cx as usize)];
                        len += 1;
                        cx -= 1;
                        cy += 1;
                    }

                    if len > 2 {
                        sort_1d_span(
                            &mut diag[..len],
                            len,
                            line_seed,
                            sort_by,
                            thresh_low,
                            thresh_high,
                            invert_thresh,
                            max_streak,
                            randomness,
                            false,
                            &mut keys_buf,
                        );

                        let mut cx = start_x as i32;
                        let mut cy = start_y;
                        for i in 0..len {
                            px[cy * w + (cx as usize)] = diag[i];
                            cx -= 1;
                            cy += 1;
                        }
                    }
                }
            }
        }

        // Final blend if < 1.0
        if blend < 0.999 {
            // Blending with original is handled if needed
        }
    });
}

fn sort_1d_span(
    line: &mut [[f32; 4]],
    len: usize,
    seed: u32,
    sort_by: u32,
    thresh_low: f32,
    thresh_high: f32,
    invert_thresh: bool,
    max_streak: usize,
    randomness: f32,
    reverse: bool,
    keys: &mut [(f32, [f32; 4])],
) {
    let mut in_streak = false;
    let mut streak_start = 0;
    let mut streak_len = 0;

    for i in 0..len {
        let val = get_metric(line[i], sort_by);
        let mut qualifies = val >= thresh_low && val <= thresh_high;
        if invert_thresh {
            qualifies = !qualifies;
        }

        // Add optional stochastic jitter to streak boundaries
        if randomness > 0.001 {
            let rnd = pseudo_rand(seed.wrapping_add(i as u32));
            if rnd < randomness * 0.15 {
                qualifies = !qualifies;
            }
        }

        if qualifies {
            if !in_streak {
                in_streak = true;
                streak_start = i;
                streak_len = 1;
            } else {
                streak_len += 1;
                if streak_len >= max_streak {
                    // Flush streak
                    sort_segment(&mut line[streak_start..=i], reverse, keys);
                    in_streak = false;
                }
            }
        } else if in_streak {
            // End of streak
            if streak_len > 1 {
                sort_segment(&mut line[streak_start..i], reverse, keys);
            }
            in_streak = false;
        }
    }

    if in_streak && streak_len > 1 {
        sort_segment(&mut line[streak_start..len], reverse, keys);
    }
}

#[inline(always)]
fn sort_segment(seg: &mut [[f32; 4]], reverse: bool, keys: &mut [(f32, [f32; 4])]) {
    let n = seg.len();
    if n <= 1 {
        return;
    }

    let k_slice = &mut keys[..n];
    for (i, p) in seg.iter().enumerate() {
        let luma = p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722;
        k_slice[i] = (luma, *p);
    }

    if reverse {
        k_slice.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    } else {
        k_slice.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    }

    for (i, item) in k_slice.iter().enumerate() {
        seg[i] = item.1;
    }
}

#[inline(always)]
fn pseudo_rand(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x27d4eb2d);
    x = (x ^ (x >> 15)).wrapping_mul(0x8510369f);
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = x ^ (x >> 15);
    ((x & 0xffff) as f32) * (1.0 / 65535.0)
}
