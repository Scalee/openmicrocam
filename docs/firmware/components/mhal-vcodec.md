# mhal-vcodec — the Wave5 video codec in mhal.ko (VCODEC glue, Wave5 host library, VENC / VDEC wrappers)
<!-- ev:mhal-vcodec -->

The H.264 / H.265 codec of the camera is one Chips&Media Wave5-class VPU (product code faked as WAVE521 by the host
library) with its own firmware CPU (the VCPU), which runs `chagall.bin`. Three blocks of `mhal.ko` drive it: **VCODEC**
(350 functions) — the SigmaStar glue (platform device, clocks, bridge bank 1629, interrupt, memory, MIU protect, firmware
load, sysfs / procfs, reset) together with a Chips&Media `vpuapi` host library of 164 functions; **VENC** (57 functions)
— `MHAL_VENC_Wrapper_*` with the thin `MHAL_MFE_*` (H.264) and `MHAL_MHE_*` (H.265) entry sets used by `mi_venc.ko`; and
**VDEC** (31 functions) — `MHAL_VDEC_*`, used by `mi_vdec.ko`. Encoder and decoder share one core, one firmware image,
one register window, one interrupt and one set of clocks. This document covers the glue, the codec clocks and the
throughput limits visible from the host, the interrupt, the host side of the host ↔ VCPU register interface, the host
library compared with the StarFive and mainline Wave5 code, the VENC and VDEC wrappers with their parameter structures,
reset, and the unused parts.

Shared facts owned elsewhere: the DT `venc` node in the device tree as a whole, the CLKGEN tree and its PLLs, the
voltage-ctrl driver and the core rail — `kernel-platform.md` §5.1, §5.4, §14; the OTP2 words and the
derivation of the JPE / VENC throughput flag from them — `ipl.md` §7.2, §7.3; mhal's block map, the character devices,
all 33 module parameters with their defaults (`VenMiuPChn`, `VdeMiuPChn`, `drv_codec40_common.fw_load_path`, the eight
`drv_venc_wrapper.*` knobs) and the DT properties mhal reads — `mhal-core.md` §2, §4, §5, §7; mi_sys's MMA heaps and MIU
protect — `mi_sys-2-interfaces.md` §7.2, §7.7; the MIU client table — `kernel-platform.md` §12; the mi_venc side (device bring-up,
the 0xd0-byte input descriptor, events, reset handshake, module parameters `max_h26x_task`, `max_h26x_width/height`,
`use_ring_ref`) — `mi_venc.md` §3, §4, §7.4, §7.5, §10; the mi_vdec side and the meaning of the VDEC eStatus values —
`mi_jpd-mi_vdec.md` §2, §2.11.5; `chagall.bin` (container, version tag, VCPU code, firmware side of the interface) —
`chagall-fw.md` §2, §4, §5, §6; the app's codec settings — `prog_uvc.md` §3.2.

Binary: `/config/modules/4.9.227/mhal.ko`. Addresses are Ghidra addresses (file `.text` offset +
0x40000028). "Host register 0xNNN" = byte offset in the Wave5 host register window (§2.1).

Tags: **[R]** read from the binary (decompile, disassembly, `.data`) or from a cited file; **[S]** stated by a source
(mainline / StarFive / SDK header, log string used as a name); **[I]** inference.

## 1. The block [R]

### 1.1 Layers

| layer | functions | what it is |
|---|---|---|
| SigmaStar glue (VCODEC, non-library part) | `codec_probe`, `venc_probe`, `venc_clock_*`, `codec_isr*`, `codec_irq_handler`, `codec_dev_*`, `codec_load_firmware`, `codec_miu_protect_*`, `codec3_*`, `Venc*Proc*`, sysfs handlers | platform device, clocks, bridge 1629, interrupt, memory, MIU protect, firmware load, sysfs / procfs, reset (§2, §3, §9) |
| Wave5 host library (VCODEC) | 164 functions with the vpuapi prefixes (`VPU_*`, `Product*`, `Wave5Vpu*`, `WaveVpu*`, instance pool, locks, debug) | Chips&Media vpuapi 5.5.58, compared with StarFive and mainline in §5; the register protocol with the VCPU in §4 |
| VENC wrapper | `MHAL_VENC_Wrapper_*`, `MHAL_MFE_*`, `MHAL_MHE_*`, helpers | encoder API for mi_venc (§6, §7) |
| VDEC wrapper | `MHAL_VDEC_*` | decoder API for mi_vdec (§8) |

`MHAL_VCODEC_Init` / `MHAL_VCODEC_Deinit` (0x400ba368 / 0x400ba398) call `drv_codec_init` / `deinit`, which return −1,
so both always return 8; nothing calls them (**unused stub**). `codec_module_func_reserve` and `vcodec_common_func_reserve`
are empty placeholders (unused).

### 1.2 One core, shared by H.264, H.265 and decode

- One Wave5 VPU: one host register window (DT `vpu-bit` 0x1f345200, 0x800 bytes = banks **1A29–1A2C**), one IRQ (GIC SPI
  0x35 "mhe-irq"), `MAX core` 1 in the library (`ProductVpuScan` 0x400c87d4 sets only core 0). `MHAL_MFE_*` and
  `MHAL_MHE_*` are wrappers of the same `MHAL_VENC_Wrapper_*` (0x400c81f4…0x400c87a8), and `MHAL_VDEC_*` drive the same
  core. H.264 and H.265 channels and the MP4 player therefore time-share one engine.
- No dual-core / two-pipe mode exists in this build: `WaveVpuGetProductId` (0x400cde24) is hard-wired to return 6, and the
  register accessor `codec3_read_register` (0x400bf7c8) answers offset 0x1044 (`W5_PRODUCT_NUMBER`) with the constant
  **0x5210** (`WAVE521_CODE`, single core) without touching the hardware (§4.6). The dual-core branch of `Wave5VpuReset`
  depends on `VPU_RET_VPU_CONFIG1` bit 26 read from the firmware / hardware; whether it is ever set is not known.
  "2ndPU" in the clock names is the firmware CPU (vcpu) clock, not a second encoder [I from the vce / vcpu
  wording in `venc_dev_init`].
- The codec is chosen per instance at `CreateInstance`, not per wrapper (§7.1).

## 2. Resources and module glue [R unless tagged]

### 2.1 Platform device, probe, remove

- One platform driver serves two DT nodes: `codec_probe` / `codec_remove` (0x400afb2c / 0x400afae4) dispatch by the device
  name: contains "jpe" → `jpe_probe` (`mhal-scl-jpe-cmdq.md` §4.3), contains "venc" → `venc_probe`. DT node `venc`:
  `compatible = "sstar,venc"`, `reg` = `vpu-bit` 0x1f345200+0x800 (banks 1A29–1A2C, the Wave5 host registers),
  `venc-brige` 0x1f2c5200+0x100 (bank **1629**), `hw-uart0` 0x1f203c00 (101E, pad mux), `hw-uart1` 0x1f207800 (103C,
  UART1 bank), `venc-gp` = 1629 again; `interrupts = <0 0x35 4>` "mhe-irq"; clocks vhe / mfe / ven_axi (§2.2); only
  `clocks` / `clock-names = "CKG_venc"` besides — no other property feeds the codec.
- `venc_probe` (0x400b14d0): DT node must be available (else −19); allocates the two global mhal devices
  (`MHAL_VENC_Alloc_GDev(0)`, `MHAL_VDEC_Alloc_GDev(0)`); driver struct 0x11c bytes (`CamDevmKmalloc`); char device region
  "venc" (65 minors) with `venc_ops` — `venc_open` / `venc_ioctl` / `venc_release` only print "not support please enable
  MACRO MHAL_VENC_UT_ENABLE" and return −14 (**unused** debug interface, like JPE's; `mhal-core.md` §4). A second stub
  set `vcodec_*` (`vcodec_ops`) exists with the same behaviour but is never installed — nothing refers to `vcodec_ops`
  (unused). Then: class device "venc" under `msys_get_sysfs_class()`; IRQ by name "mhe-irq" →
  `request_threaded_irq(irq, codec_isr, NULL, 0, "codec_isr", drv)` — **the whole interrupt handling runs in hard-IRQ
  context** (no thread function); MMIO windows: every `IORESOURCE_MEM` resource → `drv+0x90+4i` (start) and `drv+0xa4+4i`
  (size), read back through `CodecDevGetRegVirt(i)` = start − 0x22000000 (RIU physical → kernel VA 0xFDxxxxxx); clocks
  with `of_clk_get` for up to 16 DT clocks into `dev+0x44…` (§2.2); OTP flag (§2.3); links `drv` into both mhal devices
  (`+0x10c`); sysfs (`venc_sys_init`, §2.8); mutex `drv+0xe4`, atomics `+0x10c` (pre-reset count), `+0x114` (ISR
  running); `codec_attr_parameters_init` (§2.6). Prints "vcodec prob sucess".
- `venc_remove` (0x400b174c) undoes it: class device, chrdev region, `cdev_del`, clocks off + `clk_put` ×16, `free_irq`,
  sysfs, mutex, release both mhal devices, attr deinit.
- `vcodec_init_device` (0x400b0eac) sets the clock defaults (§2.2).

### 2.2 Codec clocks
<!-- ev:mhal-vcodec.2.2 -->

Three clocks (DT `venc` node in + `venc_probe`); the CLKGEN registers and the PLLs behind
the parents are `kernel-platform.md` §5.1:

| # | DT clock (CLKGEN reg) | driver name / sysfs | parents (index: rate) | default |
|---|---|---|---|---|
| 0 | `CLK_vhe` (1038.68, mux bits 2–4, gate bit 7) | "vce" core clock, `ven_clock` | 0 `ven_pll` (programmable), 1 utmi 480, 2 mpll 432, 3 upll 384, 4 upll 320, 5 mpll 288, 6 utmi 192, 7 mpll 123 MHz | 384 MHz |
| 1 | `CLK_mfe` (1038.69, mux bits 2–3, gate bit 0) | "vcpu" / 2nd PU, `ven_clock_2nd` | 0 mpll 216, 1 upll 384, 2 upll 320, 3 `ven_pll` | 384 MHz |
| 2 | `CLK_ven_axi` (1038.69, mux bits 10–12, gate bit 8) | AXI, `ven_clock_axi` | 0 upll 384, 1 upll 320, 2 mpll 216, 3 `ven_pll`, 4–7 VOID | 384 MHz |

- **Defaults**: globals `g_venc_clock`, `g_venc_clock_2ndpu`, `g_venc_clock_axi` (`.data+0x1c678/7c/80`) = 0x16e36000 =
  **384 MHz each**. `vcodec_init_device` sets the per-device **maximum `dev+0xc0` = 480 MHz** (also the 2nd-PU and AXI
  maxima `dev+0xcc` / `+0xdc` = 480, unused by the setters) and `dev+0xe1` = 1 ("ven_pll allowed"). No DT property and no
  module parameter feeds these (the only codec module parameters are `VenMiuPChn` / `VdeMiuPChn`, §2.6).
- **Rate pick** `venc_clock_rate(i, dev, req)` (0x400b1094): walks the parents of clock *i* and takes the **lowest parent
  rate ≥ req, else the highest**; the chosen parent index + 1 is stored (`dev+0xb8/0xc4/0xd4`).
  `vcodec_clock_special_source_disable` (0x400b0ecc) returns 0 — no parent is excluded; `venc_get_resource_clk` and
  `venc_set_source_clk` return 0 (unused).
- **ven_pll programming** (`venc_clock_init` 0x400b12f8, `venc_clock_venpll` 0x400b104c): if `req` is one of **300, 348,
  420, 468, 504, 576 MHz**, ven_pll (parent 0 of vhe, parent 3 of mfe / axi) is first rounded and set to `req`, and the
  pick above then finds it. Any other value only selects among the fixed PLL outputs.
- **Consistency rules** (`venc_dev_init` 0x400b17f8): if vce and vcpu are both ven_pll rates but different, vcpu is forced
  to 384 MHz; if axi is a different ven_pll rate from vce or vcpu, axi is forced to 384 (one PLL, one rate).
- **When applied**: `venc_dev_init` (from `MHAL_VENC_Wrapper_CreateDevice` 0x400c12c4 and `MHAL_VDEC_CreateDevice`) runs
  `venc_clock_init(0/1/2, dev, g_venc_clock…)` then `venc_clock_on(dev, 1)` (set rate, prepare, enable each of the
  three). **The clocks stay on for the device's lifetime** — no per-frame gating (unlike JPE): `venc_clock_on` is called
  only from `venc_dev_init`, `venc_clock_deinit` and `codec_dev_reset`.
- **Voltage** (`change_voltage` 0x400b12ac): after any clock init, if any of the three rates > 384 MHz →
  `set_core_voltage("core_power", 2, 1000)` (1.0 V), else 900 mV; clock deinit drops back to 900. `core_power` probes
  and its rail is already held at 1000 mV from probe on by the TEMPERATURE demander, so the codec's 1.0 V request is
  honoured but changes nothing, and the codec runs at 1.0 V [I: if the GPIO reaches the regulator]
  (`kernel-platform.md` §5.4).
- **sysfs** (`venc_sys_init` 0x400b216e) on the class device `venc` (class from `msys_get_sysfs_class()`, i.e.
  `/sys/class/mstar/venc/` [I: same class as `vif0` / `isp0`], e.g. `/sys/class/mstar/venc/ven_clock`): `ven_clock`,
  `ven_clock_2nd`, `ven_clock_axi` (the other attributes in §2.8).
  - write (`ven_clock_set` 0x400b1fbc, `_2ndpu_set` 0x400b1fb4, `_axi_set` 0x400b1fac): decimal Hz, clamped to
    **1 … 480 000 000**, then `venc_clock_init(i, dev, value)` (rate set at once + voltage) — **refused** with "venc dev
    created, no support set clk, pls set before createdev" **while the mhal VENC device is created**
    (`g_stMhalVencDev+0x108` = `b_vpuInit`, set by CreateDevice, cleared by the last DestroyDevice). The lock looks only
    at the VENC device: with just the VDEC device (MP4 player; same core and clocks) created, a write goes through and
    retunes the running clocks at once.
  - read (`ven_clock_get` 0x400b2168): lists the selectable rates ≤ 480 MHz — ven_pll rounded from 1024 MHz downwards in
    16 MHz steps, then each fixed parent — and marks the current one with ` <--`.
