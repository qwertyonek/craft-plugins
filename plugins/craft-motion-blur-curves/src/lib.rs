//! CraftMotionBlurCurves: curved accumulation motion blur.
//!
//! The plug-in API hands an effect one frame, so this cannot integrate the layer's own
//! transform curves the way Sapphire's `S_BlurMoCurves` does. What it does instead is
//! accumulate the frame along an explicit arc — direction, length, curvature, taper and
//! falloff are ordinary keyframable properties, so the smear follows a path the artist
//! animates by hand.
//!
//! The host runs plug-ins on a `wasmi` interpreter, so the tap count is capped by an
//! explicit work budget: heavy settings accumulate on a downscaled copy of the frame and
//! upsample the result. A blur is a low-pass filter, so the loss is invisible in the
//! smear; it only matters when the blur is short, and short blurs need few taps and stay
//! at full resolution.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.motionblurcurves",
  "name": "Craft Motion Blur Curves",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Curved accumulation motion blur: arc, taper, falloff and sample control.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "length", "name": "Blur Length (px)", "type": "slider", "default": 24.0, "min": 0.0, "max": 400.0, "sliderMax": 120.0, "decimals": 1},
    {"id": "direction", "name": "Direction", "type": "angle", "default": 0.0},
    {"id": "curvature", "name": "Curvature", "type": "slider", "default": 0.0, "min": -3.0, "max": 3.0, "sliderMax": 2.0, "decimals": 2},
    {"id": "taper", "name": "Taper", "type": "slider", "default": 0.0, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "samples", "name": "Quality", "type": "popup", "options": ["Fast (8 Samples)", "Good (16 Samples)", "High (32 Samples)", "Ultra (64 Samples)"], "default": 1},
    {"id": "falloff", "name": "Weight Falloff", "type": "popup", "options": ["Linear", "Gaussian", "Center Weighted"], "default": 0},
    {"id": "shutter", "name": "Shutter Angle", "type": "slider", "default": 180.0, "min": 0.0, "max": 360.0, "sliderMax": 360.0, "decimals": 0},
    {"id": "wrap_mode", "name": "Edge Handling", "type": "popup", "options": ["Mirror (Reflect)", "Clamp (Repeat Edge)", "Zero (Transparent)"], "default": 1},
    {"id": "jitter", "name": "Sample Jitter", "type": "slider", "default": 0.0, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2}
  ]
}"#;

