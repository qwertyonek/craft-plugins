//! CraftDisplacer: Advanced subpixel displacement mapping, glass refraction & chromatic warp.
//! Zero-allocation procedural liquid turbulence and luminance normal-vector displacement.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.displacer",
  "name": "Craft Displacer",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Glass refraction, fluid turbulence displacement & chromatic warp.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "amount_x", "name": "Max Displace X", "type": "slider", "default": 25.0, "min": -200.0, "max": 200.0, "sliderMax": 100.0, "decimals": 1},
    {"id": "amount_y", "name": "Max Displace Y", "type": "slider", "default": 25.0, "min": -200.0, "max": 200.0, "sliderMax": 100.0, "decimals": 1},
    {"id": "mode", "name": "Displacement Source", "type": "popup", "options": ["Luminance Gradients (Glass/Refract)", "Procedural Fluid Turbulence", "Radial Ripple / Shockwave"], "default": 0},
    {"id": "scale", "name": "Noise / Wave Scale", "type": "slider", "default": 10.0, "min": 0.5, "max": 100.0, "sliderMax": 50.0, "decimals": 1},
    {"id": "evolution", "name": "Turbulence Evolution", "type": "angle", "default": 0.0},
    {"id": "chromatic_split", "name": "Chromatic Dispersion", "type": "slider", "default": 15.0, "min": 0.0, "max": 50.0, "sliderMax": 30.0, "decimals": 1},
    {"id": "smoothness", "name": "Refraction Smoothness", "type": "slider", "default": 2.0, "min": 1.0, "max": 10.0, "decimals": 1},
    {"id": "center", "name": "Center (Ripple Mode)", "type": "point", "default": [0.5, 0.5]},
    {"id": "wrap_mode", "name": "Edge Handling", "type": "popup", "options": ["Mirror (Reflect)", "Clamp (Repeat Edge)", "Zero (Transparent)"], "default": 0}
  ]
}"#;

thread_local! {
    static BUF: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static SCRATCH: RefCell<Vec<[f32; 4]>> = const { RefCell::new(Vec::new()) };
    static GRAD_MAP: RefCell<Vec<[f32; 2]>> = const { RefCell::new(Vec::new()) };
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
    // p[1]: amount_x
    // p[2]: amount_y
    // p[3]: mode (0: Luminance Glass, 1: Turbulence, 2: Ripple)
    // p[4]: noise scale
    // p[5]: evolution angle (degrees)
    // p[6]: chromatic_split
    // p[7]: smoothness (Sobel delta)
    // p[8], p[9]: center point [x, y] in frame pixels
    // p[10]: wrap_mode (0: Mirror, 1: Clamp, 2: Zero)
    let amount_x = (p[1] as f32) * effective_scale;
    let amount_y = (p[2] as f32) * effective_scale;
    let mode = p[3].round() as u32;
    let noise_scale = (p[4] as f32).max(0.1);
    let evolution = (p[5] as f32).to_radians();
    let chromatic = (p[6] as f32) * effective_scale;
    let smoothness = (p[7] as f32).clamp(1.0, 10.0) * effective_scale;
    let (center_x, center_y) = if p.len() >= 10 {
        (p[8] as f32, p[9] as f32)
    } else {
        ((w as f32) * 0.5, (h as f32) * 0.5)
    };
    let wrap_mode = if p.len() >= 11 { p[10].round() as u32 } else { 0 };

    if amount_x.abs() < 1e-4 && amount_y.abs() < 1e-4 {
        return 0; // Identity passthrough
    }

    render_displace_zero_alloc(
        px,
        w,
        h,
        time,
        amount_x,
        amount_y,
        mode,
        noise_scale,
        evolution,
        chromatic,
        smoothness,
        center_x,
        center_y,
        wrap_mode,
    );

    0
}

#[inline(always)]
fn get_luma(c: [f32; 4]) -> f32 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

/// Fast pseudo-random 2D simplex/gradient noise
#[inline(always)]
fn hash_noise_2d(x: f32, y: f32, z: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = x - (xi as f32);
    let yf = y - (yi as f32);

    let sx = xf * xf * (3.0 - 2.0 * xf);
    let sy = yf * yf * (3.0 - 2.0 * yf);

    let h = |ix: i32, iy: i32| -> f32 {
        let mut n = (ix as u32).wrapping_mul(0x374765a9)
            ^ (iy as u32).wrapping_mul(0x632be5ab)
            ^ ((z * 100.0) as u32).wrapping_mul(0x8510369f);
        n = (n ^ (n >> 15)).wrapping_mul(0x7feb352d);
        ((n & 0xffff) as f32) * (2.0 / 65535.0) - 1.0
    };

    let g00 = h(xi, yi);
    let g10 = h(xi + 1, yi);
    let g01 = h(xi, yi + 1);
    let g11 = h(xi + 1, yi + 1);

    let ix0 = (1.0 - sx) * g00 + sx * g10;
    let ix1 = (1.0 - sx) * g01 + sx * g11;

    (1.0 - sy) * ix0 + sy * ix1
}