- **When a write takes effect on the camera** [R for the rule, I for prog_uvc's timing]: mi_venc creates dev 0 (MFE+MHE)
  at the first H.26x `CreateChn` (`mi_venc.md` §3). prog_uvc does that only when the host first selects H.264 / H.265 (or
  the recorder starts, `St_VencModuleInit`), so after boot and before the first H.26x stream a write of `ven_clock` is
  accepted; afterwards the device exists until prog_uvc exits (fd close → mi_venc DeInit → `DestroyDevice`, users 0 →
  `b_vpuInit` 0).
- **Highest reachable**: vce 480 MHz (utmi_480m parent; or 468 via ven_pll) = +25 % over the 384 default; vcpu 384 (its
  only parent above 384 is ven_pll; ven_pll rates above 480 are cut by the clamp; 420 / 468 possible); axi 384 (same).
  How encode time splits between vce (pixel pipeline) and vcpu (firmware CPU) is not known [I: for large frames the pixel
  pipeline normally dominates].
- **The firmware is told the vcpu clock** (`Wave5VpuInit` 0x400cdfc4): init register 0x160 = vcpu rate / 100 000
  (`dev_info+0x144`, written by `venc_clock_init(1)`), and 0x12c = UART divisor from the same clock when the firmware UART
  is on (§4.2). A vcpu change after VPU init is not re-sent.
- The vhe mux (1038.68 bits 2–4) can also be switched by a direct RIU write behind the clock framework; the driver's view
  (`ven_clock` read, proc) then stays at its own value [R]; whether such a switch while encoding glitches or hangs the
  core is not known [I: possible].

### 2.3 OTP throughput cap

- `venc_probe` reads the OTP2 (eFuse) words **101f.2e, 101f.2f, 101f.2c, 101f.26** (RIU VA 0xfd203eb8 / ebc / eb0 / e98)
  and derives one flag, stored at **mhal VENC device +0x180** (byte); the rule is the JPE flag's and is given in `ipl.md` §7.3
  (VIF's `HalVif_OTP_VENC_Fps_Capability` reads a simpler variant of the same words, also there). The
  values of these words on this chip are not known (`ipl.md` §9).
- Check `venc_check_res_rate` (0x400c053c): with the flag set, **W × H × fpsNum / (fpsDen + 1) > 0x9f60000 (167 116 800
  pixel/s) → refused** ("W:%d H:%d Fps(%d/%d) not support"): SetParam resolution / crop / RC (both codecs) returns 8, and
  **`MHAL_VENC_Wrapper_EncodeOneFrame` returns 8 for every frame** (no encode at all, not a slowdown). The denominator is
  stored minus one (`+0x22` u16 + 1).
- Scale [arithmetic]: 167.1 Mpx/s = 3840×2160 at 20.1 fps, 2560×1440 at 45.3, 1920×1080 at 80.6 fps. With the flag set,
  any H.26x mode above that is refused at channel setup or per frame — a hard "no frames", not a soft fps ceiling [I].
- The flag lives in kernel memory once derived; nothing in the driver changes it.

### 2.4 Bridge bank 1629 ("venc-brige" / "venc-gp")

16-bit registers, `reg n` at byte `4n` of the bank. `codec_dev_preset` (0x400ba206) asserts that both DT windows are
0xfd2c5200. Values are [R]; meanings are [I] unless noted — no reference describes this bank.

| reg | write | where | meaning [I unless noted] |
|---|---|---|---|
| 0x00 | = 6 | preset | bridge mode / config |
| 0x01 | = 0x4040 | preset | bridge AXI settings (two equal bytes: one per direction) |
| 0x21 | `\|= 0xfe` then `&= 0xff01` (bits 1–7 set, then cleared) | preset; `\|= 0xfe` only in DestroyDevice and pre-reset | sub-block resets: held in reset while the VPU is off |
| 0x22 | = 0xfe (preset), = 0xff (pre-reset) | | reset / isolation mask (bit 0 also set on reset) |
| 0x23 | bit 0 pulsed 1 → 0 | `codec_irq_handler` entry, post-reset, pre-reset | interrupt acknowledge / bridge interrupt clear |
| 0x26 | `\|= 1` | preset | bridge enable |
| 101E reg 0x54 | low nibble = 6 (preset, only with the firmware UART on), cleared by `fw_log_ctrl 0` | | pad mux of UART0 to the VPU's own UART [R bank, I meaning] |

Preset also sets the codec device-info flags used at VPU init: `+0xd8` = 1, `+0xc8/cc/d0/d4` = 0 (→ init register 0x13c =
0x10) and `+0x44` = 3 (→ register 0x164), then `_UpdateMaxResAndSramSize` (§2.5).

### 2.5 Memory: device info, common buffer, SRAM, VLC / CINFO

- **Device info** = one static struct (`codec3_get_device_info(0)` → `g_codec_fw_path+0x198`; any other core index →
  NULL). `codec3_init` (0x400c0034) hard-codes the VPU register window — physical 0x1f345200 (`+0x14`), kernel VA
  0xfd345200 (`+0x18`); the DT `vpu-bit` resource is read by probe but not used here — product code 0x5210 (`+0x40`), a
  default SRAM window address 0x1d000 / size 0x26800 (`+0x34/+0x38`, replaced per frame by mi_venc's IMI info), a 64-slot
  instance pool, then the common memory below; if the VCPU program counter (host register 0x4) reads 0 it clears the
  command registers 0x100–0x1ff. Register access is plain 32-bit MMIO at VA + offset (`codec3_write_register` /
  `codec3_read_register`), FIO through 0x20 / 0x24.
- **Common buffer** — `codec3_allocate_common_memory` (0x400bfe38): one **0xea000-byte (936 KiB)** MMA buffer "vcodec_dev"
  (from the MIU-protect heap when that heap exists, §2.6, else the default MMA heap), laid out as:

  | part | size | device-info fields (phys / VA / size) | use |
  |---|---|---|---|
  | firmware code | 0x80000 (512 KiB) | `+0x8c` / `+0x90` / `+0x88` | `chagall.bin` image; VCPU address 0 (§2.7) |
  | decoder task memory | 0x48000 | `+0x94` / `+0x98` / `+0x9c` | three task buffers of 0x18000 (init registers 0x140 / 0x144 / 0x154, §4.2) |
  | temp | 0x1d000 | `+0xa0` / `+0xa4` / `+0xa8` | firmware temp buffer (init registers 0x11c / 0x120 and 0x158 / 0x15c) |
  | — | 0x2000 | `+0xdc` / `+0xe0` / `+0xe4` | not identified |
  | 0x1000 block | 0x1000 | `+0xe8` / `+0xec` / `+0xf0` | init register 0x150; the firmware writes its clock value into the first word (§4.2) |
  | ramlog | 0x2000 | `+0xf4` / `+0xf8` / `+0xfc` | firmware RAM log (init registers 0x148 / 0x14c; sysfs `ramlog`) |

- **VLC / CINFO** ("ven_tmp" buffer, `codec3_get_vlc_cinfo_total_size`): optional, **zero on this build** because preset
  sets `+0xd8 = 1`, which makes both sizes 0 (unused). Formulas kept in the code: VLC = W·H·3/8·3/2 rounded to 4 KiB, min
  2048×1088; CINFO = ⌈W/16⌉·⌈H/16⌉·8, min 1280×720; ×2 each.
- **SRAM** (secondary-AXI line buffers): `_UpdateMaxResAndSramSize` (0x400c01b0) defaults the max resolution to
  4096×2176 (both `+0xb8/bc` and `+0xc0/c4`), sets VDEC SRAM `+0x140` = 0x13000 and VENC SRAM `+0x13c` =
  `_CalculateMaxVencSramSize(max(W, 4096), 1)` = max(⌈W/64⌉·0x180 + ⌈W⌉16·7, (⌈W/64⌉·0x24 + ⌈W⌉32)·8) + 0x10000 →
  **0x1d000** for 4096 (⌈W⌉n = W rounded up to a multiple of n). `_CalculateMaxVdecSramSize(W)` = 2·(⌈W⌉16 + 2·(⌈W⌉64 +
  2·⌈W⌉32 + ⌈10W/8⌉16)). The SRAM base / size come from mi_venc per frame (`_UpdateVencDynamicSramInfo` 0x400c11ec reads
  descriptor +0xb0 / +0xbc / +0xc8: IMI mode +0xc8 must be 1, else 8; size must be ≥ the computed minimum, else the frame
  is refused with 0xc; then `VPU_EncGiveCommand(0xe)`) and are stored with `codec3_set_sram_memory`; VPU init and each
  `CREATE_INSTANCE` pass them in host registers 0x124 / 0x128 (init) / 0x140 / 0x144 (instance). An H.26x input on M6
  therefore must carry `eImiMode` = 1 with an SRAM window of at least that size, else every frame is refused (descriptor:
  `mi_venc.md` §7.4).
- `codec3_miu_channel_buffer(addr, type)` returns 1 unless type 2 — MIU channel selection stub.
- **Allocator** `_codec3_allocate_MMA_alloc` (0x400bf2c4): goes through the mi_sys MMA callbacks the consumer registered
  (device-info `+0x100` / `+0x120` sets, or the VENC / VDEC private ones at `g_codec_fw_path+0x2b8` / `+0x298`), optional
  map + zero + cache flush; counts allocations from the protect heap against `VenMiuPChn + VdeMiuPChn`.

### 2.6 MIU protect

- Module parameters `VenMiuPChn` = 8, `VdeMiuPChn` = 8 (`.data+0x1c676` / `0x1c674`, u16, ≤ 64 each, "Invalid miu pro
  channl num" otherwise; `codec_attr_parameters_init`): the channel counts the protect heap is sized for.
- `codec_miu_protect_enable` (0x400b9c28, called from `MHAL_VENC_Wrapper_CreateDevice` before VPU init): heap size =
  `_CalculateMaxMiuProtectHeapSize(ven, vde)` = (ven + vde)·0x31000 + 0xea000 = **4 169 728 bytes** for 8 + 8; asks the
  consumer's `GetMMaHeapName` callback (`+0x14`) for a heap of that size (name into a 64-byte buffer), and if one exists,
  `SetMiuProtectMMa(name, {0x20, 0x21, 0x70, 0x73}, 4, 1)` (`+0x18`) — **MIU clients 0x20, 0x21, 0x70, 0x73** (table
  "codec_miu_protect" at `jpe_ops+0x41c`). −2 = already enabled. `codec_miu_protect_disable` reverses it (enable 0) and
  destroys the heap name. mi_venc / mi_vdec pass `mi_sys_GetModuleMMaHeapName` / `mi_sys_setMiuProtectMMa`
  (`mi_sys-2-interfaces.md` §6.1, §7.7).

### 2.7 Firmware load and version check

- `codec_load_firmware` (0x400b9f08), from `CreateDevice` (VENC and VDEC) and post-reset. Path order: (1) attr `+0xa4` —
  a path mi_venc set with SetDevConfig 0x80030019 (from its module parameter `fw_path`, `mi_venc.md` §4), if non-empty;
  (2) `g_codec_fw_path+0x80` = the buffer of module parameter `drv_codec40_common.fw_load_path` (maxlen 128), if its
  first byte (`+0x124` relative to the attr) is non-zero; (3) **`/config/venc_fw/chagall.bin`**; (4)
  `/data/ut/chagall.bin`. Size must be 1 … 0x80000 (512 KiB, the code region).
- `VPU_InitWithBitcode(0, file, (size+1)/2)` (size in 16-bit words, Chips&Media convention); 0 or 0xc (already
  initialised) is success. Then reads **24 bytes at file offset 0x1008** (inside the version tag at 0x1000,
  `chagall-fw.md` §2.1): bytes 0x1008–0x100f must be **"22031116"** (two words compared to 0x33303232 "2203" and 0x36313131
  "1116"), else "fw ver:%llx mismatch driver:%llx, pls check fwVPU_if.h and chagall.bin/chagall.h" and the load fails (−1).
  With the date matching, bytes 0x101a–0x101b ("03") are compared with 0x3330; a difference only warns "shm ver:0x%x
  mismatch driver:0x3" (shared-memory interface version 3). Nothing else of the file is checked (no length, checksum or
  product field). `create_ramlog()` after success.
- `Wave5VpuInit` (0x400cdfc4): copies the image into the code region (`CamFsRead` straight into the VA), flushes, programs
  the remap (`W5_VPU_REMAP_CTRL` = 0x80000800 | 512K>>12, `REMAP_VADDR` 0, `REMAP_PADDR` = code base — VCPU address 0 =
  file offset 0) and the init block (§4.2), `INIT_VPU`, waits busy (timeout `__VPU_BUSY_TIMEOUT`), checks
  `W5_RET_SUCCESS` (fail → reason from 0x10c).
- The VENC and VDEC devices share the load: CreateDevice skips it if the other device already loaded the firmware
  (`drv+0xd0`).

### 2.8 sysfs, procfs, logs

- **sysfs** on class device `venc` (+ link "venc" from the platform device):
  - `log_ctrl`: per-module host log level, "`<name>:<0..5>`" or "`all:<n>`"; 19 modules: mhe, mfe, sys, jpeg, jios, jops,
    jdev, jctx, jenc, wrap, dbg, vapi, vfun, waves, prod, vdi, ut, vcodec, vdec — table `ven_log_filters` `.data+0x1c590`,
    12 bytes each {id, name, level}.
  - `fw_log_ctrl`: 1 routes the VPU's hardware UART to pads — checks pad mux 0x70020 / 0x70021 = pads 4 / 5 mode 0x73,
    sets 101E.54 low nibble 6 and `+0x148` = 1 so the next VPU init sends the baud divisor; 0: off. The pad routing is
    unused unless 1 is written.
  - `ramlog`: firmware RAM log from the 0x2000 buffer (`ramlog_show`).
  - `venc_param_ctrl`: read lists the settings; write "`hvsqp_<chn>_<en>_<level>`" HVS QP per channel,
    "`mblv_<chn>_<en>`" MB-level rate control, "`minqpscaling_<mhe>_<mfe>`" globals `g_mheMinqpScaling` /
    `g_mfeMinqpScaling`; per-channel changes set flag 0x200 in the instance's change mask so the next frame sends
    `ENC_SET_PARA_CHANGE`.
  - `ven_clock`, `ven_clock_2nd`, `ven_clock_axi` (§2.2).
- **`/proc/mhe/`** — `VencProcInit` (0x400b7f84) creates `dev_info`, `ctx_info`, `fw_info`, `fw_dbg`, `dump_dbg` when the
  first VENC device is created:
  - `ctx_info` (`VencProcShowCtxInfo` 0x400b6080; write "`%li %li`" = dev ctx selection) prints per channel: resolution,
    fps, GOP, frame type, QP ranges, counters `Fire/Done/ISR/DoneISR/HwErrISR/FireIn`, real kbps and real fps over
    1 / 3 / 5 / 10 s, drop fps, **host times (ms): Total, Fire, Isr, Isr→Done, Done, Done→Start, I-frame, P-frame**, and
    **firmware times (ms): Seek, Encoding, Packaging, Total, EncStart→PackStart** as last-1 s / last-3 s averages, last I
    and last P frame, and maximum; also the custom-map "preSumQp" (§7.12). The firmware times come from the tick
    registers the firmware returns with each picture (`Wave5VpuEncGetResult` reads 0x1b8–0x1d0 =
    `W5_RET_ENC_HOST_CMD_TICK … ENCODING_END_TICK` in mainline names) [R reads, I column mapping]. Firmware Total ≈ 1/fps
    means the engine is saturated; host Total ≫ firmware Total means the bound is on the host path [I]. The values on the
    running camera have not been read.
  - `fw_info` (write "`%ld`"), `fw_dbg` (write "`<case> <path>`": 1 flush, 2 query status, 3 dump buffer to path),
    `dump_dbg` (write "`<tag> <dev> <chn> <n> <path>`", tags `vencin`, `vencout`, `vdecin`, `vdecout` — dump input / output
    buffers to files), `dev_info`.
- **Debug helpers** (`InitializeDebugEnv`, `PrintVpuStatus`, `wave5xx_vcore_status` / `_bpu_status` / `_vcpu_status`,
  `HandleEncoderError`, `PrintMemoryAccessViolationReason`, `codec3_irb_*`, `ReadRegVCE` / `WriteRegVCE`,
  `VcodecReadVcoreReg` / `VcodecReadFbbReg` / `VcodecReadVceReg`) follow the Chips&Media `vpuapi/vpuapi_debug` set — dumps
  of internal VCPU / VCE / BPU registers through the FIO window (`W5_VPU_FIO_CTRL_ADDR` 0x20 / `DATA` 0x24); reached only
  on errors and timeouts.
- `PrintVpuProductInfo` prints the firmware's own report (§4.6) at log module `dbg`, level ≥ 3.

## 3. Interrupt (GIC SPI 0x35, `codec_isr` 0x400b14c4) [R unless tagged]

### 3.1 `codec_isr` and the owners' callbacks

`codec_isr` → `codec_isr_process(irq, drv)` (0x400b07b8): ignores the IRQ unless `drv` belongs to the VENC or VDEC mhal
device and `irq == drv+0x88`; sets `drv+0x114` (ISR running), calls `codec_irq_handler`, then hands the event to the
owner's callback: VENC device `+0x184` (registered by mi_venc with SetDevConfig 0x8003001c), VDEC device `+0x184`
(SetDevConfig 0x80020008). The handler returns two numbers: the **event-record kind** (first word of the record handed
to the callback) and the **event code**. A record of kind 8 — produced by reason 0x1000 — goes instead to `+0x188` of
both devices, the same channel `codec_dev_reset` uses for its pre-reset handshake (§9): the owner's device gets it with
sub-kind 2, the other device with sub-kind 1 (the reset handshake sends 3 and waits for 4; `codec_dev_resume` sends a
zeroed record) [R: `codec_isr_process` 0x400b07b8]. The event code is stored in the encoder instance
(`+400`, except codes 4 and 6) or the decoder instance (`+0x28ec`). `MHAL_VENC_Wrapper_GetEngStatus` runs the same handler
by polling (§7.1). On M6 mi_venc requests no IRQ of its own for any engine; it relies on these callbacks, called in
hard-IRQ context (`mi_venc.md` §3.3, §7.5).

### 3.2 `codec_irq_handler` (0x400afb74)

1. Pulses bridge 1629 reg 0x23 bit 0; reads `W5_VPU_VPU_INT_STS` (0x44) — 0 → nothing; reads `W5_VPU_VINT_REASON` (0x4c).
2. Picks the instance register by reason, in this priority:

   | reason bit | instance register | encoder | decoder |
   |---|---|---|---|
   | 0x8000 (bit 15, BSBUF_FULL / EMPTY) | 0x1e4 (`BS_EMPTY_INST`) | event 4 "bitstream buffer full": `VPU_EncGiveCommand(0x48)` + `VPU_BitstreamBufGet` report the write pointer | event 4: `VPU_DecGetBitstreamBuffer` |
   | 0x800 (bit 11, SigmaStar) | 0x1f0 | event-record kind 7, no event code set: slice / partial output with the bitstream pointer | event 8, event-record kind 10, `+0x28ec` = 8 |
   | 0x1000 (bit 12, SigmaStar) | 0x1e8 (`QUEUE_CMD_DONE_INST`) | event 3, event-record kind 8 (→ `+0x188` channel), the host's reading "source buffer released", sets device `+0x154 = 1` and instance `+0xd0c = 1` | event 3, event-record kind 8 (→ `+0x188` channel), sets device `+0x154 = 2` and instance `+0x28f8 = 2` |
   | 0x40 (bit 6, INIT_SEQ) | 0x1e8 | — | event 9, picture size from 0x1f8 |
   | 0x100 (bit 8, DEC_PIC / ENC_PIC) | 0x1e8 | event 2 picture done: done counters and the IRQ timestamp (`+0x128`, used for the proc timings) | event 2 picture done |
   | 0x200 (bit 9, SET_PARAM) | 0x1e8 | acknowledged only | acknowledged only |
   | 0x2000 (bit 13, ENC_LOW_LATENCY) | 0x1fc | event 6 with the bitstream pointer | — |
   | none of these | — | index = 0x1fc bits 0–4, no slot ("int_reason 0x%x is not included … will use other_inst") | same |

3. **Instance slots.** Each instance register packs three 8-bit instance indexes — slot 0 bits 0–7, slot 1 bits 8–15,
   slot 2 bits 16–23 — whose valid flags are bits **24, 25, 26**. The bits do not select an instance class. One register
   can therefore report up to three instances with the same pending reason [R layout; I "at once"]. The handler takes the
   first valid slot (24 before 25 before 26); none valid → "error channel index overflow", index 0. The internal selector
   is 1, 2, 4 for slots 0, 1, 2 — any other value later logs "pu32ValidInstBits error".
4. **Class by lookup**: the index (< 0x40, 64 instances) is looked up first in the VENC global device's instance table
   (`MHAL_VENC_Get_GDev(0)` + 8 + 4·idx), and only if that slot is empty in the VDEC device's table; neither → "error No
   codec inst exist" and return. Both tables are filled from the one 64-slot codec-instance pool (`Vpu_GetCodecInstance`,
   §5.2), so an index names exactly one encoder or decoder instance [R pool; I uniqueness — the pool hands each slot to one
   instance].
5. **Acknowledge one slot** by writing the register back: the slot's index byte = 0xff, its valid bit cleared, the other
   two valid bits kept, and bits 28–29 = slot number (0 → 00, 1 → 01, 2 → 10): slot 0 `(v & 0xcfffffff | 0xff)` with bits
   25 / 26 kept; slot 1 `(v & 0xcfffffff | 0x1000ff00)` with bits 24 / 26 kept; slot 2 `(v & 0xcfffffff | 0x20ff0000)`
   with bits 24 / 25 kept. Bits 27, 30, 31 are never tested; the write passes them back unchanged. [I: the firmware
   re-raises the interrupt for slots still marked valid; one ISR pass serves one instance.]
6. The reason's event (table above) for the class found in step 4; then `W5_VPU_VINT_REASON_CLR` (0x34) = reason and
   `W5_VPU_VINT_CLEAR` (0x3c) = 1.

This instance-slot encoding is SigmaStar's; mainline has none of it.

### 3.3 Enabled reasons and who raises them

`Wave5VpuInit` writes `W5_VPU_VINT_ENABLE` (0x48) = **0xbd40**: bits 6, 8, 10, 11, 12, 13, 15. Mainline names bits 6
INIT_SEQ, 8 DEC_PIC / ENC_PIC, 10 ENC_SRC_RELEASE, 13 ENC_LOW_LATENCY, 15 BSBUF_EMPTY / FULL [S]; stock `W5_VPU_VINT_REASON`
assigns bits 0–9, 14, 15 only, so bits 11 and 12 are SigmaStar / build additions. Where the firmware raises them is
`chagall-fw.md` §6.4: bit 11 a partial (slice-level) output report with its bitstream position, instance in 0x1f0; bit
12 the first completion reported to an instance pending across a SLEEP / WAKEUP cycle, instance in 0x1e8 [R mechanics,
I meaning]. The host's "source buffer released" (event 3, §3.2) is the host driver's reading of bit 12; the firmware
gives it no name.

## 4. Host ↔ VCPU register interface — host side

The host talks to the VCPU firmware through the Wave5 host register window: a control block (0x000–0x0ff) and a command
window (0x100–0x1ff), with the Chips&Media command protocol (`W5_COMMAND` 0x100, `RET_SUCCESS` 0x108, `RET_FAIL_REASON`
0x10c, `CMD_INSTANCE_INFO` 0x110, busy flag `W5_VPU_BUSY_STATUS` 0x70). Names are the mainline `wave5-regdefine.h` ones
[S]. What the firmware does with each register, and the firmware internals behind it (dispatchers, handlers,
scheduler): `chagall-fw.md` §6.

### 4.1 The windows as the VCPU sees them [R]

The VCPU addresses of the control block and the command window, and what the firmware reads and writes in each:
`chagall-fw.md` §6.1. Host register 0x1044 (`W5_PRODUCT_NUMBER`) lies outside the 0x800-byte DT window and is never read by
the host (§4.6).

### 4.2 VPU init: the init block (`Wave5VpuInit` → `INIT_VPU`)

`Wave5VpuInit` writes these registers before `INIT_VPU`; `Wave5VpuSleepWake` re-sends 0x134–0x15c (7 registers) on wake.
Host values [R mhal]:

| host reg | host value |
|---|---|
| 0x11c, 0x120 | temp buffer base / size (§2.5) |
| 0x124, 0x128 | SRAM base / size (§2.5) |
| 0x12c | firmware-UART divisor in bits 31:16 (mainline `W5_HW_OPTION` slot), only with `fw_log_ctrl 1` |
| 0x134, 0x138 | VLC / CINFO sizes (0 on this build, §2.5) |
| 0x13c | feature bits = 0x10 (from preset `+0xc8/cc/d0/d4` = 0, `+0xd8` = 1) |
| **0x140** | number of decoder task buffers = 3 |
| **0x144** | decoder task memory base |
| 0x148, 0x14c | ramlog base / size (0x2000) |
| 0x150 | 0x1000-block base |
| **0x154** | size of one decoder task buffer = 0x18000 (96 KiB) |
| 0x158, 0x15c | temp buffer |
| 0x160 | vcpu clock / 100 kHz (`dev_info+0x144`, §2.2) |
| **0x164** | mode word = 3 (`dev_info+0x44`, set by preset) |
| 0x168 | not written by the host |
| 0x48 (control) | `VINT_ENABLE` = 0xbd40 (§3.3) |

What the firmware does with each, which of them it reads only on `INIT_VPU` and which it reads in other commands:
`chagall-fw.md` §6.3. In this firmware's INIT_VPU layout the mainline `W5_CMD_INIT_NUM_TASK_BUF` / `_ADDR_TASK_BUF0` /
`_TASK_BUF_SIZE` (0x134 / 0x138 / 0x178 in mainline [S]) sit at 0x140 / 0x144 / 0x154 [R use, S mainline offsets]. The
mainline names of other commands at these offsets (`W5_CMD_ENC_SEQ_INDEPENDENT_SLICE`, `_RC_TARGET_RATE`,
`_RC_BIT_RATIO_LAYER_4_7` in ENC_SET_PARAM; `RET_QUERY_BW_*`) do not describe these values.

### 4.3 Commands

Command codes of `W5_COMMAND` [R firmware; names S mainline enum]: 1 INIT_VPU, 2 WAKEUP_VPU, 4 SLEEP_VPU, 8
CREATE_INSTANCE, 0x10 FLUSH_INSTANCE, 0x20 DESTROY_INSTANCE, 0x40 INIT_SEQ, 0x80 SET_FB, 0x100 DEC / ENC_PIC, 0x200
ENC_SET_PARAM, 0x800 (not in the mainline enum), 0x4000 QUERY, 0x8000 UPDATE_BS, among others. QUERY options (0x104) [names S
mainline `enum query_opt`]: 0 GET_VPU_INFO, 1 SET_WRITE_PROT, 2 GET_RESULT, 3 UPDATE_DISP_FLAG, 4 GET_BW_REPORT, 5
GET_BS_RD_PTR, 6 GET_BS_WR_PTR, 7 GET_SRC_BUF_FLAG, 8 SET_BS_RD_PTR. Which commands the firmware executes at once and which
it queues, the registers each command and QUERY option reads and writes, the options outside the mainline enum and the
interrupt each ends with: `chagall-fw.md` §6.2.

Host-library selectors (`VPU_EncGiveCommand` / `VPU_GiveCmdDec` `CodecCommand` values) seen on the encode path: 0x46
queue-status query (`Wave5VpuQueryStatus`, §6.1), 0x48 before reading a result or a bitstream pointer, 0xe set the
per-frame SRAM info, 0x49 at STREAM_ON, 0x42 `ENC_SET_PARA_CHANGE`; decoder `GiveCmdDec` 8 re-reads the sequence and 0x28
sends the scaler size (§8). **0x300** (`Wave5VpuEncGiveCommand`, which writes host register 0x300) is mhal's own selector,
not a command code (`chagall-fw.md` §6.2).

Command-queue depth: `Wave5VpuBuildUpEncParam` (0x400d05be) writes `W5_CMD_NUM_CQ_DEPTH_M1` (0x13c in CREATE_INSTANCE) = 2
→ **3 commands per instance** (StarFive writes `COMMAND_QUEUE_DEPTH − 1` with a depth of 4).

### 4.4 The firmware's interrupt raiser

The firmware's only writer of `VINT_REASON`, the reasons it accepts and the instance register it fills for each:
`chagall-fw.md` §6.4. The host's decode of the instance registers is §3.2.

### 4.5 Properties read from the firmware (`SetupWave5Properties` (0x400cdc4c), tail of `Wave5VpuInit`)

`GET_VPU_INFO` (`W5_QUERY_OPTION` 0, command 0x4000): product name ← 0x11c (4 chars), product version ← 0x120, firmware
version ← 0x118, customer id ← 0x140, std def 0 / 1 ← 0x124 / 0x128, conf feature ← 0x12c, conf date ← 0x130, conf revision
← 0x134, conf type ← 0x138. productId is set to **6** without reading `W5_PRODUCT_NUMBER`. For id 6: supported decoders and
encoders = 0x1001 (bit 0 AVC, bit 12 HEVC), GDI on; HEVC 10-bit = conf feature bit 3, AVC 10-bit = bit 11 when conf
revision > 167455 (same rule as StarFive); backbone / dual-core flags from 0x78 / 0x7c bits 15 / 26 / 23 / 27.
`Wave5VpuGetVersion` (0x400cdf18) repeats the query and returns 0x118 (revision).

### 4.6 Product id: no hardware check

- `WaveVpuGetProductId` returns 6; `CREATE_INSTANCE` stores "0x1044" into each instance (`+0x18c8`), but
  `codec3_read_register` never reads that offset and returns **0x5210** (`WAVE521_CODE`); `codec3_init` also stores 0x5210
  as the device's product code (`dev_info+0x40`). The capabilities (AVC + HEVC encode and decode) are hard-coded for id 6
  (§4.5). The firmware is matched by its date tag only (§2.7).
- The firmware's own report (name, version, configuration from `GET_VPU_INFO`) is printed by `PrintVpuProductInfo` (log
  module `dbg`, level ≥ 3, i.e. after "dbg:3" is written to `/sys/class/mstar/venc/log_ctrl` before an H.26x stream
  starts; `CreateInstance` reports "Failed to PrintVpuProductInfo()", so it runs at instance creation [I]). The values it
  prints on this camera have not been read.
- What the silicon would report in `W5_PRODUCT_NUMBER` is not known (a 32-bit read of physical 0x1f346244 =
  0x1f345200 + 0x1044 would show it; the address is outside the mapped window and may be unreachable).
- Mainline `wave5_vpu_get_product_id` would reject 0x5210 ("Unsupported product id"; only 0x521c / 0x5150 pass) [S].

## 5. The Wave5 host library compared with StarFive and mainline

### 5.1 Coverage and version
<!-- ev:mhal-vcodec.5.1 -->

- Reference: StarFive Chips&Media vpuapi
  with the Wave5 register map from StarFive `wave511/code/vpuapi/wave/wave5_regdefine.h`; mainline Linux wave5 for names
  and product codes. The per-function comparison lists, per function, the host
  registers read / written (and FIO reads / writes) in mhal vs StarFive; it catches every changed command layout but not
  changed values in an unchanged register.
- **Name match** [R]: of the **164** library functions (VCODEC names with the vpuapi prefixes, glue excluded) **134 have
  a StarFive namesake**; **116 of those access exactly the same register set** (or none), **18 differ** (§5.3). The other
  30 are renames or SigmaStar additions (§5.2). Coda9 support and the GDI-tiled frame-buffer helpers are absent.
- API version printed by `PrintVpuProductInfo`: **5.5.58** [R, constants 5, 5, 0x3a]. The StarFive copy has no version
  macro to compare.

### 5.2 Renamed, added and removed functions [R names; I for "renamed" pairs, matched by role]

| mhal | StarFive equivalent | note |
|---|---|---|
| `InitInstPoolForCodec`, `AllocateCodecInfo`, `GetInstOfCodec`, `FreeCodecInstance`, `Vpu_GetCodecInstance`, `Vpu_ReleaseCodecInstance` | `InitCodecInstancePool`, `GetCodecInstance`, `FreeCodecInstance` | instance pool, 64 slots (index < 0x40, §3.2) instead of `MAX_NUM_INSTANCE` 16 |
| `CheckInstanceValidity`, `CheckCodecInstanceEffective`, `CheckDecOpenInfo` | `CheckEncInstanceValidity` / `CheckDecInstanceValidity`, `CheckDecOpenParam` | |
| `LockCore` / `UnlockCore`, `LockDispFlag` / `UnlockDispFlag` | `EnterLock` / `LeaveLock`, `EnterDispFlagLock` | CamOs mutexes (`codec3_lock`, `codec3_disp_lock`) |
| `PendInstance` / `UnpendInstance` / `GetPendedInstance` / `GetPendedInstanceIndex` | `SetPendingInst` / `GetPendingInst` / `GetPendingInstIdx` | |
| `TiledMapTypeSet` | `SetTiledMapType` | |
| `VPU_EncInitInfoGet`, `VPU_BitstreamBufGet`, `VPU_EncUpdateNewBitstreamBuffer`, `VPU_GiveCmdDec` | `VPU_EncGetInitialInfo`, `VPU_EncGetBitstreamBuffer`, `VPU_EncUpdateBitstreamBuffer`, `VPU_DecGiveCommand` | |
| `ProductVpuDecGetVlcInfo` / `ProductVpuDecUpdateVlcInfo`, `Wave5VpuDecGetVlcInfo` / `Wave5VpuDecUpdateVlcInfo` | — | SigmaStar: decoder VLC buffer hand-over (mi_vdec `AddVlcBuffer`) |
| `Wave5VpuDecSetRdPtr`, `Wave5UpdateLinearInfo`, `Wave5VpuFlush`, `Wave5VpuQueryStatus`, `Wave5VpuDumpBuffer`, `InitializeDebugEnv` | — | additions: decoder read-pointer set, linear (non-compressed) output info, flush, queue status query (selector 0x46), debug |
| — | `SetupWave5Properties` | inlined as `SetupWave5Properties` (tail of `Wave5VpuInit`), §4.5 |
| — | `Wave5VpuDecUpdateFramebuffer`, `Wave5VpuEncSetLarcData`, `VPU_Init`, `InitializeVPU`, Coda9 and GDI V1 / V2 tiling | removed |

### 5.3 Functions whose register usage differs [R registers; names from mainline / StarFive regdefine [S]; reading [I] unless it points to §4]

| function | only in mhal | only in StarFive | reading |
|---|---|---|---|
| `WaveVpuGetProductId` | — | R 0x1044 `PRODUCT_NUMBER` | hard-wired `return 6` (§4.6) |
| `Wave5VpuInit` | W 0x134–0x164 (13 regs), 0x12c, interrupt enable 0xbd40 | — | the init block of §4.2 [R] |
| `Wave5VpuSleepWake` | W 0x134–0x15c (7) | — | the same block re-sent on wake |
| `Wave5VpuBuildUpEncParam` | W 0x130 (= 0), 0x140 / 0x144 (SRAM base / size), 0x148 | — | `CREATE_INSTANCE` carries the SRAM window per instance; CQ depth 3 (StarFive 4) |
| `Wave5VpuBuildUpDecParam` | W 0x130 | W 0x194, R 0x1044 | |
| `Wave5VpuEncInitSeq` | W 0x180, 0x1bc–0x1c8 | — | SigmaStar RC extensions after `LOOK_AHEAD_RC` (AVBR / HVS QP / MB-level RC; §7) |
| `Wave5VpuEncParaChange` | W 0x124, 0x148, 0x150, 0x180, 0x1b8–0x1c8 | — | the same extensions changeable at run time (`venc_param_ctrl`, RC SetParam); 0x188 / 0x18c carry the VUI block address / length [R] (§7.6) |
| `Wave5VpuEncode` | R/W 0x120, W 0x128, 0x130, 0x134, 0x140, 0x15c, 0x19c, 0x1a0, 0x1a4 | — | 0x120 read-modify-write (bits 7, 8 per picture: force-IDR / skip-style flags), 0x124 secondary-AXI use from the per-frame SRAM info, 0x130 / 0x134 custom map (ROI / QP map) option and address, 0x140 HEVC-only extra, 0x1a0 / 0x1a4 IMI / ring input window (bit 28 enable) — the SCL → VENC ring mode |
| `Wave5VpuEncRegisterFramebuffer` | W 0x1a4, 0x1ac, 0x1dc | W 0x134–0x140, 0x1b4, 0x1b8 | reference frames are not registered one address per buffer; one base + stride per pool — the "ring reference / inplace" mode (`use_ring_ref`); the firmware's SET_FB reads match (§4.3) [R] |
| `Wave5VpuDecRegisterFramebuffer` | W 0x1a4, 0x1ac, 0x1dc | W 0x134–0x140, 0x1b4, 0x1d4, 0x1d8 | same scheme for the decoder |
| `Wave5VpuEncGetResult` | R 0x140, 0x160 | R 0x194, 0x198 | average QP / VCL NUT kept, two StarFive report fields dropped; full register map §7.10 |
| `Wave5VpuDecode` | W 0x124, 0x154–0x180 | — | per-picture output (linear / scaled) descriptors |
| `Wave5VpuDecGetResult` | R 0x188–0x19c, 0x1a8–0x1b0 | R 0x1a0 | extra output reports |
| `Wave5VpuDecFlush` | W 0x104 | — | flush option |
| `Wave5VpuGetBwReport` | R 0x118–0x144 | — | bandwidth report filled (StarFive stub) |
| `Wave5VpuEncGiveCommand` | W 0x300 | — | mhal-only selector 0x300 (§4.3) [R] |
| `Wave5VpuWaitInterrupt` | — | W 0x4c, R/W 0x1e4 / 0x1e8 / 0x1ec, R 0x1fc | instance-register handling moved into `codec_irq_handler` (§3.2) |
| `VPU_DecFrameBufferFlush` | R 0x1d8 | — | |

### 5.4 Mainline Linux

Mainline `wave5-hw.c` is a rewrite of the same command protocol for the TI J721S2 Wave521C and the Wave515; it shares the
base register map, has none of the SigmaStar extensions above (init block layout, extended RC registers, ring-reference
frame-buffer scheme, packed instance registers in the IRQ), reads `W5_PRODUCT_NUMBER` and accepts only 0x521c / 0x5150
[R]. The mainline and StarFive firmware images cannot be loaded by this host library as they are: they carry no version
tag (→ "fw ver mismatch", refused) and exceed the 0x80000 size limit (§2.7) — StarFive `wave511/firmware/chagall.bin`
949 120 B (StarFive), TI `cnm/wave521c_k3_codec_fw.bin` 999 104 B
(linux-firmware) [R]; and they would not understand the SigmaStar init block, the packed instance
registers or the ring-reference frame buffers [I].

## 6. Throughput: frames in flight and the limits visible from the host

### 6.1 Frames in flight

- **Firmware command queue depth 3 per instance** (§4.3). Queue status is polled with `VPU_EncGiveCommand(…, 0x46)`
  before each frame; while it reports full (3), `EncodeOneFrame` **sleeps 5 ms and retries, up to 10 times**, then fails
  with 0x12 ("COMMAND_QUEUE FULL" / "retry timeout") [R, 0x400c6858]. A stalled engine therefore costs 5 ms granularity,
  not a spin.
- **Host side**: mi_venc's module parameter `max_h26x_task` (`/sys/module/mi_venc/parameters/max_h26x_task`, default 2,
  clamped 1…4, perm 0644, read once in mi_venc `InitDev`) is the number of input frames mi_venc hands to the engine
  before waiting for a done (`mi_venc.md` §4). With 2 in flight the engine can start frame n+1 while the host collects
  frame n [I for the effect size].