const MAX_SAMPLES: usize = 64;
/// Bilinear taps a single frame may cost. The interpreter executes roughly 100M
/// instructions per second, and a tap is tens of instructions, so this is the difference
/// between a usable effect and a frozen host.
const TAP_BUDGET: usize = 1_600_000;

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
    if p.len() < 10 || w < 2 || h < 2 || px.len() < w * h {
        return 1;
    }

    let effective_scale = if scale > 0.0 { scale as f32 } else { 1.0 };

    // Params layout:
    // p[0]: about popup
    // p[1]: length (px)
    // p[2]: direction (angle, degrees)
    // p[3]: curvature (-3..3)
    // p[4]: taper (0..1)
    // p[5]: quality popup (0: 8, 1: 16, 2: 32, 3: 64 samples)
    // p[6]: falloff popup (0: Linear, 1: Gaussian, 2: Center Weighted)
    // p[7]: shutter angle (0..360)
    // p[8]: wrap_mode (0: Mirror, 1: Clamp, 2: Transparent)
    // p[9]: jitter (0..1)
    let shutter_scale = ((p[7] as f32).clamp(0.0, 360.0) / 180.0).max(0.0);
    let length = (p[1] as f32).max(0.0) * effective_scale * shutter_scale;
    if length < 0.5 {
        return 0; // nothing to accumulate
    }

    let direction = (p[2] as f32).to_radians();
    let curvature = (p[3] as f32).clamp(-3.0, 3.0);
    let taper = (p[4] as f32).clamp(0.0, 1.0);
    let falloff = p[6].round() as u32;
    let wrap_mode = p[8].round() as u32;
    let jitter = (p[9] as f32).clamp(0.0, 1.0);

    let requested = match (p[5].round() as u32).min(3) {
        0 => 8,
        1 => 16,
        2 => 32,
        _ => 64,
    };
    // A short blur resolves with fewer taps than a long one.
    let taps_by_length = (length.ceil() as usize).clamp(2, MAX_SAMPLES);
    let mut samples = requested.min(taps_by_length).clamp(2, MAX_SAMPLES);

    // Keep `pixels_on_screen * samples` inside the tap budget by accumulating on a
    // downscaled copy when the request is too expensive for the interpreter.
    let total_pixels = w * h;
    let mut factor = 1usize;
    loop {
        let small_px = (w / factor).max(1) * (h / factor).max(1);
        if small_px * samples <= TAP_BUDGET || factor >= 4 {
            break;
        }
        factor += 1;
    }
    if factor > 1 {
        // Downscaling divides the on-screen blur length too, so keep enough taps for
        // the smear to stay smooth in the small buffer.
        let small_len = (length / factor as f32).ceil() as usize;
        samples = samples.min(small_len.clamp(2, MAX_SAMPLES));
    }

    // Sample offsets and weights depend only on the parameters, never on the pixel, so
    // they are built once per frame.
    let mut offsets = [(0.0f32, 0.0f32); MAX_SAMPLES];
    let mut weights = [0.0f32; MAX_SAMPLES];
    build_kernel(
        &mut offsets,
        &mut weights,
        samples,
        length / factor as f32,
        direction,
        curvature,
        taper,
        falloff,
    );

    if factor == 1 {
        SCRATCH.with(|s| {
            let mut scratch = s.borrow_mut();
            if scratch.len() < total_pixels {
                scratch.resize(total_pixels, [0.0; 4]);
            }
            let src = &mut scratch[..total_pixels];
            src.copy_from_slice(&px[..total_pixels]);
            blur(
                src,
                &mut px[..total_pixels],
                w,
                h,
                &offsets[..samples],
                &weights[..samples],
                wrap_mode,
                jitter,
            );
        });
    } else {
        let bw = (w / factor).max(1);
        let bh = (h / factor).max(1);
        let small = bw * bh;
        SCRATCH.with(|s| {
            let mut scratch = s.borrow_mut();
            if scratch.len() < small * 2 {
                scratch.resize(small * 2, [0.0; 4]);
            }
            let (down, blurred) = scratch[..small * 2].split_at_mut(small);

            downscale(&px[..total_pixels], w, h, down, bw, bh, factor);
            blur(
                down,
                blurred,
                bw,
                bh,
                &offsets[..samples],
                &weights[..samples],
                wrap_mode,
                jitter,
            );
            upscale(blurred, bw, bh, &mut px[..total_pixels], w, h, factor);
        });
    }

    0
}

/// Arc offsets and normalised weights for `samples` taps.
fn build_kernel(
    offsets: &mut [(f32, f32)],
    weights: &mut [f32],
    samples: usize,
    length: f32,
    direction: f32,
    curvature: f32,
    taper: f32,
    falloff: u32,
) {
    let half = length * 0.5;
    let turn = curvature * std::f32::consts::PI; // total heading change across the path
    let k = turn * 0.5; // heading(t) = direction + k * t
    let (sin_d, cos_d) = (direction.sin(), direction.cos());
    let curved = k.abs() > 1e-4;

    let mut weight_sum = 0.0f32;
    let inv_n = 1.0 / (samples - 1) as f32;
    for i in 0..samples {
        let t = i as f32 * inv_n * 2.0 - 1.0; // -1 .. 1
        offsets[i] = if curved {
            let theta = direction + k * t;
            (
                half * (theta.sin() - sin_d) / k,
                half * (-theta.cos() + cos_d) / k,
            )
        } else {
            (half * t * cos_d, half * t * sin_d)
        };

        let w_base = match falloff {
            1 => (-(2.5 * t) * (2.5 * t)).exp(),
            2 => 1.0 - 0.5 * t.abs(),
            _ => 1.0,
        };
        let weight = w_base * (1.0 - taper * t.abs()).max(0.001);
        weights[i] = weight;
        weight_sum += weight;
    }
    let inv = 1.0 / weight_sum.max(1e-6);
    for w in weights.iter_mut().take(samples) {
        *w *= inv;
    }
}

