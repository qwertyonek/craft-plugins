# Craft Plugins Architecture & Optimizations

## Repositories & Build
- `craft-plugins`: `/home/qwertyonek/NEXTCLOUD/HOBBY/WIBECODE/craft-plugins`
- `dev/effectcraft`: `/home/qwertyonek/dev/effectcraft`
- Target wasm binaries: `dist/*.wasm` and `~/.config/effectcraft/Plug-ins/`

## Build environment quirks
- rustup `stable` toolchain can arrive broken (rustc binary missing while the component claims "up to date", or a mixed-version install where cargo 1.99 + rustc 1.97 gives E0514). Repair: `rustup toolchain uninstall stable && rustup toolchain install stable --profile minimal --target wasm32-unknown-unknown --component rustc,cargo`. System `/usr/bin/rustc` has no wasm32 std (no `rust-wasm` package), so use rustup only.
- `cargo build` can print "Finished" without recompiling after a source file is restored via `cp` (mtime granularity). If a test result contradicts the source, run `cargo clean -p <crate>` — do not trust "Finished in 0.2s" for a fresh build artifact.
- Headless A/B verification of a plugin: Node `WebAssembly.instantiate(module, {})` returns the instance directly when given a compiled Module; then `ec_alloc`/`ec_render`, params as `f64[8]`, pixels premultiplied `f32[4]`, export name `memory`. Harness lives in `~/.hermes/cache/scratch/halftone_ab.js`.

## 1. Craft Glow (org.craftplugins.glow)
- **Target Engine:** EffectCraft WebAssembly Plugin API v1 (`wasmi` bytecode interpreter).
- **Core Architecture:**
  1. Zero Heap Allocations: thread_local scratch pool.
  2. Adaptive Downscale: 4x downscale for <=960p, 8x downscale for >960p (240-480px buffer).
  3. Peter Kovesi O(1) Sliding-Window Box Blur: Dual-octave (Core + Bloom) running sum with branchless unrolled inner loops.
  4. Direct Scanline Bilinear Reconstruction: zero intermediate buffers, sparse skip of zero-alpha segments, unrolled continuous bilinear lerp for 4- and 8-pixel spans.

## 2. Craft Chromatic (org.craftplugins.chromatic)
- **Target Engine:** EffectCraft WebAssembly Plugin API v1, category "by qwertyonek".
- Modes: Radial (optical lens dispersion, quadratic barrel falloff) and Linear (angle-based RGB split). Subpixel bilinear sampling with edge clamp; 512-entry distance-squared LUT removing sqrt/powf from the inner loop; thread_local source scratch, in-place render.

## 3. Craft Halftone & Dither (org.craftplugins.halftone) — v1.0.1
- Modes: Bayer 8x8 / Bayer 4x4 ordered dither, CMYK newspaper halftone, Mono halftone circles; palettes RGB / B&W / GameBoy / CGA / Cyberpunk.
- **Fixed bug (#1, v1.0.1):** screen-space cell wrap used `%`, which in Rust keeps the dividend sign, so every negative rotated coordinate was sampled one cell off — diagonal seam where mono halftone went black and CMYK blew out to white. Both mono and CMYK paths now use `rem_euclid(cell_size)`.
- Regression test `tests::halftone_screen_angle_is_continuous`: flat grey 96x96, angles 0/15/45/75/135/200, asserts both halves non-blank and |left-right| < 2% mean luminance across the y=x diagonal.
- Measured CMYK 45deg left/right: 100%/77.3% (broken) vs 40.9%/40.8% (fixed).

## Plugin API v1 hard limits (for answering user requests)
- `ec_render` receives ONE frame (premultiplied RGBA f32, in place) plus `time` and `scale`. No host transform curves, no neighbouring frames, no layer/glyph/range data.
- Consequence: per-character text animation is impossible inside a plugin (belongs to upstream text animators); Sapphire `S_BlurMoCurves` is only approximable with keyframed arc-blur params.

## Open GitHub issues (all three replied 2026-11-14)
- #1 halftone screen-angle seam -> fixed in 1.0.1, dist rebuilt, CI green.
- #2 text animation -> explained API limits, offered reveal/typewriter-style pixel effect.
- #3 S_BlurMoCurves alternative -> offered `craft-motion-blur-curves` param spec, awaiting reply.

## Repo housekeeping
- `scripts/Craft Composer.jsx` (677 lines, ExtendScript panel for After Effects, unrelated to the WASM suite) is untracked — decide whether to track or drop.
