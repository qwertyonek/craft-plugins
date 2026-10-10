//! CraftGlow: Physically-accurate inverse-square glow with chromatic aberration.
//! Zero-allocation O(1) sliding-window box-blur architecture with sparse bilinear reconstruction.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.glow",
  "name": "Craft Glow",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Physically accurate inverse-square glow with chromatic halo.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "radius", "name": "Radius", "type": "slider", "default": 50.0, "min": 1.0, "max": 500.0, "sliderMax": 200.0, "decimals": 1},
    {"id": "brightness", "name": "Brightness", "type": "slider", "default": 1.0, "min": 0.0, "max": 10.0, "sliderMax": 4.0, "decimals": 2},
    {"id": "threshold", "name": "Threshold", "type": "slider", "default": 20.0, "min": 0.0, "max": 100.0, "decimals": 1},
    {"id": "smoothness", "name": "Threshold Smoothness", "type": "slider", "default": 30.0, "min": 0.0, "max": 100.0, "decimals": 1},
    {"id": "falloff", "name": "Falloff (Inverse Square)", "type": "slider", "default": 1.8, "min": 0.5, "max": 4.0, "decimals": 2},
    {"id": "tint", "name": "Tint", "type": "color", "default": [1.0, 1.0, 1.0, 1.0]},
    {"id": "aberration", "name": "Chromatic Aberration", "type": "slider", "default": 15.0, "min": -100.0, "max": 100.0, "decimals": 1},
    {"id": "glow_only", "name": "Glow Only", "type": "checkbox", "default": false}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
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

/// One reusable, 8-byte-aligned buffer for parameters and pixels.
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
pub unsafe extern "C" fn ec_render(
    pixels: i32,
    width: i32,
    height: i32,
    params: i32,
    nparams: i32,
    _time: f64,
    scale: f64,
) -> i32 {
    if nparams < 11 || width <= 0 || height <= 0 {
        return 1;
    }

    let p = unsafe { std::slice::from_raw_parts(params as *const f64, nparams as usize) };
    let px = unsafe {
        std::slice::from_raw_parts_mut(pixels as *mut [f32; 4], (width * height) as usize)
    };

    render_frame(px, width as usize, height as usize, p, scale)
}

pub fn render_frame(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    p: &[f64],
    scale: f64,
) -> i32 {
    if p.len() < 11 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };
    let radius = (p[1].max(1.0) as f32) * effective_scale;
    let brightness = p[2].max(0.0) as f32;
    let threshold = (p[3] / 100.0).clamp(0.0, 1.0) as f32;
    let smoothness = (p[4] / 100.0).clamp(0.001, 1.0) as f32;
    let falloff = p[5].clamp(0.5, 4.0) as f32;
    let tint = [
        p[6].max(0.0) as f32,
        p[7].max(0.0) as f32,
        p[8].max(0.0) as f32,
        p[9].clamp(0.0, 1.0) as f32,
    ];
    let aberration = (p[10] / 100.0) as f32;
    let glow_only = if p.len() > 11 { p[11] != 0.0 } else { false };

    if brightness <= 0.0 {
        if glow_only {
            for c in px.iter_mut() {
                *c = [0.0, 0.0, 0.0, 0.0];
            }
        }
        return 0;
    }

    render_glow_zero_alloc(
        px,
        w,
        h,
        radius,
        brightness,
        threshold,
        smoothness,
        falloff,
        tint,
        aberration,
        glow_only,
    );

    0
}

