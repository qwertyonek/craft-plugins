//! CraftCameraShake: Cinematic organic handheld camera shake and impact trauma.
//! Procedural multi-octave gradient noise with subpixel translation, rotation, auto-scale, and motion blur.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.shake",
  "name": "Craft Camera Shake",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Cinematic organic handheld camera shake, micro-jitter & impact trauma.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "amplitude", "name": "Shake Amplitude", "type": "slider", "default": 15.0, "min": 0.0, "max": 100.0, "sliderMax": 50.0, "decimals": 1},
    {"id": "frequency", "name": "Frequency (Hz)", "type": "slider", "default": 3.0, "min": 0.1, "max": 25.0, "sliderMax": 15.0, "decimals": 1},
    {"id": "rotation", "name": "Rotational Shake", "type": "slider", "default": 2.0, "min": 0.0, "max": 30.0, "sliderMax": 15.0, "decimals": 2},
    {"id": "octaves", "name": "Motion Complexity", "type": "slider", "default": 3.0, "min": 1.0, "max": 4.0, "decimals": 0},
    {"id": "seed", "name": "Random Seed", "type": "slider", "default": 1337.0, "min": 0.0, "max": 99999.0, "decimals": 0},
    {"id": "motion_blur", "name": "Motion Blur Samples", "type": "popup", "options": ["Off", "Fast (3 Samples)", "Smooth (5 Samples)"], "default": 1},
    {"id": "shutter", "name": "Shutter Angle", "type": "slider", "default": 180.0, "min": 0.0, "max": 360.0, "sliderMax": 360.0, "decimals": 0},
    {"id": "wrap_mode", "name": "Edge Handling", "type": "popup", "options": ["Mirror (Reflect)", "Clamp (Repeat Edge)", "Zero (Transparent)"], "default": 0},
    {"id": "auto_scale", "name": "Auto-Scale Compensation", "type": "checkbox", "default": true}
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

    // Params layout:
    // p[0]: about popup
    // p[1]: amplitude
    // p[2]: frequency
    // p[3]: rotation
    // p[4]: octaves
    // p[5]: seed
    // p[6]: motion_blur popup (0: Off, 1: 3 samples, 2: 5 samples)
    // p[7]: shutter angle (0..360)
    // p[8]: wrap_mode (0: Mirror, 1: Clamp, 2: Transparent)
    // p[9]: auto_scale (bool: 0.0 or 1.0)
    let amplitude = (p[1] as f32) * effective_scale;
    let frequency = (p[2] as f32).max(0.01);
    let rotation_deg = p[3] as f32;
    let octaves = (p[4].round() as usize).clamp(1, 4);
    let seed = p[5] as f32;
    let mb_mode = p[6].round() as usize;
    let shutter = (p[7] as f32).clamp(0.0, 360.0);
    let wrap_mode = p[8].round() as u32;
    let auto_scale = p[9] > 0.5;

    if amplitude < 0.001 && rotation_deg.abs() < 0.001 {
        return 0; // No shake needed
    }

    render_shake_zero_alloc(
        px,
        w,
        h,
        time,
        amplitude,
        frequency,
        rotation_deg,
        octaves,
        seed,
        mb_mode,
        shutter,
        wrap_mode,
        auto_scale,
    );

    0
}

/// Computes smooth, organic 1D multi-octave gradient noise with analytical cubic easing.
#[inline(always)]
fn smooth_noise_1d(t: f32, seed_offset: f32) -> f32 {
    let p = t + seed_offset;
    let i0 = p.floor() as i32;
    let f = p - (i0 as f32);

    // Smoothstep cubic S-curve: 3f^2 - 2f^3
    let s = f * f * (3.0 - 2.0 * f);

    // Deterministic pseudo-random gradient hash without division
    let g0 = hash_gradient(i0);
    let g1 = hash_gradient(i0 + 1);

    (1.0 - s) * g0 + s * g1
}

#[inline(always)]
fn hash_gradient(n: i32) -> f32 {
    let mut x = (n as u32).wrapping_mul(0x27d4eb2d);
    x = (x ^ (x >> 15)).wrapping_mul(0x8510369f);
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = x ^ (x >> 15);
    // Maps to range [-1.0, 1.0]
    ((x & 0xffff) as f32) * (2.0 / 65535.0) - 1.0
}

#[inline(always)]
fn multi_octave_noise(t: f32, seed_offset: f32, octaves: usize) -> f32 {
    let mut val = 0.0f32;
    let mut freq = 1.0f32;
    let mut amp = 1.0f32;
    let mut total_amp = 0.0f32;

    for i in 0..octaves {
        let octave_seed = seed_offset + (i as f32) * 137.5;
        val += smooth_noise_1d(t * freq, octave_seed) * amp;
        total_amp += amp;
        freq *= 2.15; // Non-harmonic lacunarity
        amp *= 0.5;   // Persistence
    }

    if total_amp > 0.0 { val / total_amp } else { 0.0 }
}

struct Transform2D {
    cos_theta: f32,
    sin_theta: f32,
    tx: f32,
    ty: f32,
    cx: f32,
    cy: f32,
}