- `EncodeOneFrame` is asynchronous: it fills the picture parameters (`SetEncPicParam`), posts the SRAM info, an optional
  parameter change (`ENC_SET_PARA_CHANGE`, 0x42), then `VPU_EncStartOneFrame` and returns; the result is collected in
  `EncodeFrameDone` after the IRQ. Per-frame host overhead is therefore ISR + done handling, shown by `/proc/mhe/ctx_info`
  (§2.8).

### 6.2 Limits

| limit | value | where [R] | changeable |
|---|---|---|---|
| picture size | W ≤ device max W and W·H ≤ device max W·H; W multiple of 8, H of 2 | SetParam 0x80000000 / 0x80010000 | max from mi_venc `max_h26x_width` / `max_h26x_height` (4096×2176, sent with SetDevConfig 0x80030016 before CreateDevice) |
| absolute | 8192×8192 | `ProductCheckEncOpenParam` | no |
| pixel rate | 167.1 Mpx/s if the OTP flag is set | §2.3 | no |
| crop | inside the picture, X / Y multiple of 16 | SetParam 0x80000001 | per channel |
| frames in flight | firmware 3 per instance, mi_venc 2 | §6.1 | `max_h26x_task` (at mi_venc device creation) |
| SRAM (line buffers) | must be ≥ `_CalculateMaxVencSramSize(width, …)` or the frame is refused (error 0xc) | `_UpdateVencDynamicSramInfo` 0x400c11ec | sized at device creation from the max resolution |
| firmware handshake timeout | `__VPU_BUSY_TIMEOUT` (global) for register handshakes | `codec3_wait_vpu_busy` | — |
| codec clocks | vce / vcpu / axi 384 MHz default, ≤ 480 MHz | §2.2 | sysfs `ven_clock*` while no VENC device exists; above 480 MHz only with the clamp `dev+0xc0` changed in `vcodec_init_device` (the literal at 0x400b0eac) |

