# kjd_lvgl — the on-screen menu app (LVGL)
<!-- ev:kjd_lvgl -->

`/customer/kjd_lvgl` is the camera's on-screen menu: an NXP **GUI Guider** project on LVGL 8.3.0-dev that draws a
1600×900 ARGB8888 overlay into `/dev/fb0` over the video, reads the six front-panel buttons from the SAR ADC and the IR
remote from `/dev/input/event0`, keeps the user's settings in `/customer/ws.bin`, `/customer/ws.ini` and
`/customer/Line.ini`, and turns every action on the picture (ISP values, mirror, crop, zoom, freeze, photo, recording,
playback, standby, factory reset) into a 65-byte message to prog_uvc over TCP 127.0.0.1:8888 [R]. It links no MI or SDK
library and makes no MI call; its only direct effects on the system are the framebuffer, GPIO46, the system clock, the
switch of the USB port to mass storage, SD formatting and a few file copies [R].

Elsewhere: the server side of the menu link — what prog_uvc does for each id, the reply formats, the `ws.bin` layout with
its apply rules and the first-boot creation of `ws.bin` / `default_ws.bin` — `prog_uvc.md` §6.1–§6.4, the picture functions
behind the ids (mirror, crop, zoom, split view, photo, recording, player) — `prog_uvc.md` §7; the demo.sh start line and the
`sleep 3` between the two apps — `rootfs-customer.md` §3.4; `sd_hotplug.sh` and `usb_otg.sh` — `rootfs-customer.md` §4.1,
§4.2; the factory test `tmp`, which reads the same buttons and drives GPIO46 — `rootfs-customer.md` §8; GPIO46 (pad, LED
colour) and the board wiring of the buttons — `kernel-platform.md` §6.3; the SAR driver and its ioctls, the IR receiver, its
keymap and the physical remote — `kernel-drivers.md` §3.1, §3.2; the RTC behind `hwclock` — `kernel-drivers.md` §2.4;
`fbdev.ko`, `/config/fbdev.ini` and fb0 — `rgn-gop.md` §5; the mass-storage and webcam gadgets — `usb-gadget.md` §1, §4; the
inventory row of the binary and its files — `firmware-image.md` §3.7.

**Tags.** [R] read from the binary, its decompile or disassembly, a file of the dump, a public source, or observed by
running the binary's own code in a CPU emulator on the laptop ("[R emulator]" where only the emulator run shows it);
[S] stated by a source; [I] inference. A heading tagged [R] covers every statement under it unless marked otherwise.

**Binary.** `/customer/kjd_lvgl`, ELF ARM32, Thumb-2, not stripped, BuildID `3231aa4a…`. Addresses
are kjd_lvgl virtual addresses; `ui[N]` is the object pointer at `guider_ui + 4·N` (§5.1).

## 1. What the binary is made of

### 1.1 Identity and build [R]

GUI Guider project in the `lv_port_linux_frame_buffer` layout: LVGL, `lv_drivers` (fbdev, evdev), the separate `lv_lib_png`
repo, and every LVGL demo and example linked in. Toolchain: SigmaStar gcc 9.1.0 (2020.07), glibc 2.30, Thumb-2, `-O2`. The
executable is not linked with section garbage collection (all demos and examples are in it), so a missing global function
means a missing definition [I].

**Debug information.** `.debug_info` has 12 compile units, all from the toolchain (crt*, libgcc); the app itself has only the
ELF symbol table — function and global names, and file names through `STT_FILE` symbols. There is no `msg_node` struct in
the debug information; the message format (§4) comes from the code.

**NEEDED:** `libm.so.6`, `libpthread.so.0`, `libc.so.6` only. No MI or SDK library: everything that touches video goes
through prog_uvc (§4).

**Vendor source files** (from `STT_FILE`): `main.c` (all logic), `setup_scr_screen.c`, `gui_guider.c`, `events_init.c`,
`widgets_init.c`, `custom.c` (§10); six GUI-Guider font files `lv_font_SourceHanSerifSC_Regular_16.c`,
`lv_customer_font_SourceHanSerifSC_Regular_16.c`, `lv_customer_font_SourceHanSerifSC_Regular_20.c`,
`lv_customer_font_mysh_16.c`, `lv_font_montserratMedium_16.c`, `lv_font_montserratMedium_12.c` (§7); five image files
`_log_300x137.c`, `__photo_60x60.c`, `_stop_60x60.c`, `_scream_85x62.c`, `_freeze_85x62.c` (§5.1).

**Other components** [R `STT_FILE` + symbols]: `lv_drivers` with the files of every driver present (win_drv, wayland,
AD_touch, FT5406EE8, XPT2046, evdev, libinput, xkb, gtkdrv, GC9A01, ILI9341, R61581, SHARP_MIP, SSD1963, ST7565, UC1610,
drm, fbdev, sdl, sdl_common, sdl_gpu), of which only fbdev and evdev have code in the binary — so the vendor's
`lv_drv_conf.h` has `USE_FBDEV 1`, `USE_EVDEV 1` and everything else compiled out [I: from which driver functions exist];
a second `lodepng.c` / `lv_png.c` plus `png_decoder_test.c` after lv_drivers in `.text` = the separate `lv_lib_png` repo
(§1.3, libs row).

### 1.2 LVGL version [R]

The linked LVGL is not a release: it is the LVGL `master` branch at commit **`533e325f21ea6fd046fcf46e620ef5c5a3e8f94d`**
(2022-06-07 12:52 +0200, "fix(draw) avoid use-after-free when drawing arcs (#3399)"), i.e. **8.3.0-dev** — that tree's
`lvgl.h` says `LVGL_VERSION_MAJOR 8, MINOR 3, PATCH 0, INFO "dev"`, one month before the v8.3.0 release (2022-07-06). The
vendor's build directory, seen in a qrcodegen assert string, is named `ido-lvgl8.2/lvgl` — only a folder name [R string; I
meaning].

Evidence:

1. **Source-file set.** The ELF keeps one `STT_FILE` symbol per compiled `.c`, also for files whose code is compiled out
   (`lv_draw_sdl_*.c`, `lv_ffmpeg.c`, `lv_rlottie.c` …), so it lists the exact LVGL tree. The set
   includes 8.3-only files (`lv_draw_sw_transform.c`, `lv_fragment.c`, `lv_imgfont.c`, `lv_gpu_swm341_dma2d.c`). Against
   releases: v8.2.0 lacks 22 of the binary's files; v8.3.0–v8.3.4 contain all of them; v8.3.5+ lack
   `lv_gpu_nxp.c` / `lv_gpu_nxp_vglite.c`. v8.3.0 also has `lv_draw_layer.c`, `lv_draw_sw_layer.c`, `lv_ime_pinyin.c`, which
   the binary does not have, and v8.3.0's `refr_obj` calls `lv_draw_layer_create`, which the binary lacks — so not the
   release. Over every commit `v8.2.0..v8.3.0` (comparing, per commit, the binary's files missing in
   the commit and the commit's files missing in the binary) exactly the 28 commits from `029eef79c` (2022-05-24, adds the last missing
   file) to `ca9aff611` (2022-06-14) match; the next commit `f57593518` adds the layer files. The two commit files never in
   the binary are the templates `lv_templ.c` / `lv_objx_templ.c` (never compiled).
2. **Function set** (FUNC symbols against the function definitions of each commit): the public functions added by
   `eef838f51` (2022-05-30, `lv_textarea_set/get_password_bullet`) are present → at or after that commit; those added by
   `9a48de0f8` (2022-06-09, `lv_obj_set_local_style_prop_meta`, `lv_style_set_prop_meta` …) are absent.
3. **Code markers** inside that window, read in the decompile:

| commit (date) | change | in the binary | so |
|---|---|---|---|
| `9ac8ce69f` (06-02) | `lv_obj_draw` COVER_CHECK: blend mode ≠ NORMAL → NOT_COVER | yes: style prop 0x67 test after `bg_opa < 0xfd` (0x3917c) | ≥ |
| `e59c83b45` (06-03) | `lv_anim_t.deleted_cb` (+4 B), called in `lv_anim_del` and when an animation ends | yes: `lv_anim_del` 0x65ec4 calls `a[4]` before `lv_mem_free`; GUI Guider's `ui_move_animation` fills the field | ≥ |
| `ff6316c74` (06-06) | msgbox button padding revert | not decidable: with `LV_DPI_DEF 130` (§1.3) both versions compile identically | — |
| `533e325f2` (06-07) | arc: free the inner radius mask only if it was initialised | yes: `lv_draw_sw_arc` 0x46b7c frees `mask_in` under a flag and the empty-inner path frees only one param | ≥ |
| `018a8cfda` (06-08) | `lv_lru_remove_lru_item` becomes a global function | no: only `lv_lru_create/del/set/get/remove` are symbols; the static version is inlined into `lv_lru_set` (660 B) | < |
| `03c43d95a` (06-08) | example include path only | — | — |

So the tree is `533e325f2`, or byte-identical to it in every compiled file (no later candidate exists before `018a8cfda`).
Vendor patches on top of LVGL are not excluded by file and symbol comparison alone (§11); the only non-LVGL function found
inside an LVGL object is the tick function (§1.3, `LV_TICK_CUSTOM_SYS_TIME_EXPR`).

Differences of this tree from the v8.3.x releases [R source; I which matter]: no layer rendering for transformed or
semi-transparent widgets (added by `f57593518`, in v8.3.0); no `inherit` / `initial` style values; `lv_refr_get_top_obj`
iterates the children `i = 0..cnt−1` (read at 0x414f0), where 8.3.1 iterates top-down; and the later 8.3.x fixes. kjd_lvgl
sets no transform, blend-mode or layer style: the boot-time UI build uses 22 style setters, none of them these, and
the rest of the decompiled app sets none.

### 1.3 `lv_conf.h` [R]