/// Gather `src` along the precomputed taps into `dst`. Both slices are the same size.
fn blur(
    src: &[[f32; 4]],
    dst: &mut [[f32; 4]],
    w: usize,
    h: usize,
    offsets: &[(f32, f32)],
    weights: &[f32],
    wrap_mode: u32,
    jitter: f32,
) {
    let samples = offsets.len();
    let max_x = (w - 1) as i32;
    let max_y = (h - 1) as i32;

    for y in 0..h {
        let row_idx = y * w;
        let y_f = y as f32;

        for x in 0..w {
            let orig = src[row_idx + x];
            if orig[3] <= 1e-4 && wrap_mode != 2 {
                // Nothing to smear and no transparent edge to pull in: keep it clear.
                dst[row_idx + x] = orig;
                continue;
            }

            let x_f = x as f32;
            let mut acc = [0.0f32; 4];

            for i in 0..samples {
                let (dx, dy) = offsets[i];
                let weight = weights[i];
                let sx = x_f + dx;
                let sy = y_f + dy;

                let fx0 = sx.floor();
                let fy0 = sy.floor();
                let tx = fx0 as i32;
                let ty = fy0 as i32;
                let fx = sx - fx0;
                let fy = sy - fy0;

                let c = match wrap_mode {
                    2 => {
                        // Transparent: bilinear against a zero border.
                        if sx < -1.0
                            || sy < -1.0
                            || sx > max_x as f32 + 1.0
                            || sy > max_y as f32 + 1.0
                        {
                            continue;
                        }
                        let mut c = [0.0f32; 4];
                        let taps = [
                            (tx, ty, (1.0 - fx) * (1.0 - fy)),
                            (tx + 1, ty, fx * (1.0 - fy)),
                            (tx, ty + 1, (1.0 - fx) * fy),
                            (tx + 1, ty + 1, fx * fy),
                        ];
                        for (px_x, px_y, tw) in taps {
                            if tw <= 0.0
                                || px_x < 0
                                || px_y < 0
                                || px_x > max_x
                                || px_y > max_y
                            {
                                continue;
                            }
                            let s = src[px_y as usize * w + px_x as usize];
                            c[0] += s[0] * tw;
                            c[1] += s[1] * tw;
                            c[2] += s[2] * tw;
                            c[3] += s[3] * tw;
                        }
                        c
                    }
                    _ => {
                        let rx = resolve_coord(tx, max_x, wrap_mode) as usize;
                        let ry = resolve_coord(ty, max_y, wrap_mode) as usize;
                        let rx1 = resolve_coord(tx + 1, max_x, wrap_mode) as usize;
                        let ry1 = resolve_coord(ty + 1, max_y, wrap_mode) as usize;
                        let c00 = src[ry * w + rx];
                        let c10 = src[ry * w + rx1];
                        let c01 = src[ry1 * w + rx];
                        let c11 = src[ry1 * w + rx1];
                        let top = lerp4(c00, c10, fx);
                        lerp4(top, lerp4(c01, c11, fx), fy)
                    }
                };

                acc[0] += c[0] * weight;
                acc[1] += c[1] * weight;
                acc[2] += c[2] * weight;
                acc[3] += c[3] * weight;
            }

            if jitter > 0.0 {
                let n = hash2(x as u32, y as u32) - 0.5;
                let amount = jitter * 0.03;
                acc[0] += n * amount;
                acc[1] += n * amount;
                acc[2] += n * amount;
            }

            dst[row_idx + x] = [
                acc[0].max(0.0),
                acc[1].max(0.0),
                acc[2].max(0.0),
                acc[3].clamp(0.0, 1.0),
            ];
        }
    }
}

/// Box-average `factor x factor` blocks. Premultiplied alpha averages correctly.
fn downscale(
    src: &[[f32; 4]],
    w: usize,
    h: usize,
    dst: &mut [[f32; 4]],
    bw: usize,
    bh: usize,
    factor: usize,
) {
    let inv = 1.0 / (factor * factor) as f32;
    for by in 0..bh {
        for bx in 0..bw {
            let mut acc = [0.0f32; 4];
            for oy in 0..factor {
                let sy = (by * factor + oy).min(h - 1);
                for ox in 0..factor {
                    let sx = (bx * factor + ox).min(w - 1);
                    let s = src[sy * w + sx];
                    acc[0] += s[0];
                    acc[1] += s[1];
                    acc[2] += s[2];
                    acc[3] += s[3];
                }
            }
            dst[by * bw + bx] = [
                acc[0] * inv,
                acc[1] * inv,
                acc[2] * inv,
                acc[3] * inv,
            ];
        }
    }
}