No level (MB/s per H.264 level), no fps table and no "max instances × resolution" budget exists in mhal [R: none in its
strings or in the SetParam / CreateInstance paths]. The level the stream carries is chosen by the firmware [I:
Chips&Media firmware picks `level_idc` when the host leaves it 0; the mhal parameter set has no level field].

### 6.3 What the camera's encoder path uses [R prog_uvc / mi_venc / mhal; I where marked]

- prog_uvc's channel attributes (`prog_uvc.md` §3.2; set at prog_uvc 0x1ec08): H.264 profile 1 → mi_venc sends id 0x8000000a
  profile_idc **77 (Main)** (0 → 66 Baseline, ≥ 2 → 100 High; `venc_h264_profile_idc` (0x1dcb4) in mi_venc); `u32BFrameNum` = 2 is set but
  **ignored**: mi_venc's attribute apply (`venc_apply_attr` (0x1df64) → resolution, profile, `venc_apply_attr_tail` (0x1de20)) never reads the B-frame
  or ref-count fields, and mhal's default GOP is IPPP with one reference (§7.12) — the stream has no B-frames. H.265:
  the profile / B fields are left from the stack and unused the same way. RC: H264CBR / H265CBR, GOP 30, bitrate by
  size.
- Slice split: prog_uvc asks {1, 15} for H.264 (15 macroblock rows per slice → 1080p = 5 slices, 4K = 9 slices) and
  {1, (H+31)/32} for H.265; mi_venc sends mhal 0x80000009 / 0x80010008 {0x14, v2, enable, rows, **ll = !bByFrame**}
  (`MI_VENC_IMPL_SetH264SliceSplit`); mhal turns it into independent slices of rows × MB-width macroblocks (`waveParam`
  +0x368 / +0x36c for AVC, +0xe0 / +0xe4 for HEVC, rows halved to 64-pixel CTU rows) and stores `ll` at instance +0x2c5.
  prog_uvc's `bByFrame = 1` makes `ll = 0`, so there are **no per-slice low-latency interrupts and the stream is collected
  once per frame**; the slices only change the bitstream (slice headers, no prediction across slice borders) [R for ll, I
  for the effect: one pipeline, no slice parallelism, no throughput gain or loss worth counting].
- Rate control cost [I]: CBR with GOP 30 adds no encoder passes. Re-encoding or dropping an oversized frame (mi_venc's
  super-frame / frame-lost handling in `_MI_VENC_DequeueInputTask` 0x24310, which sends mhal 0x8003000c / 0x8003000b,
  `mi_venc.md` §8) happens only in a super-frame mode (`SetSuperFrameCfg`) prog_uvc does not set [R for the branch, I that
  the default mode is off].
- Whether the encode time on the live H.264 stream is bound by the firmware or by the host path (the `/proc/mhe/ctx_info`
  split, §2.8), the current `ven_clock*` values and whether a 480 MHz vce clock raises the frame rate are not known — none
  of them has been read on the camera.

## 7. MHAL VENC — `MHAL_MFE_*` / `MHAL_MHE_*` / `MHAL_VENC_Wrapper_*` [R unless tagged]

### 7.1 Entry points

All 15 `MHAL_MFE_X` (0x400c81f4…0x400c84b8) and 15 `MHAL_MHE_X` (0x400c84e4…0x400c87a8) are one-line wrappers of
`MHAL_VENC_Wrapper_X`; `CreateDevice` additionally clears `g_disable_mfe` / `g_disable_mhe` (written nowhere else and never
read — dead flags, unused).

| entry | behaviour |
|---|---|
| `CreateDevice(id, &dev)` | users++; first user: the device must have the MMA callbacks (SetDevConfig 0x80030014), proc init (§2.8), MIU protect (§2.6), clocks (`venc_dev_init`, §2.2), `codec_dev_preset` (§2.4), firmware load (shared with VDEC: skipped if VDEC already loaded it, `drv+0xd0`; §2.7); `b_vpuInit` (`+0x108`) = 1; SRAM size to `+0x150` |
| `DestroyDevice(dev)` | users−−; last: bridge 1629.21 \|= 0xfe, clocks off + ramlog free, `VPU_DeInit`, MIU protect off, proc deinit, clear MMA callbacks |
| `CreateInstance(dev, codec, &inst)` | codec 0 = AVC (only if product id 6), non-zero → HEVC (std 0xc); allocates the instance (0xe48 B), its open parameters (0x498 B) and a 0x54 B work struct; `Vpu_GetCodecInstance` (64-slot pool); defaults (§7.2); registers the instance in `dev+8+4·idx`; `initSeiInfo`, `initVuiInfo`; state 1 |
| `DestoryInstance(inst)` [sic] | if state > 1 `SetParam(0x80030005)` (stream off); release the codec instance and memory |
| `SetParam` / `GetParam(inst, id, p)` | §7.3–§7.8 |
| `EncodeOneFrame(inst, in)` | needs state ≥ 6; OTP check (§2.3); per-frame SRAM (`_UpdateVencDynamicSramInfo`, §2.5); `SetEncPicParam` (§7.11); queue-full retry (§6.1); pending parameter change (`ENC_SET_PARA_CHANGE`, mask `open+0x458`); `VPU_EncStartOneFrame`; timestamps for proc |
| `EncodeFrameDone(inst, res)` | §7.10; SEI / user-data insertion (`_VenGenSeiNalu`, `_PutUserData`); updates every proc timing (§2.8) |
| `QueryBufSize(inst, q)` | zeroes `+0x18`, `+0x28`, `+0x2c`, `+0xcc` — mhal asks mi_venc for no private buffers; it allocates its own through the MMA callbacks |
| `IsrProc` | returns 0 (nothing; unused) |
| `GetEngStatus(dev, st)` | runs `codec_irq_handler` itself (polling form of §3.2) and stores the event in the instance (`+400`) |
| `EncodeAddOutBuff(dev, buf)` | finds the instance waiting on a full bitstream buffer (state 4 or 6), `VPU_EncUpdateNewBitstreamBuffer(addr, size)`, resumes. On M6 mi_venc handles a full slice area in its ISR by reserving the next ring region and does not call this for H.26x (`mi_venc.md` §7.5) |
| `EncodeCancel` (0x400c8140) | only counts (`inst+0xab4`) and returns 0 — no hardware abort (recovery is the reset path, §9) |
| `CheckInstanceStatus` (0x400c8158) | the encode-phase word `inst+0x190` (1 fired, 2, 3; anything else → 0) |

### 7.2 Instance object, open parameters, defaults
<!-- ev:mhal-vcodec.7.2 -->

- **inst** = the encoder instance (0xe48 B); **open** = its open-parameter block (`inst+8` → open, 0x498 B): StarFive
  `EncOpenParam` layout with one SigmaStar word inserted before `MESearchRange` (from there StarFive offset + 4),
  `waveParam` at `open+0xa0`. Field names of open are the StarFive names at the matching offset
  (`sf-EncWaveParam.txt`) [I where a name is given only by position].
- **Instance state** `inst+0x18c`: 1 after CreateInstance; STREAM_ON moves it 2 → 3 → 4 … 6; EncodeOneFrame needs ≥ 6.
- **Pending-change mask** `open+0x458`: a CFG-type change on an instance in state > 4 (streaming) sets a bit; the change is
  sent with the next frame as `ENC_SET_PARA_CHANGE` (0x42). Bits used: 0x2 GOP, 0x4 frame rate, 0x10 RC-advanced sizes /
  ratios / gradient, 0x20 VUI, 0x80 integer fps, 0x200 VBV window (and `venc_param_ctrl` per-channel changes), 0x400 QP
  range / AVBR knobs, 0x716 an RC-mode change.
- **`inst+0x184`**: zeroed by an RC-mode change, a GOP change, a ChangePos change, and (when module parameter
  `CHANGE_RC_MODEL_FORCE_I` ≠ 0) by a bitrate / fps / VBV change; restarts at 1 with an IDR / I picture (§7.10) [R];
  [I: the frame counter since the last I picture, so zeroing it forces an I frame].
- **Defaults** set by CreateInstance [R values, I names]: 352×288, `MESearchRange` 3, VBV 1000, frame-skip disable 1, GOP
  30, rcInitDelay? `+0x7c` 400, `+0x84` 0x6000, profile (`+0xa0`) 1 HEVC / 2 AVC, bit depth 8, decodingRefreshType 2 (IDR),
  intra QP 32, intra period 30, CU sizes 7, TMVP 1, merge 2, deblock on, cross-slice LF 1, intra trans skip 1, SAO 1,
  intraNxN 1, CU-level RC 1, HVS QP on (scale 2, max Δ 10), min / max QP 12…48 for I, P and B (`+0x328…0x33c`), GOP preset
  via `ResetgopParameter` (§7.12), `+0x460` 0x50 (ChangePos), `+0x47c` 25 (MinStillPercent), `+0x480` 100
  (MotionSensitivity), `+0x484` 5 (AVBR motion response); VUI on (`open+0x44c` = 1).

### 7.3 Per-codec parameter ids (SetParam / GetParam)

Consumer: `MHAL_VENC_Wrapper_SetParam` / `MHAL_VENC_Wrapper_GetParam`. Struct header `{u32 size, u32 version}`
(`MHAL_VENC_Version_t`); `[n]` = 32-bit word n. Ids: `0x8000_00xx` H.264, `0x8001_00xx` H.265, `0x8003_00xx` common
(§7.8). Sizes and versions [R]; names from the mi_venc callers and the I2 `mhal_venc.h` enum [S]. The H.265 ids jump
into the same code blocks as the H.264 ones (one switch at 0x400c2160 serves SetParam and GetParam).

