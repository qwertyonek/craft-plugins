//! CraftVHSGlitch: Authentic analog VHS tape damage & NTSC retro glitch suite.
//! Horizontal scanline tape drift, head tracking noise, YIQ/chroma phase smear, tape grain,
//! 2D analog tape defocus blur (O(1) separable box blur), and soft-blended dropout flares.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.vhs",
  "name": "Craft VHS Glitch",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Authentic analog VHS tape damage, tracking noise, scanline drift, tape blur & chroma bleed.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "tape_blur", "name": "Tape Defocus Blur", "type": "slider", "default": 2.0, "min": 0.0, "max": 25.0, "sliderMax": 15.0, "decimals": 1},
    {"id": "tape_drift", "name": "Horizontal Tape Drift", "type": "slider", "default": 6.0, "min": 0.0, "max": 50.0, "sliderMax": 25.0, "decimals": 1},
    {"id": "tracking_noise", "name": "Tracking Noise Bar", "type": "slider", "default": 0.35, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "tracking_speed", "name": "Tracking Bar Speed", "type": "slider", "default": 0.8, "min": 0.0, "max": 5.0, "sliderMax": 3.0, "decimals": 2},
    {"id": "chroma_bleed", "name": "Chroma Smear (YIQ Bleed)", "type": "slider", "default": 10.0, "min": 0.0, "max": 40.0, "sliderMax": 25.0, "decimals": 1},
    {"id": "tape_grain", "name": "Analog Tape Grain", "type": "slider", "default": 0.12, "min": 0.0, "max": 1.0, "sliderMax": 0.5, "decimals": 2},
    {"id": "interlacing", "name": "Interlacing Lines", "type": "slider", "default": 0.20, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "vcr_dropouts", "name": "VCR Signal Dropouts", "type": "slider", "default": 0.25, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "dropout_softness", "name": "Dropout Softness", "type": "slider", "default": 0.6, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "color_grading", "name": "Tape Color Grade", "type": "popup", "options": ["Authentic 1980s NTSC", "Warm Home Video (SP Mode)", "Dark Cyberpunk Tape", "Clean / No Tint"], "default": 0}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
    static BLUR_TEMP: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
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
    if p.len() < 10 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };

    // Params:
    // p[0]: about popup
    // p[1]: tape_blur (true 2D defocus blur)
    // p[2]: tape_drift
    // p[3]: tracking_noise (0.0..1.0)
    // p[4]: tracking_speed
    // p[5]: chroma_bleed
    // p[6]: tape_grain
    // p[7]: interlacing
    // p[8]: vcr_dropouts
    // p[9]: dropout_softness
    // p[10]: color_grading popup (0: NTSC, 1: Warm SP, 2: Cyberpunk, 3: Clean)
    let tape_blur = (p[1] as f32) * effective_scale;
    let tape_drift = (p[2] as f32) * effective_scale;
    let tracking_noise = (p[3] as f32).clamp(0.0, 1.0);
    let tracking_speed = p[4] as f32;
    let chroma_bleed = (p[5] as f32) * effective_scale;
    let tape_grain = (p[6] as f32).clamp(0.0, 1.0);
    let interlacing = (p[7] as f32).clamp(0.0, 1.0);
    let dropouts = (p[8] as f32).clamp(0.0, 1.0);
    let dropout_softness = if p.len() >= 10 { (p[9] as f32).clamp(0.0, 1.0) } else { 0.6 };
    let color_grade = if p.len() >= 11 { p[10].round() as u32 } else { 0 };

    render_vhs_zero_alloc(
        px,
        w,
        h,
        time,
        tape_blur,
        tape_drift,
        tracking_noise,
        tracking_speed,
        chroma_bleed,
        tape_grain,
        interlacing,
        dropouts,
        dropout_softness,
        color_grade,
    );

    0
}

#[inline(always)]
fn hash_noise_1d(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x27d4eb2d);
    x = (x ^ (x >> 15)).wrapping_mul(0x8510369f);
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = x ^ (x >> 15);
    ((x & 0xffff) as f32) * (1.0 / 65535.0)
}