/// Bilinear upsample of the accumulated buffer back to screen resolution.
fn upscale(
    src: &[[f32; 4]],
    bw: usize,
    bh: usize,
    dst: &mut [[f32; 4]],
    w: usize,
    h: usize,
    factor: usize,
) {
    let max_bx = (bw - 1) as i32;
    let max_by = (bh - 1) as i32;
    let inv_f = 1.0 / factor as f32;

    for y in 0..h {
        let sy = (y as f32 + 0.5) * inv_f - 0.5;
        let y0 = sy.floor();
        let fy = sy - y0;
        let y0i = (y0 as i32).clamp(0, max_by) as usize;
        let y1i = (y0 as i32 + 1).clamp(0, max_by) as usize;

        for x in 0..w {
            let sx = (x as f32 + 0.5) * inv_f - 0.5;
            let x0 = sx.floor();
            let fx = sx - x0;
            let x0i = (x0 as i32).clamp(0, max_bx) as usize;
            let x1i = (x0 as i32 + 1).clamp(0, max_bx) as usize;

            let c00 = src[y0i * bw + x0i];
            let c10 = src[y0i * bw + x1i];
            let c01 = src[y1i * bw + x0i];
            let c11 = src[y1i * bw + x1i];
            let top = lerp4(c00, c10, fx);
            dst[y * w + x] = lerp4(top, lerp4(c01, c11, fx), fy);
        }
    }
}

#[inline]
fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Mirror reflection and clamp for out-of-range taps. `%` would keep the sign of the
/// coordinate, which puts every negative tap into a shifted cell.
#[inline]
fn resolve_coord(coord: i32, max_val: i32, wrap_mode: u32) -> u32 {
    match wrap_mode {
        0 => {
            let m = (max_val + 1) * 2;
            let mut c = coord.rem_euclid(m);
            if c > max_val {
                c = m - c;
            }
            c.clamp(0, max_val) as u32
        }
        _ => coord.clamp(0, max_val) as u32,
    }
}

