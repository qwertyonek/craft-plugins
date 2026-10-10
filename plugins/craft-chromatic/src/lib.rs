//! CraftChromatic: High-performance optical lens dispersion and chromatic aberration.
//! Zero-allocation subpixel bilinear sampling with radial lens falloff and directional prism modes.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.chromatic",
  "name": "Craft Chromatic",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Optical lens dispersion and RGB chromatic aberration.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "mode", "name": "Mode", "type": "popup", "options": ["Radial (Lens)", "Linear (Directional)"], "default": 0},
    {"id": "amount", "name": "Aberration Amount", "type": "slider", "default": 20.0, "min": -100.0, "max": 100.0, "sliderMax": 50.0, "decimals": 1},
    {"id": "angle", "name": "Angle", "type": "angle", "default": 0.0},
    {"id": "center", "name": "Center", "type": "point", "default": [0.5, 0.5]},
    {"id": "falloff", "name": "Falloff", "type": "slider", "default": 2.0, "min": 0.5, "max": 4.0, "decimals": 2},
    {"id": "blur", "name": "Fringe Softness", "type": "slider", "default": 0.0, "min": 0.0, "max": 20.0, "sliderMax": 10.0, "decimals": 1},
    {"id": "tint", "name": "Tint", "type": "color", "default": [1.0, 1.0, 1.0, 1.0]}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
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
    if nparams < 12 || width <= 0 || height <= 0 {
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
    if p.len() < 12 || w == 0 || h == 0 {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };
    let mode = p[1].round() as u32; // 0 = Radial, 1 = Linear
    let amount = (p[2] as f32) * effective_scale;
    let angle = p[3] as f32;
    let center_x = p[4] as f32;
    let center_y = p[5] as f32;
    let falloff = (p[6] as f32).clamp(0.5, 4.0);
    let blur = (p[7] as f32).max(0.0) * effective_scale;
    let tint = [
        p[8].max(0.0) as f32,
        p[9].max(0.0) as f32,
        p[10].max(0.0) as f32,
        p[11].clamp(0.0, 1.0) as f32,
    ];

    if amount.abs() < 1e-4 && (tint[0] - 1.0).abs() < 1e-4 && (tint[1] - 1.0).abs() < 1e-4 && (tint[2] - 1.0).abs() < 1e-4 {
        return 0;
    }

    render_chromatic_zero_alloc(
        px,
        w,
        h,
        mode,
        amount,
        angle,
        center_x,
        center_y,
        falloff,
        blur,
        tint,
    );

    0
}

fn render_chromatic_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    mode: u32,
    amount: f32,
    angle: f32,
    center_x: f32,
    center_y: f32,
    falloff: f32,
    blur: f32,
    tint: [f32; 4],
) {
    let total_pixels = w * h;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < total_pixels {
            scratch.resize(total_pixels, [0.0; 4]);
        }
        let src = &mut scratch[..total_pixels];
        src.copy_from_slice(px);

        if mode == 1 {
            // Linear (Directional) Mode
            let angle_rad = angle.to_radians();
            let dir_x = angle_rad.cos() * (amount * 0.25);
            let dir_y = angle_rad.sin() * (amount * 0.25);

            render_linear(px, src, w, h, dir_x, dir_y, blur, tint);
        } else {
            // Radial (Optical Lens) Mode
            render_radial(px, src, w, h, amount, center_x, center_y, falloff, blur, tint);
        }
    });
}

fn render_linear(
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    w: usize,
    h: usize,
    dir_x: f32,
    dir_y: f32,
    blur: f32,
    tint: [f32; 4],
) {
    let src_ptr = src.as_ptr();
    let max_x = (w - 1).max(1);
    let max_y = (h - 1).max(1);
    let has_blur = blur > 0.5;
    let b_offset = blur * 0.5;

    for y in 0..h {
        let row_idx = y * w;
        let y_f = y as f32;

        let ry = y_f + dir_y;
        let by = y_f - dir_y;

        let dst_row = &mut dst[row_idx..row_idx + w];
        let src_row = &src[row_idx..row_idx + w];

        for x in 0..w {
            let x_f = x as f32;
            let orig = src_row[x];

            let rx = x_f + dir_x;
            let bx = x_f - dir_x;

            let r = if has_blur {
                (unsafe {
                    sample_bilinear_fast(src_ptr, w, max_x, max_y, rx - b_offset, ry, 0)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, rx, ry, 0)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, rx + b_offset, ry, 0)
                }) / 3.0
            } else {
                unsafe { sample_bilinear_fast(src_ptr, w, max_x, max_y, rx, ry, 0) }
            };

            let g = orig[1];

            let b = if has_blur {
                (unsafe {
                    sample_bilinear_fast(src_ptr, w, max_x, max_y, bx - b_offset, by, 2)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, bx, by, 2)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, bx + b_offset, by, 2)
                }) / 3.0
            } else {
                unsafe { sample_bilinear_fast(src_ptr, w, max_x, max_y, bx, by, 2) }
            };

            let a = orig[3];

            dst_row[x] = [
                r * tint[0],
                g * tint[1],
                b * tint[2],
                (a * tint[3]).min(1.0),
            ];
        }
    }
}

