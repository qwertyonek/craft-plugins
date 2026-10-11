//! CraftAnamorphicFlare: Cinematic Hollywood anamorphic lens streaks & chromatic flare rays.
//! Multi-pass separable sliding-window streak diffusion with specular highlight thresholding
//! and cyan/blue/spectral chromatic prism aberration.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.anamorphic",
  "name": "Craft Anamorphic Flare",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Hollywood anamorphic optical lens streaks, specular flares & spectral rays.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "streak_length", "name": "Streak Length", "type": "slider", "default": 120.0, "min": 5.0, "max": 800.0, "sliderMax": 400.0, "decimals": 0},
    {"id": "angle", "name": "Streak Angle", "type": "angle", "default": 0.0},
    {"id": "threshold", "name": "Highlight Threshold", "type": "slider", "default": 0.70, "min": 0.0, "max": 1.5, "sliderMax": 1.0, "decimals": 2},
    {"id": "intensity", "name": "Flare Intensity", "type": "slider", "default": 1.5, "min": 0.0, "max": 10.0, "sliderMax": 5.0, "decimals": 2},
    {"id": "chromatic_split", "name": "Spectral Dispersion", "type": "slider", "default": 12.0, "min": 0.0, "max": 50.0, "sliderMax": 30.0, "decimals": 1},
    {"id": "color_mode", "name": "Flare Tint Style", "type": "popup", "options": ["Sci-Fi Cyan / Blue (Cooke Anamorphic)", "Warm Gold / Amber (Panavision)", "Spectral Prism Rainbow", "Clean White / Source Color"], "default": 0},
    {"id": "tint_color", "name": "Custom Flare Tint", "type": "color", "default": [0.25, 0.65, 1.0, 1.0]},
    {"id": "streak_only", "name": "Flares Only (Transparent Base)", "type": "checkbox", "default": false}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
    static ACCUM: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
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
    if pixels == 0 || width <= 0 || height <= 0 || params == 0 || nparams < 8 {
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
    _time: f64,
    scale: f64,
) -> i32 {
    if p.len() < 8 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };

    // Params layout:
    // p[0]: about popup
    // p[1]: streak_length
    // p[2]: angle (deg)
    // p[3]: threshold
    // p[4]: intensity
    // p[5]: chromatic_split
    // p[6]: color_mode popup (0: Cyan/Blue, 1: Warm Gold, 2: Rainbow, 3: Custom/Source)
    // p[7..10]: tint_color [r, g, b, a]
    // p[11]: streak_only (bool)
    let length = ((p[1] as f32) * effective_scale).max(2.0);
    let angle_deg = p[2] as f32;
    let threshold = (p[3] as f32).max(0.0);
    let intensity = (p[4] as f32).max(0.0);
    let chroma = (p[5] as f32) * effective_scale;
    let color_mode = p[6].round() as u32;

    let tint = if p.len() >= 11 {
        [p[7] as f32, p[8] as f32, p[9] as f32, p[10] as f32]
    } else {
        [0.25, 0.65, 1.0, 1.0]
    };

    let streak_only = if p.len() >= 12 { p[11] > 0.5 } else { false };

    if intensity <= 0.001 {
        if streak_only {
            for px_val in px.iter_mut() {
                *px_val = [0.0; 4];
            }
        }
        return 0;
    }

    render_flare_zero_alloc(
        px,
        w,
        h,
        length,
        angle_deg,
        threshold,
        intensity,
        chroma,
        color_mode,
        tint,
        streak_only,
    );

    0
}

#[inline(always)]
fn get_luma(c: [f32; 4]) -> f32 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