/// Zero-heap-allocation, single-buffer downsample & sparse direct bilinear reconstruction.
fn render_glow_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    radius: f32,
    brightness: f32,
    threshold: f32,
    smoothness: f32,
    falloff: f32,
    tint: [f32; 4],
    aberration: f32,
    glow_only: bool,
) {
    // Target 240-480px buffer for real-time responsiveness and silky-smooth continuous gradients
    let downscale = if w <= 960 { 4usize } else { 8usize };

    let gw = (w / downscale).max(1);
    let gh = (h / downscale).max(1);
    let glow_slice_len = gw * gh * 4;

    // Slices in thread_local scratch buffer:
    // 0: buf0 (source bright pass, gw * gh * 4)
    // 1: buf1 (blur working, gw * gh * 4)
    // 2: accum (accumulated glow, gw * gh * 4)
    // 3: temp (horizontal blur scratch, gw * gh * 4)
    let total_scratch = glow_slice_len * 4;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < total_scratch {
            scratch.resize(total_scratch, 0.0);
        }

        let (buf0, rest1) = scratch.split_at_mut(glow_slice_len);
        let (buf1, rest2) = rest1.split_at_mut(glow_slice_len);
        let (accum, rest3) = rest2.split_at_mut(glow_slice_len);
        let temp = &mut rest3[..glow_slice_len];

        // 1. Threshold & Downsample pass into buf0
        let half_knee = smoothness * 0.5;
        let t_min = (threshold - half_knee).max(0.0);
        let t_max = threshold + half_knee;
        let inv_knee = 1.0 / (t_max - t_min).max(1e-5);

        for gy in 0..gh {
            let sy = (gy * downscale).min(h - 1);
            let src_row = sy * w;
            let dst_row = gy * gw;

            for gx in 0..gw {
                let sx = (gx * downscale).min(w - 1);
                let c = px[src_row + sx];
                let a = c[3];
                let out_idx = (dst_row + gx) * 4;

                if a <= 1e-5 {
                    buf0[out_idx] = 0.0;
                    buf0[out_idx + 1] = 0.0;
                    buf0[out_idx + 2] = 0.0;
                    buf0[out_idx + 3] = 0.0;
                    continue;
                }

                let inv_a = if (a - 1.0).abs() < 1e-4 { 1.0 } else { 1.0 / a };
                let r = c[0] * inv_a;
                let g = c[1] * inv_a;
                let b = c[2] * inv_a;
                let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;

                let factor = if lum <= t_min {
                    0.0
                } else if lum >= t_max {
                    1.0
                } else {
                    let t = (lum - t_min) * inv_knee;
                    t * t * (3.0 - 2.0 * t)
                };

                buf0[out_idx] = c[0] * factor;
                buf0[out_idx + 1] = c[1] * factor;
                buf0[out_idx + 2] = c[2] * factor;
                buf0[out_idx + 3] = a * factor;
            }
        }

        // 2. Dual-Octave Core + Bloom
        accum.fill(0.0);

        let base_r = ((radius / downscale as f32) * 0.35).round() as usize;
        let r_core = base_r.clamp(1, 16);
        let r_bloom = (base_r * 3).clamp(3, 48);

        // Octave 1: Core
        buf1.copy_from_slice(buf0);
        box_blur_o1(buf1, temp, gw, gh, r_core);
        box_blur_o1(buf1, temp, gw, gh, r_core);

        let w_core = 1.0f32;
        for i in 0..glow_slice_len {
            accum[i] += buf1[i] * w_core;
        }

        // Octave 2: Bloom
        box_blur_o1(buf1, temp, gw, gh, r_bloom);
        box_blur_o1(buf1, temp, gw, gh, r_bloom);

        let w_bloom = 1.0f32 / 3.0f32.powf(falloff);
        for i in 0..glow_slice_len {
            accum[i] += buf1[i] * w_bloom;
        }

        // 3. Direct Scanline Bilinear Reconstruction (Zero Intermediate Buffer)
        let tint_r = tint[0] * brightness;
        let tint_g = tint[1] * brightness;
        let tint_b = tint[2] * brightness;
        let tint_a = tint[3] * brightness;

        let has_aberration = aberration.abs() > 0.01;
        let max_gw = (gw - 1) as isize;
        let inv_downscale = 1.0 / downscale as f32;

        for gy in 0..gh {
            let next_gy = (gy + 1).min(gh - 1);
            let row0 = &accum[gy * gw * 4..(gy + 1) * gw * 4];
            let row1 = &accum[next_gy * gw * 4..(next_gy + 1) * gw * 4];

            let y_start = gy * downscale;
            let y_end = ((gy + 1) * downscale).min(h);

            for y in y_start..y_end {
                let yi = y - y_start;
                let ty = (yi as f32) * inv_downscale;
                let inv_ty = 1.0 - ty;

                let w0_r = inv_ty * tint_r;
                let w0_g = inv_ty * tint_g;
                let w0_b = inv_ty * tint_b;
                let w0_a = inv_ty * tint_a;

                let w1_r = ty * tint_r;
                let w1_g = ty * tint_g;
                let w1_b = ty * tint_b;
                let w1_a = ty * tint_a;

                let px_row = &mut px[y * w..(y + 1) * w];

                for gx in 0..gw {
                    let next_gx = (gx + 1).min(gw - 1);
                    let x0 = gx * downscale;
                    let x1 = ((gx + 1) * downscale).min(w);
                    let span = x1 - x0;
                    if span == 0 {
                        continue;
                    }

                    let (left_r, left_g, left_b, left_a, right_r, right_g, right_b, right_a) =
                        if has_aberration {
                            let center_gx = (gx as isize) - (gw as isize / 2);
                            let shift = (center_gx * (aberration as isize)) / (gw as isize * 2);
                            let rx0 = ((gx as isize + shift).clamp(0, max_gw) as usize) * 4;
                            let bx0 = ((gx as isize - shift).clamp(0, max_gw) as usize) * 4;
                            let rx1 = (((next_gx as isize) + shift).clamp(0, max_gw) as usize) * 4;
                            let bx1 = (((next_gx as isize) - shift).clamp(0, max_gw) as usize) * 4;
                            let g0 = gx * 4;
                            let g1 = next_gx * 4;
                            (
                                row0[rx0] * w0_r + row1[rx0] * w1_r,
                                row0[g0 + 1] * w0_g + row1[g0 + 1] * w1_g,
                                row0[bx0 + 2] * w0_b + row1[bx0 + 2] * w1_b,
                                row0[g0 + 3] * w0_a + row1[g0 + 3] * w1_a,
                                row0[rx1] * w0_r + row1[rx1] * w1_r,
                                row0[g1 + 1] * w0_g + row1[g1 + 1] * w1_g,
                                row0[bx1 + 2] * w0_b + row1[bx1 + 2] * w1_b,
                                row0[g1 + 3] * w0_a + row1[g1 + 3] * w1_a,
                            )
                        } else {
                            let i0 = gx * 4;
                            let i1 = next_gx * 4;
                            (
                                row0[i0] * w0_r + row1[i0] * w1_r,
                                row0[i0 + 1] * w0_g + row1[i0 + 1] * w1_g,
                                row0[i0 + 2] * w0_b + row1[i0 + 2] * w1_b,
                                row0[i0 + 3] * w0_a + row1[i0 + 3] * w1_a,
                                row0[i1] * w0_r + row1[i1] * w1_r,
                                row0[i1 + 1] * w0_g + row1[i1 + 1] * w1_g,
                                row0[i1 + 2] * w0_b + row1[i1 + 2] * w1_b,
                                row0[i1 + 3] * w0_a + row1[i1 + 3] * w1_a,
                            )
                        };

                    // Sparse skip: if glow is zero in this span and not glow_only, skip entire span
                    if !glow_only && left_a <= 1e-5 && right_a <= 1e-5 {
                        continue;
                    }

                    if span == 8 {
                        let dr = (right_r - left_r) * 0.125;
                        let dg = (right_g - left_g) * 0.125;
                        let db = (right_b - left_b) * 0.125;
                        let da = (right_a - left_a) * 0.125;

                        let mut cur_r = left_r;
                        let mut cur_g = left_g;
                        let mut cur_b = left_b;
                        let mut cur_a = left_a;

                        if glow_only {
                            for p in &mut px_row[x0..x0 + 8] {
                                *p = [cur_r, cur_g, cur_b, cur_a.min(1.0)];
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        } else {
                            for p in &mut px_row[x0..x0 + 8] {
                                p[0] += cur_r;
                                p[1] += cur_g;
                                p[2] += cur_b;
                                p[3] = (p[3] + cur_a).min(1.0);
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        }
                    } else if span == 4 {
                        let dr = (right_r - left_r) * 0.25;
                        let dg = (right_g - left_g) * 0.25;
                        let db = (right_b - left_b) * 0.25;
                        let da = (right_a - left_a) * 0.25;

                        let mut cur_r = left_r;
                        let mut cur_g = left_g;
                        let mut cur_b = left_b;
                        let mut cur_a = left_a;

                        if glow_only {
                            for p in &mut px_row[x0..x0 + 4] {
                                *p = [cur_r, cur_g, cur_b, cur_a.min(1.0)];
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        } else {
                            for p in &mut px_row[x0..x0 + 4] {
                                p[0] += cur_r;
                                p[1] += cur_g;
                                p[2] += cur_b;
                                p[3] = (p[3] + cur_a).min(1.0);
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        }
                    } else {
                        let inv_span = 1.0 / span as f32;
                        let dr = (right_r - left_r) * inv_span;
                        let dg = (right_g - left_g) * inv_span;
                        let db = (right_b - left_b) * inv_span;
                        let da = (right_a - left_a) * inv_span;

                        let mut cur_r = left_r;
                        let mut cur_g = left_g;
                        let mut cur_b = left_b;
                        let mut cur_a = left_a;

                        if glow_only {
                            for p in &mut px_row[x0..x0 + span] {
                                *p = [cur_r, cur_g, cur_b, cur_a.min(1.0)];
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        } else {
                            for p in &mut px_row[x0..x0 + span] {
                                p[0] += cur_r;
                                p[1] += cur_g;
                                p[2] += cur_b;
                                p[3] = (p[3] + cur_a).min(1.0);
                                cur_r += dr;
                                cur_g += dg;
                                cur_b += db;
                                cur_a += da;
                            }
                        }
                    }
                }
            }
        }
    });
}

/// O(1) Sliding-window separable box blur with unrolled inner loop.
fn box_blur_o1(data: &mut [f32], temp: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w < 2 || h < 2 {
        return;
    }

    let inv_window = 1.0 / (2 * r + 1) as f32;

    // Horizontal pass: data -> temp
    for y in 0..h {
        let row_offset = y * w;
        let mut acc = [0.0f32; 4];

        for x in 0..=r {
            let sx = x.min(w - 1);
            let idx = (row_offset + sx) * 4;
            acc[0] += data[idx];
            acc[1] += data[idx + 1];
            acc[2] += data[idx + 2];
            acc[3] += data[idx + 3];
        }
        let edge_idx = row_offset * 4;
        let rf = r as f32;
        acc[0] += rf * data[edge_idx];
        acc[1] += rf * data[edge_idx + 1];
        acc[2] += rf * data[edge_idx + 2];
        acc[3] += rf * data[edge_idx + 3];

        for x in 0..w {
            let out_idx = (row_offset + x) * 4;
            temp[out_idx] = acc[0] * inv_window;
            temp[out_idx + 1] = acc[1] * inv_window;
            temp[out_idx + 2] = acc[2] * inv_window;
            temp[out_idx + 3] = acc[3] * inv_window;

            let add_x = (x + r + 1).min(w - 1);
            let sub_x = if x >= r { x - r } else { 0 };

            let add_idx = (row_offset + add_x) * 4;
            let sub_idx = (row_offset + sub_x) * 4;

            acc[0] += data[add_idx] - data[sub_idx];
            acc[1] += data[add_idx + 1] - data[sub_idx + 1];
            acc[2] += data[add_idx + 2] - data[sub_idx + 2];
            acc[3] += data[add_idx + 3] - data[sub_idx + 3];
        }
    }

    // Vertical pass: temp -> data
    for x in 0..w {
        let mut acc = [0.0f32; 4];

        for y in 0..=r {
            let sy = y.min(h - 1);
            let idx = (sy * w + x) * 4;
            acc[0] += temp[idx];
            acc[1] += temp[idx + 1];
            acc[2] += temp[idx + 2];
            acc[3] += temp[idx + 3];
        }
        let edge_idx = x * 4;
        let rf = r as f32;
        acc[0] += rf * temp[edge_idx];
        acc[1] += rf * temp[edge_idx + 1];
        acc[2] += rf * temp[edge_idx + 2];
        acc[3] += rf * temp[edge_idx + 3];

        for y in 0..h {
            let out_idx = (y * w + x) * 4;
            data[out_idx] = acc[0] * inv_window;
            data[out_idx + 1] = acc[1] * inv_window;
            data[out_idx + 2] = acc[2] * inv_window;
            data[out_idx + 3] = acc[3] * inv_window;

            let add_y = (y + r + 1).min(h - 1);
            let sub_y = if y >= r { y - r } else { 0 };

            let add_idx = (add_y * w + x) * 4;
            let sub_idx = (sub_y * w + x) * 4;

            acc[0] += temp[add_idx] - temp[sub_idx];
            acc[1] += temp[add_idx + 1] - temp[sub_idx + 1];
            acc[2] += temp[add_idx + 2] - temp[sub_idx + 2];
            acc[3] += temp[add_idx + 3] - temp[sub_idx + 3];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        assert_eq!(ec_api_version(), 1);
        assert!(MANIFEST.contains("by qwertyonek"));
    }

    #[test]
    fn test_render_glow_smoke() {
        let mut pixels = vec![[1.0f32, 1.0f32, 1.0f32, 1.0f32]; 64 * 64];
        let params = [
            0.0f64,  // about
            50.0f64, // radius
            1.0f64,  // brightness
            20.0f64, // threshold
            30.0f64, // smoothness
            1.8f64,  // falloff
            1.0f64, 1.0f64, 1.0f64, 1.0f64, // tint
            15.0f64, // aberration
            0.0f64,  // glow_only
        ];

        let ret = render_frame(&mut pixels, 64, 64, &params, 1.0);
        assert_eq!(ret, 0);
        assert!(pixels[0][0] >= 1.0);
    }
}