fn render_displace_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    time: f64,
    amount_x: f32,
    amount_y: f32,
    mode: u32,
    noise_scale: f32,
    evolution: f32,
    chromatic: f32,
    smoothness: f32,
    center_x: f32,
    center_y: f32,
    wrap_mode: u32,
) {
    let total_pixels = w * h;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < total_pixels {
            scratch.resize(total_pixels, [0.0; 4]);
        }
        let src = &mut scratch[..total_pixels];
        src.copy_from_slice(px);

        GRAD_MAP.with(|gm| {
            let mut grad_map = gm.borrow_mut();
            if grad_map.len() < total_pixels {
                grad_map.resize(total_pixels, [0.0; 2]);
            }
            let grads = &mut grad_map[..total_pixels];

            let max_x = (w - 1).max(1);
            let max_y = (h - 1).max(1);
            let src_ptr = src.as_ptr();

            // Compute Displacement Vector Field (grads)
            match mode {
                0 => {
                    // Mode 0: Luminance Glass Refraction (Sobel normal vectors)
                    let delta = smoothness.max(1.0);
                    let inv_2delta = 1.0 / (2.0 * delta);

                    for y in 0..h {
                        let y_f = y as f32;
                        let y_prev = (y_f - delta).clamp(0.0, max_y as f32);
                        let y_next = (y_f + delta).clamp(0.0, max_y as f32);
                        let row_idx = y * w;

                        for x in 0..w {
                            let x_f = x as f32;
                            let x_prev = (x_f - delta).clamp(0.0, max_x as f32);
                            let x_next = (x_f + delta).clamp(0.0, max_x as f32);

                            let luma_l = get_luma(sample_clamped(src_ptr, w, max_x, max_y, x_prev, y_f));
                            let luma_r = get_luma(sample_clamped(src_ptr, w, max_x, max_y, x_next, y_f));
                            let luma_u = get_luma(sample_clamped(src_ptr, w, max_x, max_y, x_f, y_prev));
                            let luma_d = get_luma(sample_clamped(src_ptr, w, max_x, max_y, x_f, y_next));

                            let dx = (luma_r - luma_l) * inv_2delta * 2.0;
                            let dy = (luma_d - luma_u) * inv_2delta * 2.0;

                            grads[row_idx + x] = [dx * amount_x, dy * amount_y];
                        }
                    }
                }
                1 => {
                    // Mode 1: Procedural Fluid Turbulence
                    let inv_scale = (1.0 / (noise_scale * 15.0)).max(0.001);
                    let t_evol = (time as f32) * 1.5 + evolution;

                    for y in 0..h {
                        let row_idx = y * w;
                        let ny = (y as f32) * inv_scale;

                        for x in 0..w {
                            let nx = (x as f32) * inv_scale;
                            // 2-octave turbulence
                            let n1 = hash_noise_2d(nx, ny, t_evol);
                            let n2 = hash_noise_2d(nx * 2.1, ny * 2.1, t_evol + 17.3) * 0.5;
                            let n_x = n1 + n2;

                            let m1 = hash_noise_2d(nx + 103.5, ny + 91.2, t_evol);
                            let m2 = hash_noise_2d((nx + 103.5) * 2.1, (ny + 91.2) * 2.1, t_evol + 17.3) * 0.5;
                            let n_y = m1 + m2;

                            grads[row_idx + x] = [n_x * amount_x * 0.5, n_y * amount_y * 0.5];
                        }
                    }
                }
                _ => {
                    // Mode 2: Radial Ripple / Shockwave
                    let inv_wave_len = (1.0 / (noise_scale * 10.0)).max(0.001);
                    let phase = (time as f32) * 6.0 + evolution;

                    for y in 0..h {
                        let row_idx = y * w;
                        let dy = (y as f32) - center_y;

                        for x in 0..w {
                            let dx = (x as f32) - center_x;
                            let dist = (dx * dx + dy * dy).sqrt();

                            if dist > 1e-4 {
                                let norm_x = dx / dist;
                                let norm_y = dy / dist;
                                let wave = (dist * inv_wave_len - phase).sin();
                                let falloff = 1.0 / (1.0 + dist * 0.002);
                                let d = wave * falloff;

                                grads[row_idx + x] = [norm_x * d * amount_x, norm_y * d * amount_y];
                            } else {
                                grads[row_idx + x] = [0.0, 0.0];
                            }
                        }
                    }
                }
            }

            // Subpixel Resampling with optional Chromatic Separation
            let has_chroma = chromatic > 0.001;
            let chroma_factor = chromatic * 0.2;

            for y in 0..h {
                let row_idx = y * w;
                let y_f = y as f32;
                let dst_row = &mut px[row_idx..row_idx + w];

                for x in 0..w {
                    let x_f = x as f32;
                    let disp = grads[row_idx + x];

                    if !has_chroma {
                        // Monolithic channel warp
                        let sx = x_f + disp[0];
                        let sy = y_f + disp[1];
                        dst_row[x] = sample_bilinear_rgba(src_ptr, w, max_x, max_y, sx, sy, wrap_mode);
                    } else {
                        // High-end Chromatic Dispersion split
                        let rx = x_f + disp[0] * (1.0 + chroma_factor);
                        let ry = y_f + disp[1] * (1.0 + chroma_factor);

                        let gx = x_f + disp[0];
                        let gy = y_f + disp[1];

                        let bx = x_f + disp[0] * (1.0 - chroma_factor);
                        let by = y_f + disp[1] * (1.0 - chroma_factor);

                        let samp_r = sample_bilinear_rgba(src_ptr, w, max_x, max_y, rx, ry, wrap_mode);
                        let samp_g = sample_bilinear_rgba(src_ptr, w, max_x, max_y, gx, gy, wrap_mode);
                        let samp_b = sample_bilinear_rgba(src_ptr, w, max_x, max_y, bx, by, wrap_mode);

                        dst_row[x] = [
                            samp_r[0],
                            samp_g[1],
                            samp_b[2],
                            samp_g[3],
                        ];
                    }
                }
            }
        });
    });
}

