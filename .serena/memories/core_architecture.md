# Craft Plugins Architecture & Optimizations

## Repositories & Build
- `craft-plugins`: `/home/qwertyonek/NEXTCLOUD/HOBBY/WIBECODE/craft-plugins`
- `dev/effectcraft`: `/home/qwertyonek/dev/effectcraft`
- Target wasm binary: `dist/craft_glow.wasm` and `~/.config/effectcraft/Plug-ins/craft_glow.wasm`

## Craft Glow (org.craftplugins.glow)
- **Target Engine:** EffectCraft WebAssembly Plugin API v1 (`wasmi` bytecode interpreter, single-threaded execution inside sandboxed VM).
- **Core Architecture:**
  1. **Zero Heap Allocations:** All scratch working buffers live in static `thread_local SCRATCH` pool.
  2. **Adaptive Downscale:** 4x downscale for <=960p, 8x downscale for >960p, targeting an optimal 240–480px low-frequency glow buffer.
  3. **Peter Kovesi O(1) Sliding-Window Box Blur:** Dual-octave (Core + Bloom) running sum with branchless and loop-unrolled inner loops. Independent of blur radius.
  4. **Direct Scanline Bilinear Reconstruction:**
     - Zero intermediate buffers (`temp_h` eliminated).
     - Sparse skip: segments with zero alpha are bypassed in 0 nanoseconds.
     - Unrolled continuous bilinear lerp for 4- and 8-pixel spans.
     - 100% continuous mathematical gradients with zero rectangular block mosaic or pixelation.
