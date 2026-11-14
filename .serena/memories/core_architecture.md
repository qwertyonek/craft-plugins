# Craft Plugins Architecture & Optimizations

## Repositories & Build
- `craft-plugins`: `/home/qwertyonek/NEXTCLOUD/HOBBY/WIBECODE/craft-plugins` (10 plug-ins, v1.1.0 released)
- `dev/effectcraft`: `/home/qwertyonek/dev/effectcraft`
- Target wasm binaries: `dist/*.wasm`; host loads from `~/.config/effectcraft/Plug-ins/`
- Skill with the full workflow: `wasm-pixel-plugins` (harness in its `scripts/plugin_harness.js`).

## ABI v1 layout (easy to get wrong in a harness)
One `ec_alloc(params_bytes + pixels_bytes)` per frame: params (f64) at base, pixels (premultiplied f32 RGBA) at `base + params_bytes`; `ec_render(pixels, w, h, base, nparams, time, scale)`. `ec_alloc` returns the SAME pointer every call, so allocating twice lets params overwrite the first rows of the frame.

## Build environment quirks
- rustup `stable` can arrive broken (rustc missing / mixed cargo+rustc versions -> E0514). Repair: `rustup toolchain uninstall stable && rustup toolchain install stable --profile minimal --target wasm32-unknown-unknown --component rustc,cargo`.
- `cargo build` may skip a rebuild after `cp`-restoring a source file; use `cargo clean -p <crate>` when output contradicts the source.
- Release workflow: push tag `v*` -> CI runs `cargo test --workspace`, builds all wasm and attaches them to the GitHub Release.

## 1. Craft Glow (org.craftplugins.glow)
Adaptive downscale (4x/8x) + Peter Kovesi O(1) sliding-window box blur, dual octave (core + bloom), direct scanline bilinear reconstruction, zero heap allocations.

## 2. Craft Chromatic (org.craftplugins.chromatic)
Radial (lens dispersion) and linear (prism) RGB split; 512-entry distance-squared LUT; subpixel bilinear with edge clamp; thread_local scratch, in-place.

## 3. Craft Halftone & Dither (org.craftplugins.halftone) — v1.0.1
Bayer 8x8/4x4, CMYK newspaper halftone, mono circles; palettes RGB/B&W/GameBoy/CGA/Cyberpunk. Bug #1 fixed: cell wrap used `%` (keeps dividend sign) -> diagonal seam; now `rem_euclid`. Regression test `halftone_screen_angle_is_continuous`.

## 4. Craft Motion Blur Curves (org.craftplugins.motionblurcurves) — new in v1.1.0
Arc accumulation blur: length, direction, curvature (total heading change = curvature*180 deg), taper, quality 8/16/32/64, falloff linear/gaussian/center, shutter, wrap mirror/clamp/transparent, jitter. Kernel: offsets+weights precomputed once per frame; gather with bilinear taps. Tap budget 1.6M per frame; above it accumulate on a downscaled copy (factor up to 4) and bilinear upsample. 1080p default went 335 ms -> 41 ms at wasm level. Tests: weight normalisation, centroid stability (straight exact, arc bounded), curvature bends (vertical variance 0 vs 22), passthrough bounds, 4K budget.

## 5. Craft Reveal (org.craftplugins.reveal) — new in v1.1.0
Animated reveal/wipe: reveal %, direction (L>R/R>L/T>B/B>T/diagonal/radial), edge softness, build style smooth/lines/blocks, step size, cursor + width + glow, invert. Mask multiplies premultiplied RGBA. Tests: identity at 100%, hidden at 0%, monotone coverage, edge selection per direction, stepped bands, invert complement, soft-edge partials.

## Plugin API v1 hard limits (answers to user requests)
One frame + time + scale only: no glyphs/ranges (per-character text animation impossible), no host transform curves (S_BlurMoCurves only approximable), no previous frames, no other layers/masks.

## GitHub issues (answered 2026-11-14)
- #1 halftone seam -> fixed 1.0.1, release v1.0.1.
- #2 text animation -> explained limits, shipped craft-reveal, comment added.
- #3 S_BlurMoCurves -> shipped craft-motion-blur-curves, comment added, awaiting feedback.

## Repo housekeeping
`scripts/Craft Composer.jsx` (AE ExtendScript panel, unrelated to EffectCraft) deliberately left untracked.