#[inline(always)]
fn hash_noise_2d(x: u32, y: u32, seed: u32) -> f32 {
    let mut n = x.wrapping_mul(0x374765a9)
        ^ y.wrapping_mul(0x632be5ab)
        ^ seed.wrapping_mul(0x8510369f);
    n = (n ^ (n >> 15)).wrapping_mul(0x7feb352d);
    ((n & 0xffff) as f32) * (1.0 / 65535.0)
}

fn render_vhs_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    time: f64,
    tape_blur: f32,
    tape_drift: f32,
    tracking_noise: f32,
    tracking_speed: f32,
    chroma_bleed: f32,
    tape_grain: f32,
    interlacing: f32,
    dropouts: f32,
    dropout_softness: f32,
    color_grade: u32,
) {
    let total_pixels = w * h;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < total_pixels {
            scratch.resize(total_pixels, [0.0; 4]);
        }
        let src = &mut scratch[..total_pixels];
        src.copy_from_slice(px);

        let src_ptr = src.as_ptr();
        let max_x = (w - 1).max(1);
        let max_y = (h - 1).max(1);

        let t_f = time as f32;
        let time_seed = ((t_f * 60.0).floor() as u32).wrapping_mul(1337);

        // Tracking noise bar vertical position
        let tracking_cycle = (t_f * tracking_speed * 0.4) % 1.2 - 0.1;
        let tracking_center_y = tracking_cycle * (h as f32);
        let tracking_height = (h as f32) * 0.08;

        // Color grade tint coefficients
        let (tint_r, tint_g, tint_b, saturation) = match color_grade {
            1 => (1.05, 1.0, 0.92, 1.15), // Warm Home Video
            2 => (0.85, 1.10, 1.25, 1.25), // Cyberpunk Cold Tape
            3 => (1.00, 1.0, 1.00, 1.00), // Clean
            _ => (1.02, 0.98, 0.94, 1.10), // Authentic 1980s NTSC
        };

        for y in 0..h {
            let row_idx = y * w;
            let y_f = y as f32;
            let dst_row = &mut px[row_idx..row_idx + w];

            // 1. Horizontal scanline drift (smooth analog tape undulation + gentle micro-jitter)
            let line_seed = (y as u32).wrapping_mul(1983) ^ time_seed;
            let line_jitter = (hash_noise_1d(line_seed) - 0.5) * 2.0;

            let tape_wave = ((y_f * 0.015 + t_f * 2.5).sin() + (y_f * 0.004 - t_f * 1.2).cos()) * 0.5;
            let mut scanline_shift = (line_jitter * 0.2 + tape_wave) * tape_drift;

            // 2. Tracking noise bar displacement and snow
            let mut in_tracking_bar = false;
            let mut tracking_noise_intensity = 0.0f32;

            if tracking_noise > 0.001 {
                let dist_to_tracking = (y_f - tracking_center_y).abs();
                if dist_to_tracking < tracking_height {
                    in_tracking_bar = true;
                    let factor = 1.0 - (dist_to_tracking / tracking_height);
                    let violent_jitter = (hash_noise_1d(line_seed.wrapping_add(99)) - 0.5) * 2.0;
                    scanline_shift += violent_jitter * factor * tracking_noise * 30.0;
                    tracking_noise_intensity = factor * tracking_noise;
                }
            }

            // 3. Interlacing alternate scanline darkening
            let interlace_dim = if interlacing > 0.001 && (y % 2 == 1) {
                1.0 - (interlacing * 0.30)
            } else {
                1.0
            };

            // 4. Random VCR signal dropout line (softer streak with fade out)
            let is_dropout_line = dropouts > 0.001
                && (hash_noise_1d(line_seed.wrapping_add(555)) < (dropouts * 0.012));
            let dropout_brightness = 0.5 + 0.5 * (1.0 - dropout_softness * 0.4);

            for x in 0..w {
                let x_f = x as f32;

                // Base luma position
                let base_x = (x_f + scanline_shift).clamp(0.0, max_x as f32);
                let base_y = y_f;

                let px_g = sample_bilinear(src_ptr, w, max_x, max_y, base_x, base_y);

                // Chroma bleed (Red shifts right, Blue shifts left with soft trailing)
                let (r_val, b_val) = if chroma_bleed > 0.1 {
                    let rx = (base_x + chroma_bleed).clamp(0.0, max_x as f32);
                    let bx = (base_x - chroma_bleed * 0.75).clamp(0.0, max_x as f32);
                    let px_r = sample_bilinear(src_ptr, w, max_x, max_y, rx, base_y);
                    let px_b = sample_bilinear(src_ptr, w, max_x, max_y, bx, base_y);
                    (px_r[0], px_b[2])
                } else {
                    (px_g[0], px_g[2])
                };

                let mut r = r_val * tint_r;
                let mut g = px_g[1] * tint_g;
                let mut b = b_val * tint_b;
                let a = px_g[3];

                // Apply saturation / color grade
                if (saturation - 1.0f32).abs() > 0.01 {
                    let luma = r * 0.2126 + g * 0.7152 + b * 0.0722;
                    r = luma + (r - luma) * saturation;
                    g = luma + (g - luma) * saturation;
                    b = luma + (b - luma) * saturation;
                }

                // Apply Interlacing
                r *= interlace_dim;
                g *= interlace_dim;
                b *= interlace_dim;

                // 5. Analog Tape Grain (subtle high-frequency noise)
                if tape_grain > 0.001 {
                    let grain = (hash_noise_2d(x as u32, y as u32, time_seed) - 0.5) * tape_grain * 0.35;
                    r = (r + grain).clamp(0.0, 1.0);
                    g = (g + grain).clamp(0.0, 1.0);
                    b = (b + grain).clamp(0.0, 1.0);
                }

                // 6. Tracking noise bar static (soft-blended magnetic snow)
                if in_tracking_bar && tracking_noise_intensity > 0.01 {
                    let snow = hash_noise_2d((x as u32).wrapping_mul(7), y as u32, time_seed);
                    if snow > 0.55 {
                        let snow_val = (snow - 0.55) * 2.2 * tracking_noise_intensity;
                        let blend = tracking_noise_intensity * 0.7;
                        r = (r * (1.0 - blend) + snow_val).clamp(0.0, 1.0);
                        g = (g * (1.0 - blend) + snow_val).clamp(0.0, 1.0);
                        b = (b * (1.0 - blend) + snow_val).clamp(0.0, 1.0);
                    }
                }

                // 7. VCR dropout line glitch (soft, blended analog head flares)
                if is_dropout_line {
                    let spark = hash_noise_1d((x as u32) ^ time_seed);
                    if spark > 0.4 {
                        let spark_strength = (spark - 0.4) * 1.66 * dropout_brightness;
                        let blend = (spark_strength * (1.0 - dropout_softness * 0.5)).clamp(0.0, 1.0);
                        r = r * (1.0 - blend) + 0.95 * blend;
                        g = g * (1.0 - blend) + 0.95 * blend;
                        b = b * (1.0 - blend) + 0.98 * blend;
                    }
                }

                dst_row[x] = [r, g, b, a];
            }
        }
    });

    // 8. O(1) Sliding-Window 2D Tape Defocus Blur pass
    if tape_blur > 0.2 {
        let radius = (tape_blur.round() as usize).clamp(1, 25);
        apply_separable_box_blur_2d(px, w, h, radius);
    }
}