fn render_flare_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    length: f32,
    angle_deg: f32,
    threshold: f32,
    intensity: f32,
    chroma: f32,
    color_mode: u32,
    custom_tint: [f32; 4],
    streak_only: bool,
) {
    let total_pixels = w * h;

    // Use 2x downscaled processing buffer for ultra-long cinema streaks (up to 800px) with zero frame drops
    let dw = (w + 1) / 2;
    let dh = (h + 1) / 2;
    let down_pixels = dw * dh;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < down_pixels * 3 {
            scratch.resize(down_pixels * 3, [0.0; 4]);
        }

        let (down_bright, rest) = scratch.split_at_mut(down_pixels);
        let (down_streak, down_temp) = rest.split_at_mut(down_pixels);

        // 1. Downsample & Extract Specular Highlights above threshold
        let inv_knee = 1.0 / (0.2f32).max(1e-4);
        for dy in 0..dh {
            let sy = dy * 2;
            let d_row = dy * dw;
            let s_row = sy * w;

            for dx in 0..dw {
                let sx = dx * 2;
                let c = px[s_row + sx];
                let luma = get_luma(c);

                if luma > threshold {
                    // Soft knee highlight curve
                    let factor = ((luma - threshold) * inv_knee).min(1.0) * (luma - threshold + 0.2);
                    down_bright[d_row + dx] = [
                        c[0] * factor,
                        c[1] * factor,
                        c[2] * factor,
                        c[3] * factor,
                    ];
                } else {
                    down_bright[d_row + dx] = [0.0; 4];
                }
            }
        }

        // 2. Anamorphic 3-Octave Exponential Streak Diffusion (Core + Body + Wide Flare)
        // Octave 1: Sharp Core streak
        // Octave 2: Main body streak
        // Octave 3: Ultra-wide cinema horizontal trail
        let rad1 = ((length * 0.15) as usize).clamp(1, dw - 1);
        let rad2 = ((length * 0.45) as usize).clamp(1, dw - 1);
        let rad3 = ((length * 1.00) as usize).clamp(1, dw - 1);

        down_streak.fill([0.0; 4]);

        let angle_rad = angle_deg.to_radians();
        let is_pure_horizontal = angle_deg.abs() < 0.5 || (angle_deg - 180.0).abs() < 0.5;

        if is_pure_horizontal {
            // High-speed O(1) separable horizontal running sum passes
            apply_fast_horizontal_streak(down_bright, down_streak, down_temp, dw, dh, rad1, 0.45);
            apply_fast_horizontal_streak(down_bright, down_streak, down_temp, dw, dh, rad2, 0.35);
            apply_fast_horizontal_streak(down_bright, down_streak, down_temp, dw, dh, rad3, 0.20);
        } else {
            // Directional angled streak
            let dir_x = angle_rad.cos();
            let dir_y = angle_rad.sin();
            apply_angled_directional_streak(down_bright, down_streak, dw, dh, dir_x, dir_y, length * 0.5);
        }

        // 3. Tint Palette & Spectral Chromatic Dispersion
        let (tint_r, tint_g, tint_b) = match color_mode {
            0 => (0.15, 0.65, 1.25), // Sci-Fi Cyan / Blue (Cooke Anamorphic)
            1 => (1.25, 0.85, 0.25), // Warm Gold / Amber (Panavision)
            3 => (custom_tint[0], custom_tint[1], custom_tint[2]),
            _ => (1.0, 1.0, 1.0),    // Spectral Prism Rainbow
        };

        let has_chroma = chroma > 0.1 || color_mode == 2;
        let chroma_shift = (chroma * 0.4).max(1.0);

        // 4. Bilinear Upsample and HDR Screen Add to Output
        let max_dx = dw - 1;
        let max_dy = dh - 1;
        let streak_ptr = down_streak.as_ptr();

        for y in 0..h {
            let fy = (y as f32) * 0.5;
            let y0 = fy.floor() as usize;
            let y1 = (y0 + 1).min(max_dy);
            let wy = fy - (y0 as f32);
            let wy0 = 1.0 - wy;

            let row_idx = y * w;
            let row_d0 = y0 * dw;
            let row_d1 = y1 * dw;

            for x in 0..w {
                let fx = (x as f32) * 0.5;
                let x0 = fx.floor() as usize;
                let x1 = (x0 + 1).min(max_dx);
                let wx = fx - (x0 as f32);
                let wx0 = 1.0 - wx;

                let p00 = unsafe { *streak_ptr.add(row_d0 + x0) };
                let p10 = unsafe { *streak_ptr.add(row_d0 + x1) };
                let p01 = unsafe { *streak_ptr.add(row_d1 + x0) };
                let p11 = unsafe { *streak_ptr.add(row_d1 + x1) };

                let w00 = wx0 * wy0;
                let w10 = wx * wy0;
                let w01 = wx0 * wy;
                let w11 = wx * wy;

                let flare_g = (p00[1] * w00 + p10[1] * w10 + p01[1] * w01 + p11[1] * w11) * intensity;

                let (flare_r, flare_b) = if has_chroma {
                    // Sample Red shifted outward, Blue shifted inward (optical prism)
                    let rx0 = (fx + chroma_shift).floor() as usize;
                    let rx0_c = rx0.min(max_dx);
                    let bx0 = if fx >= chroma_shift { (fx - chroma_shift).floor() as usize } else { 0 };

                    let pr = unsafe { *streak_ptr.add(row_d0 + rx0_c) };
                    let pb = unsafe { *streak_ptr.add(row_d0 + bx0) };
                    (pr[0] * intensity, pb[2] * intensity)
                } else {
                    let flare_monolith = (p00[0] * w00 + p10[0] * w10 + p01[0] * w01 + p11[0] * w11) * intensity;
                    (flare_monolith, flare_monolith)
                };

                let final_flare_r = flare_r * tint_r;
                let final_flare_g = flare_g * tint_g;
                let final_flare_b = flare_b * tint_b;

                if streak_only {
                    let flare_a = (final_flare_r.max(final_flare_g).max(final_flare_b)).min(1.0);
                    px[row_idx + x] = [final_flare_r, final_flare_g, final_flare_b, flare_a];
                } else {
                    let orig = px[row_idx + x];
                    // HDR Energy-conserving Additive composite
                    px[row_idx + x] = [
                        orig[0] + final_flare_r,
                        orig[1] + final_flare_g,
                        orig[2] + final_flare_b,
                        orig[3],
                    ];
                }
            }
        }
    });
}

