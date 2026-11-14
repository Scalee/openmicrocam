# ldc-2-eptz-lib — the map-generation (eptz) library inside mi_ldc.ko: table layout, tiling, projection and lens models, text configuration, MVE and Simd
<!-- ev:ldc-2-eptz-lib -->

The part of `mi_ldc.ko` that turns a view's parameters (pan / tilt / rotate / zoom, a projection model, a lens
calibration) into the table ("bin") the LDC engine consumes, and the image helpers linked beside it:

- **The map-generation library ("eptz lib", ≈86 KB, ≈250 named + ≈45 unnamed functions)** — SigmaStar algorithm code with
  no source: a work buffer and runtime handle (§1), per output-grid node a source position from one of the projection
  models (§4) through the lens model (§9), optional crop to the valid source area (§5), the cut of the output into engine
  blocks under the hardware limits (§3), and the byte layout of the table (§2). Two-sensor stitching is present but
  unreachable (§6); file helpers and the table joiner (§7); fixed-point math, checks, parsing and the return codes (§8);
  the text configuration (§10).
- **MVE** (`MI_MVE_*`, 9 functions, 3.5 KB) — SigmaStar's software image helpers on Simd (§11.1).
- **Simd** — a vendor copy of the public ermig1979/Simd library (MIT): 100 template instances in comdat sections plus ≈50
  stripped `.text` helpers, ≈220 KB (§11.2–§11.5).

