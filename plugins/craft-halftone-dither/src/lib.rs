//! CraftHalftoneDither: Retro matrix ordered dithering, newspaper CMYK halftone screens,
//! and arcade 1-bit / 4-color retro palette quantization.
//!
//! Author: qwertyonek

use std::cell::RefCell;

const MANIFEST: &str = r#"{
  "api": 1,
  "id": "org.craftplugins.halftone",
  "name": "Craft Halftone & Dither",
  "category": "by qwertyonek",
  "version": "1.0.0",
  "author": "qwertyonek",
  "description": "Retro ordered dithering, CMYK newspaper halftone dots & arcade palettes.",
  "params": [
    {"id": "about", "name": "Author", "type": "popup", "options": ["CraftPlugins by qwertyonek", "v1.0.0 (High Performance)", "github.com/qwertyonek/craft-plugins"], "default": 0},
    {"id": "mode", "name": "Effect Mode", "type": "popup", "options": ["Ordered Dither (Bayer 8x8)", "Ordered Dither (Bayer 4x4)", "CMYK Newspaper Halftone Dots", "Mono Halftone Circles"], "default": 0},
    {"id": "palette", "name": "Color Palette", "type": "popup", "options": ["Full Color (RGB Dither)", "Monochrome 1-Bit (B&W)", "GameBoy Classic (4-Green)", "CGA Mode 1 (Cyan/Magenta)", "Cyberpunk Neon (4-Color)"], "default": 0},
    {"id": "cell_size", "name": "Pixel / Dot Size", "type": "slider", "default": 4.0, "min": 1.0, "max": 32.0, "sliderMax": 16.0, "decimals": 0},
    {"id": "pattern_angle", "name": "Screen Angle", "type": "angle", "default": 45.0},
    {"id": "contrast", "name": "Contrast Boost", "type": "slider", "default": 1.2, "min": 0.5, "max": 3.0, "sliderMax": 2.5, "decimals": 2},
    {"id": "dither_spread", "name": "Dither Spread", "type": "slider", "default": 0.35, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2},
    {"id": "blend", "name": "Mix with Original", "type": "slider", "default": 1.0, "min": 0.0, "max": 1.0, "sliderMax": 1.0, "decimals": 2}
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
    // p[1]: mode (0: Bayer 8x8, 1: Bayer 4x4, 2: CMYK Dots, 3: Mono Circles)
    // p[2]: palette (0: RGB, 1: B&W, 2: GameBoy, 3: CGA, 4: Cyberpunk)
    // p[3]: cell_size
    // p[4]: pattern_angle
    // p[5]: contrast
    // p[6]: dither_spread
    // p[7]: blend (mix)
    let mode = p[1].round() as u32;
    let palette = p[2].round() as u32;
    let cell_size = ((p[3] as f32) * effective_scale).max(1.0);
    let angle_deg = p[4] as f32;
    let contrast = (p[5] as f32).max(0.1);
    let spread = (p[6] as f32).clamp(0.0, 1.0);
    let blend = (p[7] as f32).clamp(0.0, 1.0);

    if blend < 0.001 {
        return 0; // Passthrough
    }

    render_dither_zero_alloc(
        px,
        w,
        h,
        mode,
        palette,
        cell_size,
        angle_deg,
        contrast,
        spread,
        blend,
    );

    0
}

// 8x8 Bayer Matrix normalized to [0.0, 1.0)
#[rustfmt::skip]
const BAYER_8X8: [f32; 64] = [
     0.0/64.0, 32.0/64.0,  8.0/64.0, 40.0/64.0,  2.0/64.0, 34.0/64.0, 10.0/64.0, 42.0/64.0,
    48.0/64.0, 16.0/64.0, 56.0/64.0, 24.0/64.0, 50.0/64.0, 18.0/64.0, 58.0/64.0, 26.0/64.0,
    12.0/64.0, 44.0/64.0,  4.0/64.0, 36.0/64.0, 14.0/64.0, 46.0/64.0,  6.0/64.0, 38.0/64.0,
    60.0/64.0, 28.0/64.0, 52.0/64.0, 20.0/64.0, 62.0/64.0, 30.0/64.0, 54.0/64.0, 22.0/64.0,
     3.0/64.0, 35.0/64.0, 11.0/64.0, 43.0/64.0,  1.0/64.0, 33.0/64.0,  9.0/64.0, 41.0/64.0,
    51.0/64.0, 19.0/64.0, 59.0/64.0, 27.0/64.0, 49.0/64.0, 17.0/64.0, 57.0/64.0, 25.0/64.0,
    15.0/64.0, 47.0/64.0,  7.0/64.0, 39.0/64.0, 13.0/64.0, 45.0/64.0,  5.0/64.0, 37.0/64.0,
    63.0/64.0, 31.0/64.0, 55.0/64.0, 23.0/64.0, 61.0/64.0, 29.0/64.0, 53.0/64.0, 21.0/64.0,
];