/// High-performance O(1) horizontal streak box filter accumulator
fn apply_fast_horizontal_streak(
    src: &[[f32; 4]],
    dst_accum: &mut [[f32; 4]],
    temp: &mut [[f32; 4]],
    w: usize,
    h: usize,
    r: usize,
    weight: f32,
) {
    let inv_diam = (1.0 / (2 * r + 1) as f32) * weight;

    for y in 0..h {
        let row_offset = y * w;
        let s_row = &src[row_offset..row_offset + w];
        let d_accum_row = &mut dst_accum[row_offset..row_offset + w];

        let mut sum_r = s_row[0][0] * (r + 1) as f32;
        let mut sum_g = s_row[0][1] * (r + 1) as f32;
        let mut sum_b = s_row[0][2] * (r + 1) as f32;
        let mut sum_a = s_row[0][3] * (r + 1) as f32;

        for i in 1..=r {
            let p = s_row[i.min(w - 1)];
            sum_r += p[0];
            sum_g += p[1];
            sum_b += p[2];
            sum_a += p[3];
        }

        for x in 0..w {
            d_accum_row[x][0] += sum_r * inv_diam;
            d_accum_row[x][1] += sum_g * inv_diam;
            d_accum_row[x][2] += sum_b * inv_diam;
            d_accum_row[x][3] += sum_a * inv_diam;

            let left_idx = if x >= r { x - r } else { 0 };
            let right_idx = (x + r + 1).min(w - 1);

            let p_left = s_row[left_idx];
            let p_right = s_row[right_idx];

            sum_r += p_right[0] - p_left[0];
            sum_g += p_right[1] - p_left[1];
            sum_b += p_right[2] - p_left[2];
            sum_a += p_right[3] - p_left[3];
        }
    }
}

/// Angled directional streak accumulator
fn apply_angled_directional_streak(
    src: &[[f32; 4]],
    dst_accum: &mut [[f32; 4]],
    w: usize,
    h: usize,
    dx: f32,
    dy: f32,
    radius: f32,
) {
    let steps = 16;
    let inv_steps = 1.0 / (steps as f32);
    let step_dist = (radius / (steps as f32)).max(0.5);

    for y in 0..h {
        let y_f = y as f32;
        let row_idx = y * w;
        let dst_row = &mut dst_accum[row_idx..row_idx + w];

        for x in 0..w {
            let x_f = x as f32;
            let mut acc = [0.0f32; 4];

            for i in -steps..=steps {
                let dist = (i as f32) * step_dist;
                let sx = (x_f + dx * dist).clamp(0.0, (w - 1) as f32) as usize;
                let sy = (y_f + dy * dist).clamp(0.0, (h - 1) as f32) as usize;
                let c = src[sy * w + sx];
                acc[0] += c[0];
                acc[1] += c[1];
                acc[2] += c[2];
                acc[3] += c[3];
            }

            dst_row[x][0] += acc[0] * inv_steps * 0.5;
            dst_row[x][1] += acc[1] * inv_steps * 0.5;
            dst_row[x][2] += acc[2] * inv_steps * 0.5;
            dst_row[x][3] += acc[3] * inv_steps * 0.5;
        }
    }
}