| H.264 id | H.265 id | name | size / ver | content and checks |
|---|---|---|---|---|
| 0x80000000 | 0x80010000 | RESOLUTION | 0x14 / 1 | {W, H, fmt 0 NV12 / 1 NV21 / 2 YUYV / 3 YVYU}; W ≤ device max W, W·H ≤ max area, W % 8 = 0, H % 2 = 0; MB (16) or CTU (64) counts; padding to 8 / 16; OTP check |
| 0x80000001 | 0x80010001 | CROP | 0x1c / 1 | {enable, x, y, w, h}; inside the picture, x / y % 16 = 0; OTP check |
| 0x80000002 | 0x80010002 | REF | 0x18 / 1 | {base, enhance, bEnablePred}: enhance 0 → normal IPPP; bEnablePred && enhance 1 → TSVC-2 (custom GOP of 2, temporal layers); bEnablePred && enhance 3 && base 1 → TSVC-3 (custom GOP of 4); otherwise long-term-reference mode (LTR period enhance+1 or the intra period); changing the type on a started instance → 0x10; TSVC disables GDR |
| 0x80000003 | 0x80010003 | VUI | §7.6 | aspect ratio, video signal, timing, HRD, bitstream restriction |
| 0x80000004 | 0x80010004 | DBLK | 0x14 / 1 | {disable_deblocking_filter_idc 0–2, alpha / beta offset −6…6, tc / beta offset −6…6} |
| 0x80000005 | — | ENTROPY | 0x10 / 1 | {modeI, modeP}: must be equal; CABAC refused for Baseline |
| 0x80000006 | 0x80010005 | TRANS | 0x14 / 1 (H.265 own case) | chroma QP index offset −12…12 |
| 0x80000007 | 0x80010006 | INTRA_PRED | — | accepted, ignored (returns 0) |
| 0x80000008 | 0x80010007 | INTER_PRED | — | accepted, ignored |
| 0x80000009 | 0x80010008 | I_SPLIT_CTL | 0x14 / **2** | {bEnable u8, rowCount, ll u8} (§6.3) |
| 0x8000000a | — | PROFILE | 0xc / 1 | profile_idc 66 → Baseline, 77 → Main, other → High; Baseline forces CAVLC |
| 0x8000000b | 0x80010009 | ROI | 0x28 / **2** | mode 0 area {index < 16, enable, abs-QP (refused), QP −32…31, x, y, w, h aligned}, mode 1 QP map {enable, …}, mode 2 background fps {src, dst < src} (not with QP map / CQP) |
| 0x8000000c | 0x8001000a | RC | 0xb0 / **2** | §7.4 |
| 0x8000000d | 0x8001000b | RC advanced | 0x34 / 1 | §7.5 |
| 0x8000000e | 0x8001000c | USER_DATA | 0x10 / 1 | {buf, len 1…1024}; up to 4 queued ("Cnt overflow max 4") |
| 0x8000000f | 0x8001000e | SEI | 0x2c / 1 (H.264), 0x60 / 1 (H.265) | §7.7 |
| — | 0x8001000d | — | — | unsupported (error 8) |

GetParam mirrors these with the same sizes (H.264 GetParam table index = id − 0x80000000, 16 entries). mi_venc passes every
field of its H264InterPred / H264IntraPred / H265InterPred structures, but mhal ignores the INTRA_PRED and INTER_PRED ids.

### 7.4 RC — ids 0x8000000c (H.264) / 0x8001000a (H.265), `{0xb0, 2}`

Size 0xb0 = 44 words. Writer: mi_venc `S_MI_VENC_ConvertRc` (`mi_venc.md` §6.2); reader: the case at `caseD_8000000c`
(0x400c3642 + table).

| word | content |
|---|---|
| [0], [1] | size 0xb0, version 2 (anything else → 3, "parameter size … does not match") |
| [2] | `eRcMode`: **1 CBR, 2 VBR, 3 AVBR, 4 UBR, 5 FIXQP** for H.264; **6 CBR, 7 VBR, 8 AVBR, 9 UBR, 10 FIXQP** for H.265 [R codes; names from the log strings "Cbr/Vbr/Avbr/Ubr invalid …", "fix qp" and from mi_venc's mapping]. mi_venc never sends 4 or 9 |
| [3]…[0x27] | mode union (37 words; its size is set by the MJPEG member of the same struct — Qfactor, two 64-byte Q tables, fps, max / min Qfactor — used by JPE id 0x80020003, `mhal-scl-jpe-cmdq.md` §4.5) |
| [0x28]…[0x2b] | `au32ThrdI[1]`, `au32ThrdP[1]`, `u32RowQpDelta`, `pu8PerceptureQpMap` by the I2 header [S]; **never read** by the H.26x case and never written by mi_venc — unused |

The mode code is not checked against the instance codec: an H.264 instance accepts codes 6–10 and vice versa.

Per mode (word → check → destination):

| mode | [3] | [4] | [5] | [6] | [7] | [8] | [9] |
|---|---|---|---|---|---|---|---|
| CBR 1/6 | Gop 1…0xffff → `open+0xc8` (`waveParam.intraPeriod`) | StatTime — **not read** | FpsNum (not checked) | FpsDen (not checked; 0 reaches a division) | BitRate ≠ 0 → `open+0x90` (`bitRate`) | FluctuateLevel — **not read** | — |
| VBR 2/7 | Gop | — (not read) | FpsNum ≠ 0 | FpsDen ≠ 0 | MaxBitRate ≠ 0 → `open+0x90`, but only while `open+0x460` (ChangePos) is 5…100 | MaxQp | MinQp |
| AVBR 3/8 | as VBR | — | | | as VBR | MaxQp | MinQp |
| UBR 4/9 | Gop | — | FpsNum ≠ 0 | FpsDen ≠ 0 | BitRate (0 allowed) → `open+0x90` | MaxQp | MinQp |
| FIXQP 5/10 | Gop | FpsNum ≠ 0 | FpsDen ≠ 0 | IQp 8…48 → `open+0xc4` (`waveParam.intraQP`) | PQp 8…48 → `open+0xcc` | — | — |

Common handling:
- **Rate-control switch**: `open+0x9c` (`rcEnable`) = 1 for CBR / VBR / AVBR / UBR, 0 for FIXQP; SigmaStar sub-mode
  `open+0x488` = 0 CBR, 1 VBR, 2 AVBR, 3 UBR, −1 FIXQP; a change of either sets mask **0x716** and zeroes `inst+0x184`;
  `inst+0x4cc` = copy of the sub-mode (read by GetParam). `open+0x35c` = 0 after CBR, 1 after VBR / AVBR / UBR / FIXQP
  [R]; its meaning for the firmware is not known [I: a "variable bitrate" flag]. `open+0xcc` holds the FIXQP P-QP;
  StarFive has `forcedIdrHeaderEnable` at that offset — how the firmware reads it is not known.
- **GOP** change → `open+0xc8`, mask 0x2, `inst+0x184` = 0; with long-term reference on (`open+0x294`,
  `waveParam.useLongTerm`) and `inst+0x100` = 0 also the LTR period `inst+0xf8` = GOP.
- **Frame rate** → `open+0x20` (`frameRateInfo`) = `num | (den − 1) << 16`; while `num` or `den` exceeds 16 bits both are
  halved ("fps changed to %d/%d"); change → mask 0x4. CBR and FIXQP also store `open+0x24c` = num·1000/den.
- **VBV / CPB window** (CBR, VBR, AVBR; not UBR, not FIXQP) → `open+0x3c` (`vbvBufferSize`, ms) = clip(5 frame periods,
  25000, GOP length in ms) with frame period = den·1000/num; change → mask 0x200.
- **QP range** (VBR / AVBR / UBR): MaxQp and MinQp must lie in 12…48 with Max ≥ Min, else a warning and both clipped (Max
  to 12…48, Min to 12…Max); MaxQp → `open+0x32c/0x334/0x33c` (`maxQpI/P/B`), MinQp → `open+0x328/0x330/0x338`
  (`minQpI/P/B`) — one value for I, P and B; change → mask 0x400.
- **Integer fps** `open+0x24` = num / den (0 → 1), change → mask 0x80 ("realtime fps changed to %d" when ring input
  `open+0x428` is on).
- **VUI timing follow-up**: when the instance's VUI carries timing (`inst+0xaf0` ≠ 0, §7.6) the timing is recomputed:
  H.264 `num_units_in_tick` 1001, `time_scale` = 2002·num/den; H.265 1001 and 1001·num/den (`inst+0xaf4/+0xaf8`); with
  mask bit 0x4 set the VUI is regenerated (`venc_regen_vui` (0x400c0490)).
- A bitrate / fps / VBV change zeroes `inst+0x184` only when module parameter `CHANGE_RC_MODEL_FORCE_I` ≠ 0 (default 1,
  `mhal-core.md` §5).
- FIXQP on an instance with background-fps ROI (`inst+0xe8` ≠ 0) logs "CQP not support BGFps" and clears
  `inst+0xe8/+0xec`.
- Last step: the OTP pixel-rate check `venc_check_res_rate` (§2.3) → 8 when over the cap.
- Errors: GOP / bitrate / fps out of range → 3 with "Cbr invalid parameter, gop:%d, bit rate:%d", "Vbr/Avbr invalid
  Gop=… u32MaxBitRate=… u32SrcFrmRateDen=… u32SrcFrmRateNum=…", "Ubr invalid Gop=…", "fix qp:%d %d %d %d %d" (IQp, PQp,
  num, den, gop).

**GetParam** (same id, same size / version) rebuilds the struct from open: `rcEnable` = 1 → mode from the sub-mode (0 →
1/6, 1 → 2/7, 2 → 3/8, 3 → 4/9); [3] GOP, **[4] = 0**, [5] num (`open+0x20` low 16), [6] den, [7] `bitRate`; CBR: [8] = 0;
VBR / AVBR / UBR: [8] `maxQpP`, [9] `minQpP`. `rcEnable` = 0 → mode 5/10, [3] GOP, [4] num, [5] den, [6] IQp, [7] PQp.
StatTime and FluctuateLevel therefore read back as 0.

### 7.5 RC advanced — ids 0x8000000d (H.264) / 0x8001000b (H.265), `{0x34, 1}`

13 words: [0] size, [1] version, [2] sub-mode, [3]…[12] mode union. Writer: mi_venc `S_MI_VENC_SetRcParam`, which sends
GetParam first and then SetParam (`mi_venc.md` §6.3); the I2 header `mhal_venc.h` has no such struct [R].
Sub-mode codes: 1 / 4 CBR, 2 / 5 VBR, 3 / 6 AVBR (H.264 / H.265); the code is not checked against the codec, and any other
value only resets `open+0x460` to 0x50 and returns 0.

| word | CBR 1/4 | VBR 2/5 | AVBR 3/6 |
|---|---|---|---|
| [3] | MaxQp (P/B) → `maxQpP`, `maxQpB` | IPQPDelta (s8) → `open+0x464` | IPQPDelta → `open+0x464` |
| [4] | MinQp (P/B) → `minQpP`, `minQpB` | ChangePos 5…100 → `open+0x460` | ChangePos 5…100 → `open+0x460` |
| [5] | IPQPDelta (s8) → `open+0x464` | MinIQp → `minQpI` | MinIQp → `minQpI` |
| [6] | MaxIQp → `maxQpI` | MaxIPProp ≤ 256 → `open+0x478` | MaxIPProp → `open+0x478` |
| [7] | MinIQp → `minQpI` | MaxIQp → `maxQpI` | MaxIQp → `maxQpI` |
| [8] | MaxIPProp ≤ 256 → `open+0x478` | MaxISize → `open+0x470` | MaxISize → `open+0x470` |
| [9] | MaxISize → `open+0x470` | MaxPSize → `open+0x474` | MaxPSize → `open+0x474` |
| [10] | MaxPSize → `open+0x474` | — | MinStillPercent 5…100 → `open+0x47c` |
| [11] | — | — | MaxStillQp — **not read** (GetParam returns 0) |
| [12] | — | — | MotionSensitivity ≤ 100 → `open+0x480` |

Checks and side effects:
- CBR is refused (3, "bit rate control is closed now when set rc_advance for cbr, please open it first") when `rcEnable`
  = 0 (FIXQP active); VBR / AVBR are not checked against `rcEnable`.
- QP pairs: values outside 12…48 or max < min → warning and clip (max to 12…48, min to 12…max); CBR checks both the P pair
  and the I pair, VBR / AVBR only the I pair.
- Max I / P frame size > 0x0fffffff → warning, clipped to 0x0fffffff. MaxIPProp > 256 → warning, set to 256.
- ChangePos outside 5…100 (VBR / AVBR), MinStillPercent outside 5…100 or MotionSensitivity > 100 (AVBR) → 3 with
  "Vbr/Avbr invalid parameters:s32IPQPDelta=…, s32ChangePos=…[5,100],u32MaxIPProp=…".
- Module-parameter knobs copied on every call (definitions and defaults `mhal-core.md` §5): `GRADIENT_MODEL_LEVEL` →
  `open+0x490` (all three modes); AVBR only `AVBR_MOTION_RESPONSE_LEVEL` → `+0x484`, `AVBR_MD_DOWN_SPEED_RATIO` → `+0x485`,
  `AVBR_STILL_BITRATE_COMPENSATE` → `+0x486`. The gradient and motion-response values thus come from the parameters, not
  from the struct.
- Mask bits: frame sizes, MaxIPProp, IPQPDelta, ChangePos, MinStillPercent, gradient → 0x10; QP values,
  MotionSensitivity, the three AVBR knobs → 0x400. A ChangePos change also zeroes `inst+0x184`.
- After a CBR call `open+0x460` is reset to 0x50 (80) — so the VBR / AVBR rule "MaxBitRate is taken only while ChangePos is
  5…100" (§7.4) is always met after CBR [R for the store; I for the purpose].
- GetParam returns the same words from these fields (CBR [3] `maxQpP`, [4] `minQpP`, [5] delta sign-extended, [6]
  `maxQpI`, [7] `minQpI`, [8] prop, [9] / [10] sizes; VBR / AVBR in their own order, AVBR [11] = 0).

**The MI-side RC blocks.** The IQ-server API ids 0x3103 / 0x3104 (`isp-6-userspace.md` §6.5.3) move the MI structs of the M6
header (`mi_venc_datatype.h` 618–846) [S], and every size the service checks matches them [R sizes vs S header]:
0x3103 = `MI_VENC_RcAttr_t` (36 bytes at `MI_VENC_ChnAttr_t` +0x28…+0x4b: +0x00 `eRcMode` — 1 H264CBR, 2 H264VBR, 3
H264ABR, 4 H264FIXQP, 5 H264AVBR, 6 MJPEGCBR, 7 MJPEGFIXQP, 8 H265CBR, 9 H265VBR, 10 H265FIXQP, 11 H265AVBR —, +0x04 the
mode union (7 words max), +0x20 `pRcAttr`, a pointer, unused); 0x3104 = `MI_VENC_RcParam_t` (56 bytes: +0x00
`au32ThrdI[1]`, +0x04 `au32ThrdP[1]`, +0x08 `u32RowQpDelta` — these three are not passed on by mi_venc —, +0x0c the mode
union (10 words max), +0x34 `pRcParam`). Union members (word order):

| struct | mode | size the service checks | fields |
|---|---|---|---|
| RcAttr | CBR 1 / 8 | 24 | u32Gop, u32StatTime, u32SrcFrmRateNum, u32SrcFrmRateDen, u32BitRate, u32FluctuateLevel |
| RcAttr | VBR 2 / 9, AVBR 5 / 11 | 28 | u32Gop, u32StatTime, u32SrcFrmRateNum, u32SrcFrmRateDen, u32MaxBitRate, u32MaxQp, u32MinQp |
| RcAttr | FIXQP 4 / 10 | 20 | u32Gop, u32SrcFrmRateNum, u32SrcFrmRateDen, u32IQp, u32PQp |
| RcAttr | ABR 3 | ("not support") | u32Gop, u32StatTime, u32SrcFrmRateNum, u32SrcFrmRateDen, u32AvgBitRate, u32MaxBitRate — mi_venc refuses mode 3 |
| RcAttr | MJPEG CBR 6 / FIXQP 7 | ("not support") | u32BitRate, u32SrcFrmRateNum, u32SrcFrmRateDen / u32SrcFrmRateNum, u32SrcFrmRateDen, u32Qfactor |
| RcParam | CBR 1 / 8 | 32 | u32MaxQp, u32MinQp, s32IPQPDelta, u32MaxIQp, u32MinIQp, u32MaxIPProp, u32MaxISize, u32MaxPSize |
| RcParam | VBR 2 / 9 | 28 | s32IPQPDelta, s32ChangePos, u32MaxIQp, u32MinIQp, u32MaxIPProp, u32MaxISize, u32MaxPSize |
| RcParam | AVBR 5 / 11 | 40 | s32IPQPDelta, s32ChangePos, u32MinIQp, u32MaxIPProp, u32MaxIQp, u32MaxISize, u32MaxPSize, u32MinStillPercent, u32MaxStillQp, u32MotionSensitivity |
| RcParam | MJPEG CBR 6 | (0) | u32MaxQfactor, u32MinQfactor |

