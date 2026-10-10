# CraftPlugins — Community Motion Design Plugins for EffectCraft

> Сборник open-source визуальных и моушн-дизайн плагинов для **EffectCraft** (опенсорсного аналога Adobe After Effects на Rust).
> Плагины компилируются в кроссплатформенный **WebAssembly (WASM)** и работают на Linux, Windows и macOS без внешних зависимостей.

---

## 🎯 Цель проекта
Создать топовый набор независимых плагинов визуальных эффектов (вдохновленных культовыми плагинами для After Effects от Video Copilot, Plugin Everything и Red Giant) в формате `.wasm`, готовых к установке в EffectCraft в 1 клик.

---

## 🚀 Дорожная карта плагинов (Что разрабатываем)

### 1. `craft-glow` (Аналог Deep Glow от Plugin Everything)
* **Приоритет:** ВЫСОКИЙ (Первый плагин)
* **Суть:** Физически корректное, сочное свечение с затуханием по обратному квадрату (`1/r²`), порогом отсечения яркости (Threshold), радиусом, интенсивностью и хроматическим расхождением спектра по краям ореола.
* **Параметры:**
  * `Radius` (Slider, default: 50.0)
  * `Brightness` (Slider, default: 1.0)
  * `Threshold` (Slider, default: 0.6)
  * `Falloff` (Slider, default: 2.0 — закон обратных квадратов)
  * `Tint` (Color, default: White)
  * `Aberration` (Slider, default: 5.0 — сдвиг RGB каналов в свечении)

### 2. `craft-chromatic` (Аналог Quick Chromatic Aberration / QCA)
* **Приоритет:** ВЫСОКИЙ
* **Суть:** Радиальное или линейное расхождение цветовых каналов (RGB Split), эмуляция дисперсии линзы, виньетирование аберраций от центра к краям.
* **Параметры:**
  * `Amount` (Slider)
  * `Mode` (Popup: Radial / Linear)
  * `Angle` (Angle — для линейного сдвига)
  * `Blur Channels` (Checkbox)

### 3. `craft-camera-shake` (Аналог Boris FX Sapphire S_Shake)
* **Приоритет:** СРЕДНИЙ
* **Суть:** Натуральное процедурное дрожание камеры (Perlin / Simplex шум), толчки, микро-вибрации, авто-скейл для предотвращения черных краев и motion blur.

### 4. `craft-displacer` (Аналог Displacer Pro)
* **Приоритет:** СРЕДНИЙ
* **Суть:** Смещение пикселей (Displacement Map) с контролем вращения, масштабирования, хроматики и субпиксельного сглаживания.

### 5. `craft-halftone-dither` (Ретро-дизеринг и газетный растр)
* **Приоритет:** СРЕДНИЙ
* **Суть:** Матричный дизеринг (Bayer 4x4 / 8x8), полутоновые точки (CMYK/RGB Halftone dots), палитры ретро-консолей.

---

## 🛠 Технический контекст и архитектура EffectCraft Plug-in API v1

### Как работает плагин в EffectCraft:
1. Плагин — это `.wasm` библиотека (компилируется с `--target wasm32-unknown-unknown`).
2. В рантайме EffectCraft исполняет его через изолированный интерпретатор `wasmi` (fuel limit на кадр, защита от зависаний).
3. Плагин экспортирует следующие C-функции:
   * `ec_api_version() -> i32` — возвращает `1`.
   * `ec_manifest_ptr() -> i32` и `ec_manifest_len() -> i32` — возвращает UTF-8 JSON-манифест с описанием параметров и категории в Effects & Presets.
   * `ec_alloc(bytes: i32) -> i32` — аллоцирует буфер для параметров и пикселей.
   * `ec_render(pixels: i32, width: i32, height: i32, params: i32, nparams: i32, time: f64, scale: f64) -> i32` — обрабатывает кадр in-place (RGBA `f32` premultiplied).

### Формат пикселей:
* `f32` значения от `0.0` до `1.0` (может превышать `1.0` в 32 bpc HDR проектах).
* Premultiplied RGBA (цвет помножен на альфу).
* Обработка: `(r, g, b) = (c[0] / a, c[1] / a, c[2] / a)`. После эффекта: `c[i] = v * a`.

---

## 📦 Сборка и тестирование

### 1. Сборка плагина в `.wasm`:
```bash
cargo build --release --target wasm32-unknown-unknown
```
Скомпилированный файл появится в:
`target/wasm32-unknown-unknown/release/<plugin_name>.wasm`

### 2. Тестирование в установленном EffectCraft:
* Запустить `effectcraft`.
* Создать композицию, добавить слой (картинку / solid).
* В верхнем меню нажать: **`Effect ▸ Load Effect Plug-in…`** и выбрать `.wasm` файл.
* Эффект появится в панели **Effects & Presets** под своей категорией и будет доступен на слое с полным UI и кейфреймами.
* Также можно закинуть `.wasm` в `~/.config/effectcraft/Plug-ins/` для автозагрузки при старте.

---

## 📌 Окружение на машине
* **OS:** Arch Linux (`qwertyonek-arch`)
* **EffectCraft binary:** `/usr/local/bin/effectcraft` (с нашей кастомной кнопкой Browse для LUT)
* **EffectCraft source:** `~/dev/effectcraft`
* **Rust target wasm:** `wasm32-unknown-unknown` (уже установлен в `rustup`)