// 4x4 Bayer Matrix
#[rustfmt::skip]
const BAYER_4X4: [f32; 16] = [
     0.0/16.0,  8.0/16.0,  2.0/16.0, 10.0/16.0,
    12.0/16.0,  4.0/16.0, 14.0/16.0,  6.0/16.0,
     3.0/16.0, 11.0/16.0,  1.0/16.0,  9.0/16.0,
    15.0/16.0,  7.0/16.0, 13.0/16.0,  5.0/16.0,
];

// GameBoy 4-shade green palette
const PALETTE_GAMEBOY: [[f32; 3]; 4] = [
    [0.059, 0.220, 0.059], // Darkest olive
    [0.188, 0.384, 0.188],
    [0.545, 0.675, 0.059],
    [0.608, 0.737, 0.059], // Lightest lime
];

// CGA Mode 1 palette
const PALETTE_CGA: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],       // Black
    [0.333, 1.0, 1.0],     // Cyan
    [1.0, 0.333, 1.0],     // Magenta
    [1.0, 1.0, 1.0],       // White
];

// Cyberpunk Neon 4-color palette
const PALETTE_CYBERPUNK: [[f32; 3]; 4] = [
    [0.05, 0.02, 0.12],    // Deep midnight purple
    [0.98, 0.08, 0.52],    // Neon pink
    [0.05, 0.90, 0.95],    // Neon cyan
    [0.98, 0.95, 0.20],    // Acid yellow
];

#[inline(always)]
fn quantize_palette_closest(rgb: [f32; 3], pal: &[[f32; 3]]) -> [f32; 3] {
    let mut best_idx = 0;
    let mut best_dist = f32::MAX;

    for (i, p) in pal.iter().enumerate() {
        let dr = rgb[0] - p[0];
        let dg = rgb[1] - p[1];
        let db = rgb[2] - p[2];
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_idx = i;
        }
    }

    pal[best_idx]
}

