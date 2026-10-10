# CraftPlugins Core Architecture

- **Project:** CraftPlugins - VFX and Motion Design WebAssembly plugins for EffectCraft.
- **Repository:** `qwertyonek/craft-plugins` (GitHub, Private).
- **Architecture Standard:**
  - Rust workspace with `--target wasm32-unknown-unknown`.
  - Zero-dependency WASM ABI conforming to EffectCraft Plug-in API v1.
  - C exports: `ec_api_version`, `ec_manifest_ptr`, `ec_manifest_len`, `ec_alloc`, `ec_render`.
  - Buffer format: 8-byte aligned linear memory, parameters (f64 slice) followed by pixels (premultiplied RGBA f32).
  - Multi-language README standard: English (README.md), Russian (README.ru.md), Chinese (README.zh-CN.md) with Shields.io badges and architecture diagrams.
- **First Plugin Target:** `craft-glow` (Deep Glow equivalent with inverse-square falloff, threshold, chromatic aberration halo, and HDR support).