The MI VBR member's order (MaxIQp before MinIQp) differs from the mhal RC-advanced word order (MinIQp, MaxIPProp, MaxIQp);
mi_venc's `S_MI_VENC_SetRcParam` re-orders them (`mi_venc.md` §6.3). AVBR `u32MaxStillQp` reaches mhal but is never read.

### 7.6 VUI — ids 0x80000003 (H.264) / 0x80010003 (H.265)

**Sizes**: SetParam checks neither size nor version (the case only makes sure the instance buffer exists, "vui buf
error" otherwise); GetParam requires **H.264 `{0x70, 2}`**, **H.265 `{0x120, 1}`**. mi_venc builds exactly these (`chn+0x59c`,
0x70 B, version 2 / `chn+0x60c`, 0x120 B, version 1) and fills only the
aspect-ratio, timing and video-signal groups from `MI_VENC_ParamH26xVui_t` — every HRD and bitstream-restriction field below
arrives **zero** from mi_venc (unused on this camera, specified for completeness). The M6 MI header has only those three
groups (`mi_venc_datatype.h` 497–567) [S]; the I2 mhal header has the same three groups and no HRD [S].

**Where the values go**: SetParam copies the fields into the instance's VUI parameter block at **`inst+0xabc`** (H.264
0x110 B, H.265 0x164 B used, 0x170 B copied), then `venc_regen_vui` copies that block raw into the first part of the
instance's 0x1000-byte "ven_inst_<idx>" buffer (`inst+0x164` phys / `+0x168` VA, length `inst+0x160` = 0x110 / 0x170),
flushes it, and sets `open+0x44c` = 1 (VUI on; also set at CreateInstance), `open+0x450` = length, `open+0x454` = address,
mask **0x20**. `Wave5VpuEncParaChange` then writes command-window registers **0x188 = address, 0x18c = length** (both 0
when `open+0x44c` = 0 or mask 0x20 is clear). The firmware receives a parameter block, not a ready RBSP, and writes the
VUI / HRD itself (strings `hevc_encode_vui`, `write_hrd_parameters`, `write_sei_buffering_period`, `write_sei_pic_timing` in
`chagall.bin`) [R strings; I that these are its consumers]. The block layout is therefore the firmware's own; the names
below are the H.264 / H.265 Annex E syntax elements assigned **by order and width** [I unless a name is given by a header].

If `timing_info_present` ≠ 0 and `num_units_in_tick` or `time_scale` is 0, mhal fills them from the RC frame rate: H.264
1001 and 2·1001·num/den, H.265 1001 and 1001·num/den (the RC case keeps them current, §7.4).

**H.264 struct (0x70 B)** — param offset → instance field (default from `initVuiInfo`) [R offsets / defaults]:

| param | type | inst | default | name |
|---|---|---|---|---|
| +0x08 | u8 | +0xabc | 0 | aspect_ratio_info_present_flag [S I2 / MI] |
| +0x09 | u8 | +0xac0 | 1 | aspect_ratio_idc [S] |
| +0x0a | u8 | +0xacc | 0 | overscan_info_present_flag [S] |
| +0x0b | u8 | +0xad0 | 0 | overscan_appropriate_flag [S] |
| +0x0c / +0x0e | u16 | +0xac4 / +0xac8 | 1 / 1 | sar_width / sar_height [S] |
| +0x10 | u8 | +0xaf0 | **1** | timing_info_present_flag [S] |
| +0x11 | u8 | +0xafc | 0 | fixed_frame_rate_flag [S] |
| +0x14 / +0x18 | u32 | +0xaf4 / +0xaf8 | 0 / 0 (→ auto) | num_units_in_tick / time_scale [S] |
| +0x1c…+0x22 | u8 ×7 | +0xad4, +0xad8, +0xadc, +0xae0, +0xae4, +0xae8, +0xaec | 0, **5**, **1**, 0, 1, 1, 1 | video_signal_type_present, video_format, video_full_range, colour_description_present, colour_primaries, transfer_characteristics, matrix_coefficients [S] |
| +0x24 / +0x25 | u8 | +0xb04 / +0xb08 | 0 / 0 | nal_hrd_parameters_present_flag / vcl_hrd_parameters_present_flag [I] |
| +0x28 / +0x2c | u32 | +0xb0c / +0xb10 | 0 / 2 | two of cpb_cnt_minus1, bit_rate_scale, cpb_size_scale — which two, and in which order, is not known [I] |
| +0x30…+0x3c | u32 ×4 | +0xb14… | [0] 0x5b8c | bit_rate_value_minus1[SchedSelIdx 0–3] [I] |
| +0x40…+0x4c | u32 ×4 | +0xb24… | [0] 0xb70 | cpb_size_value_minus1[0–3] [I] |
| +0x50…+0x53 | u8 ×4 | +0xb34… (u32 each) | 0 | cbr_flag[0–3] [I] |
| +0x54 / +0x58 / +0x5c | u32 & 0x1f | +0xb44 / +0xb48 / +0xb4c | 23 / 23 / 23 | initial_cpb_removal_delay_length_minus1, cpb_removal_delay_length_minus1, dpb_output_delay_length_minus1 (5-bit, masked) [I] |
| — | — | +0xb50 | 24 | time_offset_length — not settable [I] |
| +0x60 | u8 | +0xba4 | 0 | low_delay_hrd_flag [I] (`+0xba8`, default 0, not settable: pic_struct_present_flag [I]) |
| +0x64 / +0x65 | u8 | +0xbac / +0xbb0 | 0 / 0 | bitstream_restriction_flag / motion_vectors_over_pic_boundaries_flag [I] |
| +0x68 / +0x6c | u32 | +0xbb4 / +0xbb8 | 0 / 0 | max_bytes_per_pic_denom / max_bits_per_mb_denom [I] |
| — | — | +0xbbc, +0xbc0, +0xbc4 | 6, 7, 0 | log2_max_mv_length_horizontal / _vertical, max_num_reorder_frames — not settable [I] |
| — | — | +0xbc8 | 1 | max_dec_frame_buffering [I]; SetParam rewrites it to 2 when `inst+0xab8` = 2, else 1 [R] |

Bytes of the H.264 struct never read (padding): +0x12–0x13, +0x23, +0x26–0x27, +0x61–0x63, +0x66–0x67.

**H.265 struct (0x120 B)** [R offsets / defaults]:

| param | type | inst | default | name |
|---|---|---|---|---|
| +0x08…+0x0f | as H.264 | as H.264 | as H.264 | aspect ratio / overscan / SAR [S] |
| +0x10 | u8 | +0xaf0 | 1 | vui_timing_info_present_flag [S] |
| +0x14 / +0x18 | u32 | +0xaf4 / +0xaf8 | 0 / 0 (→ auto) | vui_num_units_in_tick / vui_time_scale [S] |
| +0x1c | u8 | byte +0xb00 | 0 | vui_poc_proportional_to_timing_flag [I] |
| +0x20 | u32 | +0xafc | 0 | vui_num_ticks_poc_diff_one_minus1 [S I2 `u32NumTicksPocDiffOneMinus1`] |
| +0x24 | u8 | byte +0xb01 | 0 | vui_hrd_parameters_present_flag [I] |
| +0x28…+0x2e | u8 ×7 | +0xad4…+0xaec | 0, 5, 1, 0, 1, 1, 1 | video signal group [S] |
| +0x30 / +0x31 | u8 | byte +0xb28 / +0xb29 | 0 | nal_ / vcl_hrd_parameters_present_flag [I] |
| +0x32 | u8 | byte +0xb2a | 0 | sub_pic_hrd_params_present_flag [I] — this and every field down to +0x10f is copied only when +0x30 or +0x31 is non-zero [R] |
| +0x34 | u32 | +0xb2c | 0 | tick_divisor_minus2 [I] |
| +0x38 | u32 & 0x1f | +0xb30 | 23 | du_cpb_removal_delay_increment_length_minus1 [I] |
| +0x3c | u8 | byte +0xb34 | 0 | sub_pic_cpb_params_in_pic_timing_sei_flag [I] |
| +0x40 | u32 & 0x1f | +0xb38 | 23 | dpb_output_delay_du_length_minus1 [I] |
| +0x44 / +0x48 / +0x4c | u32 | +0xb3c / +0xb40 / +0xb44 | 0x40 / 0x40 / 0 | bit_rate_scale / cpb_size_scale / cpb_size_du_scale [I]; the 0x40 defaults do not fit 4-bit fields — the meaning of the stored value is not known |
| +0x50 / +0x54 / +0x58 | u32 & 0x1f | +0xb48 / +0xb4c / +0xb50 | 23 ×3 | initial_cpb_removal_delay_length_minus1, au_cpb_removal_delay_length_minus1, dpb_output_delay_length_minus1 [I] |
| +0x5c…+0x5f | u8 ×4 | bytes +0xb54… | 1 | fixed_pic_rate_general_flag[sub-layer 0–3] [I] |
| +0x60…+0x63 | u8 ×4 | bytes +0xb58… | 1 | fixed_pic_rate_within_cvs_flag[0–3] [I] |
| +0x64…+0x70 | u32 ×4 | +0xb5c… | 0 | elemental_duration_in_tc_minus1[0–3] [I] |
| +0x74…+0x77 | u8 ×4 | bytes +0xb6c… | 0 | low_delay_hrd_flag[0–3] [I] |
| +0x78…+0x84 | u32 ×4 | +0xb70… | 1 | cpb_cnt_minus1[0–3] [I] |
| +0x88 / +0x98 / +0xa8 / +0xb8 | u32 ×4 each | +0xb80 / +0xb90 / +0xba0 / +0xbb0 | [0] 0x3d09, [1] 0x7a12 / [0] 0x5eec, [1] 0x2f76 / 0 / [0] 0x3d09, [1] 0x7a12 | NAL sub-layer HRD: bit_rate_value_minus1, cpb_size_value_minus1, cpb_size_du_value_minus1, bit_rate_du_value_minus1 [I] |
| +0xc8…+0xcb | u8 ×4 | bytes +0xbc0… | [0] 0, [1] 1 | NAL cbr_flag [I] |
| +0xcc / +0xdc / +0xec / +0xfc | u32 ×4 each | +0xbc4 / +0xbd4 / +0xbe4 / +0xbf4 | 0 | VCL sub-layer HRD, same four arrays [I] |
| +0x10c…+0x10f | u8 ×4 | bytes +0xc04… | 0 | VCL cbr_flag [I] |
| +0x110 | u8 | byte +0xc08 | 0 | bitstream_restriction_flag [I] |
| +0x111 / +0x112 / +0x113 | u8 | bytes +0xc09 / +0xc0a / +0xc0b | 1 / 1 / 1 | tiles_fixed_structure_flag, motion_vectors_over_pic_boundaries_flag, restricted_ref_pic_lists_flag [I] |
| +0x114 / +0x118 / +0x11c | u32 | +0xc0c / +0xc10 / +0xc14 | 0 | min_spatial_segmentation_idc, max_bytes_per_pic_denom, max_bits_per_min_cu_denom [I] |
| — | — | +0xc18 / +0xc1c | 7 / 6 | log2_max_mv_length_horizontal / _vertical — not settable [I] |

The four-entry arrays: the HRD defaults (cpb_cnt_minus1 = 1 with two different bit rates and CPB sizes at index 0 / 1 and
cbr_flag 0 / 1) fit "index = SchedSelIdx, one set shared by all sub-layers" better than "index = sub-layer" [I]; either way
at most four entries exist. GetParam returns every field of both structs from the instance block. Which syntax element
each [I] field is — and the two H.264 words +0x28 / +0x2c — would be settled by the firmware's VUI writer
(`hevc_encode_vui`, `write_hrd_parameters` in `chagall.bin`); not known.

### 7.7 SEI — ids 0x8000000f `{0x2c, 1}` (H.264) / 0x8001000e `{0x60, 1}` (H.265)

SetParam needs the instance SEI buffer (`inst+0x170` phys / `+0x174` VA, the 0xe0-byte part of "ven_inst_<idx>" after the
VUI area; "sei buffer error" / "vbSeiMessageBuf not init" otherwise — both return 0, i.e. silently ignored); size /
version mismatch → 3. `[2]` = payload type: **0 = buffering period, 1 = picture timing**; any other → "Unsupport Sei
payLoad Type:%d" (returns 0). The values go to the instance SEI block (`inst+0xc20`, cleared by `initSeiInfo`: 0x3c B
H.264, 0xd8 B H.265), then `venc_copy_sei` (0x400c03c2) copies the relevant part into the SEI buffer and flushes it. Every frame,
`SetEncPicParam` passes the buffer to the firmware in the picture parameters (EncParam `+0xf4` bit 0 enable, `+0xf6` u16
size 0xe0, `+0xf8` address), next to the user-data SEI area (`+0xec` bit 0, `+0xee` length, `+0xf0` address = the
0x620-B area filled by `_PutUserData`, id 0x8000000e). The firmware writes the SEI NAL itself
(`write_sei_buffering_period`, `write_sei_pic_timing`, `h265_write_sei` in `chagall.bin` strings) [R strings; I consumer].
Nothing in mi_venc sends these ids (only USER_DATA) — **unused on this camera** [R: no caller with 0x8000000f /
0x8001000e in mi_venc].

Names are the Annex D (H.264) / Annex D (H.265) syntax elements matched by order and type [I]:

| struct | type | words | instance field | SEI buffer |
|---|---|---|---|---|
| H.264 0x2c | 0 BP | [3]…[6] initial_cpb_removal_delay[SchedSelIdx 0–3]; [7]…[10] initial_cpb_removal_delay_offset[0–3] | byte `+0xc24` = 1 (present), `+0xc28…+0xc44` | +0x04…+0x27 (0x24 B: present word + 8 words) |
| H.264 0x2c | 1 PT | [3] cpb_removal_delay; [4] dpb_output_delay; [5]…[10] unused | byte `+0xc48` = 1, `+0xc4c`, `+0xc50`, `+0xc54` = 0 | +0x28…+0x37 (0x10 B) |
| H.265 0x60 | 0 BP | [3] u8 irap_cpb_params_present_flag; [4] cpb_delay_offset; [5] dpb_delay_offset; [6] u8 concatenation_flag; [7] au_cpb_removal_delay_delta_minus1; [8]…[11] initial_cpb_removal_delay[0–3]; [12]…[15] initial_cpb_removal_offset[0–3]; [16]…[19] initial_alt_cpb_removal_delay[0–3]; [20]…[23] initial_alt_cpb_removal_offset[0–3] | byte `+0xc24` = 1, byte `+0xc25`, `+0xc28`, `+0xc2c`, byte `+0xc30`, `+0xc34`, `+0xc38…+0xc74` | +0x04…+0x9b (0x98 B) |
| H.265 0x60 | 1 PT | [3] au_cpb_removal_delay_minus1; [4] pic_dpb_output_delay; [5] pic_dpb_output_du_delay; [6] num_decoding_units_minus1; [7] u8 du_common_cpb_removal_delay_flag; [8] du_common_cpb_removal_delay_increment_minus1; [9]…[12] num_nalus_in_du_minus1[0–3]; [13]…[16] du_cpb_removal_delay_increment_minus1[0–3]; [17]…[23] unused | `+0xcbc` = 1, `+0xcc0…+0xccc`, byte `+0xcd0`, `+0xcd4`, `+0xcd8…+0xcf4` | +0x9c…+0xd7 (0x3c B) |

Word +0x00 of the SEI buffer is never written by mhal. H.265 picture timing carries no pic_struct / source_scan_type /
duplicate_flag (no frame-field info) [I from the layout]; at most four decoding units / four CPB specs [R array sizes].
GetParam (H.264 `{0x2c, 1}`, H.265 `{0x60, 1}`) returns the selected type's words from the instance block.

### 7.8 Common ids (`0x8003_00xx`)

SetParam table index = id − 0x80030004; all other ids in 0x80030001…0x80030025 are accepted and ignored, beyond → 8.

