# Craft Plugins Architecture & Optimizations

## Repositories & Build
- `craft-plugins`: `/home/qwertyonek/NEXTCLOUD/HOBBY/WIBECODE/craft-plugins`
- `dev/effectcraft`: `/home/qwertyonek/dev/effectcraft`
- Target wasm binaries: `dist/*.wasm` and `~/.config/effectcraft/Plug-ins/`

## 1. Craft Glow (org.craftplugins.glow)
- **Target Engine:** EffectCraft WebAssembly Plugin API v1 (`wasmi` bytecode interpreter).
- **Core Architecture:**
  1. Zero Heap Allocations: thread_local scratch pool.
  2. Adaptive Downscale: 4x downscale for <=960p, 8x downscale for >960p (240–480px buffer).
  3. Peter Kovesi O(1) Sliding-Window Box Blur: Dual-octave (Core + Bloom) running sum with branchless unrolled inner loops.
  4. Direct Scanline Bilinear Reconstruction:
     - Zero intermediate buffers (temp_h eliminated).
     - Sparse skip: segments with zero alpha bypassed instantly.
     - Unrolled continuous bilinear lerp for 4- and 8-pixel spans.

## 2. Craft Chromatic (org.craftplugins.chromatic)
- **Target Engine:** EffectCraft WebAssembly Plugin API v1.
- **Category:** "by qwertyonek"
- **Features:**
  1. Modes:
     - Radial (Optical Lens): realistic optical dispersion radiating from center, quadratic barrel falloff.
     - Linear (Directional): angle-based RGB channel split (prism / glitch).
  2. Subpixel Bilinear Sampling: Fast interior sampling path with edge clamping fallback.
  3. Precomputed LUT: 512-entry distance-squared LUT completely eliminating sqrt and powf from the inner per-pixel loop.
  4. Zero-allocation: thread_local source scratch buffer for in-place rendering.