It can be reconstructed whole: the commit's own `lv_conf_template.h` with every value set to what the binary shows; 11
values differ from the template. Each option was decided by one
probe — symbol present / absent, struct and object sizes, constants in the decompile (function
named in the table).

**Values that differ from the template:**

| option | value | evidence |
|---|---|---|
| `LV_COLOR_DEPTH` | **32** | draw buffer `buf` = 0x57e400 = 1600×900×4 B; colour maths on 32-bit words (`lv_color_lighten` 0x68550) |
| `LV_COLOR_SCREEN_TRANSP` | **1** | `lv_draw_sw_blend_basic` (0x47c34, 8664 B) branches on `drv->screen_transp` (byte 0x10 bit 0x40), i.e. the `fill_argb/map_argb` code is compiled (inlined) |
| `LV_MEM_SIZE` | **64 MiB** (0x4000000) | `work_mem_int.7099` is 0x4000000 B in .bss; `lv_mem_init` → `lv_tlsf_create_with_pool(work_mem_int, 0x4000000)` |
| `LV_DISP_DEF_REFR_PERIOD` | **16** ms | `lv_disp_drv_register`: `lv_timer_create(_lv_disp_refr_timer, 0x10, disp)` |
| `LV_TICK_CUSTOM` | **1** | no `lv_tick_inc` symbol; `lv_tick_get` 0x65b04 = inlined `custom_tick_get` |
| `LV_TICK_CUSTOM_SYS_TIME_EXPR` | `(custom_tick_get())` | `custom_tick_get` 0x65a88 is a **global function inside the `lv_hal_tick.c` object** (between `$t` 0x65a88 of that file and `lv_tick_get`), with its `static uint64_t start_ms` (`start_ms.13257`, 8 B): `gettimeofday` → ms; the first call stores the start, it returns ms since the first call (wraps as uint32). So either the tick header named by `LV_TICK_CUSTOM_INCLUDE` defines the function or the vendor edited `lv_hal_tick.c` [I: either; the header name is not in the binary]. Stock `lv_port_linux_frame_buffer` has the same function in its `main.c` [I] |
| `LV_IMG_CACHE_DEF_SIZE` | **16** | `lv_init`: `lv_img_cache_set_size(0x10)` |
| `LV_FONT_MONTSERRAT_8/10/12/16/18/20/22` | **1** (14 stays 1; 24…48 stay 0) | `lv_font_montserrat_8 … _22` objects present (28 B each); 24–48 absent |
| `LV_FONT_DEJAVU_16_PERSIAN_HEBREW` | **1** | object present (0x1fb1ec) |
| `LV_USE_BIDI` | **1** | `_lv_bidi_process`, `_lv_bidi_process_paragraph`, `_lv_bidi_detect_base_dir` present |
| `LV_USE_ARABIC_PERSIAN_CHARS` | **1** | `_lv_txt_ap_proc`, `_lv_txt_ap_calc_bytes_cnt` present |
| `LV_USE_DEMO_MUSIC` | **1** (unused) | `lv_demo_music`, `_lv_demo_music_main_create` (2744 B) … present; never called by the vendor code (no caller outside the demo) |
| `LV_DEMO_MUSIC_ROUND` | **1** (unused) | `grid_rows.12550` is in .data (non-const → SQUARE or ROUND branch) and `_lv_demo_music_main_create` sets `pad_left/right = LV_HOR_RES/6` (only under ROUND) |

**Values equal to the template (checked):**