/// O(1) Separable 2D Box Blur (Sliding Window Running Sum).
/// Constant time per pixel regardless of blur radius. Stays 100% within fuel limits.
fn apply_separable_box_blur_2d(px: &mut [[f32; 4]], w: usize, h: usize, r: usize) {
    let total = w * h;

    BLUR_TEMP.with(|bt| {
        let mut temp_cell = bt.borrow_mut();
        if temp_cell.len() < total {
            temp_cell.resize(total, [0.0; 4]);
        }
        let temp = &mut temp_cell[..total];

        // Horizontal Pass: px -> temp
        let inv_diam_h = 1.0 / (2 * r + 1) as f32;
        for y in 0..h {
            let row_offset = y * w;
            let src_row = &px[row_offset..row_offset + w];
            let dst_row = &mut temp[row_offset..row_offset + w];

            let mut sum_r = src_row[0][0] * (r + 1) as f32;
            let mut sum_g = src_row[0][1] * (r + 1) as f32;
            let mut sum_b = src_row[0][2] * (r + 1) as f32;
            let mut sum_a = src_row[0][3] * (r + 1) as f32;

            for i in 1..=r {
                let px_val = src_row[i.min(w - 1)];
                sum_r += px_val[0];
                sum_g += px_val[1];
                sum_b += px_val[2];
                sum_a += px_val[3];
            }

            for x in 0..w {
                dst_row[x] = [
                    sum_r * inv_diam_h,
                    sum_g * inv_diam_h,
                    sum_b * inv_diam_h,
                    sum_a * inv_diam_h,
                ];

                let left_idx = if x >= r { x - r } else { 0 };
                let right_idx = (x + r + 1).min(w - 1);

                let p_left = src_row[left_idx];
                let p_right = src_row[right_idx];

                sum_r += p_right[0] - p_left[0];
                sum_g += p_right[1] - p_left[1];
                sum_b += p_right[2] - p_left[2];
                sum_a += p_right[3] - p_left[3];
            }
        }

        // Vertical Pass: temp -> px
        let inv_diam_v = 1.0 / (2 * r + 1) as f32;
        for x in 0..w {
            let mut sum_r = temp[x][0] * (r + 1) as f32;
            let mut sum_g = temp[x][1] * (r + 1) as f32;
            let mut sum_b = temp[x][2] * (r + 1) as f32;
            let mut sum_a = temp[x][3] * (r + 1) as f32;

            for i in 1..=r {
                let p = temp[i.min(h - 1) * w + x];
                sum_r += p[0];
                sum_g += p[1];
                sum_b += p[2];
                sum_a += p[3];
            }

            for y in 0..h {
                px[y * w + x] = [
                    sum_r * inv_diam_v,
                    sum_g * inv_diam_v,
                    sum_b * inv_diam_v,
                    sum_a * inv_diam_v,
                ];

                let top_y = if y >= r { y - r } else { 0 };
                let bot_y = (y + r + 1).min(h - 1);

                let p_top = temp[top_y * w + x];
                let p_bot = temp[bot_y * w + x];

                sum_r += p_bot[0] - p_top[0];
                sum_g += p_bot[1] - p_top[1];
                sum_b += p_bot[2] - p_top[2];
                sum_a += p_bot[3] - p_top[3];
            }
        }
    });
}