#[inline(always)]
fn sample_clamped(
    src_ptr: *const [f32; 4],
    stride: usize,
    max_x: usize,
    max_y: usize,
    x: f32,
    y: f32,
) -> [f32; 4] {
    let xi = (x.round() as usize).min(max_x);
    let yi = (y.round() as usize).min(max_y);
    unsafe { *src_ptr.add(yi * stride + xi) }
}

#[inline(always)]
fn sample_bilinear_rgba(
    src_ptr: *const [f32; 4],
    stride: usize,
    max_x: usize,
    max_y: usize,
    x: f32,
    y: f32,
    wrap_mode: u32,
) -> [f32; 4] {
    let (fx, fy) = (x.floor(), y.floor());
    let x0 = fx as i32;
    let y0 = fy as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;

    let wx1 = x - fx;
    let wx0 = 1.0 - wx1;
    let wy1 = y - fy;
    let wy0 = 1.0 - wy1;

    // Fast interior path
    if x0 >= 0 && x1 <= max_x as i32 && y0 >= 0 && y1 <= max_y as i32 {
        let (x0, x1, y0, y1) = (x0 as usize, x1 as usize, y0 as usize, y1 as usize);
        unsafe {
            let p00 = *src_ptr.add(y0 * stride + x0);
            let p10 = *src_ptr.add(y0 * stride + x1);
            let p01 = *src_ptr.add(y1 * stride + x0);
            let p11 = *src_ptr.add(y1 * stride + x1);

            let w00 = wx0 * wy0;
            let w10 = wx1 * wy0;
            let w01 = wx0 * wy1;
            let w11 = wx1 * wy1;

            return [
                p00[0] * w00 + p10[0] * w10 + p01[0] * w01 + p11[0] * w11,
                p00[1] * w00 + p10[1] * w10 + p01[1] * w01 + p11[1] * w11,
                p00[2] * w00 + p10[2] * w10 + p01[2] * w01 + p11[2] * w11,
                p00[3] * w00 + p10[3] * w10 + p01[3] * w01 + p11[3] * w11,
            ];
        }
    }

    let resolve_coord = |coord: i32, max_val: usize| -> Option<usize> {
        match wrap_mode {
            0 => {
                let m = (max_val * 2) as i32;
                let mut c = coord % m;
                if c < 0 { c += m; }
                if c > max_val as i32 { c = m - c; }
                Some(c as usize)
            }
            1 => Some(coord.clamp(0, max_val as i32) as usize),
            _ => {
                if coord < 0 || coord > max_val as i32 { None } else { Some(coord as usize) }
            }
        }
    };

    let sample_wrapped = |cx: i32, cy: i32| -> [f32; 4] {
        match (resolve_coord(cx, max_x), resolve_coord(cy, max_y)) {
            (Some(rx), Some(ry)) => unsafe { *src_ptr.add(ry * stride + rx) },
            _ => [0.0; 4],
        }
    };

    let p00 = sample_wrapped(x0, y0);
    let p10 = sample_wrapped(x1, y0);
    let p01 = sample_wrapped(x0, y1);
    let p11 = sample_wrapped(x1, y1);

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