impl Transform2D {
    #[inline(always)]
    fn transform(&self, x: f32, y: f32) -> (f32, f32) {
        let dx = x - self.cx;
        let dy = y - self.cy;
        let rx = self.cos_theta * dx - self.sin_theta * dy + self.cx + self.tx;
        let ry = self.sin_theta * dx + self.cos_theta * dy + self.cy + self.ty;
        (rx, ry)
    }
}

fn compute_transform_at(
    t: f64,
    amplitude: f32,
    frequency: f32,
    rotation_deg: f32,
    octaves: usize,
    seed: f32,
    auto_scale: bool,
    w: f32,
    h: f32,
) -> Transform2D {
    let t_f = (t as f32) * frequency;

    // Organic offsets for X, Y, and Angle
    let nx = multi_octave_noise(t_f, seed, octaves);
    let ny = multi_octave_noise(t_f, seed + 101.3, octaves);
    let nr = multi_octave_noise(t_f, seed + 203.7, octaves);

    let offset_x = nx * amplitude;
    let offset_y = ny * amplitude;
    let angle_rad = (nr * rotation_deg).to_radians();

    // Auto-scale compensation to eliminate borders during strong shakes
    let scale_factor = if auto_scale {
        let max_disp = amplitude / (w.min(h) * 0.5);
        let rot_disp = angle_rad.abs() * 0.5;
        1.0 + (max_disp + rot_disp).min(0.2)
    } else {
        1.0
    };

    let cos_t = angle_rad.cos() / scale_factor;
    let sin_t = angle_rad.sin() / scale_factor;

    Transform2D {
        cos_theta: cos_t,
        sin_theta: sin_t,
        tx: offset_x,
        ty: offset_y,
        cx: w * 0.5,
        cy: h * 0.5,
    }
}

fn render_shake_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    time: f64,
    amplitude: f32,
    frequency: f32,
    rotation_deg: f32,
    octaves: usize,
    seed: f32,
    mb_mode: usize,
    shutter: f32,
    wrap_mode: u32,
    auto_scale: bool,
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
        let w_f = w as f32;
        let h_f = h as f32;
        let max_x = (w - 1).max(1);
        let max_y = (h - 1).max(1);

        // Motion blur sampling setup
        let (num_samples, dt_range) = match mb_mode {
            2 => (5, (shutter / 360.0) * (0.5 / frequency.max(1.0))), // Smooth 5 samples
            1 => (3, (shutter / 360.0) * (0.5 / frequency.max(1.0))), // Fast 3 samples
            _ => (1, 0.0),                                            // Off
        };

        let mut transforms = [
            Transform2D { cos_theta: 1.0, sin_theta: 0.0, tx: 0.0, ty: 0.0, cx: 0.0, cy: 0.0 },
            Transform2D { cos_theta: 1.0, sin_theta: 0.0, tx: 0.0, ty: 0.0, cx: 0.0, cy: 0.0 },
            Transform2D { cos_theta: 1.0, sin_theta: 0.0, tx: 0.0, ty: 0.0, cx: 0.0, cy: 0.0 },
            Transform2D { cos_theta: 1.0, sin_theta: 0.0, tx: 0.0, ty: 0.0, cx: 0.0, cy: 0.0 },
            Transform2D { cos_theta: 1.0, sin_theta: 0.0, tx: 0.0, ty: 0.0, cx: 0.0, cy: 0.0 },
        ];

        for i in 0..num_samples {
            let offset_factor = if num_samples > 1 {
                ((i as f32) / ((num_samples - 1) as f32)) - 0.5
            } else {
                0.0
            };
            let sample_time = time + (offset_factor * dt_range) as f64;
            transforms[i] = compute_transform_at(
                sample_time,
                amplitude,
                frequency,
                rotation_deg,
                octaves,
                seed,
                auto_scale,
                w_f,
                h_f,
            );
        }

        let inv_samples = 1.0 / (num_samples as f32);

        for y in 0..h {
            let row_idx = y * w;
            let y_f = y as f32;
            let dst_row = &mut px[row_idx..row_idx + w];

            for x in 0..w {
                let x_f = x as f32;
                let mut acc = [0.0f32; 4];

                for s_idx in 0..num_samples {
                    let (sx, sy) = transforms[s_idx].transform(x_f, y_f);
                    let sample = sample_bilinear(src_ptr, w, max_x, max_y, sx, sy, wrap_mode);
                    acc[0] += sample[0];
                    acc[1] += sample[1];
                    acc[2] += sample[2];
                    acc[3] += sample[3];
                }

                dst_row[x] = [
                    acc[0] * inv_samples,
                    acc[1] * inv_samples,
                    acc[2] * inv_samples,
                    acc[3] * inv_samples,
                ];
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

    // Fast path: fully interior coordinates
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

    // Boundary wrapping
    let resolve_coord = |coord: i32, max_val: usize| -> Option<usize> {
        match wrap_mode {
            0 => {
                // Mirror (Reflect)
                let m = (max_val * 2) as i32;
                let mut c = coord % m;
                if c < 0 { c += m; }
                if c > max_val as i32 { c = m - c; }
                Some(c as usize)
            }
            1 => {
                // Clamp (Repeat edge)
                Some(coord.clamp(0, max_val as i32) as usize)
            }
            _ => {
                // Transparent (Zero)
                if coord < 0 || coord > max_val as i32 {
                    None
                } else {
                    Some(coord as usize)
                }
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