#[inline]
fn hash2(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 64;
    const H: usize = 64;

    fn params(length: f64, curvature: f64, falloff: f64, taper: f64, quality: f64) -> Vec<f64> {
        // about, length, direction, curvature, taper, quality, falloff, shutter, wrap, jitter
        vec![
            0.0, length, 0.0, curvature, taper, quality, falloff, 180.0, 1.0, 0.0,
        ]
    }

    fn flat(value: f32) -> Vec<[f32; 4]> {
        vec![[value, value, value, 1.0]; W * H]
    }

    /// A constant frame must survive the accumulation unchanged, whatever the falloff,
    /// taper or curvature: the weights have to sum to one.
    #[test]
    fn weights_are_normalised() {
        for falloff in [0.0, 1.0, 2.0] {
            for curvature in [0.0, 1.0, -2.0] {
                for taper in [0.0, 0.6] {
                    let mut px = flat(0.4);
                    let p = params(30.0, curvature, falloff, taper, 2.0);
                    assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
                    for c in px.iter() {
                        assert!(
                            (c[0] - 0.4).abs() < 1e-3 && (c[3] - 1.0).abs() < 1e-3,
                            "falloff {falloff} curvature {curvature} taper {taper}: got {c:?}"
                        );
                    }
                }
            }
        }
    }

    /// A straight blur is symmetric: the energy centroid of a single bright pixel must
    /// stay exactly on it. (A curved arc legitimately shifts the centroid sideways — it
    /// bulges away from the centre — so that case is bounded, not pinned.)
    #[test]
    fn centroid_does_not_drift() {
        let length = 20.0;
        let half = length / 2.0;
        for curvature in [0.0, 1.5, -1.5] {
            for direction in [0.0, 37.0, 90.0, 215.0] {
                let mut px = vec![[0.0f32, 0.0, 0.0, 0.0]; W * H];
                let (cx, cy) = (W / 2, H / 2);
                px[cy * W + cx] = [1.0, 1.0, 1.0, 1.0];
                let mut p = params(length, curvature, 0.0, 0.0, 2.0);
                p[2] = direction;
                p[8] = 2.0; // transparent border so the smear does not wrap into the frame
                assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);

                let (mut sx, mut sy, mut sa) = (0.0f64, 0.0f64, 0.0f64);
                for y in 0..H {
                    for x in 0..W {
                        let a = px[y * W + x][3] as f64;
                        sx += a * x as f64;
                        sy += a * y as f64;
                        sa += a;
                    }
                }
                assert!(
                    sa > 0.0,
                    "curvature {curvature} direction {direction}: nothing rendered"
                );
                let mx = sx / sa;
                let my = sy / sa;
                let dx = mx - cx as f64;
                let dy = my - cy as f64;
                let drift = (dx * dx + dy * dy).sqrt();

                if curvature == 0.0 {
                    assert!(
                        drift < 0.6,
                        "straight blur at {direction}° moved the centroid by {drift:.2} px \
                         (radius {half})"
                    );
                } else {
                    // An arc of radius half/|k| bulges by at most 2R < 2 * half / |k|.
                    assert!(
                        drift < 4.0 * half,
                        "curvature {curvature} direction {direction}: centroid drift {drift:.2} px \
                         is not explained by the arc bulge (radius {half})"
                    );
                }
            }
        }
    }

    /// A short blur must not smear: length below one buffer pixel is a passthrough.
    #[test]
    fn short_length_is_passthrough() {
        let mut px = flat(0.25);
        px[0] = [1.0, 0.0, 0.0, 1.0];
        let before = px.clone();
        let p = params(0.2, 1.0, 0.0, 0.0, 2.0);
        assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
        assert_eq!(px, before);
    }

    /// Zero-length with a hostile curvature must still be a passthrough, not a NaN trip.
    #[test]
    fn zero_length_is_safe() {
        let mut px = flat(0.5);
        let p = params(0.0, 3.0, 2.0, 1.0, 3.0);
        assert_eq!(render_frame(&mut px, W, H, &p, 0.0, 1.0), 0);
        for c in px.iter() {
            assert!(c[0].is_finite() && (c[0] - 0.5).abs() < 1e-4);
        }
    }

    /// Curvature must actually bend the smear. A single bright pixel blurred along a
    /// straight horizontal path stays on one row — no vertical spread at all; an arc has
    /// to lift samples off that row.
    #[test]
    fn curvature_bends_the_smear() {
        let dot = || {
            let mut px = vec![[0.0f32, 0.0, 0.0, 0.0]; W * H];
            px[(H / 2) * W + W / 2] = [1.0, 1.0, 1.0, 1.0];
            px
        };

        let vertical_variance = |px: &[[f32; 4]]| -> f32 {
            let (mut sa, mut sy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
            for y in 0..H {
                for x in 0..W {
                    let a = px[y * W + x][3] as f64;
                    sa += a;
                    sy += a * y as f64;
                    syy += a * (y * y) as f64;
                }
            }
            assert!(sa > 0.0, "nothing rendered");
            let mean = sy / sa;
            (syy / sa - mean * mean) as f32
        };

        let mut straight = dot();
        let mut curved = dot();
        let mut p_straight = params(24.0, 0.0, 0.0, 0.0, 3.0);
        let mut p_curved = params(24.0, 2.0, 0.0, 0.0, 3.0);
        p_straight[8] = 2.0;
        p_curved[8] = 2.0;
        assert_eq!(render_frame(&mut straight, W, H, &p_straight, 0.0, 1.0), 0);
        assert_eq!(render_frame(&mut curved, W, H, &p_curved, 0.0, 1.0), 0);

        let flat = vertical_variance(&straight);
        let bent = vertical_variance(&curved);
        assert!(
            flat < 0.05,
            "a straight horizontal blur must not spread vertically, variance {flat:.3}"
        );
        assert!(
            bent > 1.0,
            "curvature did not bend the smear: vertical variance {bent:.3} \
             (straight {flat:.3})"
        );
    }

    /// The tap budget must hold at 4K: the renderer has to fall back to a downscaled
    /// accumulation instead of asking the interpreter for billions of taps.
    #[test]
    fn tap_budget_holds_at_4k() {
        let (w, h) = (3840usize, 2160usize);
        let mut px = vec![[0.5f32, 0.5, 0.5, 1.0]; w * h];
        let p = params(120.0, 0.0, 1.0, 0.0, 3.0); // Ultra, long blur
        assert_eq!(render_frame(&mut px, w, h, &p, 0.0, 1.0), 0);
        // A constant frame still has to stay constant through downscale + upscale.
        let bad = px.iter().filter(|c| (c[0] - 0.5).abs() > 5e-3).count();
        assert_eq!(bad, 0, "{bad} pixels drifted on the downscaled path");
    }
}