fn render_radial(
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    w: usize,
    h: usize,
    amount: f32,
    center_x: f32,
    center_y: f32,
    falloff: f32,
    blur: f32,
    tint: [f32; 4],
) {
    let src_ptr = src.as_ptr();
    let max_x = (w - 1).max(1);
    let max_y = (h - 1).max(1);

    let max_radius = ((w.max(h) as f32) * 0.5).max(1.0);
    let max_radius_sq = max_radius * max_radius * 2.0;
    let inv_max_radius_sq = 1.0 / max_radius_sq;
    let lut_scale = inv_max_radius_sq * 256.0;
    let strength = amount * 0.005;
    let has_blur = blur > 0.5;
    let b_offset = blur * 0.3;

    // Precompute 512-entry LUT to completely eliminate sqrt and powf from inner loop
    let mut lut = [0.0f32; 512];
    for (i, v) in lut.iter_mut().enumerate() {
        let norm_r_sq = (i as f32) / 256.0;
        let norm_r = norm_r_sq.sqrt();
        *v = norm_r.powf(falloff) * strength;
    }

    for y in 0..h {
        let row_idx = y * w;
        let y_f = y as f32;
        let dy = y_f - center_y;
        let dy2 = dy * dy;

        let dst_row = &mut dst[row_idx..row_idx + w];
        let src_row = &src[row_idx..row_idx + w];

        for x in 0..w {
            let x_f = x as f32;
            let dx = x_f - center_x;
            let dist2 = dx * dx + dy2;

            let lut_idx = ((dist2 * lut_scale) as usize).min(511);
            let factor = lut[lut_idx];

            let orig = src_row[x];

            if factor.abs() < 1e-5 {
                dst_row[x] = [
                    orig[0] * tint[0],
                    orig[1] * tint[1],
                    orig[2] * tint[2],
                    (orig[3] * tint[3]).min(1.0),
                ];
                continue;
            }

            let shift_x = dx * factor;
            let shift_y = dy * factor;

            let rx = x_f + shift_x;
            let ry = y_f + shift_y;

            let bx = x_f - shift_x;
            let by = y_f - shift_y;

            let r = if has_blur {
                (unsafe {
                    sample_bilinear_fast(src_ptr, w, max_x, max_y, rx - b_offset, ry, 0)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, rx, ry, 0)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, rx + b_offset, ry, 0)
                }) / 3.0
            } else {
                unsafe { sample_bilinear_fast(src_ptr, w, max_x, max_y, rx, ry, 0) }
            };

            let g = orig[1];

            let b = if has_blur {
                (unsafe {
                    sample_bilinear_fast(src_ptr, w, max_x, max_y, bx - b_offset, by, 2)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, bx, by, 2)
                        + sample_bilinear_fast(src_ptr, w, max_x, max_y, bx + b_offset, by, 2)
                }) / 3.0
            } else {
                unsafe { sample_bilinear_fast(src_ptr, w, max_x, max_y, bx, by, 2) }
            };

            let a = orig[3];

            dst_row[x] = [
                r * tint[0],
                g * tint[1],
                b * tint[2],
                (a * tint[3]).min(1.0),
            ];
        }
    }
}

#[inline(always)]
unsafe fn sample_bilinear_fast(
    src_ptr: *const [f32; 4],
    w: usize,
    max_x: usize,
    max_y: usize,
    x: f32,
    y: f32,
    chan: usize,
) -> f32 {
    let ix = x as usize;
    let iy = y as usize;

    if ix < max_x && iy < max_y && x >= 0.0 && y >= 0.0 {
        let fx = x - ix as f32;
        let fy = y - iy as f32;

        unsafe {
            let p = src_ptr.add(iy * w + ix);
            let p00 = (*p)[chan];
            let p10 = (*p.add(1))[chan];
            let p01 = (*p.add(w))[chan];
            let p11 = (*p.add(w + 1))[chan];

            let top = p00 + (p10 - p00) * fx;
            let bot = p01 + (p11 - p01) * fx;

            top + (bot - top) * fy
        }
    } else {
        let cx = x.clamp(0.0, max_x as f32);
        let cy = y.clamp(0.0, max_y as f32);

        let x0 = cx as usize;
        let y0 = cy as usize;
        let x1 = (x0 + 1).min(max_x);
        let y1 = (y0 + 1).min(max_y);

        let fx = cx - x0 as f32;
        let fy = cy - y0 as f32;

        unsafe {
            let p00 = (*src_ptr.add(y0 * w + x0))[chan];
            let p10 = (*src_ptr.add(y0 * w + x1))[chan];
            let p01 = (*src_ptr.add(y1 * w + x0))[chan];
            let p11 = (*src_ptr.add(y1 * w + x1))[chan];

            let top = p00 + (p10 - p00) * fx;
            let bot = p01 + (p11 - p01) * fx;

            top + (bot - top) * fy
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
        assert!(MANIFEST.contains("Craft Chromatic"));
    }

    #[test]
    fn test_chromatic_smoke() {
        let mut pixels = vec![[0.5f32, 0.5f32, 0.5f32, 1.0f32]; 64 * 64];
        let params = [
            0.0f64,  // about
            0.0f64,  // mode (Radial)
            20.0f64, // amount
            0.0f64,  // angle
            32.0f64, 32.0f64, // center
            2.0f64,  // falloff
            0.0f64,  // blur
            1.0f64, 1.0f64, 1.0f64, 1.0f64, // tint
        ];

        let ret = render_frame(&mut pixels, 64, 64, &params, 1.0);
        assert_eq!(ret, 0);
    }
}