#[inline(always)]
fn sample_bilinear(
    src_ptr: *const [f32; 4],
    stride: usize,
    max_x: usize,
    max_y: usize,
    x: f32,
    y: f32,
) -> [f32; 4] {
    let (fx, fy) = (x.floor(), y.floor());
    let x0 = fx as usize;
    let y0 = fy as usize;
    let x1 = (x0 + 1).min(max_x);
    let y1 = (y0 + 1).min(max_y);

    let wx1 = x - fx;
    let wx0 = 1.0 - wx1;
    let wy1 = y - fy;
    let wy0 = 1.0 - wy1;

    unsafe {
        let p00 = *src_ptr.add(y0 * stride + x0);
        let p10 = *src_ptr.add(y0 * stride + x1);
        let p01 = *src_ptr.add(y1 * stride + x0);
        let p11 = *src_ptr.add(y1 * stride + x1);

        let w00 = wx0 * wy0;
        let w10 = wx1 * wy0;
        let w01 = wx0 * wy1;
        let w11 = wx1 * wy1;

        [
            p00[0] * w00 + p10[0] * w10 + p01[0] * w01 + p11[0] * w11,
            p00[1] * w00 + p10[1] * w10 + p01[1] * w01 + p11[1] * w11,
            p00[2] * w00 + p10[2] * w10 + p01[2] * w01 + p11[2] * w11,
            p00[3] * w00 + p10[3] * w10 + p01[3] * w01 + p11[3] * w11,
        ]
    }
}