| group | values | evidence |
|---|---|---|
| colour | `LV_COLOR_16_SWAP 0` (no effect at 32 bpp [I]); `LV_COLOR_MIX_ROUND_OFS 0`; `LV_COLOR_CHROMA_KEY lv_color_hex(0x00ff00)` | `lv_color_lighten` has no rounding offset; `lv_disp_drv_init` stores chroma key 0xff00ff00 |
| memory | `LV_MEM_CUSTOM 0`, `LV_MEM_ADR 0`, `LV_MEM_BUF_MAX_NUM 16`, `LV_MEMCPY_MEMSET_STD 0` | `lv_tlsf_*` + static pool (LVGL's own TLSF allocator); `lv_mem_buf` 128 B = 16 × 8, `lv_mem_buf_get` loops to 0x10; `lv_memcpy`/`lv_memset` present |
| HAL | `LV_INDEV_DEF_READ_PERIOD 30`, `LV_DPI_DEF 130` | `lv_indev_drv_register`: timer 0x1e; `lv_disp_drv_init` dpi field 0x82; `lv_obj_class` width/height_def 130 |
| (lv_conf_internal defaults) | scroll limit 10, scroll throw 10, long press 400 ms, repeat 100 ms, gesture limit 50, gesture min velocity 3 | `lv_indev_drv_init`: words 0x32030a0a, 0x00640190 |
| drawing | `LV_DRAW_COMPLEX 1`, `LV_SHADOW_CACHE_SIZE 0`, `LV_CIRCLE_CACHE_SIZE 4`, `LV_GRADIENT_MAX_STOPS 2`, `LV_GRAD_CACHE_DEF_SIZE 0`, `LV_DITHER_GRADIENT 0`, `LV_DISP_ROT_MAX_BUF 10240` | mask/arc/shadow code present; no `sh_cache` object; `_lv_circle_cache` 112 B = 4 × 28; gradient `dir` at grad-dsc offset 0x11 (2 × 8-B stops); `lv_gradient_get` frees and `lv_mem_alloc(0)`s the cache; no dither functions; `draw_buf_flush` uses `lv_mem_buf_get(0x2800)` and 0xa00 px rows |
| `LV_LAYER_SIMPLE_*` | not in this tree (no layers yet, §1.2) | — |
| GPU | `LV_USE_GPU_ARM2D/STM32_DMA2D/SWM341_DMA2D/NXP_PXP/NXP_VG_LITE/SDL` all **0** | none of their init/ctx functions present (the files exist as empty objects) |
| log | `LV_USE_LOG 0` (`LV_LOG_*` therefore irrelevant) | no `_lv_log_add`, `lv_log_register_print_cb` |
| asserts | `LV_USE_ASSERT_NULL 1`, `LV_USE_ASSERT_MALLOC 1`, `LV_USE_ASSERT_STYLE 0`, `LV_USE_ASSERT_MEM_INTEGRITY 0`, `LV_USE_ASSERT_OBJ 0`, `LV_ASSERT_HANDLER while(1);` | `lv_anim_timeline_stop(NULL)` and `lv_disp_drv_register` on failed alloc end in an empty infinite loop; `lv_style_t` is 8 B with `prop_cnt` at offset 7 (no sentinel; `ui_init_style` tests byte 7); `lv_mem_test` has no caller; `lv_obj_is_valid` called only by vendor code |
| monitors | `LV_USE_PERF_MONITOR 0`, `LV_USE_MEM_MONITOR 0`, `LV_USE_REFR_DEBUG 0` | no `perf_monitor`/`mem_monitor` code; `lv_rand` is called only by chart examples and `lv_lru_create` |
| printf | `LV_SPRINTF_CUSTOM 0`, `LV_SPRINTF_USE_FLOAT 0` | `lv_snprintf`/`lv_vsnprintf` present; `_vsnprintf` 0x6a994 has no `f`/`e`/`g` case. The vendor's own `%0.1f` goes through glibc `sprintf` (§4.1) |
| misc | `LV_USE_USER_DATA 1`, `LV_ENABLE_GC 0`, `LV_BIG_ENDIAN_SYSTEM 0`, `LV_USE_LARGE_COORD 0` | `lv_obj_t` 36 B and `lv_font_t`/class structs 28 B include `user_data`; GC roots are ordinary globals (`_lv_circle_cache`, `_lv_grad_cache_mem` …); little-endian ARM; coordinates are `short` everywhere |
| attributes | `LV_ATTRIBUTE_*` empty, `LV_ATTRIBUTE_MEM_ALIGN_SIZE 1`, `LV_EXPORT_CONST_INT` default | [I]: nothing in the code needs other values; not provable from the binary |
| fonts | `LV_FONT_MONTSERRAT_14 1`, 24–48 `0`, `_12_SUBPX 0`, `_28_COMPRESSED 0`, `SIMSUN_16_CJK 0`, `UNSCII_8/16 0`; `LV_FONT_DEFAULT &lv_font_montserrat_14`; `LV_FONT_FMT_TXT_LARGE 0`; `LV_USE_FONT_COMPRESSED 0`; `LV_USE_FONT_SUBPX 0` (`LV_FONT_SUBPX_BGR` irrelevant); `LV_FONT_CUSTOM_DECLARE` empty [I: GUI Guider declares its fonts in `gui_guider.h`] | objects present/absent; `lv_style_prop_get_default` returns `&lv_font_montserrat_14` for TEXT_FONT (0x57) and `lv_disp_drv_register` passes it to `lv_theme_default_init`; glyph dsc 8 B with 20-bit `bitmap_index`; `lv_font_get_bitmap_fmt_txt` returns NULL for `bitmap_format ≠ 0`; `lv_draw_sw_letter` returns when `font->subpx ≠ 0` |
| text | `LV_TXT_ENC LV_TXT_ENC_UTF8`, `LV_TXT_BREAK_CHARS " ,.;:-_"`, `LV_TXT_LINE_BREAK_LONG_LEN 0` (PRE/POST_MIN_LEN unused), `LV_TXT_COLOR_CMD "#"` | `_lv_txt_encoded_next = lv_txt_utf8_next` etc.; string at 0x1fb120 used by `_lv_txt_get_next_line` 0x6cf68; no long-word split code; 0x23 test in the same function |
| bidi | `LV_BIDI_BASE_DIR_DEF` AUTO **or** LTR | `_lv_bidi_detect_base_dir` returns 0 (LTR) when no strong char — both settings compile to that |
| widgets | all `LV_USE_<widget> 1` (arc, bar, btn, btnmatrix, canvas, checkbox, dropdown, img, label, line, roller, slider, switch, textarea, table; animimg, calendar + both headers, chart, colorwheel, imgbtn, keyboard, led, list, menu, meter, msgbox, span, spinbox, spinner, tabview, tileview, win) | every widget file has its functions (`file_presence_533e325f2.txt`) |
| widget options | `LV_LABEL_TEXT_SELECTION 1`, `LV_LABEL_LONG_TXT_HINT 1`, `LV_ROLLER_INF_PAGES 7`, `LV_TEXTAREA_DEF_PWD_SHOW_TIME 1500`, `LV_CALENDAR_WEEK_STARTS_MONDAY 0` (default day/month names), `LV_SPAN_SNIPPET_STACK_SIZE 64` | `lv_label_t` 76 B with `sel_start` at +0x3c (after the 12-B hint); `lv_roller_set_options` `*7`; textarea constructor 0x5dc; names "Su","Mo",… and "January"… in .rodata (0x1fa708, 0x1fa764); `snippet_stack` 1284 B = 64 × 20 + 4 |
| themes | `LV_USE_THEME_DEFAULT 1` (`_DARK 0`, `_GROW 1`, `_TRANSITION_TIME 80`), `LV_USE_THEME_BASIC 1`, `LV_USE_THEME_MONO 1` | `lv_theme_default_init(…, dark=0, &lv_font_montserrat_14)` from `lv_disp_drv_register`; grow style sets transform width+height; `lv_style_transition_dsc_init(…, 0x50, 0x46)` and `(…, 0x50, 0)`; basic/mono init present |
| layouts | `LV_USE_FLEX 1`, `LV_USE_GRID 1` | present |
| file systems | `LV_USE_FS_STDIO/POSIX/WIN32/FATFS 0` | no `lv_fs_*_init`; the PNG decoder that is used opens files with `fopen` itself (libs row) |
| libs | `LV_USE_PNG 0`, `LV_USE_BMP 0`, `LV_USE_SJPG 0`, `LV_USE_GIF 0`, `LV_USE_QRCODE 0`, `LV_USE_FREETYPE 0`, `LV_USE_RLOTTIE 0`, `LV_USE_FFMPEG 0` | no `lv_bmp_init`, `lv_split_jpeg_init`, `lv_gif_create`, `lv_qrcode_create`, `lv_freetype_init`, `lv_rlottie_*`, `lv_ffmpeg_init`. LVGL's own `src/extra/libs/png` is compiled out; `lv_png_init` (0x8757c) and `lodepng_*` sit **after** `fbdev_init`/`evdev_init` in .text, i.e. come from the separate **`lv_lib_png`** repo (its `png_decoder_test.c` is linked too); its `decoder_info` 0x874b0 checks the `png` suffix and reads the size with `fopen/fseek(16)/fread(8)` — no LVGL fs driver, no drive letter; `decoder_open` 0x87384 loads the file with `lodepng_load_file` (stdio) and `lodepng_decode32`. `qrcodegen.c` code is present although `LV_USE_QRCODE 0`: that file has no `LV_USE_QRCODE` guard in this tree [R source], so it compiles regardless |
| others | `LV_USE_SNAPSHOT 0`, `LV_USE_MONKEY 0`, `LV_USE_GRIDNAV 0`, `LV_USE_FRAGMENT 0`, `LV_USE_IMGFONT 0`, `LV_USE_MSG 0` | no `lv_snapshot_take`, `lv_monkey_create`, `lv_gridnav_add`, `lv_fragment_create`, `lv_imgfont_create`, `lv_msg_init` |
| examples/demos | `LV_BUILD_EXAMPLES 1`; `LV_USE_DEMO_WIDGETS 0`, `_KEYPAD_AND_ENCODER 0`, `_BENCHMARK 0`, `_STRESS 0`; music: `LV_DEMO_MUSIC_LARGE 0`, `_AUTO_PLAY 0`, `_SQUARE` / `_LANDSCAPE` not determinable (ROUND already selects every branch they would) | `lv_example_*` present (unused); no `lv_demo_widgets/benchmark/stress/keypad_encoder`; music images are the non-`_large` ones (`img_lv_demo_music_btn_play` …); no `auto_step_cb` |

The 64 MiB LVGL heap is far more than one 1600×900 screen of about two hundred widgets uses; it occupies address space in
.bss [I].

## 2. Start-up and threads

### 2.1 Start-up (`main` 0x114a0) [R]

1. `lv_init`, `lv_png_init`, `fbdev_init`: open `/dev/fb0` O_RDWR, `FBIOBLANK 0`, `FBIOGET_FSCREENINFO`,
   `FBIOGET_VSCREENINFO`, mmap `smem_len` (§8).
2. Display driver: hor 1600 × ver 900, one full-screen draw buffer `buf` (1600×900×4 B in .bss, no second buffer),
   `screen_transp = 1`, flush = `fbdev_flush`.
3. Input driver: `evdev_init` (`/dev/input/event0`, O_RDWR|O_NONBLOCK), registered as `LV_INDEV_TYPE_POINTER`. It has no
   effect in practice: there is no LVGL group or keypad device, and every key is handled by the vendor threads (§2.2, §6).
4. `signal(SIGUSR1, sd_in_re)`, `signal(SIGUSR2, sd_in_re)` — SD inserted / removed, sent by `/customer/sd_hotplug.sh`
   (§9.2).
5. `setup_ui(&guider_ui)` (builds every screen, all hidden except the root, §10.2), `events_init` (empty), hide the message
   label.
6. TCP socket → `connect(127.0.0.1:8888)`. On failure: `perror`, `exit(1)` — no retry; prog_uvc must already listen
   (demo.sh starts kjd_lvgl 3 s after prog_uvc, `rootfs-customer.md` §3.4).
7. Threads: `background_task` (SAR buttons), `mythread_work` (IR remote), `Read_socked_task` (replies from prog_uvc).
8. Sets the system clock to **2025-06-29 12:12:12** on every start (`localtime` → fixed fields → `mktime` → `stime`); on
   failure returns 1.
9. `readStudentFromFile` (`/customer/ws.bin` → `iqvalues`; exits if the file is missing), `readSettingsAndLines(ws.ini,
   Line.ini)` (§3).
10. GPIO46: `echo 46 > export`, `direction out`, `value 1` through `system()` (§9.1).
11. `findFile()` (SD state from `/tmp/sdcard_info`, §9.2), `set_default_values` (menu value labels), create the 16 overlay
    lines, `draw_lines` (reticle image).
12. Main loop: `lv_timer_handler(); usleep(5000);` forever.

There is no shutdown path: no SIGTERM / SIGINT handler (the strings "Unable to catch SIGINT/SIGTERM" belong to the
SIGUSR1 / SIGUSR2 `signal()` calls) and no clean-up.

### 2.2 Threads [R]

All four threads call LVGL directly without a lock, although LVGL is not thread-safe; the SD signal handler does too (§9.2).

| thread | entry | blocks on | role |
|---|---|---|---|
| main | 0x114a0 | `usleep(5000)` | `lv_timer_handler` loop; LVGL timers (2 s message hide, 1 s record clock, 1 s playback clock, 2 s one-push timer) |
| background_task | 0x204e0 | `ioctl(/dev/sar, 0x6101)` poll, `usleep(100000)` | the six front-panel buttons on the SAR ADC (§6.1) |
| mythread_work | 0x21600 | `read(/dev/input/event0)` blocking, then `usleep(50000)` | IR remote (`EV_KEY` and `EV_MSC` scancodes, §6.2) |
| Read_socked_task | 0x16b88 | `read(sock, 1023)` | replies 0xDD / 0x1F / 0x1C from prog_uvc (§4.3) |

## 3. Files and persisted state

### 3.1 Files [R]

| path | access | format / when |
|---|---|---|
| `/customer/ws.bin` | read at start (`fread` 88 B into `iqvalues`); written after almost every image-setting change: `open(O_WRONLY\|O_CREAT\|O_TRUNC, 0600)`, `write` 88, `fsync` | 88 bytes = 22 × int32 LE, shared with prog_uvc (§3.2) |
| `/customer/default_ws.bin` | read on reply 0x1F "2" (factory reset) | same 88-byte format; written only by prog_uvc |
| `/customer/ws.ini` | read at start; updated in place by `saveValueToIni` (`fopen` r+, find the line `key=`, seek to its start, `fprintf("%s=%d\n")`, or append if absent), then `system("sync")` | text (§3.3) |
| `/customer/Line.ini` | read at start; rewritten whole on every line edit (`[LineVector]\n` + 16 × `LINE_NODE%d=%d,%d,%d,%d\n`), then `sync` | §3.3 |
| `/customer/1600x900.png` | `lv_img_set_src` with the bare path; opened by `lv_lib_png` with `fopen` / `lodepng_load_file`, no LVGL fs driver (§1.3) | reticle overlay, 1600×900 RGBA (`assets/1600x900.png`) |
| `/tmp/sdcard_info` | read at start and on SIGUSR1, then **truncated** (`fopen "w"`) | first line containing `mount`, 2nd token, `/dev/` prefix stripped = the block device (`mmcblk0p1`) |
| `/mnt/log/version.txt` | if present on the SD card: copied to `/customer/version.txt` (the old one removed) | — |
| `/mnt/sigmastar1024_600.jpg` | if present on the SD card: copied to `/misc/sigmastar1024_600.jpg` | the boot logo `sigmastar1024_600.jpg` [I; the file's role: `firmware-image.md` §1.4] |
| `/mnt/PHOTO`, `/mnt/VIDEO` | `opendir` / `readdir`; `mkdir -p` through `system` | photo / video file lists (at most 100 each) |
| `/mnt` | `statfs` | free-space check (< 10 MiB free = "SD full"), SD info page |

### 3.2 `ws.bin` / `iqvalues` — client side [R]

The layout, prog_uvc's apply rules, the creation of `ws.bin` and `default_ws.bin` at prog_uvc's first start and the values in
the dump are `prog_uvc.md` §6.4. kjd_lvgl keeps the file in the global `iqvalues`, holds every value in the **UI scale**
(prog_uvc converts), writes the whole 88 bytes back after a change, and touches only the fields below; the other words
(0x2c unused; 0x30, 0x34 the raw sharpness bytes; 0x38–0x44 the manual exposure SensorGain, FNx10, ISPGain, shutter µs) are
written by prog_uvc only (it creates the file in `exposure_file_create` (0x25a98) and applies it in `exposure_file_apply` (0x251f0)) and pass through kjd_lvgl
unchanged. The names are the UI's [I].

| off | UI name | UI values | changed by (§4.2 id) |
|---|---|---|---|
| 0x00 | Bright | 1..50 | 0x01 |
| 0x04 | Contrast | 1..50 | 0x02 |
| 0x08 | Hue (saturation) | 1..50 | 0x03 |
| 0x0c | Edge (sharpness) | 1..50 | 0x04 |
| 0x10 | Color | 1 B&W, 2 colour | 0x05 |
| 0x14 | Frequence (flicker) | 1 50 Hz, 2 60 Hz | 0x06 |
| 0x18 | Mirror | 1 normal, 2 horizontal, 3 vertical, 4 invert | 0x07 |
| 0x1c | Display (aspect) | 1 16:9, 2 16:10, 3 4:3 | 0x08 |
| 0x20 | Exposure (AE) | 1 off (manual), 2 on (auto) | 0x09, 0x0A |
| 0x24 | EV | float −3.0..+3.0 | 0x0B |
| 0x28 | AWB | 1 off (manual), 2 on (auto) | 0x0C, 0x0D |
| 0x48 | Red gain | 0..255 | 0x0E; reply 0xDD |
| 0x4c | Blue gain | 0..255 | 0x11; reply 0xDD |
| 0x50 | Green gain (AWB Gr) | 0..255 | 0x0F; reply 0xDD |
| 0x54 | zoom | 0..40, label "X1.0".."X5.0" | 0x12 |

The manual exposure values that prog_uvc sends back after one-push AE (reply 0xAA) are not stored: kjd_lvgl has no case for
that reply (§4.3).

### 3.3 `ws.ini` and `Line.ini` [R]

**ws.ini** keys read at start: `poho_size` (photo size 1..3), `video_size` (record size 1 4K, 2 1080P), `reticle_flag`
(1 off, 2 on), `linenum`, `linedirty`, `linepose`, `lincolor`, `linewidget`, `lanugnuge` (UI language 1..9, §5.6). Parser:
`fgets` + `strtok("=")`; it stops at the first line without `=`. Keys written but **never read back**: `linenumber`,
`line_position`, `line_thickness`, `line_color` (the reader looks for `linenum` etc.). Because `saveValueToIni` overwrites
in place without shifting the rest, a longer value eats the first characters of the next line, and unknown keys are
appended — the source of the stray `LINE_NODE…` / `line_position=` lines in the dumped ws.ini [I, consistent with the
dump].

**Line.ini:** `[LineVector]`, then `LINE_NODE<n>=<state>,<pos>,<color>,<width>` for n = 1..16 (quotes around the value are
tolerated on read). state 1 off, 2 horizontal, 3 vertical; pos in px; color 1 red `#FF0000`, 2 green `#00FF00`, 3 blue
`#0000FF`, 4 black, 5 white; width 1..4. The built-in defaults written on factory reset are `1,20,1,3` … `1,470,1,3`
(§4.3).

## 4. The menu link to prog_uvc — client side (TCP 127.0.0.1:8888)

### 4.1 Transport and packet [R]

kjd_lvgl is the client: one TCP connection for its whole life, opened once at start (§2.1 step 6), never reopened. If
prog_uvc dies, every `send` fails with a `perror` and the UI keeps running. The server, its per-client thread and what a
client disconnect does to prog_uvc (accept loop `menu_server_accept` (0x274fc) → handler thread `menu_client_thread` (0x25f08)) — `prog_uvc.md` §6.1.

Packet, both directions, fixed **65 bytes**: byte 0 = id, bytes 1..64 = NUL-terminated ASCII payload, the rest zero
(`prog_uvc.md` §6.2). kjd_lvgl builds the payload with `sprintf("%d")` (`sendData` 0x13ed8: `int sendData(int sock, uint8_t
id, int value)`, `perror` on a short send), with `sprintf("%0.1f")` (EV, two inlined copies in `expose_ok` 0x17b64 and
`expose_AE` 0x18f60) or with `snprintf(…, 0x40, "%s", fileName)` (paths, inlined at four places) — at most 63 characters +
NUL. `sockfded` is the socket. Neither side frames the TCP stream: kjd_lvgl reads up to 1023 bytes and parses only the first
65, so two replies arriving back-to-back lose the second [I]. No acknowledgements, retries or timeouts; the only replies are
the four of §4.3.

### 4.2 Ids sent [R]

Twenty-nine ids; every one has a case in prog_uvc's handler `menu_client_thread`. Not sent: 0x10 (no case in prog_uvc either), 0x1C
(reply only), 0x20 (`MI_ISP_IQ_SetWDR` in prog_uvc, no UI). The "prog_uvc" column names the action; its calls and arguments are
`prog_uvc.md` §6.2–§6.3 and §7.

| id | payload | sent when | prog_uvc |
|---|---|---|---|
| 0x01 | "1".."50" | Menu › Bright: OK +1, AE −1 (wrap 50↔1), OK held = repeat | `MI_ISP_IQ_SetBrightness`, value ×2 |
| 0x02 | "1".."50" | Menu › Contrast (same keys) | `MI_ISP_IQ_SetContrast`, ×2 |
| 0x03 | "1".."50" | Menu › Hue (the label says "Hue", the value is saturation) | `MI_ISP_IQ_SetSaturation`, ×2.5 |
| 0x04 | "1".."50" | Menu › Edge (sharpness) | `MI_ISP_IQ_SetSharpness` |
| 0x05 | "1" B&W / "2" colour | Menu › Color OK / AE toggle; IR scancode 0x0C anywhere | `MI_ISP_IQ_SetColorToGray(v == 1)` |
| 0x06 | "1" 50 Hz / "2" 60 Hz | Menu › Frequence OK / AE toggle | `MI_ISP_AE_SetFlicker` (1 → 50 Hz, else 60 Hz) |
| 0x07 | "1".."4" | Menu › Mirror OK (1→2→3→4→1), AE (reverse); IR 0x07 (1/3↔2), IR 0x16 (1/2→3, 3→1) | `mirror_display(v)` 0x214f8 |
| 0x08 | "1".."3" | Menu › Display › OK on 16:9 / 16:10 / 4:3 | `tailoring_init(v)` 0x21b60 (HDMI crop) |
| 0x09 | "1" AE off / "2" AE on | Exposure › Exposure OK; IR 0x09 on the live screen | `MI_ISP_AE_SetExpoMode` 4 (manual) / 0 (auto) |
| 0x0A | "1", then "2" after 2 s | Exposure › One Push AE OK: sends "1", sets AE = on; an LVGL one-shot timer 2 s later sends "2" and sets AE = off | "1": AE auto with the default target; else freeze the current exposure as manual, **reply 0xAA** |
| 0x0B | "-3.0".."3.0" (`%0.1f`) | Exposure › EV: OK +0.3, AE −0.3, wraps at ±3.0, near 0 snaps to 0.0; row listed only when AE is on | AE target from EV, AE auto; 0.0 only sets auto |
| 0x0C | "1" off / "2" on | AWB › AWB OK; IR 0x0D on the live screen | `MI_ISP_AWB_SetAttr` op = manual / auto |
| 0x0D | "1" or "3", then "2" or "4" after 2 s | AWB › One Push AWB OK: first value 1 if AE on, 3 if AE off; the 2 s timer sends 2 (AE on) / 4 (AE off) and sets AWB = off | 1: AWB auto; 3: remember exposure, AE + AWB auto; 2 / 4: lock the current WB gains as manual (4 also restores the manual exposure), **reply 0xDD** |
| 0x0E | "0".."255" | AWB › Red: OK +1 / AE −1 (wrap), only when AWB is off | AWB manual R |
| 0x0F | "0".."255" | AWB › Green | AWB manual Gr |
| 0x11 | "0".."255" | AWB › Blue (AE below 0 wraps to **25**, not 255) | AWB manual B |
| 0x12 | "0".."40" | live screen: UP / IR `KEY_POWER` +1, DOWN / IR `KEY_0` −1; held ≥ 1 s → repeat every 150 ms | `zoom_init(v)` 0x21e48 |
| 0x13 | "<n>" | photo, size 3 (1M 1280×720) | `photo_capture_1m` (0x15f10) → `UVC_StartCapture_photo(0)`, saved to `/mnt/PHOTO/P%05d.jpg`, number n + 1 |
| 0x14 | "<n>" | photo, size 2 (2M 1920×1080) | `photo_capture_2m` (0x15f2c) → mode 1, same naming |
| 0x15 | "<n>" | photo, size 1 (4K 3840×2160) | `photo_capture_4k` (0x15f48) → mode 2, same naming |
| 0x16 | "<n>" | start recording, record size 1080P (`video_size` 2) | `KJD_Video_Record` to `/mnt/VIDEO/V%05d.mp4` (n + 1), 1920×1080 |
| 0x17 | "<n>" | start recording, record size 4K (`video_size` 1) | `KJD_Video_Record`, 3840×2160 |
| 0x18 | "2" (4K) / "1" (1080P) | stop recording (OK again, MENU while recording, SD full while recording) | `KJD_Close_Record` |
| 0x19 | "2" → "3" → "1" | live screen ON short press (or IR 0x18): 1st freeze (snowflake icon), 2nd compare view (screen icon), 3rd back to live | 2 freeze (`MI_ISP_StopChannel`); 3 compare view (`kjd_start_vdisp`, two-port SCL split, `MI_DISP_PauseInputPort` on port 0); 1 back to live (`MI_DISP_ClearInputPortBuffer` / `MI_DISP_ResumeInputPort`, `kjd_stop_vdisp`) |
| 0x1A | "/mnt/PHOTO/<file>" | photo viewer: open (Playback › PHOTO), UP / DOWN previous / next | `kjd_photo_init` on first use, then JPEG decode to the display (`photo_view_decode` (0x15f64)) |
| 0x1B | "/mnt/VIDEO/<file>" | video list › OK on a file | MP4 player start `mp4_player_start` (0x15578), **reply 0x1C** |
| 0x1D | "1" exit photo viewer, "2" pause, "3" resume, "4" stop video | MENU in the viewer → 1; OK during playback toggles 2 / 3; end of clip, MENU or SD removed → 4 | 1 `kjd_photo_deinit`; 2 / 3 pause flag; 4 player stop `mp4_player_stop` (0x155e4) |
| 0x1E | "0" / "1" / "2" enter standby (by freeze state 1 / 2 / 3), "3" leave | ON held ≥ 3 s → standby; ON short while in standby → wake | `standby_set` (0x25e50): HDMI (and UVC) off after undoing freeze / compare; 3 on again |
| 0x1F | "1" | System › Factory Reset › YES | apply the default settings, mirror, zoom; **reply 0x1F "2"** |

Timing: messages are sent synchronously from the key thread at the moment of the key action. One-push AE / AWB send their
second message from a 2000 ms LVGL one-shot timer (`my_single_execution_callback`), which sends nothing if the user has left
the Exposure / AWB page by then. Zoom and values repeat every 150 ms while a button is held [R; R emulator].

### 4.3 Replies handled (`Read_socked_task`) [R]

| id | payload | trigger | kjd_lvgl reaction |
|---|---|---|---|
| 0xAA | `"%d-%d-%d-%d"` exposure values | 0x0A with a value ≠ 1 | **none** — no case; the manual exposure never reaches `ws.bin` through kjd_lvgl |
| 0xDD | `"R-G-B"` (gains >> 5) | 0x0D 2 / 4 | store R → 0x48, G → 0x50, B → 0x4c; update the three labels; write `ws.bin` |
| 0x1C | `"%d"` duration, s | 0x1B, only if the duration > 0 | `Total_time`; playback stops when the 1 s counter exceeds it |
| 0x1F | `"2"` | 0x1F | load `default_ws.bin` into `iqvalues`, photo size 1, write `ws.bin`, rewrite `Line.ini` from the built-in defaults (`1,20,1,3` … `1,470,1,3`), reticle 1, linenumber 1, linedirty 1, linepose 20, linewidget 3, lincolor 1 → ws.ini, reload, redraw the lines |

## 5. User interface

### 5.1 Layout and look [R; R emulator]

Everything is one LVGL screen `ui[0]` (300×850, transparent) over the video; panels are containers that are shown and
hidden. `lv_ui guider_ui` is 816 B: word 0 is the screen, the word at offset 4 is GUI Guider's `screen_del` flag (written by
`setup_ui`, §10.2), so there are 203 object slots. Coordinates are in the
1600×900 OSD space. Menu rows: buttons 300×40, background `#AFCEED`, 1 px white border, radius 0, item text left at x = 10,
value text at x ≈ 193 (98 px wide); text black, the **selected row's text green `#43CF7C`**, disabled rows grey `#D3D7CF`.
Font: `lv_font_SourceHanSerifSC_Regular_16`; Arabic uses `lv_font_dejavu_16_persian_hebrew` with LVGL's Arabic shaping
(`alabo()` switches the fonts, `qita()` switches back) (§7.1).

Images, compiled in as ARGB8888 (`assets/`): `_log_300x137` (the "4K ULTRA HD" logo under the main-menu rows, `ui[35]`),
`photo_60x60` (photo-mode icon), `stop_60x60` (pause icon), `scream_85x62` (compare icon), `freeze_85x62` (freeze icon); the
reticle is the file `/customer/1600x900.png` (§3.1).

Live-screen overlays:

| object | slot | position | content |
|---|---|---|---|
| zoom / mode label | `ui[97]`, `ui[98]` | top-left 10,20 | "X1.1", or "4K" / "1080P" when switching to video mode; hidden after 2 s |
| photo-mode icon | `ui[93]` | 10,10 | `photo_60x60` |
| record clock | `ui[201]` | 1230,50 | red `HH:MM:SS` |
| message label | `ui[202]` | 1230,50, 171×51 | red, right-aligned, auto-hidden after 2 s: SD messages, the "1/5" photo counter, the playback clock. Too small for long SD messages, which are cut ("The SD card has been") [R emulator] |
| pause icon | `ui[94]` | — | `stop_60x60` |
| freeze icon | `ui[95]` | 10,800 | `freeze_85x62` |
| compare icon | `ui[96]` | 10,800 | `scream_85x62` |
| video file list | `ui[203]` | full screen | §5.5 |
| reticle | — | full screen | `/customer/1600x900.png` |
| user lines | — | — | 16 LVGL `lv_line` objects with numbered labels: a horizontal line's label at x = 1570, y = pos − 15; a vertical line's at x = pos + 5, y = 875 |

`ui[70]` (a main-menu row button) carries `lv_font_montserratMedium_16`; its only child `ui[71]` is an always-empty label.
`ui[90]` is the Arabic row of the Language list ("اللغة العربية", dejavu font). `ui[182]` holds the Format question; its
boot-time text "是否格式化数据？" is replaced by the per-language question before it is shown (§7.3).

### 5.2 State machine (`screen_flag`) [R; R emulator]

| screen_flag | screen | entered from |
|---|---|---|
| 0 | live (photo mode) | start; MENU from the main menu |
| 0x15 | live (video mode) | AE in the playback chooser |
| 1 | main menu (`text_flag` = cursor) | MENU from live or from any submenu |
| 2 | Language | menu row 0 |
| 3 | Line Set | row 1 |
| 4 | Exposure | row 2 |
| 5 | AWB | row 3 |
| 0x58 | Record Size | row 6 |
| 6 | Resolution (photo size) | row 7 |
| 7 | Display (aspect) | row 9 |
| 8 | Format SD confirm | row 14 |
| 9 | Date & Time | row 15 |
| 10 | System | row 16 |
| 12 / 13 / 14 | Factory-reset confirm / System Info / SD Info | System items 1 / 2 / 3 |
| 0x14 | playback chooser (Playback mode / PHOTO / VIDEO) | AE on the live screen (needs SD) |
| 0x32 | photo viewer | chooser › PHOTO |
| 0x33 | video file list | chooser › VIDEO |
| 0x35 | video playing | list › OK |

### 5.3 Main menu [R; R emulator]

Rows top to bottom; `text_flag` takes the values 0..5, 0x58, 6..15 in that order. "Row n" in §5.2 counts the rows from the
top (0..16).

| text_flag | label EN (ZH / RU) | value shown | OK | AE | stored |
|---|---|---|---|---|---|
| 0 | Language (语言(Language) / Language) | — | open Language | — | — |
| 1 | Line Set (线设置 / настр. лин) | — | open Line Set | — | — |
| 2 | Exposure (曝光 / «настр.лин» — the vendor's Russian string for Exposure is a copy of Line Set's) | — | open Exposure | — | — |
| 3 | AWB (白平衡 / ББ) | — | open AWB | — | — |
| 4 | Color (色彩 / Цветн) | Color / B&W | toggle → 0x05 | toggle | ws.bin 0x10 |
| 5 | Frequence (光频 / Част. св) | 50HZ / 60HZ | toggle → 0x06 | toggle | ws.bin 0x14 |
| 0x58 | Record Size (录像尺寸 / P.ВИД) | 4K / 1080P | open Record Size | — | ws.ini `video_size` |
| 6 | Resolution (图像尺寸 / P.ИЗО) | 4K / 2M / 1M | open Resolution | — | ws.ini `poho_size` |
| 7 | Mirror (镜像 / Зерк) | Normal / Horizon / Vertical / Invert | next → 0x07 | previous → 0x07 | ws.bin 0x18 |
| 8 | Display (显示器 / ЭКРАН) | 16:9 / 16:10 / 4:3 | open Display | — | ws.bin 0x1c |
| 9 | Bright (亮度 / Яркость) | 1..50 | +1 → 0x01 | −1 | ws.bin 0x00 |
| 10 | Contrast (对比度 / Контраст) | 1..50 | +1 → 0x02 | −1 | ws.bin 0x04 |
| 11 | Hue (饱和度 = saturation / Насыщ) | 1..50 | +1 → 0x03 | −1 | ws.bin 0x08 |
| 12 | Edge (清晰度 = sharpness / Чёткость) | 1..50 | +1 → 0x04 | −1 | ws.bin 0x0c |
| 13 | Format (格式化 / Формат. SD) | — | open confirm | — | — |
| 14 | Date & Time (日期设置 / Дата) | — | open Date | — | — |
| 15 | System (系统 / Сист.инф) | — | open System | — | — |

### 5.4 Submenus [R; R emulator]

Cursor UP / DOWN wraps; MENU returns to the main menu; values persist at once.

- **Language** (9 rows): OK sets `lanugnuge`, relabels everything, writes ws.ini `lanugnuge`, returns to the main menu.
- **Line Set** (6 rows): Cross Hair ON / OFF (the reticle image) · Line 1..16 (which user line is edited) · Direction OFF /
  Horizon / Vertical · Position (OK +1, AE −1, wraps at 1080 for horizontal and 1920 for vertical lines — the OSD is
  1600×900, so larger positions are off screen) · Line Color Red / Green / Blue / Black / White · Thickness 1..4. Every change
  rewrites Line.ini.
- **Exposure**: Exposure ON / OFF (AE, 0x09) · One Push AE (0x0A "1", 2 s later "2") · EV (−3.0..+3.0 in steps of 0.3,
  0x0B; the row exists only when AE is on).
- **AWB**: AWB ON / OFF (0x0C) · One Push AWB (0x0D) · Red / Green / Blue 0..255 (0x0E / 0x0F / 0x11; usable only when AWB is
  off, grey otherwise).
- **Record Size**: 4K · 1080P (OK sets `video_size`; `*` marks the current one).
- **Resolution**: 4K 3840x2160 · 2M 1920x1080 · 1M 1280x720 (`poho_size` 1 / 2 / 3; `*` marks the current one).
- **Display**: 16:9 · 16:10 · 4:3 (0x08).
- **Format**: "Do you want to format data?" YES / NO. YES with a card: the question label becomes "Format the SD card
  in...... later" (per language), then `sleep 1; umount /mnt; usleep 0.5 s; mkfs.vfat /dev/<dev>; mount -t vfat /dev/<dev>
  /mnt`, run from the key thread (the main thread keeps rendering meanwhile [I]); without a card: "SD card is not inserted".
  Back to the main menu.
- **Date & Time**: `YYYY / MM / DD  hh : mm`; OK / AE move the field cursor (5 fields, red = active), UP / DOWN change the
  field and **at once** run `date YYYY.MM.DD-hh:mm:00` + `hwclock -w`. The edited values are separate globals that start at 0,
  not from the clock: the first UP on the year gives year 1, DOWN gives 2037; ranges year 2000..2037, day by month with leap
  years. The clock is set to 2025-06-29 at every start anyway (§2.1).
- **System**: Factory Reset (confirm YES → 0x1F "1") · System Info (shows the hard-coded string `W11861322605221`, not
  `version.txt`) · S_Disk Info (SD total / used / free GB from `statfs /mnt`, needs a card).

### 5.5 Live screen and playback [R; R emulator]

- OK: photo mode → take a photo (needs a card and ≥ 10 MiB free) → 0x13 / 0x14 / 0x15 with the highest existing number;
  video mode → start / stop recording (0x16 / 0x17, 0x18); the record clock counts while recording, and an SD-full check every
  second stops the recording.
- AE (with a card): live → playback chooser; in the chooser AE → video mode (shows "4K" / "1080P" for 2 s); in video mode
  AE → photo mode (photo icon). Without a card: the "SD card is not inserted" message.
- Playback chooser: UP / DOWN PHOTO / VIDEO, OK opens. PHOTO: viewer with the label "n/total", UP / DOWN previous / next,
  MENU exits (0x1D "1"). VIDEO: the file list (white full-screen `lv_list`, 20 visible rows, selected row `#3498DB`), OK plays
  (0x1B), the clock in the message label, OK pause / resume (0x1D "2" / "3", pause icon), end of clip → 0x1D "4" and back to
  the list, MENU back.
- Clocks: the record clock is the LVGL 1 s timer `video_timer_label` (0x1206c), created in `video` (0x1cf80) with
  `lv_timer_create(video_timer_label, 1000, NULL)` → `video_timer_2`; on every tick it hides `ui[97]`, and on SD full it
  pauses itself (`lv_timer_pause`). The playback clock is the LVGL 1 s timer `video_timer_cb` (0x11ef4, held in
  `video_timer`), created in `video_back_ok` (0x1da60) and resumed per clip; at the end of a clip (`video_back_time` >
  `Total_time`) it sends 0x1D "4", shows the list `ui[203]` again, sets `screen_flag` = 0x33, `videostate` = 0,
  `isvideoRecd` = 0, pauses `video_timer` and itself, hides `ui[202]` and `ui[94]`, and leaves the clock text as it is.
- File numbering: `getMaxNumberFromFiles` takes the **three digits before the dot** of the `*.jpg` / `*.mp4` names; prog_uvc
  saves n + 1 with `%05d`. After 999 files the counter stalls and files are overwritten [I]. The lists hold at most 100
  entries.

### 5.6 Languages [R; R emulator]

`lanugnuge`: 1 中文, 2 English, 3 中文繁体, 4 日本語, 5 Français, 6 Русский, 7 Español, 8 العربية, 9 Deutsch. The SD-full message exists in all nine languages
(`"\nThe SD card is full!!!"`, `"\nSD卡已满!!!"`, `"\nSD ПОЛН!!!"`, `"\nSD PLEINE!!!"`, `"\nSD LLENA!!!"`, `"\nSD Voll!!!"`, …),
set on `ui[202]`.

## 6. Input

### 6.1 Front buttons — SAR ADC (`background_task`) [R]
<!-- ev:kjd_lvgl.6.1 -->

`/dev/sar`: `ioctl(fd, 0x6100)` once (init), then `ioctl(fd, 0x6101, &{u32 ch, u32 value})` every 100 ms (driver:
`kernel-drivers.md` §3.1). One resistor ladder on one channel; the button is decided by value windows — the same windows as in
the factory test `tmp` (`rootfs-customer.md` §8). The physical keys on the camera body: `kernel-drivers.md` §3.1.

| button | ADC value | short press (≥ 1 ms, < limit) | long press |
|---|---|---|---|
| OK | 0..30 | the OK action of the current screen (§5.3–§5.5) | ≥ 2 s: repeat the OK action every 150 ms on Bright..Edge, Mirror, EV, WB gains, Line Set |
| ON | 165..200 | live: freeze cycle (0x19); in standby: wake | ≥ 3 s: standby (0x1E, GPIO46 = 0) or wake |
| UP | 280..315 | menu cursor up / previous value; live: zoom in (0x12) | ≥ 1 s (live): zoom in every 150 ms |
| MENU | 435..470 | open / close the menu, back from submenus | — |
| DOWN | 745..780 | cursor down / next; live: zoom out | ≥ 1 s (live): zoom out every 150 ms |
| AE | 810..850 | "minus" / back action (§5.3); live: playback chooser / mode switch | ≥ 2 s: repeat the AE action every 150 ms |
| (released) | ≥ 1000 | | |

Values the camera produced (kjd_lvgl's `acl_<key> read stCfg.adc_value=:%d` lines in the UART log of 2026-10-14): OK 9, ON
176, UP 291, MENU 442, DOWN 748 — each inside its window [R]; AE does not appear in that log.

Press duration is measured with `clock()` (process CPU time) while the thread spins on the ADC; it works because the loop
burns CPU, but it is not wall time [I]. Buttons are ignored while frozen (except ON) and in standby (except ON); ON and AE are
ignored while recording.

### 6.2 IR remote (`mythread_work`) [R; R emulator]

`/dev/input/event0`, blocking reads of `struct input_event` (16 B). Two kinds of events are used:

- `EV_KEY` (type 1, value 1 or 2 = press / autorepeat): `KEY_UP 0x67`, `KEY_DOWN 0x6C` (menu cursor), `KEY_RIGHT 0x6A` = OK,
  `KEY_LEFT 0x69` = AE, `KEY_POWER 0x74` = zoom in, `KEY_0 0x0B` = zoom out (zoom only on the live screen).
- `EV_MSC` (type 4, `MSC_SCAN`, value = raw NEC scancode): `0x40` MENU · `0x08` photo · `0x1C` start / stop recording ·
  `0x5A` playback chooser · `0x5E` pause / resume playback · `0x18` freeze cycle · `0x19` reticle on / off · `0x0C` colour /
  B&W · `0x07` mirror normal ↔ horizontal · `0x16` mirror → vertical / normal · `0x09` AE toggle (live) / open Exposure
  (menu) · `0x0D` AWB toggle (live) / open AWB (menu). The scancodes `0x15` and `0x43..0x47` do nothing on `EV_MSC` — they are
  the keys taken through `EV_KEY`.

Together that is 18 of the 21 codes of the kernel's vendor NEC keymap. Which key sends which code, the keymap itself and the
three codes no product code uses — `kernel-drivers.md` §3.2.2.

## 7. Fonts

### 7.1 Which fonts, and which draw text [R]

Six fonts generated for the GUI Guider project (not part of LVGL) are linked [R `STT_FILE` + symbols]. All are LVGL
`fmt_txt` fonts, 4 bpp, plain bitmaps (`bitmap_format 0`), class-based kerning, `kern_scale 16`, no subpixel, no fallback,
`user_data 0`.

| font object (file `<name>.c`) | line h / base | glyphs | codepoints | bitmap B | used by kjd_lvgl [R xref] |
|---|---|---|---|---|---|
| `lv_font_SourceHanSerifSC_Regular_16` (0xcadb4) | 18 / 3 | 355 | 354 | 40692 | **the UI font**: `setup_scr_screen`, `language_up` (6×), `qita`, `update_line`, `create_video_list` title |
| `lv_customer_font_SourceHanSerifSC_Regular_16` (0xd639c) | 16 / 2 | 209 | 208 | 20395 | **unused** (no reference) |
| `lv_customer_font_SourceHanSerifSC_Regular_20` (0xdc530) | 20 / 2 | 209 | 208 | 30927 | **unused** |
| `lv_customer_font_mysh_16` (0xe4fe8) | 16 / 3 | 207 | 206 | 20820 | **unused** |
| `lv_font_montserratMedium_16` (0xc43f8) | 16 / 2 | 155 | 154 | 11058 | set on `ui[70]`, whose only child `ui[71]` is an always-empty label → draws nothing |
| `lv_font_montserratMedium_12` (0xc8174) | 12 / 2 | 155 | 154 | 6647 | only in the GUI-Guider style `style_screen_list_1_extra_btns_main_default`, which is initialised but never added to an object → **unused** |

Stock LVGL fonts that draw text: `lv_font_dejavu_16_persian_hebrew` (Arabic labels, set by `alabo` / `language_up`, and
`ui[90]` in the Language list) and `lv_font_montserrat_14` (the theme default): the buttons of the dynamic video-file list,
`lv_list_add_btn(list, LV_SYMBOL_VIDEO "\xEF\x80\x88", name)`, set no font, so the file names and the video symbol are drawn
with it [I: inheritance from the default theme's screen style]. The other stock fonts linked are Montserrat 8, 10, 12, 16,
18, 20, 22 (§1.3); there is no Montserrat 24–48, unscii or simsun_16_cjk.

### 7.2 Structures as linked (LVGL 533e325f2, 32-bit ARM) [R]

- `lv_font_t` (28 B): `get_glyph_dsc` → `lv_font_get_glyph_dsc_fmt_txt`, `get_glyph_bitmap` → `lv_font_get_bitmap_fmt_txt`,
  `int16 line_height`, `int16 base_line`, `u8 subpx:2`, `int8 underline_position`, `int8 underline_thickness`, pad,
  `dsc`, `fallback` (0), `user_data` (0).
- `lv_font_fmt_txt_dsc_t` (24 B, object `font_dsc`): `glyph_bitmap`, `glyph_dsc`, `cmaps`, `kern_dsc`, `u16 kern_scale`,
  `u16 cmap_num:9 | bpp:4 | kern_classes:1 | bitmap_format:2`, `cache` (→ 8-B `cache` object in .bss).
- glyph dsc (8 B): `u32 bitmap_index:20 | adv_w:12` (adv_w in 1/16 px), `u8 box_w, box_h`, `i8 ofs_x, ofs_y`; glyph 0 is the
  empty "no glyph" entry. Bitmaps: 4 bpp, one continuous MSB-first bit stream per glyph starting at byte `bitmap_index`,
  rows not padded.
- cmap (20 B): `u32 range_start`, `u16 range_length`, `u16 glyph_id_start`, `unicode_list`, `glyph_id_ofs_list`,
  `u16 list_length`, `u8 type` (0 FORMAT0_FULL, 2 FORMAT0_TINY, 3 SPARSE_TINY; type 1 not used by these fonts).
- kerning (16 B `kern_classes`): `class_pair_values` (int8, left_cnt × right_cnt), `left_class_mapping`,
  `right_class_mapping` (u8 per glyph), `u8 left_class_cnt`, `u8 right_class_cnt`.

| font | cmaps (start, length, type) | kern classes L×R |
|---|---|---|
| `lv_font_SourceHanSerifSC_Regular_16` | (0x20, 64, FULL) (0x61, 26, TINY) (0xBF, 62, SPARSE_TINY) (0x410, 62, FULL) (0x44F, 39555, SPARSE_TINY) | in the font's `.json` (§7.4) |
| `lv_customer_font_*` (all three) | (0x20, 64, FULL) (0x61, 26, TINY) (0x4E00, 20690, SPARSE_TINY) | in the `.json` |
| `lv_font_montserratMedium_12/16` | (0x20, 12, TINY) (0x2D, 82, TINY) (0xF001, 2210, SPARSE_TINY) | in the `.json` |

### 7.3 Coverage, and what the camera draws [R]

Coverage (own glyphs; full lists in `<name>.txt`, §7.4):
- **UI font** `lv_font_SourceHanSerifSC_Regular_16`: ASCII ` &()*-./0-9:?A-Z_a-z` (73), Latin-1 `¿ÉÍßäçèéíñü`, Cyrillic
  (50) `АБВГДЕЖЗИКЛМНОПРСТФЦЧЫЭЯабвдезийклмнопрстуфцщыьэяё`, Hiragana 30, Katakana 27, CJK ideographs 163 — a subset cut to
  the UI strings of languages 1–7 and 9 (Arabic is drawn with dejavu).
- `lv_customer_font_SourceHanSerifSC_Regular_16/_20`: ASCII ` &()*+-./0-9:?A-Z_a-z` (74) + 134 CJK; `mysh_16`: the same
  without `+-` (72) + 134 CJK.
- `montserratMedium_12/16`: ASCII 0x20–0x7E without `,` (94) + 60 LVGL symbols (U+F001…).

Two LVGL rules draw some codepoints with **another character's glyph** (computed on the binary's bytes by a re-implementation
of this LVGL's `get_glyph_dsc_id`; read in the binary at `lv_font_get_glyph_dsc_fmt_txt` 0x63cc4: `if (rcp <= range_length)`):
1. FORMAT0_FULL holes: a codepoint inside the range without its own glyph has offset 0 and is drawn with the range's first
   glyph. In the UI font that is the **space** for `! " # $ % ' + , ; < = > @ [ \ ] ^` and **`А`** for
   `Й У Х Ш Щ Ъ Ь Ю г ж х ч ш ъ`.
2. Off-by-one in LVGL 8 (`if(rcp > range_length) continue;` — still so in v8.3.11 / v8.4.0 [R source]): the codepoint
   `range_start + range_length` is accepted. FORMAT0_FULL reads one byte past the offset list (in this binary 0 → the first
   glyph: `` ` `` → space, `ю` → `А`); FORMAT0_TINY returns the next range's first glyph (`{` → `¿` in the UI font, `{` → `一`
   in the customer fonts, `,` → `-` and U+007F → U+F001 in montserratMedium).

So whether a character has its own glyph is decided by the coverage list, not by the cmap ranges. Every character of the
label texts of languages 1–7 and 9 has its own glyph, except two boot-time placeholders replaced
before they are shown: `ui[90]` uses dejavu, and `ui[182]` "是否格式化数据？" is overwritten with the per-language Format
question, which uses ASCII `?` [R emulator dump; that the boot text never reaches the screen is I]. The one visible defect:
the SD-full message (§5.6) shows its `!!!` as **three spaces**, because `!` is a hole of the UI font's FORMAT0_FULL range.

### 7.4 Recovered font files [R]

Every font of the image can be recovered from the binary in the `lv_font_conv` C layout (same array and symbol names as the
vendor file) and as an LVGL binary font for `lv_font_load()`; the recovered fonts compile against a reconstructed `lv_conf.h`
and decode correctly in a host build of LVGL 533e325f2. The original TrueType sources are not recoverable from the binary.
The fonts are the vendor's assets and are not published here.

## 8. Display [R]

`/dev/fb0` through lv_drivers fbdev: 1600×900, 32 bpp ARGB8888, `screen_transp = 1`, so the unused OSD has alpha 0 and the
video shows through. `fbdev_flush` copies rows from the single draw buffer (§2.1) with `memcpy` into the mmap (32 bpp path); no
double buffering, no pan. LVGL redraws on its 16 ms refresh timer from the main loop (§1.3, §2.1). The fb is the SigmaStar GOP
layer that `fbdev.ko` sets up from `/config/fbdev.ini` (FB_WIDTH 1600, FB_HEIGHT 900, FB_TIMMING_WIDTH 1920,
FB_TIMMING_HEIGHT 1080, FB_HWWIN_FORMAT 5 = ARGB8888, layer 1) and the display path scales to the HDMI output — `rgn-gop.md`
§5.2, §5.10.

## 9. Effects on the rest of the system

### 9.1 Direct side effects [R]

kjd_lvgl makes **no MI_* calls and no ioctls other than on fb0 and `/dev/sar`**; every effect on video, ISP, recording or
playback is a message to prog_uvc (§4). No process start or kill, no `/proc` writes, no firmware-upgrade flow (the SD upgrade
is U-Boot's `SdUpgradeImage` with `/customer/sd_version_recognition`, run by `sd_hotplug.sh` — `rootfs-customer.md` §4.1,
§4.3). The other effects are described where they happen: the system clock (§2.1, §5.4 Date & Time), SD formatting (§5.4
Format), the file copies and the `mkdir -p` / `sync` calls (§3.1).

| what | how | when |
|---|---|---|
| GPIO46 | `system("echo … > /sys/class/gpio/gpio46/…")`: export + out + 1 at start; 0 on standby; 1 on wake | the front status LED colour: 1 orange (UI running), 0 red (standby) [R factory-test strings; I two-colour LED] — `kernel-platform.md` §6.3 |
| SD → USB mass storage | first SD insert after start (SIGUSR1): `source /customer/usb_otg.sh rmmod_device` (rmmod g_webcam — the **UVC camera stops**), `usleep 0.3 s`, `insmod usb_f_mass_storage.ko`, `insmod g_mass_storage.ko file=/dev/<dev> removable=1`; at start with a card already present (`findFile`): only the two insmods | the SD card becomes a USB disk in place of the webcam until reboot; never reverted on removal |

### 9.2 SD card signals [R]

`/customer/sd_hotplug.sh` (mdev hotplug) mounts the card on `/mnt`, appends `mount <dev> on /mnt` to `/tmp/sdcard_info` and
sends SIGUSR1 to kjd_lvgl; on removal it sends SIGUSR2, then `umount` (the script and its order: `rootfs-customer.md` §4.1).
The handler `sd_in_re` runs LVGL and `system()` calls inside the signal handler. The script sends SIGUSR1 before it writes
`/tmp/sdcard_info`, so the handler can read an empty file and build `file=/dev/(null)` for the mass-storage gadget [R emulator
with that order].

## 10. GUI Guider support files and unused vendor code

### 10.1 What is never reached [R]

Unreferenced, checked on the raw code (every Thumb `bl/blx/b` target and `movw/movt` address pair over all of .text, and every
32-bit word of the loaded sections): the four `main.c` functions of §10.4, the two keyboard callbacks of
§10.3, the functions marked "no" in §10.2, and all LVGL demos and examples. The live counterparts of the §10.4 functions are referenced: `video_timer_label` from
`video`, `video_timer_cb` from `video_back_ok` (§5.5), `sendData` from `background_task` and `mythread_work`.

### 10.2 `gui_guider.c`, `events_init.c`, `custom.c`, `setup_scr_screen.c` [R]

| function | addr | used | behaviour |
|---|---|---|---|
| `void ui_init_style(lv_style_t *s)` | 0x22f78 | yes (4× in `setup_scr_screen`, for the four static list styles) | `if (s->prop_cnt > 1) lv_style_reset(s); else lv_style_init(s);` (`prop_cnt` = byte 7) |
| `void setup_ui(lv_ui *ui)` | 0x231c8 | yes (`main`) | `ui->screen_del = true` (byte at offset 4); `setup_scr_screen(ui)`; `lv_scr_load(ui->screen)` |
| `void init_scr_del_flag(lv_ui *ui)` | 0x231c0 | no | `ui->screen_del = true` |
| `void ui_load_scr_animation(lv_ui *ui, lv_obj_t **new_scr, bool new_scr_del, bool *old_scr_del, void (*setup_scr)(lv_ui *), lv_scr_load_anim_t anim, uint32_t time, uint32_t delay, bool is_clean, bool auto_del)` | 0x22f88 | no | `act = lv_scr_act()`; `if (auto_del && is_clean) lv_obj_clean(act)`; `if (new_scr_del) setup_scr(ui)`; `lv_scr_load_anim(*new_scr, anim, time, delay, auto_del)`; `*old_scr_del = auto_del` |
| `void ui_move_animation(void *obj, int32_t time, int32_t delay, int32_t x_end, int32_t y_end, lv_anim_path_cb_t path, uint16_t repeat_cnt, uint32_t repeat_delay, uint32_t playback_time, uint32_t playback_delay, lv_anim_start_cb_t start_cb, lv_anim_ready_cb_t ready_cb, lv_anim_deleted_cb_t deleted_cb)` | 0x22fd0 | no | one `lv_anim_t` on the stack: exec `lv_obj_set_x` from `lv_obj_get_x(obj)` to `x_end`, started; then the same struct re-used with exec `lv_obj_set_y`, `lv_obj_get_y` → `y_end`; the three callbacks are set only for the second (y) animation |
| `void ui_scale_animation(…same 13 parameters, width/height…)` | 0x23058 | no | as above with `lv_obj_set_width`/`lv_obj_get_width`, then `lv_obj_set_height`/`lv_obj_get_height` |
| `void ui_img_zoom_animation(void *img, int32_t time, int32_t delay, int32_t zoom, lv_anim_path_cb_t path, uint16_t repeat_cnt, uint32_t repeat_delay, uint32_t playback_time, uint32_t playback_delay, start_cb, ready_cb, deleted_cb)` | 0x230e0 | no | exec `lv_img_set_zoom`, from `lv_img_get_zoom(img)` to `zoom` |
| `void ui_img_rotate_animation(void *img, int32_t time, int32_t delay, lv_coord_t x, lv_coord_t y, int32_t rotate, lv_anim_path_cb_t path, uint16_t repeat_cnt, uint32_t repeat_delay, uint32_t playback_time, uint32_t playback_delay, start_cb, ready_cb, deleted_cb)` | 0x23148 | no | `lv_img_set_pivot(img, x, y)`; exec `lv_img_set_angle` from 0 to `rotate × 10` |
| `void events_init(lv_ui *ui)` (`events_init.c`) | 0x22f74 | yes (once, `main`) | empty (2 B: `bx lr`) |
| `void custom_init(lv_ui *ui)` (`custom.c`) | 0x2c178 | no | empty (2 B), GUI Guider's `custom.c` stub |

All animation helpers set `act_time = −delay` (LVGL's delay encoding) and use the 72-byte `lv_anim_t` of this LVGL (`var,
exec_cb, start_cb, ready_cb, deleted_cb, get_value_cb, user_data, path_cb, start, current, end, time, act_time,
playback_delay, playback_time, repeat_delay, repeat_cnt, flags`) [R offsets in the decompile; field names from the LVGL
header]. The parameter names are GUI Guider's [I]; types and order are read from the register use. `events_init.c` and
`custom.c` hold no other symbols.

`setup_scr_screen.c` builds the widget tree of §5 and calls `ui_init_style` for the
four static list styles `style_screen_list_1_main_main_default`, `…_main_scrollbar_default`, `…_extra_btns_main_default`,
`…_extra_texts_main_default` (8 B each in .bss); the last two are never attached to an object (§7.1).

### 10.3 `widgets_init.c` — keyboard helpers, unused [R]

The project has no keyboard and no text area (§5), so GUI Guider emitted the file but nothing registers
these callbacks (the three other `ta_event_cb` symbols are LVGL's examples):

| function | addr | behaviour |
|---|---|---|
| `void kb_event_cb(lv_event_t *e)` | 0x2c0e0 | `code = lv_event_get_code(e)`; `kb = lv_event_get_target(e)`; if `code` is `LV_EVENT_READY` (0x1f) or `LV_EVENT_CANCEL` (0x20): `lv_obj_add_flag(kb, LV_OBJ_FLAG_HIDDEN)` |
| `void ta_event_cb(lv_event_t *e)` | 0x2c104 | `code`, `ta = target`, `kb = lv_event_get_user_data(e)`; on `LV_EVENT_FOCUSED` (0x0e) or `LV_EVENT_CLICKED` (0x07): `lv_keyboard_set_textarea(kb, ta)`, move `kb` to the top (`lv_obj_move_to_index(kb, child_cnt(parent(kb)) − 1)`), show it; on `LV_EVENT_DEFOCUSED` (0x0f) or `LV_EVENT_CANCEL` (0x20; READY is 0x1f in this tree's enum): `lv_keyboard_set_textarea(kb, ta)`, `lv_obj_move_to_index(kb, 0)`, hide it |

### 10.4 Unused vendor functions in `main.c` [R]

The global `timer_thread` (0x4864c70) is read only by `timer_task` and never written, so it is always 0.

**`timer_task` (0x12330, 760 B)** — thread version of the record clock, replaced by `video_timer_label` (§5.5). Signature
`int timer_task(void)` (pthread start-routine shape [I]; returns 0).

```
lv_obj_add_flag(ui[97], HIDDEN)                       // hide photo-mode icon
loop:
  if (!video_state) return 0
  statfs("/mnt", &st)
  if (st.f_bsize * st.f_bavail / 1048576.0 < 10.0) {  // < 10 MiB free  (constant 9.5367431640625e-07 at 0x12620)
      ui[202].text = SD-full message for `lanugnuge` 1..9  // same 9 strings as video_timer_label, "…!!!"
      lv_obj_clear_flag(ui[202], HIDDEN)
      lv_timer_create(hide_label_after_delay_2, 2000, ui[202])
      if video_size == 1: send 0x18 "2"  /  if video_size == 2: send 0x18 "1"   // stop 4K / 1080P recording
                                         // short send → perror("Failed to send data to socket")
      video_state = 0
      pthread_join(timer_thread, NULL)  // timer_thread is never written → joins thread id 0
      video_time = 0
      lv_obj_add_flag(ui[201], HIDDEN); ui[201].text = "00:00:00"
      goto loop                         // → returns 0 because video_state is now 0
  }
  sleep(1)
  video_time++
  ui[201].text = snprintf(9, "%02d:%02d:%02d", t/3600, t%3600/60, t%60)
```

Against the live `video_timer_label`: the live one hides `ui[97]` on every tick, returns instead of sleeping, on SD full
pauses its own timer (`lv_timer_pause(timer)`) instead of `pthread_join`, and does not reset the clock text to "00:00:00".
Started, `timer_task` would join thread id 0 from inside itself — undefined in glibc, where the id is a pointer [I: likely a
crash].

**`video_timer_task` (0x12628, 400 B)** — thread version of the playback clock, replaced by `video_timer_cb` (§5.5).
Signature `int video_timer_task(void)`.

```
puts("video_timer_task")
loop:
  if (!isvideoRecd) return 0
  sleep(1)
  video_back_time++
  ui[202].text = "%02d:%02d:%02d" of video_back_time
  if (video_back_time <= Total_time) goto loop
send 0x1D "4"                                          // stop playback
lv_obj_clear_flag(ui[203], HIDDEN)                     // back to the video list
screen_flag = 0x33; videostate = 0; isvideoRecd = 0
lv_timer_pause(video_timer)
lv_obj_add_flag(ui[202], HIDDEN); lv_obj_add_flag(ui[94], HIDDEN)
ui[202].text = "00:00:00"
return 0
```

Against the live `video_timer_cb`: the same end-of-clip actions, but it runs once per LVGL timer tick, pauses both
`video_timer` and itself, and leaves the clock text as it is.

**`sendDataF` (0x18054, 120 B)** — `int sendDataF(int sock, uint8_t id, float value)`: argument registers r0 = socket, r1 =
id (`strb r5`), s0 = value (`vmov.f32 s16, s0`) (the decompile's parameter order is wrong). Zeroes two 65-byte buffers, byte
0 = `id`, `sprintf(buf+1, "%0.1f", (double)value)`, copies, `send(sock, pkt, 65, 0)`; returns 0 if 65 bytes were sent, else
`perror("Failed to send data to socket")` and −1. The EV message 0x0B is built by the two inlined copies of this code (§4.1).

**`sendMessageToSocket` (0x13e6c, 108 B)** — `int sendMessageToSocket(int sock, const char *text, uint8_t id)` [R registers].
Same packet, payload `snprintf(buf+1, 64, "%s", text)`; on a short send `puts("Failed to send message to socket.")` (not
`perror`) and −1, else 0. The path messages 0x1A / 0x1B and the photo-viewer messages use inlined copies (§4.1).

## 11. What is not known

- Whether the vendor patched any LVGL file beyond the tick function (§1.2, §1.3) [I: nothing observed needs a patch]; a
  file-by-file code comparison against a gcc 9.1 ARM build of `533e325f2` would settle it.
- The real SAR ADC reading of the AE button (the other five are in §6.1).
- The picture of the compare view (0x19 "3"): the path is specified (`prog_uvc.md` §6.3, §7.2); what it looks like on a
  monitor has not been observed.
- That GPIO46 = 1 shows orange and 0 red is read from the factory test's strings, not seen on the LED.