| id | name (I2 enum / mi_venc caller) | size / ver | effect |
|---|---|---|---|
| 0x80030004 | STREAM_ON (`MI_VENC_IMPL_StartRecvPicEx`) | internal buffer | allocates the work buffer "ven_work" and task buffers ("ven_task"), `VPU_EncOpen`, SRAM info (`GiveCommand 0xe`), `0x49`, `VPU_EncIssueSeqInit` / `CompleteSeqInit` (retry while 0x19 = queue full), frame buffers; state 2 → 3 → 4 … 6. Fills the instance's **0x3c-byte firmware parameter area** `inst+0xcf8` (flushed after the stores): +0x34 u16 min-QP scaling (`g_mheMinqpScaling` / `g_mfeMinqpScaling`, §2.8), +0x36 u16 `RC_SCENE_RECOVER_PATCH`, +0x38 u16 `RC_BITRATE_TOLERANCE` clipped to 0…15, +0x3a u8 `CHANGE_RC_MODEL_FORCE_I` (module parameters, `mhal-core.md` §5) [R]; the parameter `AVBR_SAD_THD` stays unread. "ven_work" has two sizes by where it comes from: with the MIU-protect codec heap valid (`GetMhalVcodecAttr()` +0x40) it is taken from that heap by `_codec3_allocate_MMA_alloc` at **0x31000 B** (size in `inst+0xcfc`); if that heap is absent or out of memory ("codec heap no mem, use mma heap") it falls back to `codec3_allocate_dma_memory` at **0x21000 B** (descriptor at device `+0x15a0`); either way the parameter area is VA + 0x20000, the last 0x1000 of the 0x21000 form [R] |
| 0x80030005 | STREAM_OFF | — | close sequence, free buffers (`vbVuiBuf` …) |
| 0x80030007, 0x8003000b, 0x80030011 | — | — | return 0 without action; mi_venc's EnableIdr sends 0x80030007, which therefore has no effect on M6 |
| 0x8003000c | frame re-encode / QP adjust (mi_venc super-frame path) | 0x14 / 1 | {flag u8 → `inst+0x4e0`, two words → `open+0x468` / `+0x46c`} unless TSVC, ll or ring input |
| 0x8003000d | (StartRecvPicEx) | 0xc / 1 | non-zero value → `inst+0x4d0` |
| 0x8003000f | "ENABLE_IDR" in the I2 enum [S]; its only sender, mi_venc, puts its `use_ring_ref` flag ("mhal inplace mode") in it [R] | 0xc / 1 | u8 → `inst+0x4c8`; 0 also resets the IDR / LTR state |
| 0x80030010 | INTRA_REFRESH (`Set/GetIntraRefresh`) | 0x14 / 1 | {enable, rows}: rows·intraPeriod < MB rows; `waveParam` intraRefreshMode / Arg; refused in TSVC |
| 0x80030015 | input source (`StartRecvPicEx`) | 0x10 / 1 | mode 2 → ring input (`open+0x428` = 1, sub-mode 1…3 → `+0x42c`); else frame input |
| 0x80030018 | custom map buffer | 0x20 / 1 | QP / mode map address ("plz set customer map buf without encoding") |
| 0x8003001b | SMART_DET (`SetSmartDetInfo`) | 0x28 / 2 | sums up to 16 u16 motion values; ≤ 256 → RC still-percent 100 − sum·100/256 for 15 frames |
| 0x80030025 | FRC changed (`_MI_VENC_OnInputPortFrcChanged`) | 0xc / 1 | realtime fps mode 0…2 → `open+0x24` (0 → 1) |

GetParam mirrors these (same sizes; RC returns mode codes 1–5 / 6–10 from `rcEnable` + the sub-mode at `inst+0x4cc`),
plus `0x80030022` {0x14, 1} (read-only info) and the device queries of §7.9.

### 7.9 Device configuration ids (`MHAL_VENC_Wrapper_SetDevConfig` / `GetDevConfig`)

| id | dir | payload | meaning |
|---|---|---|---|
| 0x80030001 | get | `+8` = IRQ number (`drv+0x88`) | `E_MHAL_VENC_HW_IRQ_NUM` |
| 0x80030003 | get | {100, 2}: alignments 8, 2, 16, 16, 8, 2, 32×4, 32, 2, 2 | resolution / crop alignment table |
| 0x8003000e | get | {0x18, 1}: 256, 128, 8192, 0x2000000 | min W, min H, max W, max pixels |
| 0x80030013, 0x80030017, 0x80030020 | set / get | — | accepted, no action |
| 0x80030014 | set | 7 words | MMA callbacks (alloc, free, map, unmap, flush, heap-name, MIU protect) → `dev_info+0x100…0x118` |
| 0x80030016 | set | {W ≤ 8192, H, W·H ≤ 0x2000000} | max encode resolution (else 4096×2176); refused once the device is created |
| 0x80030019 | set | string < 0x80 | firmware path → `g_codec_fw_path` (§2.7) |
| 0x8003001a | get | {0x18, 2}: byte `+8` = 1, u16 `+0x14` = 0x101 | capability flags (read by mi_venc at device creation; their meaning on the mi_venc side: `mi_venc.md` §2.1) |
| 0x8003001c | set | {0xc, 1, fn} | event callback → `dev+0x184` (called from the IRQ, §3.1) |
| 0x8003001d | set | {0xc, 1, out} | post-reset / recover (or resume when only a source release was pending): out 1 ok, 2 retry, 0 fail (§9) |
| 0x8003001e | get | {0x18, 1}: 48, 12, 31, −32 | QP limits (absolute max / min, relative max / min) [I order] |
| 0x8003001f | set | {0xc, 1, out} | pre-reset (handshake with VDEC, §9) |
| 0x80030021 | set | {0xc, 1, fn} | reset callback → `dev+0x188` |
| 0x80030023 | get | {0xc, 1} → max(VENC SRAM, VDEC SRAM) | needs the device created |