fn render_dither_zero_alloc(
    px: &mut [[f32; 4]],
    w: usize,
    h: usize,
    mode: u32,
    palette: u32,
    cell_size: f32,
    angle_deg: f32,
    contrast: f32,
    spread: f32,
    blend: f32,
) {
    let total_pixels = w * h;

    SCRATCH.with(|s| {
        let mut scratch = s.borrow_mut();
        if scratch.len() < total_pixels {
            scratch.resize(total_pixels, [0.0; 4]);
        }
        let src = &mut scratch[..total_pixels];
        src.copy_from_slice(px);

        let inv_cell = 1.0 / cell_size;
        let rad = angle_deg.to_radians();
        let cos_a = rad.cos();
        let sin_a = rad.sin();

        for y in 0..h {
            let row_idx = y * w;
            let y_f = y as f32;
            let dst_row = &mut px[row_idx..row_idx + w];

            for x in 0..w {
                let x_f = x as f32;
                let orig = src[row_idx + x];

                // Unpremultiply linear color
                let alpha = orig[3].clamp(0.0, 1.0);
                if alpha <= 1e-4 {
                    continue;
                }
                let inv_a = 1.0 / alpha;
                let r_lin = orig[0] * inv_a;
                let g_lin = orig[1] * inv_a;
                let b_lin = orig[2] * inv_a;

                // Contrast adjust
                let apply_contrast = |c: f32| -> f32 {
                    (((c - 0.5) * contrast) + 0.5).clamp(0.0, 1.0)
                };

                let r_c = apply_contrast(r_lin);
                let g_c = apply_contrast(g_lin);
                let b_c = apply_contrast(b_lin);
                let luma_c = (r_c * 0.2126 + g_c * 0.7152 + b_c * 0.0722).clamp(0.0, 1.0);

                let out_rgb = match mode {
                    0 | 1 => {
                        // Bayer Ordered Dithering
                        let (bx, by) = if cell_size > 1.0 {
                            let block_x = (x_f * inv_cell).floor() as usize;
                            let block_y = (y_f * inv_cell).floor() as usize;
                            (block_x, block_y)
                        } else {
                            (x, y)
                        };

                        let threshold = if mode == 0 {
                            BAYER_8X8[(by % 8) * 8 + (bx % 8)] - 0.5
                        } else {
                            BAYER_4X4[(by % 4) * 4 + (bx % 4)] - 0.5
                        };

                        let dither_shift = threshold * spread;

                        match palette {
                            1 => {
                                // 1-bit Black & White
                                let v = if luma_c + dither_shift > 0.5 { 1.0 } else { 0.0 };
                                [v, v, v]
                            }
                            2 => {
                                // GameBoy
                                let d_rgb = [
                                    (r_c + dither_shift).clamp(0.0, 1.0),
                                    (g_c + dither_shift).clamp(0.0, 1.0),
                                    (b_c + dither_shift).clamp(0.0, 1.0),
                                ];
                                quantize_palette_closest(d_rgb, &PALETTE_GAMEBOY)
                            }
                            3 => {
                                // CGA
                                let d_rgb = [
                                    (r_c + dither_shift).clamp(0.0, 1.0),
                                    (g_c + dither_shift).clamp(0.0, 1.0),
                                    (b_c + dither_shift).clamp(0.0, 1.0),
                                ];
                                quantize_palette_closest(d_rgb, &PALETTE_CGA)
                            }
                            4 => {
                                // Cyberpunk
                                let d_rgb = [
                                    (r_c + dither_shift).clamp(0.0, 1.0),
                                    (g_c + dither_shift).clamp(0.0, 1.0),
                                    (b_c + dither_shift).clamp(0.0, 1.0),
                                ];
                                quantize_palette_closest(d_rgb, &PALETTE_CYBERPUNK)
                            }
                            _ => {
                                // Full RGB Dither
                                [
                                    if r_c + dither_shift > 0.5 { 1.0 } else { 0.0 },
                                    if g_c + dither_shift > 0.5 { 1.0 } else { 0.0 },
                                    if b_c + dither_shift > 0.5 { 1.0 } else { 0.0 },
                                ]
                            }
                        }
                    }
                    2 => {
                        // CMYK Newspaper Halftone
                        // Standard printing screen angles: C: 15°, M: 75°, Y: 0°, K: 45°
                        let sample_cmyk_dot = |angle_offset: f32, val: f32| -> f32 {
                            let theta = (angle_deg + angle_offset).to_radians();
                            let (ct, st) = (theta.cos(), theta.sin());
                            let rx = x_f * ct - y_f * st;
                            let ry = x_f * st + y_f * ct;

                            let u = (rx % cell_size) / cell_size - 0.5;
                            let v = (ry % cell_size) / cell_size - 0.5;
                            let dist = (u * u + v * v).sqrt() * 2.0;

                            let dot_radius = (val * 1.414).clamp(0.0, 1.414);
                            if dist < dot_radius { 1.0 } else { 0.0 }
                        };

                        let c_val = (1.0 - r_c).clamp(0.0, 1.0);
                        let m_val = (1.0 - g_c).clamp(0.0, 1.0);
                        let y_val = (1.0 - b_c).clamp(0.0, 1.0);
                        let k_val = (1.0 - luma_c).clamp(0.0, 1.0);

                        let c_dot = sample_cmyk_dot(15.0, c_val);
                        let m_dot = sample_cmyk_dot(75.0, m_val);
                        let y_dot = sample_cmyk_dot(0.0, y_val);
                        let k_dot = sample_cmyk_dot(45.0, k_val);

                        let r_out = (1.0 - c_dot) * (1.0 - k_dot);
                        let g_out = (1.0 - m_dot) * (1.0 - k_dot);
                        let b_out = (1.0 - y_dot) * (1.0 - k_dot);

                        [r_out, g_out, b_out]
                    }
                    _ => {
                        // Monochrome Halftone Screen Circles
                        let rx = x_f * cos_a - y_f * sin_a;
                        let ry = x_f * sin_a + y_f * cos_a;

                        let u = (rx % cell_size) / cell_size - 0.5;
                        let v = (ry % cell_size) / cell_size - 0.5;
                        let dist = (u * u + v * v).sqrt() * 2.0;

                        let dot_radius = (luma_c * 1.414).clamp(0.0, 1.414);
                        let dot = if dist < dot_radius { 1.0 } else { 0.0 };

                        match palette {
                            1 => [dot, dot, dot],
                            2 => quantize_palette_closest([dot, dot, dot], &PALETTE_GAMEBOY),
                            3 => quantize_palette_closest([dot, dot, dot], &PALETTE_CGA),
                            4 => quantize_palette_closest([dot, dot, dot], &PALETTE_CYBERPUNK),
                            _ => [r_c * dot, g_c * dot, b_c * dot],
                        }
                    }
                };

                // Premultiply and interpolate blend
                let final_r = (orig[0] * (1.0 - blend) + (out_rgb[0] * alpha) * blend).clamp(0.0, 1.0);
                let final_g = (orig[1] * (1.0 - blend) + (out_rgb[1] * alpha) * blend).clamp(0.0, 1.0);
                let final_b = (orig[2] * (1.0 - blend) + (out_rgb[2] * alpha) * blend).clamp(0.0, 1.0);

                dst_row[x] = [final_r, final_g, final_b, alpha];
            }
        }
    });
}