Other documents: mi_ldc.ko as a module (composition table, parameters, exports, imports, use on this camera), the channel
attribute, the JSON view object with the para offsets of its keys and the `ldc_mode` name table, the summary of the text
options and of the calibration file `CalibPoly_new.bin`, the chip check and its consequence, the GENBIN / EPTZ / DBIN front
ends that call this library, the view resources, the per-frame consumer of the table, the blending path, the proc
interface and mhal LDC (registers 12F0 / 12F1) — `ldc-1-stack.md` (§1 "The component on this camera", §4.1
"`MI_LDC_ChnAttr_t`", §4.4 "One view object in JSON", §4.5 "Text configuration", §4.6 "Calibration data", §5.3 "Building
the tables", §5.4 "View resources (double buffer) and PTS selection", §6.1 "The table ("bin") as the per-frame path consumes
it", §6.5 "DIS (eMode 4), the "hard EIS" switch and blending", §7 "Proc interface", §8 "mhal LDC block", §9 "The same stack
in sibling firmwares of the SoC"); the chip id and the sysfs `CHIP_ID` — `kernel-platform.md` §3.2 "Chip ops, chip id and
revision"; the second copy of the same vendor Simd tree, in libmi_ive.so, and the MVE core there — `ive-video-analytics.md` §6.1, §3;
the inventory row of mi_ldc.ko — `firmware-image.md` §3.3 "SigmaStar media stack — `/config/modules/4.9.227`".

Tags: **[R]** read from the binary, a decompile, disassembly, an emulation run of the module's own code, or a file; **[S]**
stated by a source; **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.
Function names are the module's symbols (the library keeps its symbol names); functions without a symbol are named here by
what they do [I], with their address; `_sym_N` the vendor-renamed local symbols of the Simd part.

Binary (dump of 2026-10-16) [R]: `/config/modules/4.9.227/mi_ldc.ko`. "Matches the module" below means identical results
of a model and the module's code under emulation on every trial (runs of 2026-10-25) [R]. Addresses are Ghidra addresses
(= ELF `.text` symbol value − 1 (Thumb bit) + 0x10030).

Conventions: **Q15** = s32 with 15 fraction bits (value × 32768); **sll** = s64 32.32 fixed point; float = IEEE single.
**para** = the 0x16F8-byte view parameter structure (`mi_eptz_para`) filled from JSON (`ldc-1-stack.md` §4.4) or from the
text configuration (§10); **rt** = the library's runtime handle (0x1AA0 bytes at the start of the work buffer, §1.3);
**sp** = the split state inside rt (§3.1); **w** = the lens state inside rt (§9.1). Offsets are hex bytes. A JSON or text
field is named by its JSON key / para field name (`ldc_mode`, `grid_size`, `hw_mode` …). in_w / in_h, out_w / out_h =
para in_width / in_height, out_width / out_height; grid_w / grid_h = para grid_size / grid_size_h.

Float and double literals were read from the Thumb disassembly with the pc-relative literals (`vldr`) resolved to their
values, so every float / double constant below is [R]; behaviour marked [R: emulation] was run under unicorn on the
module's own code (relocations applied, VFP / NEON on, any call outside the module stops the run).

## 1. The library: parts, pipeline, work buffer, runtime handle, call order

### 1.1 Parts [R function names, call graph, strings]
- The library carries no source-file names of its own; the strings name only the mi_ldc front ends that call it,
  `mi_ldc_eptz.c`, `mi_ldc_genbin.c`, `mi_ldc_dbin.c` (`ldc-1-stack.md` §5.3).
- Function families: `eptz_*`, `mi_eptz_*` (runtime API and text configuration), `world2cam_*` and `zl_*` (lens model,
  §9), `ldc_bin_*` (table, §2, §7), `ldc_split_*` and `eis_*` (tiling, §3), the projection models `bypass_*`, `donut_*`,
  `erp_*`, `cylinder_*`, `RSC_*`, `perspective_*`, `map2bin_*` (§4), `out_set_*` (output geometry, §4.1), `map_crop_*`,
  `region_*`, `map_region_*` (crop and protection, §5), `mi_st_stitch_*`, `st_stitch_*`, `mask_stitch_*` (stitching, §6),
  and the support code (§8).
- Fixed-point helpers: `sll*` (32.32 math: sin / cos / tan / atan / atan2 / sqrt / exp / log / pow / inv, `dbl2sll` /
  `sll2dbl`), `Q15_*` / `FloatoQ15` / `Q15toFloat` / `IntoQ15` / `Q15toQn`, `clip_*`, `lerp`, `bilinear_*` (map
  resampling) (§8.1). Floating point (VFP / NEON) is used too; mi_ldc brackets the library calls with
  `kernel_neon_begin/end`.

### 1.2 Pipeline per view [R]
`mi_eptz_runtime_map_gen` → `eptz_lib_parameter_set` → `eptz_lib_map_gen` → `ldc_split_process` → `ldc_bin_gen`
(or the size / buffer pair `mi_eptz_runtime_bin_size` / `mi_eptz_runtime_bin_gen`, §1.5):
1. **Parameter set** per model (`<model>_parameter_set`, §4): pan / tilt / rotate / zoom of the fisheye dewarp
   (`eptz_*`, with the rotations `eptz_matrix_Mx_apha / My_beta / Mz_gama`, `eptz_geographic2spherical`,
   `eptz_spherical2plane`); ring radii and angles of the donut panorama (`r_inside`, `r_outside`, `theta_start`,
   `theta_end`); ERP; cylinder; RSC (region + m33, `RSC_calculate_eis_split`, `RSC_dispmap_gen`); perspective
   (`single_point_perspective_transform`); map2bin (`map2bin_map_read` / `map2bin_map_float2Q15`: external X / Y maps);
   stitch (`st_stitch_*`, `st_stitch_info.bin`, `mask_stitch_generate_mask`, `find_*_value_from_two_map*`,
   `region_overlap_*`); zoom lens (`eptz_zoom_lens_calculate`, `zoom_lens_step_calculation`, `zl_*`).
2. **Map generation** (`<model>_map_gen` / `<model>_calculate`): for every node of the output grid the model turns the
   output position into a 3-D ray and `world2cam_table` turns the ray into a sensor position (models without a lens model:
   bypass, RSC, perspective, map2bin) → a map of Q15 input positions (§4, §9).
3. **Crop** (`map_crop_process`, only with `crop_en` ≠ 0): the output is stretched over the largest valid source
   rectangle (§5.1). The other protection helpers (`map_protection`, `map_region_*`, `region_map_find_*` on float maps)
   belong to the stitch code or have no caller (§5.4).
4. **Split** (`ldc_split_*`): the output is cut into blocks ("tiles") the engine does in one run, under limits named in the
   debug strings — `HW_LIMIT_Y_SHIFT`, `HW_LIMIT_WIDTH`, `OUT_LIMIT_WIDTH`, `OUT_GRID_WIDTH_LIMIT` — with an input bounding
   box per tile (`find_vertical`, `iterative_find`), y-shift and inverted-y checks, "Over max tile number" beyond 650
   tiles; `ldc_eis_split` = 1 uses `eis_evenly_split_*` (§3).
5. **Table** (`ldc_bin_*`): per tile the block descriptor, the node grid, the delta table (source displacements from the
   node positions, `ldc_bin_DeltaYX_convert`, precision `xBaseBits` / `yBaseBits`), the mask table and the headers (§2);
   `ldc_bin_concantenate` joins tables (§7.2), `ldc_bin_write*` / `ldc_bin_read*` are the file paths (§7.1).

### 1.3 Work buffer and the runtime handle `rt` [R `eptz_get_buffer_info` 0x28518, `eptz_get_buffer_ptr` 0x2863c, `buffer_malloc` 0x37170]
`buffer_malloc(&{base, size, used}, n, &ptr)` is a bump allocator: `ptr = base + used`, `used += align4(n)`, fails (−1, ptr 0)
when `used > size`. The caller (mi_ldc, `ldc-1-stack.md` §5.3: `MemAlloc("ldc-lib-workbuf")`) allocates one work buffer
of `mi_eptz_get_buffer_info(para)` bytes and `eptz_get_buffer_ptr` carves it, after `memset(0)`, in this order:

| order | size (bytes) | pointer stored at | content |
|---|---|---|---|
| 1 | 0x1AA0 | — (it *is* `rt`) | runtime handle |
| 2, 3 | N·4 each | rt+0x198C, rt+0x1990 | **map X, map Y**: one s32 Q15 input position per output grid node |
| (crop_en ≠ 0) 4 × | N·4 | rt+0x1A84 … +0x1A90 | four node maps for `map_crop_*` (§5.1) |
| calib_type 3 | Z·4, Z·0x108, Z·2·0x5002, 2·N·4 | rt+0x18D4 (= Z), +0x18D8, …, rt+0x1A98 / +0x1A9C | zoom-lens step tables (§9.5) |
| calib_type ≠ 3 | 2 × 0x5002 | rt+0x1840, rt+0x189C | the two radius LUTs of the lens model (curve 0 / curve 1, 0x2801 u16 each, §9.1) |
| next | 0x6CDCC0 | rt+0x197C | **tile array**: 650 tiles × 0x2AE0 (§3.3) |
| next | 0xC400 | rt+0x1988 | path block: file-path strings (debug / file-based calibration, §10.3) |
| next, next | M·4 each (mode 13: N; mode 9 with `ldc_eis_split` 0: map_w·map_h nodes; else 0) | rt+0x1A58, rt+0x1A5C | external X / Y maps (map2bin) or RSC displacement maps |
| mode 9 only | 0x2136C (eis split 1) or 2 × N·4 | rt+0x1A80 or rt+0x1A70 / +0x1A74 | EIS even-split work area / RSC maps |
| mode 9 only | 2 × N·4 | rt+0x1A68, rt+0x1A6C | RSC maps |

Finally rt+0x1984 = 0x200 (status). N = `nodes_x · nodes_y` with **nodes_x = (align_up(out_w, grid_w) + grid_w) / grid_w**,
nodes_y likewise with out_h / grid_h; `eptz_get_buffer_info` stores them at para+0x143C / +0x1440 (`out_width_tile`,
`out_height_tile`) — one node every grid_w output pixels **plus one extra column / row beyond the aligned edge**. Size
formula: `0x1AA0 + 8N (+16N if crop_en) + (calib_type 3 ? 8N + 0x10C·Z + 0xA008·Z : 0xA008) + 0x6DA0C0`, plus for mode 13
`8N`, for mode 9 `8N + 8·map_tiles` (eis_split 0) or `8N + 0x21360 …` (eis_split 1: the function returns
`… + 0x6FB42C + 8N`). The tile array (0x6CDCC0, ≈7.2 MB) dominates the size.

### 1.4 `rt` layout (the fields the generator uses) [R writes in `eptz_init`, `*_out_init`, `out_set_*`, `ldc_split_init`]
| offset | content |
|---|---|
| 0x0000–0x166F | copy of para 0x0000–0x166F (`memcpy` in `eptz_init`; so rt+0x1428 = ldc_mode, +0x1450 grid_size … at the para offsets of `ldc-1-stack.md` §4.4) |
| 0x1670 / 0x1674 / 0x1678 / 0x167C | view index; pan, tilt, rotate after clamping to [min, max] (degrees, int) |
| 0x1680 | zoom · 0.01 (float; zoom clamped to [zoom_min, zoom_max] first) |
| 0x168C / 0x1690 / 0x1694 | π (float); 1.0 (later `fc`, §4); 2.0944 = 2π/3 (float) |
| 0x169C / 0x16A0 | radius_min / radius_max (also **written back into para's r_inside / r_outside**, so the JSON `r_inside` / `r_outside` are overwritten by `radius_min` / `radius_max`) |
| 0x16B0 | fov_out_1p, default 180 when 0 |
| 0x16B8 / 0x16C8 | RSC: rsc_region {x, y, w, h}; m33 (nine words, also perspective) (§4.7, §4.8) |
| 0x16F8 / 0x16FC | calibration image width / height (from the lens model, §9; = in_w / in_h for calib_type 2) |
| 0x1700 / 0x1704 | lens centre xc, yc used by the models (§4.2) |
| 0x1708 / 0x170C | Q15 resize factors: `x_resize_ratio · 0.01 · in_w / calib_width` (y likewise) |
| 0x1714 / 0x1718 | input width / height; 0x171C / 0x1720 input x / y (= 0); 0x1724 / 0x1728 = in_w/2, in_h/2 |
| 0x172C rect S | the **screen** (whole output picture): +4 w, +8 h, +0xC x, +0x10 y, +0x14 w/2, +0x18 h/2 → rt+0x1730 w, +0x1734 h, +0x1738 x, +0x173C y |
| 0x1748 rect V | the **view** inside the screen: rt+0x174C w = out_w, +0x1750 h = out_h, +0x1754 x, +0x1758 y, +0x175C / +0x1760 = view centre (w/2 − output_offset_x, h/2 − output_offset_y) |
| 0x1768 / 0x176C | map_width / map_height; 0x1770 / 0x1774 = 0; 0x1778 half map size (RSC) |
| 0x1780 / 0x1784 | grid_w / grid_h (para grid_size / grid_size_h) |
| 0x1788 / 0x178C | RSC / perspective: first node column / row ⌈V.x/grid_w⌉, ⌈V.y/grid_h⌉ |
| 0x1790 / 0x1794 | nodes_x / nodes_y (§1.3); 0x17A0 = ((nodes_x+1) >> 1)·4 |
| 0x1798 / 0x179C | effective output rotation (degrees) / **flip code** written to the table (0 or 3 = mirror + flip = 180°) |
| 0x17A8 … ≈0x18E8 | lens state `w` (§9.1); +0x18E4 = fc, +0x18E8 = pixel factor pf |
| 0x18EC … +0x198 | **split state `sp`** (§3.1); rt+0x1978 = number of tiles, rt+0x197C = tile array |
| 0x1984 | status (0x200) |
| 0x1A78 | call-order state: 2 = idle / tables done, 0 = parameters set, 1 = maps generated |

### 1.5 Call order and the `ldc_mode` dispatch [R `mi_eptz_runtime_*`, `eptz_lib_*`]
1. `mi_eptz_runtime_init(buf, size, para)` → `Platform_Check` (chip id 0xF4, §8.2) → `eptz_map_init`: carve the buffer
   (§1.3), `eptz_init` (clamps, lens model init, output layout `eptz_dst_init` → `<model>_out_init`, `ldc_split_init`),
   and for RSC with `ldc_eis_split` 0 the displacement maps (`RSC_dispmap_gen`). Returns `rt` or NULL.
2. `mi_eptz_runtime_map_gen(rt, para, &bin, &size)` = `eptz_lib_parameter_set(rt, para)` (state must be 2 → 0, else 0x218)
   then `eptz_lib_map_gen` (state 0 → 1 … 2): per-mode `<model>_parameter_set` (§4), the node maps (`<model>_map_gen`),
   `map_crop_process` if crop_en (§5.1), `ldc_split_process` (§3), `ldc_bin_gen` (§2) into a buffer the library allocates
   (`CamOsMemAlloc(0x60 + n·0x16C0)`) — size returned = 0x60 + n·0x16C0.
3. Alternatively `mi_eptz_runtime_bin_size` (parameter set + maps + split, returns the size, state 1) then
   `mi_eptz_runtime_bin_gen(rt, buf, &size)` → `ldc_bin_gen_with_ptr` into a caller buffer (mi_ldc's `ldc-loadbin`,
   `ldc-1-stack.md` §5.3). `ldc_bin_gen` and `ldc_bin_gen_with_ptr` produce identical bytes (same body, only the buffer
   source differs).
4. The `ldc_mode` dispatch (`eptz_lib_parameter_set`, `eptz_lib_map_gen`, `eptz_dst_init`):

| ldc_mode | parameter set | map gen | out init |
|---|---|---|---|
| 3, 4, 8, 10 | `donut_parameter_set(view, r_inside, r_outside, theta_start, theta_end, out_rot, fc)` | `donut_map_gen` | `donut_out_init` |
| 5 | `erp_parameter_set(view, pan, tilt, rotate, zoom_h, zoom_v, out_rot, fc, distort_intensity)` | `erp_map_gen` | `erp_out_init` |
| 6 | `bypass_parameter_set(view, out_rot)` | `bypass_calculate` | `bypass_out_init` |
| 9 | `RSC_parameter_set(view, zoom, m33, rsc_region)` | `RSC_map_gen` | `RSC_out_init` |
| 11 | `cylinder_parameter_set(view, pan, tilt, zoom_h, zoom_v, fc)` | `cylinder_map_gen` | `cylinder_out_init` |
| 13 | `map2bin_parameter_set(view)` | `map2bin_map_gen` | `map2bin_out_init` |
| 15 | `perspective_parameter_set(view, m33)` | `perspective_map_gen` | `perspective_out_init` |
| 14 | — (separate `mi_st_stitch_*` runtime, §6) | — | `st_stitch_out_init` |
| 0, 1, 2, 7, 12 and any other | `eptz_parameter_set(view, pan, tilt, rotate, zoom, out_rot, distort_intensity, fov_h, fov_v)` | `eptz_map_gen` | `eptz_out_init` → **only 0 / 1 / 2 / 7 accepted, others 0x204** (so mode 12 "stitch" is rejected at init) |

   After the model's parameter set: rt+0x175C −= output_offset_x, rt+0x1760 −= output_offset_y (output principal point),
   `ldc_split_init` again, `world2cam_fc_set(rt+0x1690)`; modes 5, 11 and the eptz default also call
   `world2cam_distort_intensity_set(distort_intensity)`. mi_ldc itself sends only modes 0–11 to the library (12–15 are
   refused before generation, `ldc-1-stack.md` §4.4).

## 2. The table ("bin") the LDC engine consumes — byte layout [R `ldc_bin_gen` 0x31e18 = `ldc_bin_gen_with_ptr` 0x32130, `ldc_bin_header_set` 0x31a70, `ldc_bin_block_set` 0x31ca8, `ldc_bin_DeltaYX_convert` 0x31b00 (read as disassembly), `ldc_split_map_downsample` 0x2a13c; consumer side mhal `MHAL_LDC_SetBlkAttr` / `HAL_LDC_SetBlockAttrEx` 0x4009b5a0]

One table per view, little-endian, size **0x60 + n·0x16C0** (n = number of blocks, ≤ 650). `ldc_bin_locate_buffer_ptr`
confirms the three parts: file header at +0, view header at +0x20, block records from +0x60. Below each part is counted
from its own start; the same bytes counted from the table start, as the per-frame path reads them, are in
`ldc-1-stack.md` §6.1, and the registers named in the "where it goes" columns in `ldc-1-stack.md` §8.4. Whether the engine
reads the table as laid out here has not been checked on hardware: every statement of this section comes from the
generator and from mhal's code; no stock table dumped on the camera (proc `dumptablebin`, `ldc-1-stack.md` §7.2) and no DBIN
container fed through `prog_uvc -L` (`ldc-1-stack.md` §5.3) has been compared with it (`docs/firmware/camera-checks.md`
P15). The four sibling builds of mi_ldc.ko generate tables of the same layout [I: same constants and function sizes;
header use not compared] (`ldc-1-stack.md` §9).

### 2.1 File header (0x20 bytes, table +0x00)
| off | type | value |
|---|---|---|
| 0x00 | s16 | ldc_mode (`ldc-1-stack.md` §4.4) — only the proc view and `_MI_LDC_EPTZ_LibBinCheck` read it |
| 0x02–0x1F | — | **not written** by the generator (whatever the buffer held; mi_ldc's `ldc-loadbin` comes from the mi_sys heap); nothing in mi_ldc reads them [I: no reads found in the consumers] |

### 2.2 View header (0x40 bytes, table +0x20)
| off | type | value | consumer in mi_ldc / mhal |
|---|---|---|---|
| 0x00 | u32 | view index (rt+0x1670) | proc; view slot (`ldc-1-stack.md` §5.4) |
| 0x04 | u32 | **n = number of blocks** ("NumRuns") | loop count |
| 0x08 | u32 | "Fmt" = sp+0x60, always **0** (§3.1) | frame attribute |
| 0x0C | u32 | "OpMode" = **hw_mode** (0–4; any other value written as 5 — but `ldc_split_init` has already replaced an invalid hw_mode by 3) | frame attribute: block-size code → 12F0.01 [6:4], 12F0.12 = 0x10 << c |
| 0x10 | u32 | flip code rt+0x179C ("eFlipMode"): **0** or **3** (both mirror bits = 180° turn, used by the 4R / 1R CM modes when rotate ∈ [90, 270], §4.1) | WDMA: bit 0 h-mirror, bit 1 v-flip |
| 0x14 | s16 ×4 | input rect x, y, w, h = rt+0x171C, +0x1720, +0x1714, +0x1718 (= 0, 0, in_w, in_h) | input offset |
| 0x1C | s16 ×4 | output rect: x = V.x + S.x, y = V.y + S.y, w = out_w, h = out_h | GetOutputPortAttr / proc |
| 0x24 | s16 ×4 | screen rect S: x, y, w, h (rt+0x1738, +0x173C, +0x1730, +0x1734) | output port size (table +0x48 / +0x4A) |
| 0x2C–0x3F | — | not written | — |

### 2.3 Block record (0x16C0 bytes each, table +0x60 + b·0x16C0)
| off | size | content | where it goes (mhal) |
|---|---|---|---|
| 0x00 | s16 ×4 | **InBlk** x, y, w, h = the tile's input bounding box (§3.4) in input pixels | x, y → 12F1.0C / .0D; w, h → 12F0.0B / .0C and 12F1.0E / .0F |
| 0x08 | s16 ×4 | **OutBlk** x = 0, y = 0, w, h | x, y → 12F0.0D / .0E; w, h → 12F0.0F / .10 |
| 0x10 | s16 ×4 | **SclBlk** x = x0·grid_w + V.x, y = y0·grid_h + V.y, w, h (same w, h as OutBlk) | mi_ldc: WDMA output address = buffer + y·stride + x |
| 0x18 | u8 ×2 | scale x, scale y = 1, 1 (the tile's float scale 1.0 truncated) | **not sent to hardware** |
| 0x1A | u8 ×2 | delta base bits x, y = `xBaseBits`, `yBaseBits` (default 4, 4): the delta precision | 12F0.13 = (x & 7) << 10 \| y << 13 \| 1 |
| 0x1C | u8 | bOutCropEn = sp+0x70 = **1** always | 12F0.0A bit 0 |
| 0x1D–0x1F | — | not written | — |
| 0x20 | 0xA0 | **engine configuration block** = the block's node grid (§2.4) | address >> 5 → 12F1.10 / 12F1.11; the engine fetches it |
| 0xC0 | 0x1500 | **delta table** (§2.5) | address >> 5 → 12F1.03 / .04, 12F1.02 bit 1 |
| 0x15C0 | 0x100 | **mask table: always zero-filled** by the generator (`memset 0x100`, §2.6) | address → 12F1.12 / .13 only when the mhal mask enable is on (12F1.02 bit 0) |

Block width / height: w = (tile node columns − 1)·grid_w, h = (tile node rows − 1)·grid_h, each clipped so that the block
ends at **align_up(out_w, 16)** / **align_up(out_h, 2)**; a block that starts at or beyond that edge aborts the build with
0x22D. SclBlk x / y get V.x / V.y added after the check.
Engine units: the three table addresses are 32-byte units (>> 5), so each block record is 32-byte aligned when the table
buffer is (0x60, 0x16C0, 0x20, 0xC0, 0x15C0 are all multiples of 0x20) [R sizes / I requirement from the >> 5].

### 2.4 Engine configuration block (0xA0 bytes, record +0x20) — the node grid of the block
| off | type | content |
|---|---|---|
| 0x00 | u16 ×42 | **xs[0..41]**: x positions (output pixels, relative to the block) of the 42 table columns |
| 0x54 | 12 bytes | all **0xFF** (two overlapping 8-byte `vst1.8` of `vmov.i8 d16, #0xff` at record +0x74 and +0x78 — Ghidra's decompile shows them wrongly as `0xff` words) |
| 0x60 | u16 ×32 | **ys[0..31]**: y positions (output pixels, relative to the block) of the 32 table rows |

[I] The block is most likely `u16 xs[48]; u16 ys[32]` with the 6 unused column slots set to 0xFFFF — 48 + 32 u16 = 0xA0
exactly. The meaning of the 12 × 0xFF bytes is not known and cannot be settled statically: they are constant, and mhal
never reads the block (the engine fetches it).
Rule for xs / ys (`ldc_split_map_downsample`), with W = tile node columns, H = tile node rows, gw / gh = grid_w / grid_h:
- x: if W ≤ 42: xs[i] = i·gw for i < W, xs[i] = (W−1)·gw for W ≤ i ≤ 41. If W > 42 (more nodes than columns): step =
  Q15((W−1)·gw) / 41 (Q15 division), xs[0] = 0, xs[41] = (W−1)·gw, xs[i] = gw·⌊round(i·step)/gw⌋ for 1 ≤ i ≤ 40
  (round = +0.5 then floor in Q15, i.e. positions snap down to a node).
- y: the same with 32 rows: ys[j] = j·gh (H ≤ 32, padded with the last), or step = Q15((H−1)·gh)/31.
- **then always**: xs[40] = xs[41]; xs[41] = xs[41] + 32 — the last real column is moved to index 40 and a virtual
  column 32 pixels to its right is appended (its delta is adjusted so that it samples the same source point, §2.5).
  When W ≥ 42 this drops the node that was at index 40. y has no such virtual row.
- Each table node (i, j) refers to the generated map node (x0 + xs[i]/gw, y0 + ys[j]/gh) of the tile.
- A tile with one table column or one table row fails with 0x208.

### 2.5 Delta table (0x1500 bytes, record +0xC0) — source displacement per node
Per node (column i 0..41, row j 0..31) the generator first computes, in Q15 pixels:
- vx = mapX(node) − bb.x·2^15, vy = mapY(node) − bb.y·2^15 (source position relative to the InBlk origin);
- a soft clamp (modes ≠ 9, all nodes; RSC mode 9: only the first / last row and column): if 0 ≤ v < 1.0 → v = 1.0;
  else if (bb.w − 3) < v < bb.w → v = bb.w − 3 (for vy: bb.h); values < 0 or ≥ bb.w are **left as they are** [R, read in
  the disassembly at 0x2a458–0x2a4a8];
- **dx = vx − xs[i]·2^15, dy = vy − ys[j]·2^15** (displacement from the output node position to its source, Q15);
- virtual column: after all nodes, column 40 := column 41 (dx and dy) and dx[j][41] −= 32·2^15 (so xs[41] + dx[41] still
  points at the source of the last real node).

Encoding (`ldc_bin_DeltaYX_convert`, disassembly 0x31b00–0x31ca6): each value becomes **s16 = round(d · 2^k)** with
k = delta base bits (`(d << k) ± 0x4000` then symmetric `>> 15`: round half away from zero; truncated to 16 bits by
`strh`, no saturation). Layout: **16 row-pairs × 21 column-pairs × 16 bytes**; the 16-byte entry e = 21·(j >> 1) + (i >> 1)
holds a 2 × 2 quad of nodes (columns 2c, 2c+1; rows 2r, 2r+1):

| entry byte | value |
|---|---|
| 0x0 / 0x2 | dy, dx of node (2c+1, 2r+1) |
| 0x4 / 0x6 | dy, dx of node (2c, 2r+1) |
| 0x8 / 0xA | dy, dx of node (2c+1, 2r) |
| 0xC / 0xE | dy, dx of node (2c, 2r) |

Base bits used per value: odd rows (2r+1) all use `yBaseBits`; even rows (2r) use `xBaseBits` **except dy of the odd
column (byte 0x8), which uses `yBaseBits`**. With the defaults xBaseBits = yBaseBits = 4 (1/16 pixel, range ±2048 px) the
asymmetry has no effect [I: a vendor slip if the hardware applies 12F0.13 [12:10] to x and [15:13] to y; with both equal it
does not matter].
The engine's mapping, as the generator's arithmetic implies [I]: output pixel (OutBlk origin + (xs[i], ys[j])) samples input
(InBlk.x + xs[i] + dx/2^k, InBlk.y + ys[j] + dy/2^k), with interpolation between the nodes of the irregular 42 × 32 grid.

### 2.6 Mask table (0x100 bytes, record +0x15C0)
Always zero in every table this library writes (`memset(…, 0, 0x100)`); mhal enables its fetch only with its own
mask-enable knob (`DRV_LDC_GetMaskTblEn`, `ldc-1-stack.md` §8). Its encoding is defined by no vendor generator on the camera
[R absence] and is not known; 0x100 bytes = 2048 bits [I]. The blend masks of the stitch code (§6) are a different thing
(software alpha blending), not this table [I].

### 2.7 What a valid table satisfies [I, from §2.1–§2.6 and §3]
- grid_w a power of 2 from 1 to 64 (block widths are multiples of 16 output pixels, §3.2); hw_mode 0–4 selects the input line buffer
  from 2048 px × 16 lines to 128 px × 256 lines (§3.1); delta base bits 4 / 4 by default.
- Per block: input bounding-box width ≤ 2048 >> hw_mode, per-row source span ≤ (16 << hw_mode) − 1 lines, output block
  width < 2048 and a multiple of 16, at most 42 table columns × 32 rows after downsampling (§3).
- InBlk = the aligned source bounding box (§3.4), xs / ys and dx / dy as §2.4–§2.5; header and view header as §2.1–§2.2;
  the table buffer 32-byte aligned (§2.3).

## 3. Tiling: `ldc_split_*` — how the output is cut into engine blocks, and the hardware limits

The tile layout the engine gets for a real configuration on this camera has not been read: the table info of
`/proc/mi_modules/mi_ldc/mi_ldc0` (NumRuns, block sizes) and mhal's `check_*` sysfs attributes (`check_clk`,
`check_inst_table_info`) show it only with prog_uvc run with `-L` and a JSON, which changes state (`ldc-1-stack.md` §1.3,
`docs/firmware/camera-checks.md` P14). Everything below is the generator's code.

### 3.1 Split state `sp` (rt+0x18EC) [R `ldc_split_init` 0x2984c; names from the debug strings of `ldc_split_find_vertical`]
| sp+ | value | meaning |
|---|---|---|
| 0x00 / 0x04 | out_w / out_h (V.w, V.h) | |
| 0x08 / 0x0C | nodes_x / nodes_y | map grid size (nodes_x is the map row stride) |
| 0x10 / 0x14 | 0x80 / 0x80 | not read by any split function [R: no reads found] |
| 0x18 / 0x1C | **42 / 32** | table columns / rows per block (§2.4) |
| 0x20 / 0x2C | grid_w / grid_h | |
| 0x24 / 0x28 | 16 / 2 | output alignment of the block end (x / y) used by `ldc_bin_gen` (§2.3) |
| 0x30 / 0x34 | 0 / 0 | origin added to xs / ys (§2.4) |
| 0x40 | 16 << hw_mode (16 … 256) | input line-buffer height; **HW_LIMIT_Y_SHIFT = this − 1** |
| 0x44 / 0x48 | xBaseBits / yBaseBits (both 4 when both are 0 in the configuration) | delta precision (§2.5) |
| 0x4C | 2 | margin m (pixels) around the source bounding box |
| 0x50 / 0x54 | 32 / 2 | input bounding box x / y alignment |
| 0x58 / 0x5C | 32 / 2 | input bounding box width / height granularity (also minimum width 32, minimum height 6 = 3·2) |
| 0x60 | 0 | "Fmt" written to the view header |
| 0x64–0x6C | 1, 1, 1 | not read [R] |
| 0x70 | 1 | bOutCropEn written to every block |
| 0x74 | 0x800 | **OUT_LIMIT_WIDTH** = 2048: output block width (pixels) must be < this |
| 0x78 | hw_mode (para+0x1480); an invalid value (not 0–4) becomes **3** | |
| 0x7C | 0x800 >> hw_mode (2048, 1024, 512, 256, 128) | **HW_LIMIT_WIDTH**: input bounding-box width must be ≤ this |
| 0x80 / 0x84 | in_w / in_h | |
| 0x88 | initial number of horizontal bands, from the clamped `rotate` angle: 0–22 → 1, 23–35 → 2, 36–145 → 3, 146–158 → 2, 159–202 → 1, 203–215 → 2, 216–325 → 3, 326–338 → 2, ≥ 339 → 1 | |
| 0x8C | 650 (max tiles), then the number of tiles produced | |
| 0x90 | tile array (rt+0x197C) | |
| 0x94 | ldc_mode | |
| 0x194 | EIS even-split work area (rt+0x1A80) | |

From the para outside `sp`: **OUT_GRID_WIDTH_LIMIT = `table_width`** (para+0x1494): a block may span fewer than
`table_width` node intervals [R argument of `ldc_split_find_vertical`]. The text configuration defaults it to 256 (§10.1); a
JSON view without `"table_width"` reads 0, which makes every block fail this test, so the split ends with 0x21E [R control
flow / I: a JSON configuration works only with table_width set].
The hardware meaning of the limits [I from the pairing 2048 >> m with 16 << m, and mhal's sysfs text "FbMode (0:2048,
1:1024, 2:512, 3:256, 4:128)", `ldc-1-stack.md` §8.8]: the engine keeps a **32768-pixel input line buffer** arranged as
(2048 >> hw_mode) pixels × (16 << hw_mode) lines; a block's whole source bounding box must fit its width, and between two
consecutive table rows the source may move by at most its height.

### 3.2 Default split (`ldc_eis_split` = 0): `ldc_split_process` → `ldc_split_find_vertical` [R 0x2ae40, 0x2a8f8]
`ldc_split_process`: for nb = sp+0x88 … sp+0x88 + 31: run `find_vertical(nb bands)`; result 0 = done, 2 = more than 650
tiles → **0x21F**, 1 = some band cannot be split → try nb + 1; after 32 tries **0x21E** ("LDC split iterative find error :
too many tiles"). `ldc_eis_split` not 0 / 1 → 0x239.
`find_vertical(nb)`:
1. Band height (node rows): hb = round((nodes_y − 1) / nb) made even (rounded half up, then the low bit cleared). Band k
   covers node rows y0 = k·hb … y1 = y0 + hb (the last band ends at nodes_y − 1). Bands share their boundary row.
2. Column step st (node columns): 1 if grid_w ∈ {16, 32, 64}, else 16 / grid_w (so block widths are multiples of 16 output
   pixels; "Invalid grid size" is printed if grid_w does not divide 16).
3. In each band, blocks are grown greedily from the left: start s = 0, end e = st; test(s, e) =
   (a) input bounding box of nodes [s..e] × [y0..y1] (§3.4) has **bb.w ≤ HW_LIMIT_WIDTH**,
   (b) **y_max_shift (§3.5) ≤ HW_LIMIT_Y_SHIFT**,
   (c) **grid_w · (e − s) < OUT_LIMIT_WIDTH** (2048),
   (d) **e − s < table_width**,
   (e) if the tile is "vertically inverted" (§3.6): **bb.h ≤ HW_LIMIT_Y_SHIFT**.
   While (a)–(e) hold and e < nodes_x − 1, e += st. When a test fails, the block [s, e − st] is emitted (it must be
   non-empty and pass (a)–(e), else the band fails → return 1); when e reaches nodes_x − 1 and passes, [s, e] is emitted.
   The next block starts at the previous end (blocks share their boundary column). If e jumps past nodes_x − 1 without
   landing on it ((nodes_x − 1) not a multiple of st) the band ends without its last block [R loop structure / I
   consequence: an out_w that is not a multiple of 16 and of grid_w leaves the right edge uncovered].
4. Each emitted tile records node range, bounding box and y_max_shift (§3.3); > 650 tiles → return 2.

Then for every tile: scale x = scale y = 1.0, a copy of the node range at +0x18, and `ldc_split_map_downsample` (§2.4, §2.5)
— failure there ("Fail to convert to delta/pos map") returns its code (0x208 when the table has 1 column or 1 row).

### 3.3 Tile record (0x2AE0 bytes; array at rt+0x197C) [R]
| off | content |
|---|---|
| 0x00 / 0x04 / 0x08 / 0x0C | x0, y0, x1, y1 (node indices, inclusive) |
| 0x10 / 0x14 | W = x1 − x0 + 1, H = y1 − y0 + 1 (nodes) |
| 0x18–0x2C | copy of 0x00–0x14 |
| 0x30 / 0x34 / 0x38 / 0x3C | input bounding box x, y, w, h (pixels) |
| 0x40 / 0x44 | f32 scale x / y (1.0) |
| 0x48 | y_max_shift |
| 0x4C | u16 xs[42] (§2.4) |
| 0xA0 | u16 ys[32] |
| 0xE0 | s32 dx[32][42], Q15 (§2.5) |
| 0x15E0 | s32 dy[32][42], Q15 |

### 3.4 Input bounding box of a node range (`ldc_split_bounding_box` 0x29b6c) [R: disassembly; a model matches the module on 1000 random node ranges]
Arguments `(split_state, mapX, mapY, range{x0, y0, x1, y1} (nodes, inclusive), out{x, y, w, h})`. Split-state fields used:
stride +0x08, margin m +0x4C, x alignment ax +0x50, y alignment ay +0x54, width granularity aw +0x58, height granularity
ah +0x5C, in_w +0x80, in_h +0x84; `ldc_split_init` sets m = 2, ax = 32, ay = 2, aw = 32, ah = 2.
- **Every node of the range counts**: its X and Y are first clipped (`clip_int`) to [0, in_w·2^15 − 1] / [0, in_h·2^15 − 1];
  the test after the clip (0 ≤ X′ < in_w·2^15, 0 ≤ Y′ < in_h·2^15, 0x29c2a…0x29c3a) is always true. So nodes outside the
  picture pull the box to the picture edge. Min / max start at 0x7FFFFFFF / 0; an empty range (y0 > y1 or x0 > x1) leaves
  them so.
- From the integer parts (⌊⌋ = `Q15toInt`, >> 15) of the min / max, C integer division (truncation toward 0):
  bx = clamp(ax·trunc((⌊minX⌋ − (m−1)) / ax), 0, in_w), by = clamp(ay·trunc((⌊minY⌋ − (m−1) − 3) / ay), 0, in_h),
  ex = clamp(⌊maxX⌋ + m, 0, in_w), ey = clamp(⌊maxY⌋ + m, 0, in_h);
- w = align_up(ex − bx + 1, aw), reduced to aw·trunc((in_w − bx)/aw) when bx + w > in_w; h likewise with ah and in_h;
- the "no box" case (bx | ex = 0, by | ey = 0, or bx = ex = in_w, or by = ey = in_h) gives w = aw, h = 3·ah; otherwise
  w = max(w, aw), h = max(h, 3·ah);
- then by = min(by, in_h − 3·ah), bx = min(bx, in_w − aw);
- finally, if h ≤ ey and by = 0: h = min(h + 2, in_h) [R, literal].

Result {bx, by, w, h} = InBlk of the block (§2.3): x multiple of 32, y even, w multiple of 32 (≥ 32), h even (≥ 6).
The oldest sibling build (anjoy-mch12s, 0x16ea4, 648 bytes) has the same arithmetic with the split-state fields 8 bytes
lower (margin +0x44, alignments +0x48 / +0x4C / +0x50 / +0x54, in_w / in_h +0x78 / +0x7C, stride +0x08) and differs only
in the node rule: no clip, a node counts only when 0 ≤ X < in_w·2^15 and 0 ≤ Y < in_h·2^15, so nodes outside the picture
are ignored. On the same 1000 random ranges (smooth maps with noise and edge values; anjoy's module run from the raw .ko
with its two `IntoQ15` and four `Q15toInt` calls relocated, in an emulator) the two builds gave different boxes in 501 calls,
always where some node of the range lay outside the picture [R: run; I: "always where" from the model, which differs only
there].

### 3.5 y_max_shift (`ldc_split_yshift` 0x29f7c, read as disassembly) [R]
For each node row r of the range (top to bottom): lo = (r = first row) ? bb.y : min of mapY over row r − 1; hi = max of
mapY over row r (unclipped Q15, integer part); b = 2·((lo − m − 2) / 2) clamped to [0, in_h]; e = hi + m clamped;
span = align_up(e − b + 1, 2) (or 2·((in_h − b)/2) if it would run past in_h). y_max_shift = max span over the rows; if the
last row's b is 0: y_max_shift = align_up(y_max_shift + 3, 2). An empty range gives 4. I.e. the number of input lines the
engine must hold to go from one table row to the next.
RSC mode (9) uses `ldc_split_yshift_rsc_mode` (0x29e2c): the same per-row span but only from the two end nodes (x0, x1) of
the rows y0, y1 − 1, y1, and the row y1 − 1 does not enter the maximum.

### 3.6 "Inverted y direction" (`ldc_split_inv_y_direction_check` 0x29ad4) [R]
- donut modes 3, 4, 8, 10: inverted iff out_w ≤ 640 (0x280);
- RSC (9): never;
- all others: compares the mean integer mapY of the first and the last node row over the first W − 1 columns: inverted iff
  mean(last row) < mean(first row) (the picture is upside down in the source, e.g. ceiling-mounted fisheye views [I]).

An inverted tile must fit its whole source height in the line buffer (test (e) of §3.2).

### 3.7 EIS split (`ldc_eis_split` = 1) [R `init_eis_split_cfg` 0x36bc0, `eis_evenly_split_method` 0x367d8, `cal_bounding_box` 0x36704, `generate_rectangle_box` 0x36344]
Configuration: origin (0, 0), input size, output size = ((nodes_x − 1)·grid_w, (nodes_y − 1)·grid_h), minimum block =
grid_w, maximum block = 2048. Square blocks of side s, starting with s = 2048: s is clamped to [grid_w, 2048] and aligned up
to grid_w; the output is covered by ⌈out_h / s⌉ × ⌈out_w / s⌉ blocks of s/grid_w node intervals each (end node clamped to
the grid); each block is tested like §3.2 (a) bb.w ≤ HW_LIMIT_WIDTH, (b) y_max_shift ≤ HW_LIMIT_Y_SHIFT, (c) grid_w ·
width_in_nodes < 2048, (e) inverted → bb.h ≤ HW_LIMIT_Y_SHIFT. If any block fails, s is halved and everything is redone;
below grid_w the method gives up (the last count stays). Each block record (0x44 bytes: four corners of the output
rectangle in nodes, four corners of the input bbox, y_max_shift) is copied into the tile array; more than 650 → 0x238
"too many tiles". The work area's block count is at +0x11000. Purpose [I]: identical blocks so that a per-frame
stabilisation shift can be applied block-wise (RSC / DIS).

### 3.8 Other split helpers [R: call sites in the decompile]
None of these has a call site in the module except `ldc_split_crop`, which the EIS split of §3.7 uses.
- `ldc_split_iterative_find(sp, mapX, mapY, hwys, nb, tiles, tw)`: the retry loop of §3.2 as a function (0x200 / 0x21E / 0x21F).
- `ldc_split_bbox(sp, mapX, mapY, hwys, nb, tiles)`: an older band splitter: bands of (nodes_y − 1)/nb rows (not made even),
  columns grown one node at a time with tests (a)(b)(c)(e), no table_width test; same tile records.
- `ldc_split_coordination_align(sp, minX, maxX, minY, maxY, out[4])`: the bbox alignment of §3.4 from given Q15 extremes
  (x: 32-aligned start, end + 2; y: 2-aligned start − 3, end + 2), clamped to the input.
- `ldc_split_scope_protect(sp, box, &w, &h)`: the right / bottom clipping step of §3.4 alone.
- `ldc_split_crop(x0, x1, y0, y1, out)`: fills {x0, y0, x1, y1, w = x1 − x0 + 1, h = y1 − y0 + 1}.
- `eis_evenly_split_tiles(buf, cfg, s)`: one pass of §3.7 for a given s (rectangles only, no tests).
- `result_statistic_info(buf, cfg, …, out)`: count, total input-bbox area, total output area and their ratio (debug).
- `up_align_double_to` / `down_align_double_to(double, a)`: ceil / floor of a double to a multiple of |a|; `clip_double`.

## 4. Projection models — what each `ldc_mode` computes for the node maps

Common to all models [R]: the node maps are filled row by row: node (i, j) sits at output pixel **x = min(i·grid_w,
out_w), y = min(j·grid_h, out_h)** (the extra last column / row is clamped onto the edge); (cx, cy) = rt+0x175C / +0x1760 =
(out_w/2 − output_offset_x, out_h/2 − output_offset_y). Each model writes mapX / mapY = input position in Q15 pixels.
Angles in degrees are turned into radians by `· π/180` with π = rt+0x168C (float 3.1415927; the constant 0.0055555557 =
1/180 is in each function). All trigonometry goes through `sine_sll` / `cosine_sll` / `tan_sll` / `atan2_sll` /
`sqrt_sll` (float in, float out; 32.32 fixed point inside, §8.1) and `FloatoQ15` (`vcvt.s32.f32 #15`, truncating,
saturating). The mode names (4R_CM, 1P_WM …) are those of the name table in `ldc-1-stack.md` §4.4.

### 4.1 Output geometry: `out_set_*` (screen rect S, view rect V, effective rotation, flip) [R 0x29154–0x297dc]
| function (modes) | S (w, h) at (0, 0) | V (w, h) = (out_w, out_h) at | effective rotation rt+0x1798 / flip rt+0x179C |
|---|---|---|---|
| `out_set_1R` (2) | (screen_w, screen_h), default (out_w, out_h) | (out_x, out_y) | rotate ∈ [90, 270] → out_rot + 180 / 3; else out_rot / 0 |
| `out_set_WM_1R` (7) | same | (out_x, out_y) | rotate + out_rot / 0 |
| `out_set_CM_4R` (0) | (2·out_w, 2·out_h) | quadrant by view index: 0 (0, 0), 1 (out_w, 0), 2 (0, out_h), ≥ 3 (out_w, out_h) | as 1R |
| `out_set_WM_4R` (1) | (2·out_w, 2·out_h) | quadrant as CM_4R | rotate + out_rot / 0 |
| `out_set_1P_CM`, `out_set_1P_WM` (donut 1P, ERP) | (screen_w, screen_h), default (out_w, out_h) | (out_x, out_y) | not set by these (the model's own) / 0 |
| `out_set_2P_CM` (3) | (out_w, 2·out_h) | view 0 → (0, 0), view ≥ 1 → (0, out_h) | rotate + out_rot; flip 3 when the angle window lies in the upper half (theta_start < 186: if theta_start < −5 it needs theta_start + 365 ≤ 190 and u = theta_end + 365, else u = theta_end + 5; flip 3 iff u ≤ 190), and then r_inside / r_outside are ordered (r_outside ≥ r_inside) and theta_start / theta_end swapped to descending |
| `out_set_2P_DM` (8) | (out_w, 2·out_h) | as 2P_CM | rotate + out_rot; flip 3 when (theta_start < 175 and theta_start + 185 ≤ 190, u = theta_end + 185) or (175 ≤ theta_start < 366, u = theta_end − 175), with u ≤ 190 (unsigned); then r_inside ≥ r_outside and theta ascending |
| `out_set_bypass` (6) | (screen_w, screen_h), or **(2·out_w, 2·out_h) when both are 0** | (out_x, out_y), or when both screen sizes are 0 by view index: 0 (0, 0), 1 **(2·out_w, 0)**, 2 (0, 2·out_h), ≥ 3 (2·out_w, 2·out_h) — views 1–3 land outside S [I: vendor slip] | out_rot / 0 |
| `out_set_RSC`, `out_set_cylinder`, `out_set_projective`, `out_set_map2bin`, `out_set_stitch` | §4.6–§4.9, §6 | | |

All set the half sizes S+0x14 / 0x18 and V+0x14 / 0x18 = w/2, h/2 (unsigned shift). `*_out_init` then set grid_w / h,
nodes_x / y = (align_up(out_w, grid_w) + grid_w)/grid_w (likewise y), rt+0x17A0 = ((nodes_x + 1) >> 1)·4, map size, and
(eptz) rt+0x1770 / 0x1774 = 0. screen_w / screen_h = para+0x165C / +0x1660, out_x / out_y = para+0x1664 / +0x1668.

### 4.2 Fisheye dewarp, modes 0 / 1 / 2 / 7 (`eptz_parameter_set` 0x34164, `eptz_calculate` 0x33c9c, disassembly checked) [R]
<!-- ev:ldc-2-eptz-lib.4.2 -->
Parameter set: pan, tilt, rotate clamped to [pan_min, pan_max] etc.; zoom (float) clamped to [zoom_min, zoom_max] and
stored ·0.01; out_rot clamped ≤ 360; distort_intensity and zoom-lens fov_h / fov_v stored; `eptz_out_init` (§4.1).
`eptz_map_gen`: calib_type 3 → `eptz_zoom_lens_calculate` (§9.5), else `eptz_calculate(pan, tilt, rotate, z, rt)`:
1. z = 0 → error 0x205.
2. Virtual camera focal length f (pixels): with pf = rt+0x18E8 (§9.3): **0.9 ≤ pf ≤ 10 → f = w.k · 1000 / pf**
   (k = w+0x128, §9.3), otherwise **f = (calib_w/2) / tan(60°)** (rt+0x1694 = 2π/3, halved).
3. Per node: (dx, dy) = (x − cx, y − cy); rotate by out_rot ω: u = dx·cosω − dy·sinω, v = dx·sinω + dy·cosω (Q15);
   ρ = √(u² + v²), φ = atan2(v, u), **θ = atan2(ρ / z, f)** (angle off the virtual optical axis; z is the zoom factor).
4. Ray: (X, Y, Z) = (sinθ·cosφ, sinθ·sinφ, cosθ) (`eptz_geographic2spherical`).
5. Rotations (Q15, c / s = cos / sin): **tilt** with angle −tilt: Y' = c_t·Y + s_t·Z, Z' = c_t·Z − s_t·Y; **pan** with angle
   −pan: X'' = c_p·X − s_p·Z', Z'' = s_p·X + c_p·Z'; **rotate** with angle rotate: X''' = c_r·X'' + s_r·Y', Y''' = c_r·Y' −
   s_r·X''.
6. Projection: calib_type 2 (ideal lens) passes the ray (X''', Y''', Z'') as is with flag 2. Otherwise (as
   `eptz_spherical2plane`): Z'' > 0 → (X'''/Z'', Y'''/Z'', −1); Z'' < 0 → (X'''/−Z'', Y'''/−Z'', +1); Z'' = 0 → divide by
   Q15 1 (= 2^−15 as float, i.e. raw 1), third component +1. The third component selects the lens curve (§9.4).
7. `world2cam_table(lens, Q15(xc), Q15(yc), ray)` (§9.4) → sensor position (u, v) in calibration-image pixels; **mapX = u ·
   rt+0x1708, mapY = v · rt+0x170C** (Q15 products: the resize from calibration size to input size, §1.4). (xc, yc) =
   rt+0x1700 / +0x1704 = the lens centre (§9.1).

`eptz_calculate` computes steps 4–6 inline. The same steps exist as functions: `eptz_geographic2spherical` (step 4),
`eptz_matrix_Mx_apha / My_beta / Mz_gama(v, c, s, out)` (the three rotations of step 5: Mx keeps x, y' = c·y + s·z,
z' = c·z − s·y; My: x' = c·x − s·z, z' = s·x + c·z; Mz: x' = c·x + s·y, y' = c·y − s·x) and `eptz_spherical2plane(v, _, out,
flag)` (step 6); their callers are the donut, ERP and cylinder models (§4.4–§4.6), not `eptz_calculate` [R: call sites]. `eptz_process` (= `eptz_map_gen` without arguments) has no callers.

### 4.3 Bypass, mode 6 ("1O") (`bypass_parameter_set` 0x2b81c, `bypass_calculate` 0x2b8a0; `bypass_map_gen` / `bypass_process` are 4-byte jumps to it) [R]
**No lens model** — rotation about the output centre and a plain resize: for each node: (dx, dy) = (x − cx, y − cy);
X = dx·cosω − dy·sinω + cx, Y = dx·sinω + dy·cosω + cy (ω = rt+0x1798 = out_rot, Q15 cos / sin); **mapX = X · in_w/out_w,
mapY = Y · in_h/out_h** (Q15 factors `FloatoQ15(in_w / (float)out_w)`). With out_rot = 0 and equal sizes this is the
identity map. So the library has no "undistort a normal lens" mode other than running a fisheye model (1R with a poly
calibration, §4.2 + §9) [I].

### 4.4 Donut → panorama, modes 3 (2P_CM), 4 (1P_CM), 8 (2P_DM), 10 (1P_DM) (`donut_parameter_set` 0x2b230, `donut_calculate` 0x2b3e8) [R]
Parameter set: r_inside, r_outside each clamped to [radius_min, radius_max] (0 → 1); if |theta_end − theta_start| > 360
then theta_end = theta_start ± 360; ordering: **CM modes (3, 4): r_inside ≥ r_outside** (swapped if not), theta_start ≤
theta_end; **DM modes (8, 10): r_outside ≥ r_inside**, theta_start ≥ theta_end; fc ≤ 0 → 1.0; then `donut_out_init`
(3 → `out_set_2P_CM`, 4 and 10 → `out_set_1P_CM`, 8 → `out_set_2P_DM`, §4.1; other modes 0x204). `eptz_init` has already
overwritten para r_inside / r_outside with radius_min / radius_max (§1.4), so at the first build the ring is the full
[radius_min, radius_max] range; later `SetChnParam` calls use the JSON values.
`donut_calculate(r_inside, r_outside, theta_start, theta_end, rt)`; out_w, out_h or the lens radius R = w+0x104 (§9.3)
equal to 0 → 0x206. Per node (the x and y counters **start at 1 instead of 0 when flip = 3**): (u, v) = rotation of
(x − cx, y − cy) by out_rot; **α = θs + (u + cx)·(θe − θs)/out_w** (radians), **r = r_inside + (v + cy)·(r_outside −
r_inside)/out_h** (pixels), **θ = (π/2)·r / R** (equidistant: the ring radius r on the sensor ↔ incidence angle);
ray = (r·cosα, r·sinα, Z) with Z = r·cosθ/sinθ (sinθ = 0 → 2^−15 + cosθ·∞·r; Z = 0 → 2^−15); then
`eptz_spherical2plane(ray, −1, calib_type)` (§4.2 step 6), `world2cam_table` (§9.4), resize by rt+0x1708 / +0x170C.

### 4.5 Equirectangular, mode 5 ("1P_WM") (`erp_parameter_set` 0x35b48, `erp_calculate` 0x35e0c, disassembly checked) [R]
Parameter set: pan, tilt clamped to their ranges, rotate clamped to [0, 360], zoom_h and zoom_v clamped to [zoom_min,
zoom_max] and ·0.01, out_rot to [0, 360], fc ≤ 0 → 1.0, distort_intensity stored; `out_set_1P_WM` (mode must be 5, else
0x204). `erp_calculate(pan, tilt, rotate, zh, zv, rt)`; zh, zv, fc or out_w = 0 → 0x207. H = fov_out_1p/2 in radians
(rt+0x16B0, default 180 → H = π/2). Per node: (u, v) = out_rot rotation of (x − cx, y − cy);
**λ = clamp(u·(π/out_w)/zh, −H, H), μ = clamp(v·(π/out_w)/zv, −H, H)** (both axes scaled by π/out_w: a zoom-1 view is
180° wide); `eptz_geographic2spherical(λ, π/2 − μ)` → (cosμ·cosλ, cosμ·sinλ, sinμ), re-ordered to the vector
**(cosμ·sinλ, sinμ, cosμ·cosλ)** (x right, y down, z forward); then Mx(−tilt), My(−pan), Mz(rotate) (the §4.2 functions),
`eptz_spherical2plane(…, −1, calib_type)`, `world2cam_table`, resize.

### 4.6 Cylinder, mode 11 (`cylinder_parameter_set` 0x2cc0c, `cylinder_calculate` 0x2cd6c) [R]
Parameter set: pan, tilt clamped, zoom_h / zoom_v clamped and ·0.01, fc ≤ 0 → 1.0; `out_set_cylinder` = `out_set_1P_WM`
layout, flip 0, no effective rotation (mode must be 11, else 0x204). `cylinder_calculate(pan, tilt, zh, zv, rt)`; zh, zv,
fc, out_w or out_h = 0 → 0x222. Per node, **no out_rot**: t = (y − cy)/(out_h·zv), λ = (x − cx)·(π/out_w)·(0.5/zh)
(90° across at zoom 1), a = atan(t); `geographic2spherical(λ, π/2 − a)` re-ordered as in ERP → (cos a·sinλ, sin a,
cos a·cosλ); Mx(−tilt), My(−pan) (no Mz); `spherical2plane`, `world2cam_table`, resize.

### 4.7 RSC, mode 9 (rolling-shutter / stabilisation warp) (`RSC_parameter_set` 0x2bb68, `RSC_map_gen` 0x2cb50, `RSC_calculate` 0x2bd04, `RSC_calculate_eis_split` 0x2c398, `RSC_dispmap_gen` 0x2c94c) [R]
**No lens model.** Parameter set: pan = tilt = rotate = out_rot = 0, zoom clamped ·0.01, the nine m33 words copied to
rt+0x16C8, rsc_region {x, y, w, h} to rt+0x16B8; `out_set_RSC`: S = (out_w, out_h) at (0, 0), **V = (region.w, region.h)
at (region.x, region.y)**, but V's half sizes are S's (so cx, cy = out_w/2, out_h/2 − offsets); nodes_x / y from V,
rt+0x1788 / +0x178C = ⌈V.x/grid_w⌉, ⌈V.y/grid_h⌉ (first node column / row), map size and half map size at rt+0x1768 /
+0x1778; an output-layout failure returns 0x226. m33 scaling (the JSON conversion of `ldc-1-stack.md` §4.4 stores m0, m1,
m3, m4 ×2^14, m2, m5 ×8, m7 ×2^25 as integers): m0, m1, m3, m4 · 2^−14; m2, m5 · 1/8; m6, m7 · 2^−25.
- `ldc_eis_split` = 0 (`RSC_calculate`): first, at init, `RSC_dispmap_gen` fills two float maps over the **map grid**
  (map_w × map_h, ⌈map/grid⌉ + 1 nodes): disp(x, y) = (x·in_w/map_w, y·in_h/map_h) (zoom, fc and tan(60°) only checked for
  0 → 0x205). Per output node at absolute (x, y) = ((⌈V.x/gw⌉ + i)·gw, (⌈V.y/gh⌉ + j)·gh): **affine only** —
  px = (|map_w/2 − cx| + m2/8 + m1·y + m0·x)/gw, py = (|map_h/2 − cy| + m5/8 + m4·y + m3·x)/gh (map-grid units; m6, m7, m8
  are only printed); `bilinear_linearInterpolation_rsc_mode` samples the displacement maps at (px, py) (plain bilinear on
  floats, no bounds check) → mapX / mapY = FloatoQ15.
- `ldc_eis_split` = 1 (`RSC_calculate_eis_split`): **full homography directly to input pixels**: den = 1 + m7·y + m6·x,
  mapX = Q15((m2/8 + m1·y + m0·x)/den), mapY = Q15((m5/8 + m4·y + m3·x)/den). Combined with the EIS even split (§3.7).
  JSON-loaded m6 is stored as a float bit pattern (`ldc-1-stack.md` §4.4) but is read here as an integer ·2^−25, so a JSON
  m33 with m6 ≠ 0 gives a wrong denominator [R both sides / I consequence].
- other `ldc_eis_split` → 0x237. `RSC_process` = the same dispatch without arguments (no callers).

### 4.8 Perspective, mode 15 (`perspective_parameter_set` 0x2d158, `perspective_process` 0x2d260) [R]
m33 copied to rt+0x16C8; `out_set_projective` (layout of `out_set_1P_WM`, flip 0), first node column / row ⌈V.x/gw⌉,
⌈V.y/gh⌉, map size kept. Per node (x, y) = (i·grid_w, j·grid_h) **starting at 0 and not clamped to the output**:
den = m8/8 + m7·y + m6·x, mapX = Q15((m2/8 + m1·y + m0·x)/den), mapY = Q15((m5/8 + m4·y + m3·x)/den) (same scaling as
§4.7, here m8 · 1/8 too). The loop runs nodes_x + ⌈V.x/gw⌉ columns per row but advances rows by nodes_x, and runs
nodes_y + ⌈V.y/gh⌉ rows — with out_x / out_y ≠ 0 it overruns the node maps [R loop bounds / I consequence]. JSON-loaded m6
and m8 are float bit patterns read as integers [I: mode 15 from JSON gives wrong maps]. mi_ldc refuses mode 15 anyway
(`ldc-1-stack.md` §4.4).
`single_point_perspective_transform(pt{int x, y}, out double[2], double m[9])`: (m0·x + m1·y + m2)/(m6·x + m7·y + m8) and
(m3·x + m4·y + m5)/(…), −1 when the denominator is 0 — no callers.

### 4.9 map2bin, mode 13 (external maps) (`map2bin_parameter_set` 0x2df64, `map2bin_map_gen` 0x2e120) [R]
`out_set_map2bin`: S = V = (out_w, out_h) at (0, 0), flip 0; nodes_x from grid_w, **nodes_y also from grid_w** [I: vendor
slip when grid_w ≠ grid_h], grid_h kept. `map2bin_map_gen`: `ldc_bin_read` of two files — X map from path-block slot
+0x4000, Y map from +0x4400 (for mode 13 = "<folder>/<para+0xA00 name>" and "<folder>/<para+0xC00 name>", §10.3) — each
nodes_x·nodes_y **f32 input positions (pixels)**, converted with FloatoQ15 into mapX / mapY. This is the library's own
"bring your own map" path; it reads files from the kernel (CamFs), so the maps must be on a filesystem the kernel sees.
mi_ldc refuses mode 13 (`ldc-1-stack.md` §4.4). `map2bin_process` = same body; `map2bin_map_read` (read only) and
`map2bin_map_float2Q15(fx, fy, qx, qy, rt)` (convert only) — no callers.

## 5. Crop and map protection: `map_crop_*`, `region_*`, `map_region_*`

### 5.1 `crop_en` ≠ 0: crop the output to the valid source area (`map_crop_process` 0x3c0f8) [R]
Runs after the model (§1.5) when para crop_en ≠ 0; the four crop node maps (rt+0x1A84 … +0x1A90) must exist, else 0x232.
1. `map_crop_find(mapX, mapY, rect, nx, ny, in_w, in_h)` (0x3be98) finds a node rectangle {max_x +0x10, min_x +0x14,
   max_y +0x18, min_y +0x1C}, kept in a **module-global** (`_LANCHOR0`+0x10 … +0x1C — shared by all views and channels,
   not re-entrant [R address / I consequence]) (§5.2–§5.3); `map_crop_find` returns the code of its refine step, it has no
   other result. No acceptable rectangle → 0x23F.
2. crop_en = 2 (keep aspect): with W = max_x − min_x + 1, H = max_y − min_y + 1 (nodes): if H/ny·nx < W the x range is
   shrunk symmetrically to H/ny·nx, else if W/nx·ny < H the y range is shrunk to W/nx·ny.
3. `map_crop_posmap_create(posX, posY, rect, nx, ny)` (0x3bf64) [R: disassembly; 300 random calls under emulation identical
   to a float32 model]: step_x = (float)(max_x − min_x) / (float)nx, step_y likewise with ny (single-precision division —
   divided by nx, ny, not nx − 1); posX(i, j) = FloatoQ15((float)min_x + (float)i · step_x), posY(i, j) =
   FloatoQ15((float)min_y + (float)j · step_y), the product rounded before the add (`vmla`, not fused); `FloatoQ15` =
   `vcvt.s32.f32 #15` (×32768 truncated toward zero, saturating). So crop node (i, j) ← the fractional source node
   (min_x + i·step_x, min_y + j·step_y) in Q15.
4. `bilinear_linearInterpolation_Q15` (0x3132c) resamples mapX and mapY at those fractional nodes (integer part = node index,
   low 15 bits = weight; standard bilinear in Q15) into the crop maps, and the map pointers are swapped (rt+0x198C / 0x1990
   ↔ rt+0x1A8C / 0x1A90) [R decompile; this step and `map_crop_resize` (§5.4) are not checked under emulation].

Effect [I]: the output picture is stretched over the largest valid region the search finds, so the engine does not sample
outside the input. On a round valid area the search keeps only a thin band or fails (§5.3 "Effect").

### 5.2 Border search: `region_map_find_{min,max}_{x,y}_inside_Q15` (0x41384, 0x40de4, 0x41e7c, 0x418fc) [R: a model matches the module on 1500 random maps per finder]
<!-- ev:ldc-2-eptz-lib.5.2 -->
**Arguments.** Maps are s32 Q15 positions, row-major, index `j·nx + i` (column i, row j). Each finder is
`f(mapX, mapY, nx, ny, in_w, in_h, win)`; `win` points to a 0x20-byte stack record of `map_crop_find` of which only
+0x10 … +0x1C are used: **+0x10 x_end, +0x14 x_start, +0x18 y_end, +0x1C y_start** (end exclusive). The result rectangle
`rect` uses the same offsets: +0x10 max_x, +0x14 min_x, +0x18 max_y, +0x1C min_y.
**Node test.** V(i, j): column clamped to [0, nx − 1], row clamped to [0, ny − 1]; valid iff 0 ≤ X ≤ IntoQ15(in_w − 1) and
0 ≤ Y ≤ IntoQ15(in_h − 1) (IntoQ15(v) = v·2^15; the bounds are inclusive).
**Call order and windows** (`map_crop_find` sets each window just before the call):

| finder | lines scanned | positions along a line | result written to |
|---|---|---|---|
| min_x | rows ny>>2 … (ny>>2)+(ny>>1)−1 | columns 0 → nx−1 (ascending) | rect+0x14 |
| max_x | the same rows | columns nx−1 → 1 (descending; the window start 0 itself is never visited) | rect+0x10 |
| min_y | columns min_x … max_x−1 | rows 0 → (ny>>1)−1 (ascending) | rect+0x1C |
| max_y | columns min_x … max_x−1 | rows ny−1 → (ny>>1)+1 (descending) | rect+0x18 |

Generally an ascending ("min") finder visits positions start … end−1, a descending ("max") one end−1 down to start+1. The
shifts are arithmetic (`asr`); with ny < 4 the min_x / max_x row window is empty or one row.
**One finder** (written for the x finders: a "line" is a row j, a position is a column i; the y finders are the same with
rows and columns exchanged, positions = rows, lines = columns; d = +1 for min, −1 for max):
1. best = 0 (min) or 0x10000 (max); held = false (not reset between lines). Empty line range (start ≥ end) → step 5.
2. mid = (line_start + line_end) >> 1. Lines up to and including mid use walk step +4, the lines after mid −4 (the walk
   goes towards the middle line). The compiled code runs the two halves as two loops; behaviour is that of one loop.
3. For each line, positions in scan order. A position p is a **hit** when
   - p is invalid and `held` is set, or
   - p is valid, the three positions before it (p−d, p−2d, p−3d) are all invalid, the three after it (p+d, p+2d, p+3d)
     are all valid, and the **diagonal walk** finds no valid node. Walk (min: only when p−1 > 0; max: only when p+1 < n,
     n = nx or ny): start at position c = p − d and line l+1; repeat { line += step; if V(c, line) valid → not a hit;
     c −= 4·d } while (min) the counter p−1−4k > 0 / (max) p+1+4k < n after the k-th step. So the nodes tested are
     (p−d−4d·(k−1), l+1+step·k), k = 1, 2, …, each clamped to the grid: a staircase from the border node towards the
     middle line and towards the line start (min) or end (max), one node per 4 positions.
   Positions outside the grid are clamped, so for min_x position 0 is never a hit (its "before" nodes clamp to itself,
   which is valid) and similarly at the far edge for the max finders.
4. On a hit: if it improves best (min: p > best; max: p < best) → best = p, held = true, **the rest of the line is
   skipped**; otherwise held = true and the scan continues. A position that is not a hit sets held = false. Consequence:
   after an improving hit the next line starts with held = true, so its leading run of invalid nodes are hits and the
   first one beyond best (min) / before best (max) improves best by exactly that position — the border can move inwards by
   one node per line this way even without a 3 + 3 pattern.
5. Return best clamped to [0, n − 1] (max finders with no hit return n − 1; min finders with no hit return 0).

**Quirk of max_y only** [R: 0x41b06…0x41b6e and 0x41c90…0x41cf8]: the three nodes on the valid side (rows a−1, a−2, a−3)
are tested on **X only**; in place of the Y test of row a−3 the code tests `(float)IntoQ15(in_h − 1) ≥ 0` (always true for
in_h ≥ 1), rows a−2 and a−1 get no Y test at all. min_x, max_x and min_y test both coordinates. The model matches the module
only with this asymmetry.

### 5.3 Refine: `region_map_find_inside_refine_Q15(mapX, mapY, nx, ny, in_w, in_h, rect)` (0x42578) [R: disassembly; `map_crop_find` with the four finders and refine matches a model on 1500 random maps]
- Node test differs from the finders: valid iff 0 ≤ X < IntoQ15(in_w) and 0 ≤ Y < IntoQ15(in_h) (upper bound = full size,
  exclusive), no clamping; only the four corners (min_x | max_x, row) are tested.
- D = max_y − min_y. D ≤ 0 → return **0x23F**, rect untouched. Otherwise top = min_y, bottom = max_y, and for
  t = 0 … ⌊(D + 1)/2⌋ − 1: if a top corner is invalid → top = min_y₀ + t (so t = 0 changes nothing); else if both bottom
  corners are valid → write rect +0x18 = bottom, +0x1C = top (x fields rewritten unchanged) and return **0x200**; else
  bottom = max_y₀ − t. Loop exhausted → 0x23F, rect keeps the four finder results.

**Effect on real geometry** [R: emulation of `map_crop_find` on synthetic maps; I: what it means]: with a valid area that is
an ellipse centred on the grid (semi-axes r × half the grid width / height): r = 1.2 (valid except the corners) on 65×49
nodes gives x 0…64, y 9…39; r = 0.95 gives x 2…62 but y only 21…26; r = 0.9 on 33×25 nodes fails with 0x23F. Likely the
diagonal walk rejects nearly every column of a curved top / bottom border [I]. So `crop_en` ≠ 0 suits maps whose valid area
is bounded by near-straight edges; on a round valid area it keeps a thin band or fails [I].

### 5.4 Helpers of the same family not on the mi_ldc path [R: called only by the stitch code of §6 or by nothing]
- `map_crop_resize(srcX, srcY, dstX, dstY, posX, posY, w, h)`: step 4 of §5.1 alone (used by `mi_st_stitch_autocrop`, §6.4);
  `map_crop_with_srcRatio(rect, w, h)`: step 2 alone, in pixels (+1 inclusive sizes), no callers.
- `region_map_find_valid_ROI(u8 mask, {w, h, …})`: column and row sums of a byte mask; from the centre outwards the last
  column / row with a non-zero sum on each side → {x0, x1, y0, y1} and size {x1 − x0 + 1, y1 − y0 + 1}; 0x233 if a side is
  empty. No callers.
- Float-map versions used by the stitch code (§6): `region_map_find_{min,max}_{x,y}` (same idea on float maps, invalid =
  negative or ≥ input size), `map_region_find` + `map_region_protect_x_min / x_max` (per row: from the row centre outwards
  in grid steps, the last column whose position is valid on both sides → the row's left / right valid sample),
  `map_region_protect(px, py, in_w, in_h, col, centre, row_info)`: an invalid position left of the centre is replaced by the
  row's left valid sample, right of the centre by the right one; `map_protection` / `find_the_neighboring_value(rt, mapX,
  mapY, …)`: an invalid float position is replaced by the first valid one found walking from the node towards the picture
  centre (quadrant-dependent direction, grid_w steps) — `map_protection` itself has no caller; `resize_bilinear` (0x3d67c):
  float-map bilinear resize — no caller.

## 6. Two-sensor stitching, mode 14 (`mi_st_stitch_*`, `st_stitch_*`, overlap and mask helpers) — present, unreachable from mi_ldc

### 6.1 Reachability and the stitch runtime [R]
The only caller of the stitch code is `mi_eptz_config_parse` (text-configuration file, §10.1), which calls
`mi_st_stitch_config_parse(para)` when ldc_mode = 14. The runtime entry points `mi_st_stitch_runtime_init`,
`mi_st_stitch_runtime_map_gen`, `mi_st_stitch_buffer_free`, `mi_st_stitch_autocrop` have **no callers**; the normal path
(`eptz_lib_parameter_set`) sends mode 14 to `eptz_parameter_set` → `eptz_out_init` → 0x204, and mi_ldc refuses modes 12–15
before generation (`ldc-1-stack.md` §4.4). So mode 14 cannot produce a table in this build.
- **`mi_st_stitch_config_parse(para)`** (0x32568): opens `"<para path string>/st_stitch_info.bin"` via CamFs (failure →
  0x21B "Open stitch_info.bin error"); sets para frame_cnt (+0x148C) = 2; grid_size must be 8, 16 or 32, else forced to 16;
  hw_mode −1 → 3. Reads **12 s32**: in_width, in_height, out_width, out_height (para +0x142C … +0x1438), then two rectangles
  {x0, y0, x1, y1} at para+0x1508 … +0x1514 and +0x1518 … +0x1524 (left / right sensor region [I from the use]); x / y
  starts are aligned up and widths aligned to the grid, and the derived overlap rectangles are written to para+0x1528 …
  +0x15DC.
- **`st_stitch_get_buffer_info` / `_get_buffer_ptr` / `_map_init`**: like §1.3 but with per-strip map pairs: for each of the
  N+1 camera strips (`stitch+0x27C` … records of 12 words, w / h at words 0x9F / 0xA0) and N overlap strips (words 0x1B /
  0x1C) two float maps each, plus one output pair (words 0xF / 0x10), the two lens LUTs, the tile array and the path block;
  then `eptz_init`.
- **`st_stitch_map_gen(rt, mapsX[], mapsY[], &bins, out)`** (0x3bb4c): for each of the 2N+1 strips: pick the strip's maps
  (read beforehand from `common_rectified_cy_mapx_%d.bin` / `…mapy_%d.bin`, file names from `file_stitch_init` (§10.3),
  via `st_stitch_map_read` + `st_stitch_map_float2Q15`), `st_stitch_map_selection` (per node row, `map_region_find` +
  `map_region_protect` replace invalid positions with the row's border samples, §5.4), `ldc_split_process` (§3), set
  input / output / screen rects for the strip (output x advances by the strip width), `ldc_bin_gen` (§2); finally
  **`ldc_bin_concantenate`** (§7.2) merges all strip tables into one view table and `ldc_bin_free` releases them.
- **`st_stitch_region_infor(stitch, out)`**: fills the blending-region list of the output descriptor (2N+1 entries of 16
  bytes: x offset, 0, width, height) and the total width.

### 6.2 Blend masks: `mask_stitch_generate_mask` (0x3dedc) and `mask_stitch_fill_mask` (0x3d064)
- **`mask_stitch_generate_mask(info, n, ratio, …)`** [R]: for each overlap a u8 blend mask (width × height, ramp width =
  2·⌊ratio·overlap⌋/2 limited to the overlap, ratio default 0.0125 when below it): the 1-D ramp comes from `ramp_resize_linear` (0x3c988)
  (linear resize, Q11 weights 2048, of a built-in 36-sample curve — the `G_gaussian_array_36` that mi_ldc's blending path
  also uses (`Init_Alpha_Map`, §11.1) [I by the constant 36 and the use]), copied to every row; with type 0x201 (a mask type
  compared here, not an error code) additional elliptical masks per sub-region (rows inside the ellipse get 0 / ramp / 0xFF).
  Each mask is then reduced by `mask_stitch_fill_mask(mask, w, w, h, dst)` into the descriptor (0x2A0 halfwords = 1344 bytes per
  mask). Error 0x229 for an inconsistent layout, 0x21C on allocation failure. These masks feed the software alpha blending
  of overlapping views (MVE, §11.1), not the engine's mask table (§2.6) [I].
- **`mask_stitch_fill_mask(src, stride, w, h, dst)`** [R: disassembly; 200 random masks (w, h 2…200, stride = w or larger) under
  emulation byte-identical to an OpenCV-formula model, nothing written past 1344 bytes]: called only from
  `mask_stitch_generate_mask` with stride = w. A bilinear resize of the u8 mask (w × h, row stride `stride`) to a fixed
  **32 columns × 42 rows** of u8 at `dst`, bit-exact with OpenCV `cv::resize(…, INTER_LINEAR)` for 8-bit data:
  - source coordinate per output column x: f = (x + 0.5)·(w / 32) − 0.5 (the scale is computed as w/32 in fixed point with
    5 fraction bits, i.e. exactly), per output row y: f = (y + 0.5)·(h · 0.023809523809523808) − 0.5 (constant = 1/42,
    double, literal); s = ⌊f⌋, fraction f − s; s < 0 → s = 0, fraction 0; s ≥ size − 1 → the last source sample with
    weight 1 (the code stores index size − 2 with weights (0, 2048), same value);
  - weights in Q11: a = saturate_s16(round_half_away((1 − frac)·2048)), b = 2048 − a (constant 2048.0, ±0.5 rounding,
    −0x8000 / 0x7FFF saturation);
  - horizontal pass per source row: S[x] = a_x·src[s_x] + b_x·src[s_x + 1] (s16); vertical: out = (((a_y·(S0 >> 4)) >> 16) +
    ((b_y·(S1 >> 4)) >> 16) + 2) >> 2, clamped to ≤ 255 (`smulbb`, `asr #16`, `+2`, `asr #2`, `cmp #0xff`);
  - memory: a 0x250-byte coefficient table (32 + 42 indices and weight pairs) and two 0x40-byte row buffers from
    `CamOsMemAlloc`, all released at the end; returns **0x200**. If any allocation fails it returns **0x21C** without
    releasing the buffers already obtained (leak) [R: 0x3d35e → 0x3d66e].

### 6.3 Overlap search: `region_overlap_horizontal_calculation` (0x3e6f0), `region_overlap_virtical_calculation` (0x3f950) and their helpers [R: disassembly; models match the module under emulation — horizontal 400 random map pairs, vertical 500, each of the four vertical helpers 1000–1500 direct calls]
No callers in the module. Maps are **float** (not Q15) pairs: camera 1 (m1x, m1y) and camera 2 (m2x, m2y), row-major with
stride nx. **ok(x, y)** = 0 ≤ x < W and 0 ≤ y < H, W = (float)in_w, H = (float)in_h (exclusive upper bound). NaN inputs
were not modelled: several tests are written so that a NaN passes, others so that it fails; the NaN behaviour follows the
individual comparisons but was not run.

**`region_overlap_horizontal_calculation(m1x, m1y, m2x, m2y, &out_min, &out_max, in_w, in_h, nx, ny)`**
- out_min starts at **99999**, out_max at **0**; both are always written at the end. ny ≤ 10 → nothing searched.
- Left edge (only when nx > 10): rows 5 … ny−6, in each row columns 5 … nx−6 ascending. A column c is a candidate when
  both cameras are ok at (c, row), camera 1 is ok at c−1 and camera 2 is not (the left edge of camera 2 inside camera 1).
  For a candidate `find_horizontal_min_value_from_two_map(&out_min, 5, row, c, …)`; when it returns 1 the rest of the row
  is skipped.
- Right edge (only when nx − 6 > 4): rows 5 … ny−6, columns nx−6 … 5 descending; candidate: both ok at c, camera 1 not ok
  at c+1, camera 2 ok at c+1 (the right edge of camera 1 inside camera 2) → `find_horizontal_max_value_from_two_map
  (&out_max, 5, row, c, …)`, row skipped on 1.
- **`find_horizontal_min_value_from_two_map(out, n, row, c, m1x, m1y, m2x, m2y, in_w, in_h, nx)`** (0x3e324): (a) walk
  left over columns c−2, c−3, … for n columns, stopping at the first column where camera 1 is not ok; count the columns
  where camera 2 is not ok; the count must equal n. (b) walk right over c+1 … c+n, stopping at the first column where
  m1x ∉ [0, W) or m2x < 0 or **m2x ≥ H** (m2x compared with the *height*); count columns with m2x < W and 0 ≤ m2y < H;
  the count must equal n. **m1y is not tested in (b).** (c) if *out > c (compared as floats): *out = c (rounded half away
  from zero from the float, i.e. c), return 1; every other path returns 0 and leaves *out.
- **`find_horizontal_max_value_from_two_map`** (0x3e500), same arguments: (a) walk right over c+2 … c+n+1, stopping at the
  first column where camera 1 is ok; count columns where camera 2 is ok; must be n. (b) walk left over c−1 … c−n, stopping
  where camera 1 is not ok; count columns where camera 2 is ok; must be n. (c) if *out < c: *out = c, return 1.
- With n = 5 and the loop bounds the walks stay inside the row except (a) of the min helper at c = 5, which reads column
  −1 = the last node of the previous row [R: index arithmetic].

Result: [out_min, out_max] = the columns where camera 2 starts and camera 1 ends, each confirmed by 5 + 5 nodes, taken as
the smallest start / largest end over the rows; 99999 / 0 when not found.

**`region_overlap_virtical_calculation(m1x, m1y, m2x, m2y, &out_a, &out_b, in_w, in_h, nx, ny)`** — the same search down
the columns, for cameras placed one above the other:
- Four accumulators: minL = minR = 99999, maxL = maxR = 0. nx ≤ 10 → nothing searched, out_a = 99999, out_b = 0.
- Top edge (ny > 10): columns 5 … nx−6, rows 5 … ny−6 ascending; a row r needs both cameras ok at (col, r). Then at r−1:
  camera 1 ok and camera 2 not → `find_virtical_min_value_from_two_map_L(&minL, 5, r, col, …)`; camera 1 not ok and
  camera 2 ok → `find_virtical_min_value_from_two_map_R(&minR, …)`. A return of 1 ends the column.
- Bottom edge (ny − 6 > 4): columns 5 … nx−6, rows ny−6 … 5 descending, both ok at (col, r). At r+1: if camera 1 is not ok
  → `find_virtical_max_value_from_two_map_L(&maxL, …)` (1 ends the column); then, whether or not camera 1 was ok, if
  camera 2 is not ok at r+1 → `find_virtical_max_value_from_two_map_R(&maxR, …)` (1 ends the column). The code guards the
  first call and the fall-through with tests written as an OR of the four bounds (`W > x || x ≥ 0 || y ≥ 0 || H > y`),
  true for every non-NaN value, so they have no effect [R: 0x3fce2…0x3fd1c, 0x3fd4a…0x3fd84; I: an `&&` typed as `||` in
  the source].
- End: **out_a = min(minL, minR), out_b = min(maxL, maxR)** — out_b is the *smaller* of the two bottom edges, and 0 unless
  both bottom searches succeeded [R: 0x3fe00…0x3fe1a].
- Helpers (0x3eab8 min_L, 0x3ee64 min_R, 0x3f204 max_L, 0x3f5a4 max_R; same argument list as the horizontal ones, the
  third argument is the row r, the fourth the column): the horizontal helpers applied along the column (positions = rows,
  stride nx; each has a separate code path for nx = 1 with the same result). `_L`: camera 1 in the role of "camera 1"
  above; `_R`: the two cameras exchanged. The vertical min helpers have **no** quirk: in (b) both coordinates of the first
  map are tested and the second map's x is bounded by W. With r = 5 (min) or r = ny−6 (max) the walks read **row −1 or
  row ny**, one row outside the map [R: confirmed by an unmapped-read fault under emulation, then modelled with guard
  rows].

### 6.4 `mi_st_stitch_autocrop(info, status)` (0x32900) [R: disassembly; 300 random camera sets (N = 0…4, both status values) under emulation identical to a model in return code, all rectangles and the float maps]
No callers. `info` is an array of 16 words (offsets in bytes):

| off | content | off | content |
|---|---|---|---|
| +0x00 | N, number of cameras | +0x20 | ny |
| +0x04 | Q15 map X of all cameras, N × nx·ny s32 back to back | +0x24 | in_w |
| +0x08 | Q15 map Y, same layout | +0x28 | in_h |
| +0x0C / +0x10 | crop position maps X / Y (nx·ny s32 each) | +0x2C / +0x30 | arrays of N pointers to the float output maps X / Y |
| +0x14 / +0x18 | resample buffers X / Y (nx·ny s32) | +0x34 | camera rectangles, N records |
| +0x1C | nx | +0x38 / +0x3C | overlap rectangles (N − 1 records) / output rectangle |

A rectangle record is 0x30 bytes of which the first six s32 are used: **x0, y0, x1, y1, w, h** (inclusive ends).
1. **N ≤ 0**: `map_crop_posmap_create(posX, posY, {max_x 0, min_x nx, max_y ny, min_y 0}, nx, ny)` (x range reversed) and
   go to step 7.
2. Per camera c (its maps at +c·nx·ny): `map_crop_find` (§5.1–§5.3) → crop rectangle; any result other than 0x200 is
   returned at once (later cameras untouched).
3. Same camera, full scan of the grid: node k valid iff 0 ≤ X < IntoQ15(in_w) and 0 ≤ Y < IntoQ15(in_h). L starts at nx,
   R at 0; for every valid node at column col: if col < L and node k+2 is valid → L = col; if col > R and node k−2 is
   valid → R = col (k ± 2 are flat indices: at the row ends they reach into the neighbouring row, at the first / last
   nodes of the block outside the camera's map — the previous / next camera's map or memory outside the block).
4. Camera rectangle: x0 / x1 from
   - status = 0x200: the crop rectangle's min_x / max_x;
   - else camera 0: min_x / R; last camera (c = N − 1, c > 0): L / max_x; middle cameras: L / R
   (so with status ≠ 0x200 the outer edges come from the crop search and the inner edges from the scan);
   y0 = min_y, y1 = max_y of the crop rectangle; w = x1 − x0 + 1, h = y1 − y0 + 1.
5. Output rectangle: x0 = min of the cameras' x0 (start nx), x1 = max of x1 (start 0), y0 = max of y0 (start 0), y1 =
   min of y1 (start ny); w, h as above. Overlap i (i = 0 … N−2): x0 = camera i+1's x0, x1 = camera i's x1, w = x1 − x0 + 1,
   y0 / y1 / h = the output's.
6. `map_crop_posmap_create(posX, posY, {max_x = out.x1, min_x = out.x0, max_y = out.y1, min_y = out.y0}, nx, ny)` (§5.1),
   then per camera `map_crop_resize(mapX_c, mapY_c, posX, posY, tmpX, tmpY, nx, ny)` and the float output maps of camera c
   get `Q15toFloat(tmp)` (= tmp / 32768 rounded to float, `vcvt.f32.s32 #15`) for all nx·ny nodes.
7. Overlap rectangles rescaled to the output grid (C `/`, truncation toward zero): x0' = nx·(x0 − out.x0) / out.w,
   w' = w·nx / out.w, y0' = ny·(y0 − out.y0) / out.h, h' = h·ny / out.h, x1' = x0' + w' − 1, y1' = y0' + h' − 1 (only when
   N − 1 > 0). Then the output rectangle is overwritten with the full grid {0, 0, nx − 1, ny − 1, nx, ny}; return 0x200.

## 7. Table I/O and joining (`ldc_bin_*` besides §2) [R]

### 7.1 File helpers (kernel file I/O through CamFs)
- `ldc_bin_read(buf, size, path)`: open read-only, read `size` bytes, close → 0x200, or 0x221 "can't open ldc bin".
  Callers: map2bin (§4.9), stitch maps (§6.1).
- `ldc_bin_write(buf, size, path)`: open for write, write, close → 0x200 / 0x220. No callers.
- `ldc_bin_write_eptz(buf, size, tilt, rotate, zoom, rot, folder)`: writes `"<folder>/out_i%d_t%d_r%d_z%4.2f_rot%d.bin"`
  (i = a module-global counter, incremented per file) — the debug dump named in mi_ldc's EPTZ front end
  (`ldc-1-stack.md` §5.3); empty folder → 0x220. No callers in this build.
- `ldc_bin_read_downsample(dst, path, rt)`: reads a **full-resolution f32 map file** (out_w × out_h values, row-major) and
  keeps one value per grid node (seek to row min(j·grid_w, out_h − 1), column min(i·grid_w, out_w − 1); grid_w is used for
  both axes) → 0x200 / 0x221. No callers.
- `ldc_bin_free(ptrs[], n)`: `CamOsMemRelease` each (NULL list → 0x21D); `mi_eptz_buffer_free(p)` likewise for one buffer.

### 7.2 `ldc_bin_concantenate(tables[], &out, &size, N, total_blocks, …)` (0x31448) — joining several views / strips into one table
Allocates 0x60 + total·0x16C0. File header and view header are copied from table 0, its block count replaced by the
total, then the blocks of all tables are appended in order. For table k = 1 … N (and again for the tables N+1 … 2N, with
the offsets of table k − N): the view header's input height (+0x1A) of each table is accumulated into the result's input
height and **every block's InBlk.y is increased by the sum of the input heights of the tables before it** (the sensors'
pictures are stacked vertically in one input frame), and the result's output width (view header +0x20) grows by each
table's output width. Used only by the stitch path (§6.1).

## 8. Support code inside the library

### 8.1 Fixed point and math [R names and constants; I origin]
- **sll** (s64 32.32): `dbl2sll` / `sll2dbl` (bit-exact IEEE double ↔ 32.32), `sllcos`, `sllsin` (range reduction with
  2/π = 0xA2F9836E·2^−32 and π/2, then the Taylor helpers `sll_taylor_sin` (0x371a0) / `sll_taylor_cos` (0x37380)), `sllexp` (helper
  `sllexp_series` (0x37588), constant 1/11 = 0x1745D174), `slllog`, `sllinv`, `slltan`, `sllatan` (helper `sllatan_series` (0x38050), 1/3 =
  0x55555555), `sllatan2`, `sllpow`, `sllsqrt`, `sllabs`. The set, names and constants match the public **math-sll**
  library (A. E. Mileski) [I: no version string]. Float wrappers `sine_sll`, `cosine_sll`, `tan_sll`, `atan2_sll(y, x)`,
  `sqrt_sll` convert float → double → sll, compute, and back to float; all trigonometry of §4 and §9 is therefore exact to
  roughly the 32-bit fraction, then rounded to float [I].
- **Q15**: `IntoQ15(i) = i << 15`; `Q15toInt(q) = q >> 15` (arithmetic, floor); `Q15toQn(q) = sign·(|q| >> 15)`
  (truncation toward zero — used after a ±0x4000 bias, so round half away from zero); `FloatoQ15(f) = vcvt.s32.f32 #15`
  (truncation toward zero, saturating); `Q15toFloat(q) = q / 32768` (`vcvt.f32.s32 #15`); `Q15_add` / `Q15_sub` plain;
  `Q15_mul(a, b) = (a·b ± 0x4000) >> 15` on 64 bits (round half away from zero); `Q15_div(a, b) = ((a << 15) ± b/2) / b`
  (64-bit, rounded); `sat16` clamps to s16.
- `clip_int(v, lo, hi)`, `clip_double`, `lerp(a, b, t)`, `bilinear_blerp(c00, c10, c01, c11, tx, ty)` (returns the
  sentinel −32768.0 = 0xC7000000 when any corner is the sentinel), `getpixel` / `putpixel` (float image {data, w, h},
  out-of-range → the sentinel), `bilinear_linearInterpolation` (float, sentinel-aware: −32768.0 where a neighbour is
  missing), `bilinear_linearInterpolation_Q15` (§5.1), `bilinear_linearInterpolation_rsc_mode` (§4.7). Only the `_Q15` and
  `_rsc_mode` variants have callers.

### 8.2 Checks, parsing, timing, debug [R]
- `Platform_Check()` (0x3c2a8; a second copy with the same message in the MVE part, `_sym_112` 0x43718, §11.1): once per module
  life, `CamOsChipId() == 0xF4` → 1, else prints "[ALGO_MSG_ERR] Unexpected IC version" and returns 0; the result is cached
  in a global. `mi_eptz_runtime_init` and MVE run it first. The value is the one user space reads from
  `/sys/devices/virtual/mstar/msys/CHIP_ID`; whether this camera's SoC gives 0xF4 is not known (not read), and what follows
  from either answer is in `ldc-1-stack.md` §4.6.
- `multiview_check_param(para)` (0x30c28): NULL → 0x240; screen_w < 1 → 0x241; the view must lie inside the screen
  (out_x ≥ 0, out_x < screen_w, out_y < screen_h, out_x + out_w ≤ screen_w, out_y + out_h ≤ screen_h) else 0x242; allowed
  modes in a multi-view configuration: **2, 4, 5, 6, 7, 10, 11**, others 0x243 (mi_ldc's "no support 2P/4R").
- `eptz_align_in(&w, &h)`: input width must be a multiple of 32 and height even, else 0x22E (values unchanged).
  `eptz_align_out(&w, &h, gw, gh)`: output sizes must be multiples of the grid; if not they are **rounded up** to it and
  0x22D is returned. Both are called by the text-configuration parser (§10.1).
- `isspace_ldc`, `sscanf_ldc(str, fmt, …)` (0x36ef0): the kernel has no `sscanf`; this small scanner knows **`%d`**
  (optional '-', one leading space skipped, decimal digits), **`%f`** (optional '-', digits with one '.', the fraction
  applied as ·0.1 per digit in float) and **`%n`** (characters consumed so far; ends the scan); any other format character
  must match literally; returns the number of converted `%d` / `%f` fields. Used by the text-configuration parser (§10.1).
- `time_log_set` (empty), `time_log_init` / `time_log_show` (return 0x200): stubs — the "EptzInit / OneBin" timings of the
  proc view come from mi_ldc, not from here.
- `debug_mi_eptz_config_param(para)` / `debug_mi_eptz_para(rt)`: `CamOsPrintf` dumps of every field when `deb_state` has
  the matching bit; `DEB_MSG_LEVL` = para `deb_state` (0 → 1); bit 0 errors, bit 1 step trace ("[DEBUG] deb_state: %d" with
  0x401 … 0x411), bits 3–10 per-module traces (split 0x20 / 0x40, donut 0x80, erp 0x800, RSC 0x10 / 0x200).
- `buffer_malloc` (§1.3). The unnamed 0x38770 … 0x38ee0 (0x38770–0x392af) are the static helpers of cJSON
  1.7.12 (`ensure`, `get_object_item`, `parse_string`, `print_value` …, by position and bodies) [I].

### 8.3 Return codes of the library [R: every `return 0x2xx` in the library functions]
| code | meaning (where) |
|---|---|
| 0x200 | OK |
| 0x201 | (value compared in `mask_stitch_generate_mask`: mask type, not an error) |
| 0x203 | lens calibration cannot be loaded (`world2cam_*init`) |
| 0x204 | ldc_mode not supported by the chosen output layout (`*_out_init`, `*_parameter_set`) |
| 0x205 | zoom 0 (`eptz_calculate`), zoom-lens step table not sorted (`eptz_zoom_lens_calculate`), RSC disp-map parameters 0 |
| 0x206 / 0x207 / 0x222 | donut / ERP / cylinder: a zero size, zoom or fc |
| 0x208 | a tile with one table column or row (`ldc_split_map_downsample`) |
| 0x20A | output buffer too small (`ldc_bin_*`) |
| 0x20C / 0x20D | text-configuration errors: NULL text / file cannot be opened or empty (§10.1) |
| 0x218 / 0x219 | API called in the wrong state (§1.5) |
| 0x21B | `st_stitch_info.bin` cannot be opened |
| 0x21C | allocation failed (mask / resize helpers) |
| 0x21D | NULL buffer to free |
| 0x21E / 0x21F | split: no band count up to start + 31 works / more than 650 tiles |
| 0x220 / 0x221 | table / map file cannot be written / read |
| 0x226 | RSC output layout failed |
| 0x229 | stitch mask layout inconsistent |
| 0x22D | output size not grid-aligned (`eptz_align_out`) or a block starting outside the output (`ldc_bin_*`) |
| 0x22E | input width not a multiple of 32 or height odd |
| 0x232 | crop buffers missing |
| 0x233 | empty ROI in a mask |
| 0x236 | JSON parse error (`mi_eptz_cjson_parse`) |
| 0x237 / 0x239 | invalid `ldc_eis_split` (RSC / split) |
| 0x238 | EIS split gives more than 650 tiles |
| 0x23F | crop: no fully valid rectangle |
| 0x240–0x243 | multi-view check (§8.2) |
| 0x244 | calib_type 3 (zoom lens) with a mode other than 2 (1R) |
| 0x245 | pixel_size given but < 90 (pf < 0.9) or > 1000 |

## 9. Lens model (`world2cam_*`) and the calibration data

### 9.1 Lens state `w` (rt+0x17A8, 0x144 bytes) [R offsets from the loaders, `world2cam_table`, `eptz_init`]
| w+ | content |
|---|---|
| 0x20 | −1; 0x134 = Q15(1) |
| 0x24 … 0x3C | the 7 header words of the calibration: **calib_w, calib_h (s32), scale, c3, c4, c5, c6 (f32)** |
| 0x40 | **curve 0** (0x5C bytes): s32 n (segments, ≤ 4); s32 knots[4]; f32 bounds[5]; s32 count[4]; u16 *lut[4]; f32 slope[4]; u16 *lut_buffer (0x5002 bytes, rt+0x1840) |
| 0x9C | **curve 1**, same layout (lut_buffer rt+0x189C) |
| 0xF8 / 0xFC | lens centre xc, yc (pixels, calibration image) |
| 0x100 | f32 1/scale |
| 0x104 | f32 R (lens radius, §9.3) |
| 0x108 … 0x120 | Q15 copies: scale, **M (radius multiplier, 0x10C)**, 1/scale, c4, c5, R (0x11C), c6 |
| 0x124 | attached calibration buffer descriptor {cfg va, cfg size, **calib va (+8)**, calib size} or 0 (`ldc-1-stack.md` §4.3) |
| 0x128 | f32 **k** (§9.3) |
| 0x12C / 0x130 | zoom lens: step count Z / array of Z step pointers (each a 0x108-byte copy of 0x24 … 0x12B) |
| 0x138 | f32 distortion term d = distort_intensity·0.01, distort_intensity clamped to [−100, 100] (`world2cam_distort_intensity_set`) |
| 0x13C | f32 **fc** = clamp(fc, …, 50.0), ≤ 0 → 1.0 (`world2cam_fc_set`; fc = JSON "fc") |
| 0x140 | f32 pf (pixel factor, rt+0x18E8, §9.3) |

### 9.2 Calibration data formats [R `world2cam_init` 0x35074, `world2cam_poly_new_init` 0x34b8c, `world2cam_init_trdu_new` 0x3428c, `world2cam_init_trdu_new_set` 0x344e8, `world2cam_poly_init` 0x34d18, `world2cam_init_trdu` 0x3483c, `world2cam_poly_zoom_lens_init` 0x34ea8]
All little-endian. **Curve record** (identical in every file):
```text
s32 n                     number of segments (the code handles up to 4)
s32 knots[n]              samples per unit of the input variable, per segment
f32 bounds[n+1]           segment limits (curve 0 ascending, curve 1 descending, §9.4)
repeat n times: s32 count_i ; u16 lut_i[count_i]      (Q15 normalised radius, 0..32767)
```
With n ≤ 0 only one bound is read and no LUT. At load: slope_i = knots_i / (bounds_{i+1} − bounds_i) (float, NEON
reciprocal for groups of 4), the LUTs are packed one after another into the 0x5002-byte buffer (≤ 0x2801 entries per
curve, not checked [R no bound check / I: a longer curve overflows the buffer]).

| calib_type | source | layout |
|---|---|---|
| 1 "poly new" | attached buffer (`eInfoType` 1, `ldc-1-stack.md` §4.1) or `<folder>/CalibPoly_new.bin` | 7 header words (calib_w, calib_h, scale, c3, c4, c5, c6) + curve 0 + curve 1 |
| 0 and any value ∉ {1, 2, 3} "old poly" (`world2cam_poly_init`; header file `Cali_LDCpoly.bin`) | files only: `<folder>/Cali_LDCpoly.bin` (the 7 header words) + `<folder>/trdu0to180.bin` (curve 0) + `<folder>/trdu180to220.bin` (curve 1) | as above, split over three files |
| 2 "ideal" | no data: xc, yc, R from the JSON (`in_xc`, `in_yc`, `in_fisheye_radius`); both xc and yc = −1 → 0x203 | — |
| 3 "zoom lens" | attached buffer only (no file path) | s32 Z, then Z records "7 header words + curve 0 + curve 1" back to back; missing buffer → 0x203 |

For types 0, 1, 3: if in_xc and in_yc are both −1, xc = in_xc_offset + calib_w/2, yc = in_yc_offset + calib_h/2; R =
float(in_fisheye_radius). "folder" = the calibration folder of the text configuration (`-i`, §10.3). Zoom-lens steps must be
sorted by their k (§9.3) ascending, else `eptz_zoom_lens_calculate` returns 0x205; calib_type 3 is allowed only with
ldc_mode 2 (0x244).

### 9.3 Derived constants (`world2cam_init` 0x35074, `eptz_init` 0x32ee8; disassembly checked) [R]
- If **c5 < 0**: c5 := −c5 and **R := (R / c5)·c3** (both with the original sign handled: R_new = −R/c5_old·c3); Q15(R) at
  w+0x11C. (For the ideal type c5 is 0, so the rule does not apply.)
- **k** (w+0x128): c6 > 0 → k = ⌊|c4 / tan(c6/2 in degrees)|·1000⌋ / 1000 (c6 = 180 uses the angle 1.5699236 rad instead
  of π/2); otherwise k = 1.0. [I: c6 = a field of view in degrees and c4 a half-size, so k is a focal length in the units
  of c4.]
- **pf** (rt+0x18E8): JSON `pixel_size` = 0 → pf = c3·1000/R (type 3: step 0's c3·1000 / step 0's R); otherwise pf =
  pixel_size·0.01, and pf < 0.9 or pixel_size > 1000 → 0x245.
- **M** (w+0x10C, per zoom step too): **0.9 ≤ pf ≤ 10 → M = Q15(c3·1000/pf)**, otherwise **M = Q15(R)**. With
  pixel_size = 0 both give R (c3·1000/(c3·1000/R)); for the ideal type c3 = 0 → pf = 0 → M = R. So M is the sensor radius
  in pixels that the normalised LUT output is scaled by; pixel_size (in 0.01 µm [I]) lets c3 be given in mm instead [I].
- rt+0x16F8 / +0x16FC = calib_w / calib_h; the resize factors rt+0x1708 / 0x170C = x / y_resize_ratio·0.01·in_w/calib_w
  (and in_h/calib_h) map calibration-image pixels to input pixels (§4.2 step 7).
- Meaning of the CalibPoly header floats, as far as the arithmetic shows [I]: **scale** normalises tanθ into the LUT input range
  (s = k·fc·tanθ/scale), **c3** is the radius scale (pixels, or mm with pixel_size), **c4, c6** give k (size / field of
  view), **c5** only matters through its sign (negative: R is derived from c3/|c5|). The LUTs themselves carry the lens
  distortion (normalised radius as a function of s).

### 9.4 `world2cam_table(w, Q15 xc, Q15 yc, ray{x, y, z}, out{u, v}, flag)` (0x35570, read as disassembly) [R]
φ = atan2(y, x), ρ = √(x² + y²) (float from Q15).
- **flag 2 (ideal)**: θ = atan2(ρ, z); r = Q15(k·fc·(2/π)·θ) — equidistant fisheye, r = 1 at θ = 90° when k·fc = 1.
- **otherwise** s = (1/scale)·k·fc·ρ, clamped to ≤ 1.0 (ρ = tanθ for the projected rays of §4.2):
  - z < 0 (the ray is in front, Z'' > 0 in §4.2) — **curve 0**, `world2cam_interpolation_LUT_0to180`: the first segment i
    (checked 0, 1, 2, 3 up to n) with bounds_i ≤ s ≤ bounds_{i+1}, none → segment 0; t = (s − bounds_i)·slope_i;
    idx = ⌊t⌋ clamped to count_i − 1, idx2 = min(idx + 1, count_i − 1); **r = lut[idx] + Q15_mul(Q15(t − ⌊t⌋), lut[idx2] −
    lut[idx])** (linear interpolation);
  - z ≥ 0 (behind, beyond 90° from the axis) — **curve 1**, inline: bounds are descending — segment 0 if bounds_1 ≤ s ≤
    bounds_0, segment i if bounds_{i+1} ≤ s ≤ bounds_i, none → 0; t = (s − bounds_i)·slope_i; **r = lut[idx] with no
    interpolation** (the fraction register is forced to 0 at 0x35660 — vendor slip or intent not known).
- distortion: d ≠ 0 → **r := r + d·r⁵** (float).
- **R_px = Q15_mul(r, M)**; **u = R_px·cos φ + xc, v = R_px·sin φ + yc** (Q15 pixels of the calibration image).

`world2cam_interpolation_LUT_180to220` and `world2cam_interpolation_LUT_find_curve0 / _curve1` (the segment search alone)
are duplicates of the inline code with no callers.

### 9.5 Zoom lens (calib_type 3) (`eptz_zoom_lens_calculate` 0x33760, `zl_world2cam_table` 0x357bc, `zl_tmp_map_generation` 0x32d68) [R]
- At init `zl_tmp_map_generation` stores for every node ρ (float, rt+0x1A98 map) and φ (Q15, rt+0x1A9C map) of the
  out_rot-rotated output offset (as §4.2 step 3).
- Per build: target focal (pixels) **f = min((calib_w/2)/tan(fov_h/2), (calib_h/2)/tan(fov_v/2))** (fov from
  `zoom_lens_param`, degrees; calib size of step 0); F = pf·f·0.001 (mm). Steps are searched by their k: F below step 0's
  k → step 0 only; between k_a and k_b → weights **w_a = (1/F − 1/k_b)/(1/k_a − 1/k_b)**, w_b = (1/k_a − 1/F)/(1/k_a −
  1/k_b) (1/F computed as 999.99994/(pf·f)); above the last → last step only (single steps use 0.5 / 0.5 of the same
  step). Interpolated 1/scale and M = Q15((w_a·c3_a + w_b·c3_b)·1000/pf).
- Per node: θ = atan2(ρ/z, f); ray and rotations as §4.2 steps 4–6; `zl_world2cam_table`: s = F·(1/scale)_interp·ρ ≤ 1;
  r_a, r_b from both steps' curves (curve 0 interpolated, curve 1 nearest, as §9.4); **r = w_a·r_a + w_b·r_b**;
  distortion; ×M; u, v around xc, yc; ×resize.
- `zoom_lens_step_calculation(F, &a, &b, Z, steps)`: the same bracket search as a function (1 = unsorted) — no callers.
- The zoom-lens model exists only in this build of mi_ldc.ko, not in the sibling builds (`ldc-1-stack.md` §9).

### 9.6 Lens helpers with no callers [R]
- `world2cam_FTheta_init(w, xc, yc, R)`: stores xc, yc (0x203 if both −1) and R / Q15(R) at offsets 0xD4 / 0xD8 / 0xE0 /
  0xF8 of a different (older) layout — a leftover f-θ initialiser.
- `world2cam_poly_focal_cal(c6, c4)`: the k formula of §9.3 as a function.

## 10. Text configuration (`mi_eptz_config_set` 0x26ab0, `mi_eptz_config_parse` 0x275c8) [R, the two switch bodies compared]

### 10.1 Entry points and parsing
Two entry points with the same option table:
- `mi_eptz_config_set({va, size}, para)` — text in memory (configuration type 2, `ldc-1-stack.md` §4.3): para 0 … 0x166F
  zeroed, **para+0x1650 = the buffer descriptor** (so a calibration blob attached to the descriptor reaches the lens loaders,
  §9.2), NULL text → 0x20C.
- `mi_eptz_config_parse(path, para)` — text file (configuration type 3): a **0x200-byte** buffer is allocated and the whole
  file (`CamFsSeek(end)` size) is read into it — **a file longer than 512 bytes overruns the kernel buffer** [R sizes / I
  consequence]; calib_type defaults to 0; open failure or empty file → 0x20D ("can't open configuration file", "the line is
  empty"); ldc_mode 14 then runs `mi_st_stitch_config_parse` (§6.1); returns the `eptz_align_in` result.

Common defaults: hw_mode −1 (→ 3 in `ldc_split_init`), x / y_resize_ratio 100, path counters (para+0x1400 … +0x1427) 0.
Parsing: skip to the next '-', dispatch on the following letter(s), scan values with `sscanf_ldc` (§8.2), continue after
the consumed characters. **All range checks only print a warning; the value is stored anyway.** Unknown letters are
skipped. At the end: `eptz_align_in` (in_width multiple of 32, in_height even, else 0x22E and the "[ERR] The input width
should be multiples of 32 …" message), `eptz_align_out` (out sizes rounded up to the grid, its 0x22D ignored),
table_width 0 → 256.

### 10.2 Options
| option | values → para field | warning when |
|---|---|---|
| `-m` | ldc_mode → +0x1428 | > 15 |
| `-s` | in_width, in_height, out_width, out_height → +0x142C … +0x1438 | not 4 values |
| `-si` | square_width, square_height, square_frame_number, square_size → +0x1604 … +0x1610 | not 4 values |
| `-sl` | ln_percentage (blending %) → +0x1634 | not 1 value; > 100 (file parser only) |
| `-sm` | map_width, map_height → +0x15F4, +0x15F8 | not 2 values |
| `-st` | stereo_detect_en, stereo_calib_en, stereo_rectify_offset_x, _y, _rot, (6th) → +0x1614, +0x1618, +0x1620, +0x1624, +0x1628, +0x162C (+0x161C skipped; the message names 5 fields, the debug dump calls one "stereo_calib_op_mode") | not 6 values |
| `-c` | in_xc, in_yc, in_fisheye_radius → +0x1444, +0x1448, +0x144C; sets in_xc_offset = in_yc_offset = −1 | not 3 values |
| `-co` | in_xc_offset, in_yc_offset, in_fisheye_radius → +0x1484, +0x1488, +0x144C; sets in_xc = in_yc = −1 (centre from the calibration size, §9.2) | not 3 values |
| `-cp` | crop_en → +0x1654 | not 1 value |
| `-d` | grid_size [grid_size_h] → +0x1450 [+0x1454]; with one value grid_size_h = grid_size | not 1, 2, 4, 8, 16, 32, 64 |
| `-e` | ldc_eis_split → +0x1490 | > 1 ("0: ldc split, 1: eis split") |
| `-f` | frame_cnt → +0x148C | not 2–4 |
| `-fv` | fov_out_1p → +0x15F0 | > 360 |
| `-h` | hw_mode → +0x1480 | > 4 |
| `-ir` | x_resize_ratio, y_resize_ratio → +0x1644, +0x1648 | not 2 values |
| `-j` | xBaseBits, yBaseBits → +0x1498, +0x149C | either not 3 or 4 ("Invalid xBaseBists …") |
| `-k` | calib_type → +0x164C | > 3 |
| `-of` | output_offset_x, output_offset_y → +0x163C, +0x1640 | not 2 values |
| `-p` | pan_min, pan_max → +0x1458, +0x145C | not 2 values |
| `-t` | tilt_min, tilt_max → +0x1460, +0x1464 | not 2 values |
| `-r` | rotate_min, rotate_max → +0x1470, +0x1474 | not 2 values (message says "tilt") |
| `-a` | radius_min, radius_max → +0x1478, +0x147C | not 2 values (message says "tilt") |
| `-z` | zoom_min, zoom_max → +0x1468, +0x146C | not 2 values |
| `-y` | table_width → +0x1494 | not in 32 … 256 or not a multiple of 32 |
| `-i` | path token → string slot 0 (para+0x000), length counter +0x1400 | — |
| `-o` | path → slot 1 (para+0x200), counter +0x1404 | — |
| `-b` | path → slot 2 (+0x400), counter +0x1408 | — |
| `-g` | path → slot 3 (+0x600), counter +0x140C | — |
| `-bx` / `-by` | file names → slot 5 (+0xA00) / slot 6 (+0xC00) | — |
| `-bp` / `-bq` | paths → slot 7 (+0xE00) / slot 8 (+0x1000) | — |

Path tokens run to the next space / tab, at most 0x3FF characters, and are **appended** to the slot (a repeated option
concatenates). The JSON parser (`ldc-1-stack.md` §4.4) fills the same para offsets; options without a JSON key: the path
slots, `-si`, `-st`, `-sl`, `-f`, `-e` (JSON has `ldc_eis_split`), `-fv` (JSON `fov_out_1p`).

### 10.3 Path block (`file_path_init` 0x427dc, into the 0xC400-byte path block of §1.3)
| path block | content |
|---|---|
| +0x200 | slot 0 (`-i`, the calibration folder) |
| +0x000 | slot 1 (`-o`, the output folder); +0x3800 = "<slot 1>/out.jpg" |
| +0x3C00 | slot 2 (`-b`) — reused by `ldc_bin_write_eptz` as its file-name buffer (§7.1) |
| +0x400 | "<slot 0>/<slot 3>" |
| +0x4000 / +0x4400 | mode 13: "<slot 0>/<slot 5>" / "<slot 0>/<slot 6>" (the map2bin X / Y map files, §4.9); other modes "<slot 1>/mapx.bin" / "mapy.bin" |
| +0xA800 / +0xAC00 / +0xB000 / +0xB400 | "<slot 0>/Cali_LDCpoly.bin", "/trdu0to180.bin", "/trdu180to220.bin", "/CalibPoly_new.bin" (§9.2) |
| +0xC000 | "<slot 0>/matrix_3x3.txt" |
| +0x4800 / +0x7800 + 0x400·k, +0x800 / +0xC00 + 0x400·k | stitch only (`file_stitch_init`): "<para+0x1200>/common_rectified_cy_mapx_%d.bin", "…mapy_%d.bin", "…/stereo_%d/in/Cap_video_%d.jpg" |

Slots 4, 7, 8 are not used by `file_path_init`.

## 11. MVE and the Simd library

### 11.1 MVE (`MI_MVE_*`) [R]
SigmaStar's software image helper on Simd, error prefix `ALGO_MSG_ERR`:
- `MI_MVE_Init` / `MI_MVE_Uninit` (handle, `Platform_Check`, §8.2); `MI_MVE_AllocateImage(handle, img, type, w, h, stride)`
  (types 0–0xF = plane count / bytes per pixel as IVE images; `CamOsMemAlloc` per plane); `MI_MVE_AllocateBuffer` /
  `MI_MVE_FreeBuffer` / `MI_MVE_FreeImage`; `MI_MVE_ConfigPadding`; `MI_MVE_Resize` (YUV420SP needs even sizes; "reduce
  size ratio should be power of 2 and up to 512"); `MI_MVE_AlphaBlending` (src1, src2, alpha, dst of equal size and format).
- `MI_MVE_MemPhysInit`, `MI_MVE_MemPhysRelease`, `MI_MVE_MemPhysAllocate`, `MI_MVE_MemPhysFree` (0x42e2c, 0x42e54,
  0x42e7c, 0x42ea4, 40 bytes each, no symbol): log "MI_MVE_MemPhys…() is not implemented yet" and return 0x10000106.
  The function starts a disassembler finds at 0x42e44 … 0x42ebc start 0x18 bytes into each stub, so each of them spans the tail of one stub and
  the string set-up of the next.
- Internal helpers: `_sym_4` (0x42ed0, 2 bytes, empty `bx lr`, called by `MI_MVE_AlphaBlending` and `MI_MVE_Resize`);
  `_sym_172` (0x42ed4, 40 bytes, handle check, "MVE handle is NULL"); `_sym_186` / `_sym_159` (0x42efc / 0x42f00, 4 bytes
  each, `return 0`, called by `MI_MVE_Init` / `MI_MVE_Uninit`); `mve_copy_rows(src, len, rows, sstride, dst, dstride)`
  (0x42f04, 44 bytes, no symbol) = row-wise `memcpy`, no caller; after `MI_MVE_AlphaBlending` two float helpers without a
  symbol, 0x4362c (52 bytes, dst[i] = a[i]·b[i]) and 0x43660 (40 bytes, dst[i] = s·src[i]), 4 floats per step, n ≤ 0 →
  return; 0x43688 (144 bytes) copy helper: n ≤ 255 or source / destination not co-aligned → `memcpy`, else a NEON block
  copy; `_sym_112` (0x43718, 72 bytes) the MVE copy of `Platform_Check`; `_sym_57`, `_sym_182`, `_sym_17` (0x43760,
  0x43ec4, 0x445f4; 1892, 1840, 1686 bytes) the resize kernels called by `MI_MVE_Resize` (float, NEON, own scratch buffers
  via `CamOsMemAlloc`) — vendor code, not Simd [I: upstream Base / Neon ResizeBilinear are integer].
- Users: only the multi-view **blending** path of mi_ldc (`_GetAndSetViewResByConfig` (0x1e494) allocates `MVE_Input1` and a Gaussian alpha map via
  `Init_Alpha_Map` / `G_gaussian_array_36`; `_MI_LDC_IMPL_AlphaBlending` (0x17a54) blends overlapping views, NV12 only, "BlendT" timing; destroy
  frees them — `ldc-1-stack.md` §6.5). Not reached with one view [I].

### 11.2 Simd: what the copy contains [R comdat section names of mi_ldc.ko; upstream sources of github.com/ermig1979/Simd at release tags]
Simd (ermig1979/Simd, MIT), NEON and Base code, ≈220 KB: **100 template instances** in comdat sections (their names survive
only in the section names `.text._ZN4Simd…`; the symbols are stripped) plus ≈50 functions in the stripped `.text` (C-API
wrappers and non-template code, §11.4–§11.5). The instances, by function: Base::HistogramConditional (6);
Neon::AbsDifferenceSum, …Masked, AbsDifferenceSums3x3, …Masked, AbsGradientSaturatedSum, AbsSecondDerivativeHistogram,
HistogramMasked, OperationBinary8u, ReduceGray2x1, ReduceGray2x2, SquaredDifferenceSum, …Masked, VectorProduct (2 each:
aligned / unaligned), HistogramConditional (12), MedianFilterRhomb3x3 / Rhomb5x5 / Square3x3 / Square5x5 (8 each),
AlphaBlending (9), AlphaFilling (9), InterleaveBgr, InterleaveBgra, InterleaveUv, LbpEstimate, OperationBinary16i,
StretchGray2x2 (1 each). They match upstream signatures from v4.3.75 to v7.2.164 (97 / 100; the 3 unmatched are
`Neon::LbpEstimate<true>`, whose instance has a changed parameter list from constant propagation, and
`Neon::ReduceGray2x1<true/false>`, a vendor file with no upstream counterpart); the dispatch conditions are the same in
every tag.
**Vendor additions** (in no upstream tag): `Base/Neon::ReduceGray2x1` and `Base::Reduce2DimSizeGray` (message "reduce size
ratio should be power of 2 and up to 512!"). libmi_ive.so carries the same additions in its `Function_tools/Simd` tree
(`ive-video-analytics.md` §6.1), so both copies are one vendor tree [I].
**Upstream functions not in the module**: Base / Neon::Fill*, Base / Neon::ResizeBilinear, Base::LbpEstimate (its wrapper
logs instead), Neon::Histogram, Base / Neon::Texture* [R: no function left to carry them].
**Reached at run time from mi_ldc**: only `SimdAlphaBlending` → `Neon::AlphaBlending` / `Base::AlphaBlending` (through
`MI_MVE_AlphaBlending`, §11.1); every other Simd function is dead code that came with the copied source files [R: caller
scan of §11.4].

### 11.3 Simd version [R unless marked]
- **Lower bound v4.3.76.** AlphaFilling (present with exactly the module's instance signatures `AlphaFilling<align,
  channelCount>(dst, stride, w, h, const uint8x16_t *channel, alpha, alphaStride)` and `AlphaFilling<align>(…, const uint8_t
  *channel, channelCount, …)`) appears in `SimdNeonAlphaBlending.cpp` in v4.3.75 (v4.2.67 … v4.2.74 have only
  AlphaBlending). In v4.3.76 `Neon::ReduceGray2x2`'s tail call changed from `ReduceGray2x2<align>` to
  `ReduceGray2x2<false>`; in the module's `Neon::ReduceGray2x2<true>` (`_sym_201`, comdat) the body loads carry the 128-bit
  alignment hint (`vld1.8 …[r:0x80]`) and the tail loads have none → v4.3.76 or later.
- **Upper bound v4.6.95.** (a) `Neon::AbsDifferenceSum<true>` (`_sym_92`) splits rows into blocks with `lsr #8` and a step
  of 0x1000 (= `A << 8`); v4.7.101 changed this to `A << 7` → earlier than v4.7.101. (b) `Neon::MedianFilterSquare5x5<true,
  1>` (`_sym_189`) has two copies of a one-row loop (row + 1, dst + dstStride; stores aligned / aligned / unaligned in
  each); v4.6.96 replaced it by a two-row loop (stores to dst and dst + dstStride) plus a one-row tail → earlier than
  v4.6.96.
- Likely **≤ v4.6.94** [I]: `SimdBaseAlphaBlending.cpp` gained `Base::AlphaPremultiply` / `AlphaUnpremultiply` in v4.6.95;
  the module has `Base::AlphaFilling` (no caller, so the file was compiled whole) directly followed by
  `Neon::AlphaBlending`, with no function between them. (Later upstream additions to the same file — AlphaPremultiply /
  AlphaUnpremultiply in the Neon file from v4.8.103, AlphaBlendingUniform from v4.9.108, AlphaBlending2x from v5.x — have no
  template instance in the module.)
- The libmi_ive.so copy of the same vendor tree is bracketed **v4.4.84 … v4.6.93** by its assertion texts
  (`ive-video-analytics.md` §6.1); for mi_ldc.ko that bracket applies only through the same-tree inference [I]. By machine code
  alone mi_ldc's copy is v4.3.76 … v4.6.95. The exact tag inside that window is not known: no upstream source change in
  the window shows in the machine code. The kernel build date (2022-07) is no bound: the vendor copy can be old.

### 11.4 The C-API wrapper file and the static constructor [R]
The C-API layer is a vendor-trimmed SimdLib.cpp (a contiguous block 0x47310 … 0x47774; of its functions only
`SimdAlphaBlending` has a symbol, `_sym_157`, 36 bytes — the rest sit in the gaps between symbols). Each `SimdX` tests the
byte **Neon::Enable** (0x8b70c) and the upstream width condition, then tail-calls `Neon::X` or `Base::X`. The static
constructor `_GLOBAL__sub_I_SimdReduceGray2x1` (`.text.startup` 0x5413c) sets Neon::Enable = 1 and the alignment word
0x8b710 = 16 **unconditionally** (no CPU / OS probe, unlike upstream); its name says the file's first function is
`SimdReduceGray2x1` [I: GCC names the constructor after the first global symbol]. In file order:

| addr | function | Neon when | Neon / Base target |
|---|---|---|---|
| 0x47310 | SimdReduceGray2x1 (vendor) | srcWidth ≥ 32 | `_sym_163` / `_sym_185` |
| 0x47334 | SimdLbpEstimate (vendor-changed) | width ≥ 18 | `_sym_199`; else **no Base fallback**: logs "ALGO_MSG_ERR … Image width should not be smaller than 18" and returns |
| 0x47374 | SimdReduce2DimSizeGray (vendor) [I name] | — | Base only `_sym_152` |
| 0x47378 | SimdAllocate(size, align): `CamOsMemAlloc(AlignLo(size + 127 + a, a))` with a = AlignHi(align, 4); returns the block + 0x40 (NULL stays NULL) | | |
| 0x47394 | SimdFree(p): `CamOsMemRelease(p − 0x40)` when p ≠ NULL | | |
| 0x4739c / 0x473a8 / 0x473b4 | SimdAlign (size + align − 1) & −align / SimdAlignment (returns the word 0x8b710 = 16) / SimdRelease (calls the object's second vtable slot when non-NULL) | | |
| 0x473c0 | SimdOperationBinary8u | width·channelCount ≥ 16 | `_sym_28` / `_sym_107` |
| 0x47430 | SimdAlphaBlending — **the only wrapper with a caller** (`MI_MVE_AlphaBlending`) | width ≥ 16 | `_sym_32` / `_sym_208` |
| 0x47454 | SimdHistogram | Base only (as upstream) | `_sym_9` |
| 0x47458 | SimdHistogramConditional | width ≥ 16 | `_sym_101` / `_sym_161` |
| 0x47484 / 0x474a8 | SimdDeinterleaveUv / SimdDeinterleaveBgr | width ≥ 16 | `_sym_188` / `_sym_14`; `_sym_122` / `_sym_76` |
| 0x474cc / 0x474f4 | SimdInterleaveUv / SimdInterleaveBgr | width ≥ 16 | `_sym_66` / `_sym_124`; `_sym_91` / `_sym_109` |
| 0x4751c / 0x47540 | SimdStretchGray2x2 / SimdReduceGray2x2 | srcWidth ≥ 16 / ≥ 32 | `_sym_145` / `_sym_142`; `_sym_3` / `_sym_192` |
| 0x47564 | SimdAbsGradientSaturatedSum | width ≥ 16 | `_sym_211` / `_sym_95` |
| 0x47588 | SimdCopy | Base only | `_sym_52` |
| 0x4758c / 0x475b4 / 0x475dc | SimdAbsDifferenceSums3x3 / SimdAbsDifferenceSum / SimdSquaredDifferenceSum | width ≥ 18 / 16 / 16 | `_sym_62` / `_sym_136`; `_sym_90` / `_sym_12`; `_sym_196` / `_sym_110` |
| 0x47604 / 0x47660 / 0x476bc / 0x47718 | SimdMedianFilterRhomb3x3 / Rhomb5x5 / Square3x3 / Square5x5 | (width − 1)·ch ≥ 16 / (width − 2)·ch ≥ 16 (3x3 / 5x5) | `_sym_156` / `_sym_44`; `_sym_205` / `_sym_116`; `_sym_155` / `_sym_123`; `_sym_168` / `_sym_26` |

### 11.5 Every function of the stripped Simd `.text` (0x44ec0 … 0x535b0) [R: ELF symbol table, call graph]
<!-- ev:ldc-2-eptz-lib.11.5 -->
The stripped functions carry vendor-renamed local symbols `_sym_N` **with sizes**; some have no symbol at all. Basis of each
name: **W** = the Neon or Base branch of a wrapper in §11.4 [R]; **C** = calls exactly the comdat template instances of that
function (the comdat section names name them) [R]; **K** = called by / calls an identified function as upstream does [R];
**O** = position in the upstream file's function order next to identified neighbours (GCC kept the files whole: the
uncalled functions are there) [I]; **S** = own message string [R]. Size = ELF symbol size, or distance to the next start
when there is no symbol. None of these has a caller outside the MVE / Simd part except `SimdAlphaBlending` (§11.4). Names
marked only O rest on file order, not on a compiled-body comparison (no ARM C++ compiler was at hand; SigmaStar's gcc 9.1
would be the match).

| addr | ELF name | size | function | basis |
|---|---|---|---|---|
| 0x44ec0 | `_sym_185` | 654 | Base::ReduceGray2x1 (vendor addition) | W |
| 0x45150 | — | 236 | Base::AbsSecondDerivativeHistogram | O |
| 0x4523c | `_sym_9` | 256 | Base::Histogram | W, K |
| 0x4533c | — | 436 | Base::HistogramMasked | O |
| 0x454f0 | — | 2712 | Base::ChangeColors | O (an inlined copy also sits in NormalizeHistogram [I]) |
| 0x45f88 | `_sym_158` | 154 | Base::NormalizedColors | K |
| 0x46024 | — | 3780 | Base::NormalizeHistogram (calls Histogram, NormalizedColors) | K |
| 0x46ee8 | `_sym_161` | 140 | Base::HistogramConditional (switch over the six comparison instances) | C, W |
| 0x46f74 | `_sym_192` | 924 | Base::ReduceGray2x2 | W |
| 0x47774 | `_sym_163` | 82 | Neon::ReduceGray2x1 (vendor addition) | C, W |
| 0x477c8, 0x47928 | — | 352, 392 | Neon::SquaredDifferenceSum32f, Neon::SquaredDifferenceKahanSum32f (float NEON, the second with more float ops) | O |
| 0x47ab0 / 0x47b04 | `_sym_196` / — | 80 / 108 | Neon::SquaredDifferenceSum / Neon::SquaredDifferenceSumMasked | C |
| 0x47b70 / 0x47cb0 / 0x47e34 | `_sym_188` / `_sym_122` / — | 320 / 386 / 468 | Neon::DeinterleaveUv / DeinterleaveBgr / DeinterleaveBgra | W, W, O |
| 0x48008 | `_sym_95` | 1000 | Base::AbsGradientSaturatedSum | W |
| 0x483f0, 0x484a4, 0x48558, 0x4860c | `_sym_156`, `_sym_155`, `_sym_205`, `_sym_168` | 178 each | Neon::MedianFilterRhomb3x3, Square3x3, Rhomb5x5, Square5x5 (switch over alignment × channelCount 1–4) | C, W |
| 0x486c0 | `_sym_3` | 82 | Neon::ReduceGray2x2 | C, W |
| 0x48714 | `_sym_107` | 5036 | Base::OperationBinary8u | W |
| 0x49ac0 / 0x49df0 | — | 816 / 736 | Base::OperationBinary16i / Base::VectorProduct | O |
| 0x4a0d0 | `_ZN4Simd4Neon11LbpEstimateILb0EEEvPKhiPhi.constprop.0` | 232 | Neon::LbpEstimate<false> row helper (constant-propagated clone; the only unrenamed local symbol) | R |
| 0x4a1b8 | `_sym_199` | 630 | Neon::LbpEstimate | C, W |
| 0x4a430 / 0x4b34c | `_sym_208` / — | 3866 / 3788 | Base::AlphaBlending / Base::AlphaFilling | W / O |
| 0x4c218 / 0x4c2f4 | `_sym_32` / — | 218 / 380 | Neon::AlphaBlending / Neon::AlphaFilling | C, W / C |
| 0x4c470 / 0x4c754 / 0x4cb34 | `_sym_124` / `_sym_109` / — | 740 / 990 / 392 | Base::InterleaveUv / InterleaveBgr / InterleaveBgra | W, W, O |
| 0x4ccbc / 0x4ccfa / 0x4cd58 | — / — / `_sym_101` | 62 / 94 / 346 | Neon::AbsSecondDerivativeHistogram / Neon::HistogramMasked / Neon::HistogramConditional | C / C / C, W |
| 0x4ceb4 / 0x4d224 / 0x4d2b4 / 0x4def4 | `_sym_12` / — / `_sym_136` / — | 880 / 144 / 3136 / 684 | Base::AbsDifferenceSum / AbsDifferenceSumMasked / AbsDifferenceSums3x3 / AbsDifferenceSums3x3Masked | W / O / W / O |
| 0x4e1a0 / 0x4e1e8 | `_sym_211` / `_sym_145` | 70 / 230 | Neon::AbsGradientSaturatedSum / Neon::StretchGray2x2 | C, W |
| 0x4e2d0 / 0x4e504 / 0x4e594 / 0x4e6b4 | `_sym_110` / — / — / — | 562 / 144 / 288 / 288 | Base::SquaredDifferenceSum / SquaredDifferenceSumMasked / SquaredDifferenceSum32f / SquaredDifferenceKahanSum32f | W / O / O / O |
| 0x4e7d4 | `_sym_142` | 638 | Base::StretchGray2x2 | W |
| 0x4ea54 / 0x4eabc / 0x4ec30 | `_sym_28` / — / — | 104 / 372 / 60 | Neon::OperationBinary8u / OperationBinary16i / VectorProduct | C, W / C / C |
| 0x4ec6c / 0x4eca0 | `_sym_52` / — | 50 / 320 | Base::Copy / Base::CopyFrame (both memcpy per row) | W / O |
| 0x4ede0 / 0x4f57c / 0x5093c / 0x51024 | `_sym_44` / `_sym_123` / `_sym_116` / `_sym_26` | 1946 / 5056 / 1768 / 4512 | Base::MedianFilterRhomb3x3 / Square3x3 / Rhomb5x5 / Square5x5 | W |
| 0x521c4 / 0x52218 / 0x52284 / 0x522cc | `_sym_90` / — / `_sym_62` / — | 80 / 108 / 68 / 84 | Neon::AbsDifferenceSum / AbsDifferenceSumMasked / AbsDifferenceSums3x3 / AbsDifferenceSums3x3Masked | C (W for the two with symbols) |
| 0x52320 / 0x52620 / 0x52a38 | `_sym_14` / `_sym_76` / — | 768 / 1046 / 488 | Base::DeinterleaveUv / DeinterleaveBgr / DeinterleaveBgra | W, W, O |
| 0x52c20 | `_sym_152` | 1618 | Base::Reduce2DimSizeGray (vendor addition; "reduce size ratio should be power of 2 and up to 512!") | W, S |
| 0x53274 / 0x53344 / 0x53460 | `_sym_66` / `_sym_91` / — | 206 / 282 / 336 | Neon::InterleaveUv / InterleaveBgr / InterleaveBgra | C, W / C, W / C |

Coverage: every byte of 0x42990 … 0x535b0 lies in one of the functions above, the MVE functions of §11.1 or the wrappers
of §11.4 [R: gap scan of ELF symbol ranges against all starts; every range without a symbol holds a single function — no
second prologue after a return — except the wrapper block of §11.4].