How mi_venc attaches on M6 (`_MI_VENC_CreateMhalDevice` 0x1b748; chip index the word at 0x4dd70 = 5;
`mi_venc.md` §3.3): for
MFE / MHE it requests no IRQ; it reads 0x80030023 (SRAM), 0x8003001a, 0x8003001e and registers its handlers with
SetDevConfig **0x8003001c** (event, called from mhal's hard-IRQ `codec_isr`) and **0x80030021** (reset handshake). For
JPE on M6 it returns before the IRQ request too (mhal's JPE owns its IRQ). The `request_threaded_irq(…, "VENC-ISR")` path
(`VENC_IP_ISR`) with the 0x80030001 IRQ number is the older chips' path.

### 7.10 `EncodeFrameDone` and its result (0x58 B)
<!-- ev:mhal-vcodec.7.10 -->

`MHAL_VENC_Wrapper_EncodeFrameDone(inst, res)`: `VPU_EncGiveCommand(0x48)`, then
`VPU_EncGetOutputInfo` into a 0x130-byte `EncOutputInfo`. Its layout is StarFive's with the reconstructed-frame
`FrameBuffer` 8 bytes longer (0x54), so from `rdPtr` on every field sits 8 bytes later. `Wave5VpuEncGetResult` fills it
from these command-window registers (mainline names [S]) [R register →
offset]:

| `EncOutputInfo` | register | | `EncOutputInfo` | register |
|---|---|---|---|---|
| +0x0c picType | 0x124 | | +0x90 POC | 0x128 |
| +0x10 numOfSlices | 0x130 | | +0x94 — | 0x18c |
| +0x14 reconFrameIndex | 0x12c | | +0x98 encSrcIdx | 0x154 |
| +0x6c rdPtr | — | | +0xa0 VCL NUT | 0x15c |
| +0x74 picSkipped | 0x134 | | +0xa4 — | 0x160 |
| +0x78 numOfIntra | 0x138 | | +0xa8 picture number | 0x158 |
| +0x7c numOfMerge | 0x13c | | **+0xac errorReason** | **0x1d8** |
| +0x80 numOfSkipBlock | 0x144 | | **+0xb0 warnInfo** | **0x1d4** |
| +0x84 avgCtuQp | 0x148 | | +0xec… tick registers | 0x1b8… (§2.8) |
| +0x88 encPicByte (also copied to +0x04 `bitstreamSize`) | 0x14c | | +0x11c / +0x120 distortion | 0x164 / 0x168 |
| +0x8c encGopPicIdx | 0x150 | | +0x124 — / +0x12c — | 0x170 / 0x190 |

The meanings of +0x94 (0x18c), +0xa4 (0x160), +0x124 (0x170) and +0x12c (0x190; its value is only logged as "venc drop
frame %d") are not known.

The result carries no checked header: mi_venc passes a zeroed 0x58-byte struct and mhal reads
nothing from it. Result words written by mhal:

| offset | content |
|---|---|
| +0x08 | status: **0** normal; **0x800000** = the firmware warned "input ring overflow" (`warnInfo` bit 23 set, or `warnInfo` 1…4); **2** = nothing output (instance drop flag `inst+0xd0c` = 1, cleared here; or an error return with `errorReason` 0x700 "skipped"). `warnInfo` 5 only logs "vpe may not start but venc start" |
| +0x10 (u64) | bitstream start address (`rdPtr`, sign-extended high word) |
| +0x18 | bytes of the encoded picture (`encPicByte`) |
| +0x20, +0x24, +0x28, +0x38 | 0 |
| +0x30 / +0x34 | picture distortion low / high (`W5_RET_ENC_PIC_DIST_LOW/HIGH` [S]) |
| +0x3c | number of intra-coded blocks (`numOfIntra`) |
| +0x40 | merge + skip blocks |
| +0x44 | skip blocks (`numOfSkipBlock`) |
| +0x48 | merge blocks (`numOfMerge`) |
| +0x4c | reference type of the picture (`venc_ref_type` (0x400c0300) from `picType` and the GOP / LTR / TSVC state): 0 IDR / I (and `inst+0x184` restarts at 1); 1 P referencing the IDR (the first P after it, or an LTR refresh point); 2 P referencing the base; 4 / 5 enhance-layer P referenced / not referenced (TSVC-2 / -3 by `inst+0xab8`; also the frames between LTR points); **6** = dropped — the values are `MI_VENC_H26xRefType_e` (`E_MI_VENC_BASE_IDR` … `ENHANCE_P_NOTFORREF`, `REF_TYPE_MAX` = 6) [S M6 `mi_venc_datatype.h` 118–128; R values]; value 3 is never produced |
| +0x50 | POC of the picture |
| +0x54 (u8) | average CTU / MB QP |

Words +0x0c, +0x1c, +0x2c and the bytes +0x55…+0x57 are not written. In the dropped case +0x20…+0x57 are zeroed,
+0x10 / +0x18 = 0, +0x4c = 6, +0x08 = 2. Return values: 4 for "report not ready" (0x1b), "VLC buffer full" (0x1c,
"ALLOCATE MORE TASK BUFFER") and other errors (after `HandleEncoderError`, which dumps the VPU state); 0 otherwise.

mi_venc reads +0x08 (2 **or 0x800000** → the frame fails and the next one is forced IDR), +0x18, +0x4c, +0x50, +0x54, and
+0x3c / +0x44 / +0x48 for `GetLastHistoStaticInfo` (intra / skip / merge counts, from firmware registers 0x138 / 0x144 /
0x13c) (`mi_venc.md` §7.6).

### 7.11 Per-frame input — `SetEncPicParam` (0x400c09a8)

Fills the per-frame `EncParam` from mi_venc's 0xd0-byte input descriptor (version 6; field map `mi_venc.md` §7.4): source
frame addresses and stride, forced IDR / skip, custom map, ring input window (`u16RingBufHeight` / `subFrameSyncMode`
checks), the SEI and user-data areas (§7.7). Descriptor offsets read: +0x14, +0x1c, +0x1e, +0x20, +0x24, +0x28, +0x38,
+0x3c, +0x40, +0x68, +0x78, +0x9c, +0xa4, +0xa8; `_UpdateVencDynamicSramInfo` reads +0xb0 / +0xbc / +0xc8 (§2.5).

### 7.12 Helpers

- `ResetgopParameter` (0x400c08ec): `gopPresetIdx` = 0 (custom) with a **one-picture custom GOP: P, POC offset 1, QP 32,
  one L0 reference** — IPPP with a single reference picture, the cheapest Wave5 structure. This is the GOP the camera's
  streams run (prog_uvc never sends the REF id). TSVC / LTR modes replace it (REF id, §7.3).
- `allocInstCommonInfo` (0x400c0644): one 0x1000-byte MMA buffer "ven_inst_<idx>" per instance, split into header areas
  0x110 (AVC) / 0x170 (HEVC) (VUI, §7.6), 0xe0 (SEI, §7.7) and 0x620 bytes (user data).
- `initVuiInfo`, `initSeiInfo`, `_VenGenSeiNalu`, `_PutUserData`: VUI defaults, SEI buffer, SEI NAL generation (user data
  unregistered), user-data queue (≤ 4 entries of ≤ 1024 bytes).
- `MHAL_NEON_VENC_ACCMAP` (0x400a095c): NEON sum of the low 6 bits of every byte of a buffer — the "preSumQp" of a custom
  QP / mode map shown in `ctx_info`.
- `venc_check_res_rate`: OTP pixel-rate check (§2.3); `venc_ref_type`: picture type → reference type for `EncodeFrameDone`
  (§7.10); `_UpdateVencDynamicSramInfo` (§2.5).

## 8. MHAL VDEC — `MHAL_VDEC_*` [R unless tagged]

Used only by the MP4 player (mi_vdec, one H.264 1080p channel opened by prog_uvc; `mi_jpd-mi_vdec.md` §2) — **unused by
the camera's streaming path**, specified for completeness. The VDEC global device is the same codec core and firmware as
VENC (§1.2). **dinst** = the decoder instance (first argument), **dopen** = its `DecOpenParam` (`dinst+4`).

### 8.1 Entry points

| entry | behaviour |
|---|---|
| `CreateDevice(id, &dev)` | as VENC's (§7.1) with the VDEC MMA callbacks (`dev_info+0x120…0x138`, SetDevConfig 0x80020003): users++, first user: proc, MIU protect, clocks, preset, firmware (skipped if VENC loaded it), decoder pool init (`vdec_pool_init` (0x400bac30)), `b_vpuInit` |
| `DestroyDevice` | last user: `VPU_DeInit` (only if no VENC user), MIU protect off |
| `CreateInstance(dev, codec, &inst)` | codec 0 → AVC, non-zero → HEVC (0xc); `VPU_GetProductInfo` |
| `StreamOn(inst, p)` | §8.2; work buffer 0x31000 B (its last 0x1000 = host parameter area); `VPU_DecOpen`; `VPU_DecUpdateBitstreamBuffer(−1, 0)` (open-ended stream); state 2 |
| `StreamOff` | flush, `VPU_DecClose`, free |
| `SeqInit(inst, p)` | `VPU_DecIssueSeqInit` (§8.2) |
| `CompletSeqInit(inst, out)` | allocates and registers frame buffers (`VPU_DecRegisterFrameBufferEx`, ring-reference scheme of §5.3) (§8.2) |
| `SeqChange` / `UpdateSeqChange` | {0x68, v2} / {0xd0, v2}: resolution change mid-stream, re-register buffers |
| `SeqDeinit` | release frame buffers |
| `DecodeOneFrame(inst, p)` | p {0xd0, v2}: per-frame SRAM (`_UpdateVdecDynamicSramInfo`), `VPU_DecStartOneFrame`; 0x12 = queue full |
| `DecodeFrameDone(inst, r)` | r {0xb0, v1}: `VPU_DecGetOutputInfo` → §8.2 |
| `AddVlcBuffer`, `GetBSBuffer` {0x28, v1}, `UpdateBSBuffer`, `SetBSRdPtr`, `FlushBSBuffer` {0x28, v1}, `ClearDispFlag` | bitstream ring handling (read / write pointers, VLC buffer hand-over of §5.2), display-flag release |
| `QueryBufSize`, `GetEngStatus`, `DecodeAddInBuff`, `DecodeCancel`, `CheckInstanceStatus` | **return 8 (not supported)** — present in mi_vdec's ops table but stubs here (unused) |

### 8.2 Descriptors

Two output modes, chosen by the StreamOn byte that lands in `dopen+0x68`: **0 = caller-buffer mode** (each DecodeOneFrame
names the output buffer; mi_vdec "normal mode") and **≠ 0 = mhal-buffer mode** (mhal allocates and registers its own frame
buffers at CompletSeqInit; output scaling / crop come from SetParam 0x8002000a; mi_vdec "meta mode") [R; mode names as
mi_vdec's].

**StreamOn `{0x40, 2}`** (16 words):

| word | content | destination | mi_vdec writes (`vdec_stream_on` (0x17490)) |
|---|---|---|---|
| [2], [3] | — not read | | 0 |
| [4], [5] | ES ring physical address, u64 (lo, hi) — logged as "BsBufferAddr:%llx" | `dopen+4` (bitstreamBuffer), `dinst+0x2900/+0x2904` | P+0xb0, P+0xb4 |
| [6] | ES ring size | `dopen+8`, `dinst+0x2908` | P+0xbc |
| [7]…[9] | — not read | | 0 |
| [10] | ES ring kernel VA (dereferenced only by the level-5 NAL-header log and an ES dump in `UpdateBSBuffer` [I: the `/proc/mhe/dump_dbg vdecin` path]) | `dinst+0x2910` | **0** (not filled) |
| [0xb] | — not read | | 0 |
| byte [0xc] | output mode (above) | `dopen+0x68` | P+0x62 `bDisableLowLatency` |
| [0xd] | maximum reference frames; 0 = use the stream's `minFrameBufferCount` | `dinst+0xc7c` | P+0x5c (`u32RefFrameNum` of the create attr) |
| [0xe] | extra display buffers, must be < 5 (else 3) | `dinst+0xc80` | 3 |
| [0xf] | — not read | | 0 |

Frame-buffer counts derived from [0xd] / [0xe] (`vdec_fb_counts` (0x400ba762), at CompletSeqInit and UpdateSeqChange): decoder buffers
`dinst+0x64` = min(`minFrameBufferCount`, [0xd] + 1) (or = the inplace / DPB count of SetParam 0x80020001 when that is
non-zero, with the warning "inplace mode(%d) but minFrameBufferCount(%d)" when it is too small); display buffers
`dinst+0x60` = [0xe] + 1 + `frameBufDelay` in mhal-buffer mode with `dopen+0x20` = 1, else 0.

**SeqInit `{0x68, 2}`**: no input word is read; the only write is **[0x17] (+0x5c) = eStatus** (0 before the command; on
failure `_ErrorWarnDrv2Api(code, …, error)` of mhal's own return code — 3 "already queued", 2 issue failed, 4 command queue
full — none of which is in the mapper's table, so the status reads 0xfffff). Return 0 / 3.

**CompletSeqInit / SeqChange `{0x68, 2}`** (26 words, output); source = the sequence info `dinst+0x8c` (StarFive
`DecInitialInfo` layout up to +0x34, +4 from `seqInitErrReason` on — `Wave5VpuDecGetSeqInfo` stores the error reason at
+0xc4 and the warning at +0xc8):

| word | content |
|---|---|
| [2], [3] | picture width, height (`picWidth`, `picHeight`) |
| [4], [5], [6], [7] | display window x, y, w, h from `picCropRect` (left, top, right − left, bottom − top) — CompletSeqInit only; SeqChange leaves them |
| [8]…[0x11] | — not written |
| [0x12], [0x13] | ES read / write pointer after the header parse (`DecInitialInfo` +0xcc / +0xd0) — written on success and on failure |
| [0x14] | — not written |
| [0x15] | display frame buffers registered (`dinst+0x60`) — mi_vdec keeps it as P+0x22c "frame-buffer count" |
| [0x16] | `minFrameBufferCount` (`DecInitialInfo` +0x30) |
| [0x17] | eStatus: 0 on success; on failure the mapped firmware error (§8.4; CompletSeqInit logs "CompleteDecodeHeader failed" unless it is 0x20000); SeqChange writes **0x20007** when re-registering the buffers fails (return 8) |
| [0x18], [0x19] | — not written |

SeqChange additionally requires decoder state 5 (else 9) and re-reads the sequence (`GiveCmdDec 8`, 300 B copied to
`dinst+0x8c`).

**DecodeOneFrame / UpdateSeqChange `{0xd0, 2}`** (52 words, input); parsed by `_UpdateVdecDynamicSramInfo` and
`vdec_parse_sram_info` (0x400ba3c8):

| offset | content | checks / destination |
|---|---|---|
| +0x08 (u8) | tiled output (0 linear, ≠ 0 tiled) | `dinst+0x58` |
| +0x09 (u8) | repeat-frame decode | only in picture-end input mode (`dopen+0x40` = 2), else 3; `dinst+0x59` (DecodeOneFrame only) |
| +0x0c | decode option: 1 or 2 kept, anything else → 0 | `dinst+0xc` (`DecParam` word 1) [I: skip-frame mode] (DecodeOneFrame only) |
| +0x40 (u64) | luma (Y) buffer physical address | `dinst+0x1c` |
| +0x4c | Y buffer size | linear: ≥ stride × output height; tiled: = stride × ⌈H⌉64 |
| +0x50 | Y stride | linear: ≥ width (16-aligned when scaling or cropping); tiled: = ⌈W⌉128 → `dinst+0x28` |
| +0x58 (u64) | chroma (Cb / CbCr) buffer address | `dinst+0x20` |
| +0x64 | C buffer size | linear: ≥ C stride × ⌈H/2⌉; tiled: = stride × ⌈H/2⌉64 → `dinst+0x34` |
| +0x68 | C stride | tiled: must equal the Y stride |
| +0x70 (u64) | Cr buffer address | `dinst+0x24` |
| +0x90, +0x94 | scaled output width, height (0 = no scaling) | even; ≥ ⌈picture/8⌉ rounded to even; ≤ picture size rounded up to 8 → `dinst+0x38/+0x3c` |
| +0x9c, +0xa0, +0xa4, +0xa8 | output crop x, y, w, h | x, w multiple of 16; y, h even → `dinst+0x48…+0x54` as {left, right, top, bottom} when the crop differs from the scaled size |
| +0xb0 | SRAM base | `work+0x212c` |
| +0xbc | SRAM size | ≥ the device's VDEC SRAM need (`dev_info+0x140`, §2.5), else 0x11 → `work+0x2128` |
| +0xc8 | IMI (SRAM) mode, must be 1 | else 8 |

In mhal-buffer mode the buffer words are ignored and the output geometry comes from SetParam 0x8002000a
(`dinst+0xc54…+0xc70`). UpdateSeqChange validates the same descriptor and, in caller-buffer mode, also sends +0x90 / +0x94
to the scaler (`GiveCmdDec 0x28`). The words +0x10…+0x3c, +0x44…+0x48, +0x54, +0x5c…+0x60, +0x6c, +0x74…+0x8c, +0x98,
+0xac, +0xb4…+0xb8, +0xc0…+0xc4, +0xcc are never read by mhal (the plane groups at +0x40 / +0x58 / +0x70 are 0x18 bytes
each: u64 address, two unread words around the size / stride pair [I: a generic buffer descriptor]). DecodeOneFrame
refuses: state < 2 → 7; after stream end in mhal-buffer mode → 8; a decode still running (`dinst+0x28ec` ∉ {0, 9}) → 0x12;
no free VLC buffer → 0x12.

**DecodeFrameDone `{0xb0, 1}`** (44 words, output) — source = `DecOutputInfo` (StarFive layout up to +0x74 [R]; from the
ES pointers on +0x18 [I: fits every field mhal and mi_vdec use there], from `sequenceChanged` on +0x20 [R: the error /
warning reads]):

| word (offset) | content |
|---|---|
| [2] (+0x08) | eStatus (§8.4): `decodingSuccess` = 0 → error mapping of `errorReason` (`DecOutputInfo`+0x27c; "decode error 0x%x eStatus 0x%x" unless 0x20001); success → warning mapping of `warnInfo` (+0x284; "decode warnning …" when non-zero) |
| [3] (+0x0c) | `sequenceChanged` (non-zero also sets decoder state 5) |
| [4]…[7] (+0x10…+0x1c) | decoded rectangle x, y, w, h (`rcDecoded`) |
| [8]…[0xb] (+0x20…+0x2c) | display rectangle x, y, w, h (`rcDisplay`) |
| [0xc], [0xd] (+0x30/+0x34) | `decPicWidth/Height` |
| [0xe], [0xf] (+0x38/+0x3c) | `dispPicWidth/Height`, or the SetParam crop (right + 1 − left, …) when one is set |
| [0x10], [0x11] (+0x40/+0x44) | ES read / write pointer |
| [0x12], [0x13] (+0x48/+0x4c) | display frame Y address (u64, high word 0) |
| [0x14] (+0x50) | **not written** |
| [0x15] (+0x54) | Cb − Y (luma plane size) |
| [0x16] (+0x58) | stride |
| [0x17] (+0x5c) | not written |
| [0x18], [0x19] (+0x60/+0x64) | Cb address (u64) |
| [0x1a] (+0x68) | not written |
| [0x1b] (+0x6c) | chroma size = min((Cb − Y)/2, frame size − (Cb − Y)) |
| [0x1c] (+0x70) | stride / 2 |
| [0x1d] (+0x74) | not written |
| [0x1e], [0x1f] (+0x78/+0x7c) | Cr address (u64) |
| [0x20]…[0x23] (+0x80…+0x8c) | not written |
| [0x24] (+0x90) | `nalType` |
| byte [0x25] (+0x94) | `DecOutputInfo`+0x2e0 (mi_vdec logs it as "LongtermRef") |
| [0x26], [0x27] (+0x98/+0x9c) | `numOfErrMBs`, `numOfTotMBs` |
| [0x28], [0x29] (+0xa0/+0xa4) | `bytePosFrameStart`, `bytePosFrameEnd` of the picture in the ES [I by position] |
| [0x2a] (+0xa8) | `indexFrameDecoded` |
| [0x2b] (+0xac) | `indexFrameDisplay` |

Not-written words keep what the caller put there (mi_vdec zeroes the struct first). The rest of DecodeFrameDone (per-frame
tick statistics for `/proc/mhe`, the stream-end flag `dinst+0xc79`, VLC buffer release) does not touch the struct.
`DecOutputInfo` between +0x78 and +0x208 is mapped only where mhal reads it; the rest of its mhal layout against
StarFive's is not known.

### 8.3 Parameter and device-configuration ids

- **SetParam / GetParam**: `0x80020001` {0x10, 1} inplace / DPB mode (< 3; `inst+0x60`, `+0x64`); `0x80020004` {0xc, 1}
  input mode 0 → 0 (frame), 1 → 2 (stream) — mi_vdec sends 1 at StartChn; `0x8002000a` {0x2c, 1} output config: display
  enable byte, scale {enable, W, H even, non-zero}, crop {enable, x, y, w, h; x, w multiple of 16, y, h even}; allowed only
  when `dopen+0x68` = 1; other ids 0x80020001…0x8002000f accepted without action, beyond → 8.
- **SetDevConfig** (index = id − 0x80020003): `0x80020003` {0x24, 2} MMA callbacks; `0x80020005` {0x14, 1} max decode
  resolution (W ≤ 4096, W·H ≤ 0x880000, before CreateDevice); `0x80020008` {0xc, 1} event callback → `dev+0x184`;
  `0x80020009` post-reset (`codec_dev_reset` phase 3; mi_vdec's "MHAL_VDEC_SWReset"); `0x8002000b` pre-reset (phase 1);
  `0x8002000c` reset callback → `dev+0x188`.
- **GetDevConfig**: `0x8002000d` max SRAM size; `0x8002000e` / `0x8002000f` capability byte = 1.
- prog_uvc never sends `0x80020005` [R]; a 4K MP4 played in its 1080p channel would then have port 0 scaled to 1920×1080
  [I]; not observed on the camera.

### 8.4 eStatus
<!-- ev:mhal-vcodec.8.4 -->

The eStatus words (DecodeFrameDone [2], SeqInit / CompletSeqInit / SeqChange [0x17]) are produced by
**`_ErrorWarnDrv2Api(code, *out, codStd, isError, seqChange)`** (0x400bad60). Two encodings share the
word: the error path (one value 0x20000…0x20007 — e.g. 0x20004 firmware watchdog timeout, 0x20005 answered by mi_vdec
with the VPU software reset of §9 — or 0xfffff, from the firmware `errorReason` / `seqInitErrReason` or mhal's
own return code) and the warning path (a bit set 0x001…0x200 from the firmware `warnInfo` bits 16–26; the H.265 / H.264
split of `warnInfo` bit 18 into 0x004 / 0x008 is the only codec-dependent bit). The full input → eStatus tables and what
mi_vdec does with each value are in `mi_jpd-mi_vdec.md` §2.11.5. The firmware's names for the errors behind eStatus
0x20002, 0x20003, 0x20006, 0x20005 (input 0x80000) and for `warnInfo` bits 16–19, 22, 25, 26 are not known (the
Chips&Media error appendix is not available; 2946 only points to it).

## 9. Reset and resume [R]

- `codec_dev_reset(dev, phase)` (0x400b0920), reached through SetDevConfig of either wrapper: VENC `0x8003001f` → phase 0,
  VENC `0x8003001d` → phase 2 (mi_venc `_MI_VENC_PreResetCodec` / `_MI_VENC_DequeueInputTask`, `mi_venc.md` §10); VDEC
  `0x8002000b` → phase 1, `0x80020009` → phase 3 (§8.3).
- Phases 0 / 1 = **pre-reset** requested by VENC / VDEC: increment `+0x10c`; unless both sides already agreed, call the
  *other* device's `+0x188` callback with status 3 and expect it to answer 4 (else "vdec/venc ctx … error for no handshake
  pre reset", return 1); then mark reset (`dev_info+0x11c` = 1), pulse 1629.23, write 1629.22 = 0xff and 1629.21 |= 0xfe,
  wait ≤ 100 ms for the ISR flag `+0x114` to clear, clocks off, `VPU_DeInit`, own callback status 4.
- Phases 2 / 3 = **post-reset**: return 2 while any instance of either device is still busy (VENC instance state ≠ 1, VDEC
  state ∉ {1, 6}); increment `+0x110`; clocks on, pulse 1629.23, `codec_dev_preset`, `codec_load_firmware`, copy the SRAM
  size (`dev_info+0xf0`) to both devices (`+0x150`), callback status 5, clear both counters.
- This is the encoder-hang recovery behind mi_venc's timeout reset (`mi_timeout_reset`, "venc recover succeed/fail") and
  mi_vdec's VPU software reset on eStatus 0x20005.
- `codec_dev_resume` (0x400b08ae): `VPU_SWReset(0, 0)` then both devices' `+0x188` callbacks with a zeroed event.

## 10. Unused parts

Specified above, unused on this camera or in this build:

- the `venc` char device (stubs returning −14) and the never-installed `vcodec_*` set (§2.1);
- `MHAL_VCODEC_Init` / `Deinit` (`drv_codec_init` returns −1), `codec_module_func_reserve`, `vcodec_common_func_reserve`
  (§1.1); `venc_get_resource_clk`, `venc_set_source_clk`, `vcodec_clock_special_source_disable` (return 0, §2.2);
- `g_disable_mfe` / `g_disable_mhe` (written, never read, §7.1);
- VENC `IsrProc` (returns 0), `EncodeCancel` (counter only), the INTRA_PRED / INTER_PRED ids (ignored), the RC struct's
  tail words [0x28]…[0x2b], StatTime and FluctuateLevel, AVBR `MaxStillQp` (§7.3–§7.5);
- the SEI ids (no sender) and the HRD / bitstream-restriction parts of VUI (sent as zero) (§7.6, §7.7);
- the VLC / CINFO temp buffer (size 0 on this build, §2.5); init register 0x168 and mode 4 of 0x164 (§4.2);
- HW-UART pad routing (only with `fw_log_ctrl 1`, §2.8);
- VDEC `QueryBufSize`, `GetEngStatus`, `DecodeAddInBuff`, `DecodeCancel`, `CheckInstanceStatus` (return 8), and the whole
  VDEC path (MP4 player only, §8).

## 11. What is not known

- The OTP2 words 101f.26 / 2c / 2e / 2f on this chip, hence whether the 167.1 Mpx/s cap is active (§2.3; `ipl.md` §9).
- The live encode-time split (`/proc/mhe/ctx_info`), the live `ven_clock*` values and the effect of a 480 MHz vce clock on
  the frame rate (§2.2, §2.8, §6.3).
- `VPU_RET_VPU_CONFIG1` bit 26 (dual core), what the silicon reports in `W5_PRODUCT_NUMBER`, and the firmware's own
  product report (§1.2, §4.6).
- The roles of bridge 1629 regs 0x00 / 0x01 / 0x21 / 0x22 / 0x26 (values known, meanings [I]; no reference, §2.4).
- What mode 4 of init register 0x164 and register 0x168 mean (§4.2); which window offset each queued-command parameter
  comes from (§4.3).
- The H.264 VUI HRD words +0x28 / +0x2c, the H.265 0x40 scale defaults and the exact syntax-element names of the VUI and SEI
  blocks — the firmware's VUI writer would settle them (§7.6, §7.7).
- `EncOutputInfo` words +0x94, +0xa4, +0x124, +0x12c; the firmware meaning of `open+0x35c` and `open+0xcc` (§7.4, §7.10).
- `DecOutputInfo` beyond what mhal reads (§8.2); the firmware's error / warning names behind several eStatus values (§8.4).
- How encode time splits between the vce and vcpu clocks (§2.2).
