# mhal-jpd — the NJPD JPEG decoder block of mhal.ko
<!-- ev:mhal-jpd -->

JPD is the JPEG-decoder block of `mhal.ko`: the driver of the NJPD engine (one hardware decoder, RIU bank 1622) together
with the software JPEG layer that runs on the CPU in front of it. Per picture the CPU parses the JPEG header (SOF, DHT,
DQT, DRI, SOS), computes the MCU-aligned and possibly scaled-down output size, writes the Huffman and quantisation tables
into memory the engine reads, programs the read window, the output address and the picture geometry, and starts the
engine; the engine decodes the entropy-coded data and writes YUV 4:2:2 (YUYV) either as a whole frame into DRAM or row by
row into a ring (on-chip IMI or DRAM) handshaked with SCL. Progressive JPEGs are decoded by the CPU and fed to the engine
as run-length words (software-VLD mode), only when a module parameter enables it. The block's only caller is `mi_jpd.ko`
through 15 exported `MHAL_JPD_*` functions. This document covers the module glue (device tree, clock, char device,
sysfs, procfs, module parameters, memory), the MHAL API with its structures and parameter ids, the decode flow, the NJPD
register map, reset, table memory and debug dump, the software JPEG layer (header parser, progressive decoder), and the
unused parts.

Shared facts owned elsewhere: mhal's block map, the `camdriver_init` order (step 13 `DrvJPDModuleInit`), the
character-device quirk shared by GE / JPD / HDMITX / PNL / DISP / MIPITX, the module-parameter list and the DT properties
mhal reads per node — `mhal-core.md` (§2.1, §3.2, §4.2, §5, §7); RIU addressing (bank → physical address), the clock tree
with the `njpd` clock's parents, the interrupt number, the DT node as part of the DTB and the MIU client groups — `kernel-platform.md` §3.1, §5.1, §4.1, §14, §12.2; the caller side (mi_jpd's device and channel records, the decode path, the timeout,
SRAM for REALTIME binds, the direct-decode ioctls) — `mi_jpd-mi_vdec.md` §1; SCL's side of the JPD row handshake and its
registers — `mhal-scl-jpe-cmdq.md` §3.8, §3.4; the photo viewer's calls — `prog_uvc.md` §7.6.

Tags: **[R]** read from the binary (disassembly, `.data` / `.rodata`, symbol table, relocations), the DT or a
cited file; **[S]** stated by a source (the Infinity2 vendor source tree "I2", the u-boot header, a log string used as a
name); **[I]** inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mhal.ko` on the camera (JPD block 32.6 KB of `.text`, 273 named functions plus 21
unnamed static ones). Addresses 0x400xxxxx are those of the module image rebased to 0x40000000: an address = the function's
`.text` offset + 0x40000028. An unnamed function is given with its address and, where it was matched, the name of its I2
counterpart (§6.1). Vendor reference source ("I2"): the Infinity2 SDK tree published as `jockyw2001/sdk` on GitHub,
`src/mhal/i2/njpd/{drv/src/drvNJPD.c, drv/src/apiJPEG.c, hal/src/halNJPD.c}` with `regNJPD.h`, `halNJPD.h`, `drvNJPD.h`,
`apiJPEG.h`, `osalNJPD.c`, `jpeg_memory.c`; u-boot `drivers/mstar/jpd/jpd_hw/drv/pub/mhal_jpd.h`.

Register notation: `bank:reg` = one 16-bit RIU register; NJPD registers alone as `J:xx` (bank 1622, register xx, hex);
`[h:l]` a bit field.

## 1. The block [R]

### 1.1 Layers

| layer | functions | source relation | reached from mi_jpd's calls |
|---|---|---|---|
| MHAL API (`MHAL_JPD_*`, 15 exports) | 15 | no public header of this API revision; u-boot `jpd_hw/mhal_jpd.h` is a later, different revision (§3.1) | 12 of 15 are imported by mi_jpd |
| context / device glue (`JPD_Ctx*`, `JPD_AllocCtx`), proc, sysfs, module init | 25 | none | yes (proc / sysfs: debug only) |
| OS layer (`JPD_OS_*`, MMA wrappers, clocks) | 18 | none | yes |
| MApi layer (`MApi_JPD_*` + 21 unnamed statics) | 35 | none (I2 has the older `apiJPEG.c` Utopia API instead) | yes |
| software JPEG header parser + software progressive decoder (`JPEG_*`) | ~30 | I2 `apiJPEG.c` (same function names, trimmed) | header parser yes; progressive path only for progressive JPEGs (§6.6) |
| MDrv layer (`MDrv_NJPD_*`) | 76 | I2 `drvNJPD.c` — same names, device index added | yes |
| HAL layer (`HAL_NJPD_*`) | 117 | I2 `halNJPD.c` / `regNJPD.h` — same register indices and bit values; device index and DT-derived bank added | yes |
| OSAL stubs (`OSAL_NJPD_*`) | 8 | I2 `osalNJPD.c` | every one returns 0 / does nothing (§2.5) |

Of the 294 functions, 91 are never reached from the 15 exports, the module init, the ISR, the proc and sysfs hooks (the
17 tail-call thunks included, e.g. `MDrv_NJPD_Rst` → `HAL_NJPD_Rst`), and no relocation outside the JPD text references
them (§7). The JPD block has no CMDQ path.

Version strings in `.data` (MSIF header at `.data+0x1b688`): "1JPD_0103006162330" (+0xe) and "NJPD.HAL.00117630" (+0x20)
[R; I that both are version tags].

### 1.2 Static globals

JPD's static globals have no symbols of their own; a disassembler names them by the nearest preceding symbol. Their real
addresses, from the symbol table and the access patterns:

| nearest symbol + offset | address | object |
|---|---|---|
| `dev_attr_ispscl_ints+0x4b8` | `.data+0x1b660` | MHAL JPD debug level, u8, default 3 (§3.6) |
| `g_u32ProgressiveMaxHeight+0x4` | `.data+0x1b66c` | requested JPD clock, default 432 MHz (the `jpdclk` sysfs store writes it) |
| `jpd_clock` | `.data+0x1b674` | clock actually set by `JPD_OS_ClkInit`, initial value 384 MHz |
| `jpd_clock+0x4` | `.data+0x1b678` | sysfs `device_attribute` {name, 0644, `check_jpdclk_show`, `check_jpdclk_store`} |
| `jpd_clock+0x34` | `.data+0x1b6a8` | HAL mutex id (−1 = none) |
| `_gpRootJpdDir+0x4` | `.bss+0x1e2a8` | `jpd_init_flag` (u8, set by `MHAL_JPD_Init`) |
| `[&_gpRootJpdDir+0x4]+4` / `+8+0x10·dev` | `.bss+0x1e2ac` / `.bss+0x1e2b0` | `g_u32ProgressiveSupport` / `g_tJpd_Interbuf[2]` (16 B each: size, phys, —, VA) |
| `[&gpJPDDeviceNode]` / `+0x4` | `.bss+0x1e2d0` / `.bss+0x1e2d4` | platform device node; the 5-entry MMA callback table (`+4` alloc … `+0x14` flush, §2.6) |
| `[&gpJPDDeviceNode+0x38]` | `.bss+0x1e308` | flag set by the unused `_NJPD_ISRHandler`; the MDrv state block named from here holds the driver debug level (byte `+0x58`) and per-device read-window address words (§5.5) |
| `[&gIspSclIntCnts+0x184]` | `.bss` | proc state (mutex, selected dev / ctx of the two proc files, device list) |
| `[&gIspSclIntCnts+0x180]` | `.bss` | JPD sysfs class pointer |
| `[&dev_attr_ispscl_ints+0x210]` | `.bss` | JPD char-device record (dev_t, cdev, device, DT node at word 0x8e) |
| `[&u32NJPDRiuBaseAdd]` | `.bss+0x1e3b8` | `{RIU virtual base, u8 HAL debug flags, NJPD0 bank byte address, NJPD1 bank byte address}` |
| `g_sstar_jpd_Cxt` | `.bss+0x1e3a0` | MApi global context slots (`MApi_JPD_Get/Set_GCtx`, unused) |
| `pFillCBFunc` | `.bss+0x1e398` | ISR callback table (one slot per device) |
| `g_JPDISR_STATUS` | `.bss+0x1e394` | event flags accumulated by the ISR |
| `gbEnableTurboFWMode` | `.bss+0x1e390` | unused flag |
| `__mem_avail__` | `.bss+0x1e3a8` | software parser memory pool (§6.6) |

## 2. Resources and module glue [R]

### 2.1 Device tree, bank, register addressing
<!-- ev:mhal-jpd.2.1 -->

DT node: `jpd@0x1F2c4400 { compatible = "sstar,njpd"; reg = <0x1f2c4400 0x100>;
interrupts = <0 0x4c 4>; clocks = <&CLK_njpd>; clock-names = "CKG_jpd"; }`. `CLK_njpd` is a `sstar,composite-clock` at
`0x1f207128` (= CLKGEN `1038:4a`), mux-shift 2, mux-width 3, gate-shift 0, eight parents (the parent list:
`kernel-platform.md` §5.1).
- `JPD_OS_GetResourceBank(dev, &base, &size)`: DT node `"sstar,njpd"` (dev 0) or `"sstar,njpd%d"` (dev ≥ 1, none in
  the camera's DT); `CamOfAddressToResource(node, 0)`; base = start − 0x22000000 (phys 0x1F2C4400 → kernel virtual
  0xFD2C4400), size = end − start. No node → "Can't find dts node", −1, and `MHAL_JPD_CreateDevice` returns 1.
- `HAL_NJPD_InitRegBase(dev, base)`: RIU virtual base = `base & 0xFFF00000` (0xFD200000); per-device bank byte address
  = `(base & 0xFFFFF) >> 1` → 0x62200 for dev 0 (= bank **1622**). Every HAL access is
  `*(u16*)(riu + (bankByteAddr + 2·reg)·2)`; byte accesses use `riu + X·2 − (X & 1)` with X = the byte address. Other
  banks are addressed as base + bank·0x400 + 4n, the base standing for bank 0x1000 [R arithmetic; I base meaning,
  consistent with the bank names `HAL_NJPD_Debug` prints, §5.5]. Only dev 0 and dev 1 slots exist.
- Interrupt: DT interrupt index 0 of the `sstar,njpd` node (0x4c), mapped and installed by `MApi_JPD_RequestIrq` (§4.5).
- DT property `EOI` (optional, read at the IRQ request, §4.5; absent in the camera's DT): 1 makes the header parser demand
  FF D9 at the end of the input (§6.2).

### 2.2 Clock

- `JPD_OS_ClkInit(dev, id, 432 MHz)`: `of_clk_get_by_name(node, "CKG_jpd")` → `dev+0x48`, its current rate → `dev+0x50`;
  `of_clk_get(node, 1)` → `dev+0x4c` (absent in the camera's DT). `JpdDevClkRate` looks at parent indices 0 and 1 only
  (stops at a NULL parent): best = 0; for each parent rate r: if best < req and best < r → best = r; if req < best and
  req ≤ r ≤ best → best = r (it climbs towards the request and, once above it, steps down to the smallest rate still ≥
  the request); `CamClkSetRate`; on success `dev+0x50` and `jpd_clock` = that rate. Failure to get the clock prints
  "clock error" and still returns 0.
- `JPD_OS_ClkOn(dev, 1)`: set rate `dev+0x50`, prepare + enable, `dev+0x5c` = 1. `JPD_OS_ClkOn(dev, 0)`: for each clock
  handle (`dev+0x48`, `+0x4c`): set parent to parent index 0, disable, unprepare; `dev+0x5c` = 0. `JPD_OS_ClkDeinit`:
  `ClkOn(0)` if on, `clk_put` both.
- `JPD_OS_SetClk(rate)` (from `MHAL_JPD_Init` with 432 MHz and from the sysfs store): for every clock of the platform
  device node (`CamOfClkGetParentCount`): `of_clk_get(i)`, `CamClkSetRate(rate)`, `clk_put`. While `gpJPDDeviceNode` is
  NULL it does nothing (§2.3).
- The HAL has its own direct clock path, `HAL_NJPD_SetClock` / `HAL_NJPD_PowerOn` / `PowerOff`, writing CLKGEN `1038:4a`:
  PowerOn = `(v & 0xFFF0) | 4` (gate bit 0 = 0 → clock on, mux [4:2] = 1), PowerOff = `(v & 0xFFF2) | 1` (gate set →
  clock stopped). Nothing calls it (§5.3, §7). [I] The composite clock and these direct writes target the same register.
- sysfs attribute `jpdclk` (`check_jpdclk_show` / `check_jpdclk_store`): the clock table offered is 432, 384, 320, 288,
  172.8, 240 MHz (0x19bfcc00, 0x16e36000, 0x1312d000, 0x112a8800, 0x0a4cb800, 0x0e4e1c00). The store accepts only the
  form `0 clk=<Hz>` (id must be 0), writes `.data+0x1b66c` and calls `JPD_OS_SetClk` with no argument passed — r0 at that
  point is the parsed value [I from the call shape] — and prints "[JPDMODULE] jpd set %d clk success.". The show prints
  the rate actually set. Which rate the camera ends up with is not known (it depends on the parent rates and on
  whether `gpJPDDeviceNode` is set, §2.3); a read of `/sys/.../jpdclk` on the camera would print it and has not been made.

### 2.3 Module init, character device, sysfs

`DrvJPDModuleInit` (step 13 of mhal's `camdriver_init`, `mhal-core.md` §3.2) works on a static record at
`.data+0x1b3b8`: {u32 major = **250**, u32 minor = **251**, cdev at +0xc (its first word initialised to the name "jpd"),
`struct file_operations` at +0x48 — all zero, no relocations, so `/dev/jpd` has no operations at all}. If major ≠ 0:
when the class pointer is 0, reuse mhal's sysfs class (`msys_get_sysfs_class()`) or create `CamClassCreate("jpd_class")`;
only when a class already exists does it instead do `cdev_init(+0xc, fops)` + `cdev_add(MKDEV(250, 251), 1)` ("[JPDMODULE]
Unable add a character device" on failure) — the cdev branch is the `else` of "class pointer was 0", so on the only call
no cdev is added (the quirk and its effect on `/dev` open: `mhal-core.md` §3.2, §4.2). Then, if not yet done,
`CamDeviceCreate(class, MKDEV(250, 251), "jpd")` → record word 0x30, and `JPD_OS_SysfsInit` (one attribute; name string
`jpdclk` [R string, I binding]); `of_find_compatible_node("sstar,njpd")` → record word 0x8e ("Get Device mode Fail!!" if
absent). Finally `JPD_OS_SetDeviceNode(&pdev_jpd)` stores the `of_node` (`+0x170`) of the static platform device "jpd"
(`.data+0x1b480`) in `gpJPDDeviceNode`. Who fills that `of_node` is not found in the block's code [I: mhal's platform
registration]; while it is NULL, `JPD_OS_SetClk` does nothing, so the clock writes of `MHAL_JPD_Init` and of the sysfs
store depend on it. `DrvJPDModuleExit` only prints (`mhal-core.md` §3.3).

### 2.4 Module parameters

Strings `mdrv_jpd.progressive_*`, defaults from `.data` (the parameter list of the whole module: `mhal-core.md` §5):

| param | variable | default | effect |
|---|---|---|---|
| `progressive_support` | `g_u32ProgressiveSupport` (`.bss`) | 0 | 0 → the per-device intermediate buffer is 2 MiB; ≠ 0 → sized for the software progressive decoder (§2.6); also gates progressive files in the header parser (§6.2) |
| `progressive_max_width` | `g_u32ProgressiveMaxWidth` | 1920 | intermediate-buffer sizing (§2.6) |
| `progressive_max_height` | `g_u32ProgressiveMaxHeight` | 1080 | same |

The progressive size limit the parser applies is SetParam 1 (§3.4, §6.4), not these two parameters.

### 2.5 OSAL stubs

`OSAL_NJPD_Mutex{Create,Obtain,Release,Delete}`, `OSAL_NJPD_ISR_{Enable,Disable,Attach,Detach}`: each `return 0` (4-byte
bodies). The HAL mutex therefore never exists (`HAL_NJPD_CreateMutex` stores 0 from the stub); locking is done above, by
the context mutex / semaphore (§3.2) and mi_jpd's module mutex.

### 2.6 Memory: MMA callbacks and the intermediate buffer

mhal JPD owns no allocator. mi_jpd hands it five callbacks with `MHAL_JPD_SetParam(NULL, 0xb, &ops)`: the 20-byte table
`MHAL_JPD_MMA_CB_t` = `{alloc(dev, heap, name, size, &phys64), free(dev, phys64), map(phys64, size, cached) → VA,
unmap(VA), flush(VA, size)}` is copied into `.bss+0x1e2d8..0x1e2e8` (`get_Allocate_info(0)`); all five must be non-NULL,
else return 1 ("JPD MMA callback function is NULL!").
- `Allocate_MMA_alloc(dev, buf{size, phys, —, VA}, heap, name, map)`: `alloc(dev, heap, name ?: "jpd_dma", size)`; success
  → buf.phys; with `map` → buf.VA = `map(phys, size, 0)` (uncached), a map failure frees again. Failure → phys = −1,
  "alloc cma fail" / "cma map fail", −1.
- `Allocate_MMA_free`: unmap if VA, free phys ("allocate_MMA_free pDevBuf->phys_addr invalid" if phys = −1), phys = −1.
- `map_phys_addr_to_virt_base(dev, …, phys64, size)` / `ummap_virt_base(VA)`: map (uncached) / unmap through the same
  table; size 0 → "size invaild,pls check".
- **Intermediate buffer** `g_tJpd_Interbuf[dev]` (name "interbuf", heap NULL, mapped), allocated in
  `MHAL_JPD_CreateDevice`, freed in `MHAL_JPD_DestroyDevice` (always slot 0): size = 2 MiB (0x200000) when
  `progressive_support == 0`, else `((W + 7) >> 3) · (((H + 7) & ~7) >> 3) · 0x186 + 0x10130` with W / H = the progressive
  max width / height parameters (0x186 = 390 bytes per 8×8 block [I: room for the software progressive coefficient
  buffers], 0x10130 bytes of tables). Uses: the APP1 copy at its start (§6.2), the software parser's
  memory pool from +0x10080 (progressive only, §6.6), the table area of IMI decodes at size/2 (§5.4), and the read
  window of the (dead) thumbnail decode (§7). [I] A baseline frame or EMI decode needs the buffer only for the APP1
  copy; IMI decodes also for the tables.

### 2.7 procfs `/proc/jpd/`

Created by the first `MHAL_JPD_CreateDevice` (`JpdProcInit`, device-count refcount; the last `DestroyDevice` removes it):

| file | read | write |
|---|---|---|
| `dev_info` | one line per registered device; the format string is `"NULL\n"` in this build — prints nothing useful | `"%li %li"` → only echoed ("val [%d] [%d] success") |
| `ctx_info` | for the selected (dev, ctx): prints `"NULL\n"` | `"<dev> <ctx>"` selects |
| `histdec_param` | for the selected (dev, ctx): output phys `ctx+0xf0`, error code `+0xc`, debug level `+0x10`, `bIsMjpeg` `+4`, decode type `+0x38`, output mode `+0x9c`, scan type `+0xf12`, image size `+0x148/0x14a`, original size `+0x14c/0x14e`, aligned w / pitch / h `+0xf20/0xf22/0xf24`, scale-down flag / ratio `+0xf10/0xf11`, frame count `+0x18`, five timing slots (`+0x4900…+0x4940`, ×10 µs, "Total Extra OneFrame trg Isr->Done") and five FPS windows (`+0x48f4`, `+0x4964`, `+0x49d4`, `+0x4a44`, `+0x4ab4`; 1 / 3 / 5 / 10 / 60 s) | `"<dev> <ctx>"` selects |

The timing / FPS fields (`+0x48f4…+0x4ab4`) are only read: no store with those offsets exists anywhere in the block [R,
search of the whole block]. [I] They stay zero (the context is `calloc`ed), so `histdec_param` shows sizes and modes only.

## 3. MHAL API (`MHAL_JPD_*`) [R]

### 3.1 API revision and public headers

u-boot `drivers/mstar/jpd/jpd_hw/drv/pub/mhal_jpd.h` (2019–2020) is a later revision of this API: it has
`SetDevConfig` / `GetDevConfig` / `BarrierOneFrame`, a `pCmdQ` field and only two `MHAL_JPD_PARAM_CMD_e` values. This mhal
(a 2022 build of an older line) exports instead `Init`, `RegisterIsrCallback`, `UnRegisterIsrCallback`, `SetDebugLvl`, and
has no CMDQ path. Its enums match u-boot's where both exist: `MHAL_JPD_Status_e` {0 FAIL, 1 DONE, 2 ERROR, 3 DECODING},
`MHAL_JPD_OutputMode_e` {0 FRAME, 1 HANDSHAKE, 2 RING, 3 IMI, 4 EMI}, the `MHAL_JPD_InputBuf_t` layout (§3.3),
`MHAL_JPD_MMA_CB_t` (5 callbacks, same order), the first two fields of `MHAL_JPD_JpgInfo_t`. The parameter ids of §3.4 /
§3.5 are this build's, not u-boot's. I2 `MHAL_JPD_OutputFmt_e` [S] says the hardware outputs only YUV422.

### 3.2 Entry points

Every function checks its pointers (NULL → −1, "Check pointer is NULL" printed when the debug level ≤ 4).

| export | arguments | does | returns |
|---|---|---|---|
| `MHAL_JPD_Init` | (ignored) | once (`jpd_init_flag`): `MApi_JPD_Init` (= `MDrv_NJPD_PowerOn`, which only sets a "powered" flag — no register write), `JPD_OS_SetClk(432 MHz)` | 0 |
| `MHAL_JPD_CreateDevice` | `u32DevId`, `&hDev` | needs Init (else **0x10**); `CamOsMemAlloc(0x100)` device record (else 3); DT bank (else **1**, record freed); `HAL_NJPD_InitRegBase`; clock init + on (432 MHz request); device mutex (`+0x20`) and semaphore (`+0`, count 1); intermediate buffer (§2.6); `JpdProcInit`; `MApi_JPD_RequestIrq` (§4.5) | 0 |
| `MHAL_JPD_DestroyDevice` | `hDev` | `JpdProcDeInit`, clock off + put, `CamOsIrqFree` (if mapped), zero + free the record, free intermediate buffer slot **0** (whatever the device id) | 0 |
| `MHAL_JPD_CreateInstance` | `hDev`, `&hInst` | `hDev` NULL → **1**; `CamOsMemCalloc(1, 0x4b08)` context (else **5**); register in the device's 16-slot table `dev+0x64` (full → release, **4**); ctx `+0x88` = device, `+0` = slot; ctx mutex `+0x60`, semaphore `+0x40`; `dev+0x54` (instance count) +1 | 0 |
| `MHAL_JPD_DestroyInstance` | `hInst` | unregister the slot, `ctx+0x48d0` release hook (`JpdCtxRelease`: semaphore deinit + free; the ctx mutex is not destroyed), instance count −1 | 0 |
| `MHAL_JPD_SetParam` | `hInst`, id, `*param` | §3.4 | 0; unknown id → **3** |
| `MHAL_JPD_GetParam` | `hInst`, id, `*param` | §3.5 | 0; unknown id → **3** |
| `MHAL_JPD_ExtractJpgInfo` | `hInst`, `MHAL_JPD_InputBuf_t*`, `MHAL_JPD_JpgInfo_t*` | header parse on the CPU (§4.1) | 0 (also on a late parse failure, §4.1); early failure → **6** |
| `MHAL_JPD_StartDecodeOneFrame` | `hInst`, `MHAL_JPD_JpgFrame_t*` (0x80 B) | needs Init; §4.2–§4.4 | **always 0** once the pointers are valid — a hardware programming failure is only printed ("Decode_state: Decode failed"); not inited → prints "JPD already initialized." (sic) and returns 0 |
| `MHAL_JPD_ResetDev` | `hDev` (mi_jpd passes a second argument, ignored) | `MApi_JPD_Rst`: `HAL_NJPD_Rst` (§5.3) + IMI off + write ring off | 0 |
| `MHAL_JPD_RegisterIsrCallback` | `hDev`, `fn(hDev, status)` | callback table slot `[devId]` (`.bss+0x1e398`) = fn | 0 |
| `MHAL_JPD_UnRegisterIsrCallback` | `hDev` | clears slot **0** (the device id is not used) | 0 |
| `MHAL_JPD_CheckDecodeStatus` | `hInst`, `&status` | status = `MApi_JPD_GetEventStatus` (§4.5, polling J:2c) | `~status & 1` (0 when DONE) — not imported by mi_jpd, unused |
| `MHAL_JPD_SetDebugLvl` | `hInst` | global MHAL debug level = byte `hInst+0x10` if < 5 (else 3) — reads a context byte nothing writes; unused | 0 / 3 |
| `MHAL_JPD_Decodetimes` | — | `return 0` (stub); unused | 0 |

mi_jpd imports the other twelve (`mhal-core.md` §2.4).

### 3.3 Structures (this build) [R offsets from the copy code; names from u-boot where they match]

`MHAL_JPD_InputBuf_t` (0x28 B; SetParam 6 and `ExtractJpgInfo`):

| off | field | ctx copy |
|---|---|---|
| 0x00 | `void *pVirtBufAddr1` | `+0xa0` |
| 0x08 | `u64 u64PhyBufAddr1` | `+0xa8` |
| 0x10 | `u32 u32BufSize1` | `+0xb0` |
| 0x14 | `void *pVirtBufAddr2` | `+0xb4` |
| 0x18 | `u64 u64PhyBufAddr2` | `+0xb8` |
| 0x20 | `u32 u32BufSize2` | `+0xc0` |

Segment 2 is the ring-wrap continuation; the header parser reads segment 1, then segment 2 (§6.1); the hardware is
given segment 1 only (§4.3).

`MHAL_JPD_JpgFrame_t` (**0x80 B**, the decode descriptor):

| off | field | ctx | read later? |
|---|---|---|---|
| 0x00 | `eDecodeMode` | — | **not copied** (decode type stays 0 = main picture) |
| 0x08–0x2f | `stInputBuf` (above) | `+0xa0…+0xc0` | yes |
| 0x30, 0x34, 0x38 | output VA[0..2] | `+0xe0…+0xe8` | VA[0] only, and overwritten (§4.2) |
| 0x40, 0x48, 0x50 | output phys[0..2] (u64) | `+0xf0…+0x104` | phys[0] only |
| 0x58, 0x5c, 0x60 | output stride[0..2] | `+0x108…+0x110` | no |
| 0x64, 0x68, 0x6c, 0x70 | four u32 (u-boot's order would be BufSize, RingBufHeight, RingTagId, LineNum) | `+0x114…+0x120` | no |
| 0x74 | output buffer size (used to map and flush the output) | `+0x124` | yes |
| 0x78 | u32 (the value SetParam / GetParam 4 handle) | `+0x98` | only by GetParam 4 |
| 0x7c | output mode | `+0x9c` | no — overwritten by the decoder with 0 / 3 / 4 (§4.3) |

No `pCmdQ`, no `bProtectEnable` (nothing copies them). mi_jpd fills input VA1 / phys1 / size1 = the ring record, VA2 =
VA1 + size1, phys2 = phys1 + size1, size2 = 0; output VA[0] = 0, phys[0] = the mi_sys frame phys (`buf+0x48`, buffer type 2) or
task words 0x1c / 0x1d (type 0); `+0x74` = `buf+0x6c` (type 2) or task word 0x1e (type 0); everything else 0 (`mi_jpd-mi_vdec.md` §1.7; its direct decode §1.6 fills
VA[0] too).

`MHAL_JPD_JpgInfo_t` as written by this build: only `u16[0]` = width, `u16[1]` = height of the **MCU-aligned, scaled
output** (ctx `+0x148/+0x14a`, §6.5). The u-boot fields `OriginWidth/Height` and the sampling factors are not written.

### 3.4 `MHAL_JPD_SetParam` ids

| id | param | effect | mi_jpd |
|---|---|---|---|
| 0 | `{u16 w, u16 h}` | baseline maximum decode resolution → ctx `+0x128/+0x12a` (SOF limit = value·8, clamped; scale-down trigger — §6.4, §6.5) | CreateChn: attr[2] / attr[3] = 0x2000 / 0x2000 |
| 1 | `{u16 w, u16 h}` | progressive maximum → `+0x12c/+0x12e` | CreateChn: the same values |
| 4 | u32 | → `+0x98` (only GetParam 4 reads it; [I] u-boot's slot for an output mode) | no |
| 6 | `MHAL_JPD_InputBuf_t*` | input buffer → `+0xa0…` (the same copy `ExtractJpgInfo` does) | before `ExtractJpgInfo` |
| 8 | u32 | **output target kind** → `+0x14`: **0** = frame into the given DRAM buffer; **0xd** = IMI (rows into on-chip SRAM handshaked with SCL; the output address is not CPU-mapped, the tables go to the intermediate buffer); **0xe** = EMI ring (rows into a DRAM ring handshaked with SCL); **9** = frame with write protection (window = output phys … +0x5000, §4.3); any other value = frame | 0 (mi_sys frame, buffer type 2) / 0xd / 0xe (REALTIME bind, chosen by task word 0x1b = 0 / 1) |
| 9 | u8 | ring **line count** → `+0x11` (written to J:4a[6:0] for IMI / EMI); default 0x20 | 0x10 (IMI / EMI only) |
| 10 | u8 | FPGA / emulator debug → `+0x12`: 1 = spin forever right after triggering the decode; 2 = write 0x3333 to `1004:00` after the trigger (`MDrv_NJPD_PZ1SetFlag`, [I] a Palladium-Z1 marker) | no |
| 0xb | `MHAL_JPD_MMA_CB_t*` (5 function pointers; the handle is ignored) | install the allocator callbacks (§2.6); any NULL → **1** | at device init |

Ids 2, 3, 5, 7 and ≥ 0xc → 3 ("Unkwon cmd"). [I] "IMI" is on-chip SRAM, as SCL's IMI mode. mi_jpd's attr word 1 is
not passed to mhal at all; only attr[2] / attr[3] reach SetParam 0 / 1 (`mi_jpd-mi_vdec.md` §1.5).

### 3.5 `MHAL_JPD_GetParam` ids

| id | out | value |
|---|---|---|
| 0 | `{u16, u16}` | baseline maximum (`+0x128/+0x12a`) |
| 1 | `{u16, u16}` | progressive maximum (`+0x12c/+0x12e`) |
| 2 | `{u16, u16}` | output (aligned, scaled) width / height `+0x148/+0x14a` — the same as `ExtractJpgInfo` |
| 3 | `{u16, u16}` | **original** SOF width / height `+0x14c/+0x14e` (the one mi_jpd uses) |
| 4 | u32 | `+0x98` (SetParam 4 / descriptor +0x78; default 1) |
| 6 | u32 | scan type `+0xf12`: 0 gray, 1 4:4:4, 2 4:2:2, 3 4:4:0, 4 4:2:0, 5 4:1:1 (§6.5) |
| 7 | u32 | output mode actually used `+0x9c`: 0 frame, 3 IMI, 4 EMI |

Ids 5 and ≥ 8 → 3. mi_jpd stores the `ExtractJpgInfo` size (aligned, scaled) as its buffer size and the GetParam 3 size
(original) as the picture size (`mi_jpd-mi_vdec.md` §1.2, §1.7); [I] the latter is the frame's visible width / height.

### 3.6 Debug level

MHAL prints: INFO when level < 2, DBG < 3, WARN < 4, ERR < 5, "NONE" messages only at level 0. The default 3 lets only
WARN and ERR print. The level is the u8 at `.data+0x1b660`; only `MHAL_JPD_SetDebugLvl` (unused) writes it. The HAL
flags byte and the MDrv debug level are separate and never set (§5.5).

## 4. Decode flow [R]

### 4.1 `MHAL_JPD_ExtractJpgInfo` — header parse on the CPU

1. Copy the InputBuf into the context (as SetParam 6); the intermediate buffer of the context's device → ctx `+0xd0`
   (phys), `+0xd8` (VA), `+0xc8` (size).
2. `MApi_JPD_DefaultConfig` (ctx mutex held): reset the parser state (`JPEG_init`, part of I2 `JPEG_init`: line count
   `+0x11` = 0x20, `+0x98` = 1, `+0x9c` = 0, clear `+0x148…+0xf10`, `+0xf10…+0xf80` and the software-decoder area
   `+0x1080…+0x48c8`, error code 0, component-id map `{1, 2, 3}`, "first start" flag `+0xf13` = 1), then
   `JPEG_Init_ParseAddr` (read pointer = segment 1, then segment 2; bytes left = size1 + size2).
3. `MApi_JPD_DecodeHdr_FirstStage`: `JPEG_locate_sof_marker` (§6.2). Failure → **6** ("MApi_JPD_DecodeHdr: Fail").
4. `MApi_JPD_DecodeHdr_LastStage`: baseline → `JPEG_locate_sos_marker` + `JPEG_DealHdrInfo` (scale info, MCU layout,
   Huffman / quantisation checks, §6.4–§6.5); progressive → scale info, MCU layout and **the whole software progressive
   decode** (`JPEG_init_progressive`, §6.6). Success → JpgInfo = `{+0x148, +0x14a}`, return 0. **Failure →
   `MApi_JPD_Rst` (hardware reset) + `MApi_JPD_Exit` (free the parser blocks) and return 0 without writing JpgInfo** —
   the caller sees success with whatever it pre-set (mi_jpd zeroes it first and then drops the record because of the zero
   size).

### 4.2 `MHAL_JPD_StartDecodeOneFrame`

1. Copy the descriptor (§3.3) into the context.
2. If ctx `+0x14` (the SetParam 8 kind) ≠ 0xd: map output phys[0] for CPU access, size = descriptor `+0x74` (uncached map
   through the MMA callback) → ctx `+0xe0` (the descriptor's VA[0] is ignored). The driver writes the decoder's tables
   into the start of this buffer (§5.4), so a frame or EMI output buffer must be CPU-mappable and at least 0x900 bytes
   [I from the table offsets].
3. Input size: baseline → `size1 · 2`; progressive → `size1 & ~0x1f`. [I] The doubling makes the read window two halves of
   size1 each (§4.3): the JPEG sits in half 0, half 1 lies **beyond the record** (in the ring pool on mi_jpd's path) and
   the hardware may prefetch it. A size ≤ 0x1007f only prints "JPD InputBufSise to short" (debug).
4. `MApi_JPD_Start_Decode` (ctx mutex held): baseline → `JPEG_StartDecode` (§4.3), and "started" (`+8` = 1) when the error
   code stays 0; progressive → feed the software-decoded coefficients row by row (§6.6).
5. Unmap the output. Return 0.

### 4.3 `JPEG_StartDecode` (0x40092f28) — programming one decode
<!-- ev:mhal-jpd.4.3 -->

Register names per §5.1; `dev` = the device id (device record `+0xac`); every write goes through the HAL.
1. `g_JPDISR_STATUS` = 0; **`HAL_NJPD_Rst(dev)`** (§5.3).
2. Aligned picture size: width rounded up to `8·Hmax`, height to `8·Vmax` (an MCU multiple of component 0's sampling).
3. Output routing (defaults: IMI off = J:01[15] = 0, write ring off = J:4a[15] = 0, mode `+0x9c` = 0):
   - kind **0xd (IMI)**: mode 3; **IMI on** (J:01[15] = 1), **write ring on** (J:4a[15] = 1); line count = `+0x11`;
     table base = intermediate buffer phys + size/2 (VA likewise) — the tables cannot live in IMI.
   - kind **0xe (EMI ring)**: mode 4; IMI off, write ring on; line count = `+0x11`; tables at the output buffer.
   - other kinds: mode 0 (frame); tables at the output buffer.
4. Baseline only: if the entropy-coded segment start (ECS = read pointer − input VA, i.e. the header length) is ≥ the
   (doubled) input size → error −233 (`E_JPEG_NOTENOUGHMEM`), nothing programmed.
5. `MDrv_NJPD_InitBuf` — buffers (one `NJPD_BufCfg`, as I2 `drvNJPD.h`):
   - read window: `MDrv_NJPD_SetReadBuffer_New(base = input phys, size, start = base + ECS)`: MIU select 0 (J:28[11:10] =
     0; `1006:7a[7]` = 1, I2 name `NJPD0_MIU1_MIU_SEL2`, bank 1006 = MIU1 in I2 [S]); MRC0 = [base, base + size/2 − 1],
     MRC1 = [base + size/2, base + size − 1]; if the start lies past MRC0's end, MRC0's end becomes `start | 0xf` and MRC1
     starts after it ("bDifferentHL"); J:10/11 = start, J:12–15 = MRC0 start / end, J:16–19 = MRC1 start / end (byte
     addresses, low / high 16 bits). Measured on the camera [R: 2026-11-11]:
   with MRC0's end inside a 16-byte unit that holds 1–8 bytes of the input, a decode may stop short of its last bits
   («MRC0 empty», no «done»); this driver does not round the window, so the caller pads the input to 16. (The thumbnail variant — read from the intermediate buffer — is reachable only with
     decode type 1, never set, §7.)
   - write: J:1a/1b = output phys[0] (raw byte address); line count ≠ 0 → J:4a[6:0] = line count.
   - table base (MDrv per-device word `+0x80`) = output phys[0], or the IMI alternative above; VA (`+0x6c`) likewise.
6. J:06/07 = aligned width / 8, height / 8 (`SetPicDimension`; with scale-down the width is the pitch-rounded `+0xf60`).
7. Scale-down (`+0xf11` ≠ 0): if the pitch-rounded width ≠ the aligned width (and baseline) → ROI on: J:00[6], J:09/0a =
   0, J:0b = width/8, J:0c = height/8 (`MDrv_NJPD_SetROI`); output width `+0x148` = rounded width / ratio.
8. Baseline: J:00[8] = "3 Huffman tables" flag (`+0xf7c`, set when Cb and Cr use different DC or AC tables); G-table
   reset (J:01[10] toggled 0 → 1); write the group-info table (`WriteGrpinf`, §5.4) and the symbol-index table
   (`WriteSymidx`). Both modes: write the IQ table (`WriteIQTbl`); flush the output buffer (`CamOsMemFlush(VA, size)`, not
   for IMI); `TableLoadingStart` (J:08 = 2, write-one-clear).
9. GlobalSetting00 value (written once, at step 13): J:00 as read right after step 6 | `(H0−1)` in [1:0] (Y_HSF1/HSF2;
   H0 = component 0's horizontal sampling) | `(V0−1)<<2` [2] (Y_VSF2) | `(components > 1)<<3` [3] (UV) | restart enable
   [5] when DRI ≠ 0 (J:05 = restart interval via `SetRSTIntv`) | ROI [6] | SWRST [4]. Scale-down factor → J:01[1:0] (0
   none, 1 ½, 2 ¼, 3 ⅛).
   **Ordering against I2**: the value is captured before step 8 sets J:00[8] (`bl MDrv_NJPD_Get_GlobalSetting00` at
   `.text` 0x93142 precedes `bl MDrv_NJPD_SetDifferentHTable` at 0x9319c on a straight path; I2 `apiJPEG.c` [S] reads
   `reg_value` after it), so the final write of step 13 clears the "3 Huffman tables" bit again. [I] JPEGs whose Cr
   component uses Huffman tables different from Cb's (`+0xf7c` = 1) get the 3-table group / symbol layout in memory but a
   decoder told "2 tables" — likely mis-decoded (not observed); ordinary JPEGs (Cb and Cr share tables) are unaffected.
10. Progressive: software VLD on (`MDrv_NJPD_SetSoftwareVLD(1)`: J:02 = (J:02 & 0xd9fb) | 0x2000 — removes the FF00 / FFFF
    stuffing and FFD9 detection, bit stream little-endian; J:01[6] = 1; J:48[0] = 0).
11. Power saving: J:0d = 0xff (all gated clocks enabled).
12. Kind 9 only: write protection: J:5b[13] = 1, lower bound J:5a/5b[12:0] = phys >> 3 (bits 3–18 / 19–31 + 3 high),
    upper bound = phys + 0x5000.
13. J:00 = the value of step 9 | 0x10 (SWRST kept set).
14. Baseline: MRC0 valid (J:08 = 0x8), 100 µs, MRC1 valid (J:08 = 0x10).
15. Device timestamp `dev+0xd4`; **DecodeEnable** (J:08 = 1). FPGA debug hooks (SetParam 10). Return 1.

Errors set ctx `+0xc` (I2 `JPEG_ErrCode` values, §6.7).

The IMI (0xd) and EMI (0xe) kinds with a ring line count of 16 (SetParam 9) work together with SCL's side of the JPD
row handshake (`1211:10[14]/[15]`, `1212/1213:38/39`; `mhal-scl-jpe-cmdq.md` §3.8) when mi_jpd's output port is
REALTIME-bound to SCL [I].

### 4.4 Read-window refill (progressive / software VLD only)

`JPEG_write_RLE` (0x40093566) appends 32-bit RLE words to the input buffer VA; when the buffer is full or at the end
it flushes the cache, starts the hardware on the first call (§4.3), clears J:29 bits 6 / 7 (MRC empty), marks the filled
half valid (J:08 = 8 or 0x10, plus `ReadLastBuffer` J:08 = 0x20 at the end) and busy-waits on `g_JPDISR_STATUS` (set by
the ISR) for decode done (bit 0) or both MRC halves empty (0xc0), up to 1 000 002 polls; any of bits 1–4 or the timeout →
`HAL_NJPD_Rst`, error 0x5566 (`E_JPEG_JPD_DECODE_ERROR`). Baseline decoding does not use it: the whole JPEG is in the
first half of the read window.

### 4.5 Interrupt and status

- `MApi_JPD_RequestIrq(dev)`, once per device: `CamIrqOfParseAndMap(node, 0)` → `dev+0x58`, optional DT `EOI` property →
  `dev+0x60` (the camera's DT: 0); clear all events (J:29 = 0xfff); `CamOsIrqRequest(irq, _JpdDevIsr, "JPD_ISR", dev)`.
- `_JpdDevIsr(irq, dev)`: `flags` = J:2c (final status); `g_JPDISR_STATUS |= flags`; clear them (J:29 = flags). Event
  bits (I2 `NJPD_EVENT_*` [S]):

| bit | event |
|---|---|
| 0 | decode done |
| 1 | minicode error |
| 2 | invalid scan |
| 3 | restart-marker error |
| 4 | RMID error |
| 5 | end-of-image error |
| 6 | MRC0 empty |
| 7 | MRC1 empty |
| 8 | write-protect hit |
| 9 | data loss |
| 10 | IBUF load done (masked) |
| 11 | Huffman table error |

  Status for the callback: **1 DONE** if bit 0 and none of 0xa3e; **2 ERROR** if any of bits 1–5, 9, 11; else **3
  DECODING** (0 FAIL is unreachable [I: it needs bit 0 together with error bits that clear it]). **The callback
  `fn(hDev, status)` runs only when bit 0 (decode done) is set** — an error interrupt without "done" is cleared and
  nobody is told; mi_jpd then hits its timeout (max(200 ms, 2 × the smoothed decode time), `mi_jpd-mi_vdec.md` §1.7).
  The ISR returns 1 (handled).
- `MApi_JPD_GetEventStatus` (for `CheckDecodeStatus`, polling; its only caller [R: call graph]): the same mapping
  from J:2c, plus write-protect → bit 6; any error (flags & 0xa3e) → ctx error −227 (`E_JPEG_DECODE_ERROR`, written
  0xffffff1d), `JPEG_free_all_blocks`, `MDrv_NJPD_Debug(dev)` (§5.5); returns 0 if the context never started a decode.
- Interrupt mask (set by every reset): J:2b = 0xf400 → bits 0–9 and 11 unmasked, 10 and 12–15 masked.

## 5. Hardware: NJPD registers, reset, table memory, debug dump

### 5.1 Register map, bank 1622 (`J:xx`) [R indices: every HAL accessor of this build matches I2 `regNJPD.h` / `halNJPD.h`; bit names from those headers [S]; "path" = written on mi_jpd's path]

| reg | I2 name | bits / use in this build | path |
|---|---|---|---|
| 00 | GLOBAL_SETTING00 | [1:0] Y_HSF (H0−1), [2] Y_VSF2 (V0−1), [3] UV present, [4] SWRST (set after the reset sequence and in the final value), [5] RST_EN (DRI), [6] ROI_EN, [7] different Q tables (3 components), [8] 3 Huffman tables, [10] YC_SWAP, [11] UV_SWAP | yes |
| 01 | GLOBAL_SETTING01 | [1:0] scale-down (0, ½, ¼, ⅛), [6] SVLD (software VLD / RLE input), [8] UV_7BIT, [9] UV_MSB, [10] GTABLE_RST, [11] HTABLE_RELOAD_EN, [12] GTABLE_RELOAD_EN, [13] QTABLE_RELOAD_EN, **[15] write to IMI (M6, not in I2)** | yes |
| 02 | GLOBAL_SETTING02 | reset value 0x1702: [1] TBC_MODE, [8] LITTLE_ENDIAN, [9] REMOVE_0xFF00, [10] REMOVE_0xFFFF, [12] HUFF_DATA_LOSS_ERROR; software VLD: & 0xd9fb (drop [2] FFD9_EN, [9], [10]) \| [13] BITSTREAM_LE | yes |
| 03 | PITCH_WIDTH | not written | — |
| 05 | RESTART_INTERVAL | DRI value | if DRI |
| 06 / 07 | IMG_HSIZE / IMG_VSIZE | aligned width / 8, height / 8 | yes |
| 08 | WRITE_ONE_CLEAR | [0] DECODE_ENABLE, [1] TABLE_LOADING_START, [3] MRC0_VALID, [4] MRC1_VALID, [5] MRC_LAST, [6] TBC_EN, [7] TBC_DONE_CLR, [8] CLEAR_CRC, [9] HANDSHAKE_SW_WOC | yes |
| 09–0c | ROI_H_START, ROI_V_START, ROI_H_SIZE, ROI_V_SIZE | 0, 0, width/8, height/8 when the scaled pitch is rounded | scale-down only |
| 0d | GATED_CLOCK_CTRL | 0xff (power saving) every decode | yes |
| 0e | MIU_READ_STATUS | [0..4] MRC0 / MRC1 / H / G / Q-table ready — read by the debug dump only | — |
| 10 / 11 | MIU_READ_START_ADDR L/H | first byte of the entropy-coded data | yes |
| 12–15 | MIU_READ_BUFFER0 START/END L/H | read half 0 | yes |
| 16–19 | MIU_READ_BUFFER1 START/END L/H | read half 1 | yes |
| 1a / 1b | MIU_WRITE_START_ADDR L/H | output phys (frame / ring / IMI address) | yes |
| 1c / 1d | MIU_WRITE_POINTER_ADDR L/H | read accessors exist, never called | — |
| 1e / 1f | MIU_READ_POINTER_ADDR L/H | current read address (status, debug dump) | debug |
| 20 / 21 | MIU_HTABLE_START_ADDR | table base + 0x400 | yes |
| 22 / 23 | MIU_GTABLE_START_ADDR | table base + 0 | yes |
| 24 / 25 | MIU_QTABLE_START_ADDR | table base + 0x800 | yes |
| 26 | MIU_HTABLE_SIZE | [7:0] symbol-index entry count (§5.4) | yes |
| 27 | SET_CHROMA_VALUE | not written | — |
| 28 | IBUF_READ_LENGTH | [4:0] max burst = 0x1f, [9:5] min = 0x10 (reset), [11:10] MIU select = 0 | yes |
| 29 / 2a / 2b | IRQ_CLEAR / IRQ_FORCE / IRQ_MASK | clear: 0xfff at reset and IRQ request, the flags in the ISR; force: accessor unused; mask: 0xf400 | yes |
| 2c / 2d | IRQ_FINAL_S / IRQ_RAW_S | final status read by the ISR and polling; raw unused | yes |
| 2f (high byte) | MIU_TLB | `HAL_NJPD_SetMTLBMode` never called | — |
| 30 / 31 | ROW_IDEX / COLUMN_IDEX | current MCU row / column (status) | debug |
| 32–34 | DEBUG_BUS_SELECT / H / L | debug dump (§5.5); J:32 written | debug |
| 35–3f | byte counters, VLD data, debug trigger, BIST | not accessed | — |
| 40–44 | TBC (table read-back RIU interface), WDATA0/1, RDATA L/H | only `MDrv_NJPD_TableRead` (never called, §7) | — |
| 45–47 | Y/CB/CR_MAX_HUFFTABLE_ADDRESS | not accessed | — |
| 48 | SPARE00 | 0x2fb at reset; [0] cleared for software VLD | yes |
| 49 | SPARE01 | accessors unused | — |
| 4a | SPARE02 | **M6 use: [15] write-ring enable, [6:0] ring line count** (`MDrv_NJPD_SetWRingBuf_ENABLE` / `LineNum`) | IMI / EMI |
| 4b–4f | SPARE03–07 | accessors unused | — |
| 50–55 | MARB_SETTING_00–05 | 54: [4:0] read burst threshold (`SetMRBurstThd`, never called) | — |
| 56 / 57 | MARB_SETTING_06 / 07 | accessors unused | — |
| 58–5b | MARB_UBOUND_0 / LBOUND_0 (write-protect window) | upper / lower bound = addr >> 3 split 16 / 13 bits; 5b[13] = MIU bound enable | kind 9 only |
| 6d | CRC_MODE | 0x200 (read mode) at reset; 0x300 write mode never used | yes |
| 70–73 | MARB_CRC_RESULT_0–3 | debug dump | debug |
| 74 | MARB_RESET (low byte) / HANDSHAKE_CNT (high) | reset: [0] 0→1, then [1] 0→1 | reset |
| 75 | SW_MB_ROW_CNT (low) / HANDSHAKE (high) | `Handshake_SW/HWRowCountMode`: never called | — |
| 76 | TOP_MARB_PORT_ENABLE | `HAL_NJPD_JPDSelect`: never called (§5.3) | — |

### 5.2 Other banks touched [R]

| register | use |
|---|---|
| `1012:43[7]`, `1006:43[7]` | MIU protect mask, set during every NJPD reset and cleared after (§5.3); in I2's naming bank 1012 = MIU0, 1006 = MIU1 [S] |
| `1006:7a[7]` | = 1 in `SetMIU` (read-window set-up, §4.3); I2 name `NJPD0_MIU1_MIU_SEL2` [S] |
| `1004:00` | = 0x3333 after the trigger, FPGA debug only (SetParam 10 = 2) |
| CLKGEN `1038:4a` | direct clock register; its accessors are unreachable (§2.2) |
| CHIPTOP `101e:42[7]` | MIU group select, written only by the unreachable `JPDSelect` (§5.3) |
| banks 1012, 1006, 1608, 160F, 1615 | read (whole, 0x80 registers each) by the debug dump (§5.5) |

### 5.3 Reset sequence `HAL_NJPD_Rst(dev)` [R] (at every decode start, by `MHAL_JPD_ResetDev`, on timeouts)

1. MIU protect mask on (`1012:43[7]` = `1006:43[7]` = 1).
2. J:29 = 0xfff (clear all events).
3. `SetOutputFormat(dev, reset, ORIGINAL)`: J:00 = 0; MARB reset (J:74[0] 0→1, J:74[1] 0→1); J:01 &= 0x0400 (clears IMI,
   scale, SVLD, reload bits).
4. J:02 = 0x1702.
5. J:00 |= 0x10.
6. J:2b = 0xf400.
7. J:48 = 0x2fb.
8. MIU protect mask off.
9. J:28 = (J:28 & 0xfc00) | (0x10 << 5) | 0x1f.
10. J:6d = 0x200.

`MApi_JPD_Rst` adds J:01[15] = 0 (IMI) and J:4a[15] = 0 (write ring). The sequence is I2's `HAL_NJPD_Rst` [S] with the
verification-mode branches removed. The output is always YUV 4:2:2 packed (YUYV) without swap: J:00[11:10] = 0 at reset.

**Never executed in this build** [R: call graph; no reference from outside the block either]: `HAL_NJPD_PowerOn`
(the CLKGEN write of §2.2, then `JPDSelect(1)`: `101e:42[7]` = 0, J:76 ports P0 / P1 on, P2 off, all read /
write bypass bits on), `HAL_NJPD_PowerOff`, `HAL_NJPD_SetClock`. [I] The clock comes up through the Linux clock framework
(`CKG_jpd`), and J:76 / `101e:42` stay at their reset values. Their values while the engine decodes are not known (not
read on the camera).

### 5.4 Table memory (read by the hardware through J:20–25) [R; layout = I2 `drvNJPD.c` [S], confirmed by this build's offsets and flush sizes]
<!-- ev:mhal-jpd.5.4 -->

Base = the output buffer start (frame / EMI) or intermediate buffer + size/2 (IMI); written by the CPU, flushed, then
J:08[1] (table loading start). [I] In frame mode the decoded picture later overwrites this area.

| offset | table | entry | size flushed | reload bit | address registers |
|---|---|---|---|---|---|
| +0x000 | group-info (G) | 4 bytes, 16 per Huffman table: DC-luma 0x000, AC-luma 0x040, DC-chroma 0x080, AC-chroma 0x0c0; with 3 tables DC-chroma2 0x100, AC-chroma2 0x140 | 0x180 | J:01[12] | J:22/23 |
| +0x400 | symbol-index (H) | 4 bytes: [0] luma, [1] chroma, [2] chroma2, [3] never written | N·4 | J:01[11] | J:20/21; J:26 = N |
| +0x800 | IQ (Q) | 64 entries × 4 bytes | 0x100 | J:01[13] | J:24/25 |

The G and H tables are the I2 arithmetic byte for byte (`read_dht` I2 `apiJPEG.c` 3500–3567, `JPEG_check_huff_tables`
4490–4553, `JPEG_WriteGrpinf` 4895; `drvNJPD.c` `MDrv_NJPD_WriteGrpinf` 911, `MDrv_NJPD_WriteSymidx` 1119 [S]; binary:
`JPEG_read_dht_marker`, `JPEG_check_huff_tables` 0x40094240, `JPEG_WriteGrpinf` 0x40092d08, `MDrv_NJPD_WriteGrpinf`
0x40092878, `MDrv_NJPD_WriteSymidx` 0x400923ec [R]). The chain from a DHT segment (record layout: §6.3):
1. **read_dht** — count = Σ num[1..16] (> 255 → −200); after storing num / val: for l = 16 down to 1: `count −= num[l];
   Symbol[l] = count; Valid[l] = (num[l] ≠ 0)`. Symbol[l] = the number of symbols with code length < l, i.e. the index of
   the first length-l symbol in `Huff_val` (also for lengths that have no codes). Symbol[0], Valid[0] = 0.
2. **check_huff_tables** (every present table) — canonical codes per ITU T.81 Annex C: huffsize[] = the lengths in order,
   huffcode[p] = code (code += 1 per symbol, `code <<= 1` per length step; p > 256 → −223); then **`Code[l] =
   huffcode[Symbol[l]]`** for l = 1…16 = the first code of length l (for a length with no codes: the code the next
   symbol would get). Work buffers `vmalloc(0x101)` / `vmalloc(0x202)` (I2: `JPEG_MALLOC`).
3. **JPEG_WriteGrpinf** (`JPEG_WriteGrpinf`) — component index of the Y / Cb / Cr ids (ctx `+0xf28/+0xf29/+0xf2a`) in the
   frame's id list (`+0xe25`, count `+0xe18`; not found → 0); DC table = `+0xe2e[idx]`, AC table = `+0xe32[idx]`; Cr only
   with "3 Huffman tables" (`+0xf7c` = 1); DHT flag = ctx `+0xf14` bit 1 (no DHT → every table number 0, counts 0). The
   struct handed down is I2's `NJPD_Grpinf` {Valid / Symbol / Code pointers for DcLuma, DcChroma, DcChroma2, AcLuma,
   AcChroma, AcChroma2; DHT; bUVHuffman; six u8 counts}, passed by value (r0 = dev, the rest on the stack).
4. **MDrv_NJPD_WriteGrpinf** (G table; VA = MDrv per-device word `+0x6c + 4·dev`, PA = `+0x80 + 4·dev`) — only if DHT:
   store the six counts per device (`+0x6a/+0x74/+0x76/+0x78/+0x7a/+0x7c + dev`); for k = 0…15 (code length k+1), entry
   (4 bytes): `[0]` = Symbol[k+1] (+ the DC symbol count of the same component for the AC tables), `[1..2]` = Code[k+1]
   (u16 LE) **only when Valid[k+1]**, else left as they were, `[3]` = Valid[k+1]; the 3-table flag (`bIs3`) is `+0x68 + dev`. Then
   always: flush VA 0x180 B, J:01[12] (G-table reload) = 1, J:22/J:23 = PA (+0).
5. **MDrv_NJPD_WriteSymidx** (H table at VA + 0x400) — only if DHT: for i = 0…11, then i = 0…161: Y column `[0]` ← DC,
   then AC `Huff_val` of the Y tables while i < count; C column `[1]` likewise; Cr column `[2]`: with 3 tables the Cr
   tables' values (while i < the Cr count), otherwise **a copy of the C values** (while i < the C count); each column
   has its own running index (DC symbols ≤ 12 first, then AC ≤ 162), byte `[3]` is never written. N = the maximum of
   the three column lengths, rounded: kept if N mod 4 ∈ {2, 3}, else (N / 4)·4 + 2 (binary: bit 1 clear → `(N >> 2)·4 +
   2`; no DHT → N = 2). Flush VA + 0x400, N·4 bytes; J:01[11] (H-table reload) = 1; J:20/J:21 = PA + 0x400; **J:26 = N** —
   `HAL_NJPD_IsNeedToPatch(1)` (the EAGLE patch → 0xff) is `return 0` in this build.
6. **WriteIQTbl** (Q table) — byte *c* of entry *k* = low 8 bits of the quantiser of component *c* (1 component: all
   three bytes the same table; 2 components: comp0, comp1, comp1; 3 components: comp0 / 1 / 2 and J:00[7] = 1); entry *k*
   = the DQT element at zig-zag index `T[k]`, `T` = the 64-byte table at `.rodata+0xd772`
   ([I] entry *k* is the coefficient at row k mod 8, column k div 8). 16-bit
   DQT tables are truncated to 8 bits. No DQT seen → default tables (`.rodata+0xd7b2` for byte 0, `+0xd832` for bytes
   1–2: the standard Annex-K luma / chroma tables in column order).

When no DHT was seen, the parser loads the four standard Annex-K Huffman tables from `.rodata+0xd8eb` (a complete DHT
segment body, length 0x1a2; `tables/default-dht.bin`) — the MJPEG convention. Without DHT the write path is the same as
I2's: counts 0 → N = 2, G table untouched, both reload bits still set.

Differences from I2 in these writers [R]: a device index everywhere (I2: the global `eNJPDNum`); the table VA / PA come
from the per-device words set by `MDrv_NJPD_InitBuf` (`+0x6c` = table VA, `+0x80` = output start, or the IMI table
address when the IMI flag is set) instead of `MS_PA2KSEG1(u32TablePhyAddr)`; cache maintenance is `CamOsMemFlush` of
exactly the written range (0x180 B / N·4 B) instead of I2's global `MsOS_FlushMemory()`, and in WriteSymidx the flush
comes after N is computed (I2: before; no effect on content); no `_NJPD_PrintMem` dumps and no verification-mode
branches. Debug lines (MDrv level bit 1, never set, §5.5) would print at DC indices 0, 5, 11 (mask 0x821) and AC indices
0, 50, 100, with labels that do not always match the column written ("ac luma else" for the DC Cr copy).

### 5.5 Debug dump `HAL_NJPD_Debug(dev)` [R; register names from I2 `halNJPD.c` 2293–2420 [S]]

Caller chain: `MApi_JPD_GetEventStatus` on an error event (flags & 0xa3e, §4.5) → **`MDrv_NJPD_Debug(dev)`** → `HAL_NJPD_Debug(dev)` (0x40098c64); then, if the MDrv debug
level (`gpJPDDeviceNode+0x38` block, byte `+0x58`) has bit 1, "[offset: MRC0: MRC1]=[a, b, c]" with a = (READ_Start/16 −
MRC0_Start/16 + 1) mod 16, b = (MRC0_End/16 − MRC0_Start/16 + 1) mod 16, c = (MRC1_End/16 − MRC1_Start/16 + 1) mod 16,
from the per-device address words at block `+0x48`, `+0x08`, `+0x18`, `+0x28`, `+0x38` (two 8-byte entries per array,
indexed by dev) — the I2 formula [S `drvNJPD.c` 1425] with I2's `MDrv_NJPD_TableRead` call removed.

`HAL_NJPD_Debug`: every print is gated by **HAL flags bit 2** (`u32NJPDRiuBaseAdd` byte +4). dev 0 uses the NJPD0 bank,
dev 1 the NJPD1 bank (the messages say "NJPD1" / "NJPD2" for them, as in I2).
1. Reads printed: J:70, J:71, J:72, J:73 (MARB_CRC_RESULT_0…3), J:00, J:01, J:02 (GLOBAL_SETTING00…02), J:0e
   (MIU_READ_STATUS), J:1e, J:1f (MIU_READ_POINTER_ADDR_L/H).
2. **Debug bus**: for select = **0x00, 0x01, … 0x7b** (124 values): write J:32 (DEBUG_BUS_SELECT) = select — as a 16-bit
   write or, if the bank byte address were odd, low byte = select and the next byte = 0 — then read **J:33 (H) and J:34
   (L)** and print `BK_NJPD1_DEBUG_BUS[H:L] = [sel][H:L]`. Then **select = 0xff**, one more read / print. J:32 is left at
   **0xff**. What each select value routes is named neither by I2 (a plain loop [S]) nor by this build; the meaning of
   0x00…0x7b is not known (no vendor register map or SoC TRM is at hand).
3. NJPD register table "NJPD | 00/08 …": J:00…J:7f, 16 rows of 8.
4. Four more 0x80-register tables, banks named by their headers: **"MIU0" = bank 1012**, **"MIU1" = bank 1006** (as in
   I2), then three not in I2: **"1608"**, **"160F"**, **"1615"** (base offsets 0xc1000, 0xc1e00, 0xc2a00). What these three
   banks are is not known (no DT entry or other note names them).

The function only reads, apart from J:32. Reachability: the HAL flags byte is written only by `HAL_NJPD_SetDbgLevel`,
reached only from `MDrv_NJPD_SetDbgLevel`, which nothing calls; the byte stays 0 (`.bss`); the MDrv level byte `+0x58`
likewise. So whenever `MApi_JPD_GetEventStatus` sees an error event, J:32 is cycled through 0x00…0x7b and left at 0xff,
and nothing is printed [R + I effect]; the dumps themselves are unreachable output (unused). `MApi_JPD_GetEventStatus`
is called only from `MHAL_JPD_CheckDecodeStatus`, which mi_jpd does not import (§3.2), so on mi_jpd's path the dump does
not run [R: call graph; I for the path].

## 6. Software JPEG layer (header parser, progressive decoder) [R unless tagged]

### 6.1 Function map

Names as the binary has them; unnamed functions matched to I2 `apiJPEG.c` by behaviour, error codes and call shape ([I]
for those names).

| binary: name, or address if unnamed | I2 `apiJPEG.c` | role |
|---|---|---|
| 0x400934d8 | `JPEG_init` (part) | parser state reset (§4.1 step 2) |
| `JPEG_Init_ParseAddr` | — | read pointers from the InputBuf (and the thumbnail copy, dead) |
| 0x40093e3c | `JPEG_get_char` | next byte from segment 1, then segment 2; synthetic EOI at the end (§6.6) |
| 0x40093e0c | `JPEG_skip_bytes` | advance |
| 0x40093e72 | `JPEG_get_octet` | entropy byte with FF00 un-stuffing (progressive decode, §6.6) |
| 0x40093ed0 | `JPEG_get_bits` (= I2 `JPEG_get_bits_2`) | 32-bit bit buffer `+0xe40`, bit count `+0xe3c` |
| 0x40093f4a | `JPEG_huff_decode` | 8-bit look-up + tree walk |
| 0x40094dae | `JPEG_process_markers` | marker loop (§6.2) |
| 0x40094bb2 | `JPEG_read_dht_marker` | DHT |
| 0x4009494a | `JPEG_read_default_dht` | built-in Annex-K tables (§5.4) |
| 0x40094d7c | `JPEG_skip_variable_marker` | length-prefixed skip |
| `JPEG_read_sof_marker` | same | SOF (§6.4) |
| `JPEG_locate_sof_marker` | same | SOI + markers up to SOF |
| `JPEG_locate_sos_marker` | same | markers up to SOS + SOS parse |
| `JPEG_Get_ScaleInfo`, `JPEG_calc_block` | parts of `_MApi_JPEG_DecodeHdr` | scale-down, MCU geometry (§6.5) |
| 0x40094818 | `JPEG_calc_mcu_block_order` | blocks per component, MCU order (§6.6) |
| 0x40094af8 | `JPEG_init_scan` | table checks, DC predictors, restart counter, bit-buffer prime |
| `JPEG_DealHdrInfo` | — | ScaleInfo + calc_block + MCU order + init_scan |
| `JPEG_check_huff_tables`, `JPEG_check_quant_tables` | same | §6.4, §5.4 |
| 0x40092d08 / `MDrv_NJPD_WriteGrpinf` | `JPEG_WriteGrpinf` | §5.4 |
| (inline in `JPEG_StartDecode`) / `MDrv_NJPD_WriteSymidx`, `WriteIQTbl` | `JPEG_WriteSymidx`, `JPEG_WriteIQTbl` | §5.4 |
| 0x40092f28 | `JPEG_StartDecode` | §4.3 |
| 0x40093566 / 0x4009368c | `JPEG_write_RLE` / `JPEG_do_RLE` | software-VLD feeding (§4.4, §6.6) |
| `JPEG_load_next_row` | same | one MCU row of coefficients → RLE |
| `JPEG_init_progressive` (0x400956c6), `JPEG_decode_block_{dc,ac}_{first,refine}` | same | software progressive decode (§6.6) |
| 0x4009567a / 0x40095620 / 0x4009526c | `JPEG_coeff_buf_open` / `JPEG_alloc` / `JPEG_coeff_buf_getp` | coefficient buffers (§6.6) |
| `JPEG_MEMORY_{init_mempool,malloc,free}`, `JPEG_free_all_blocks` | `jpeg_memory.c` | first-fit free list (8-byte headers, split if ≥ 0x20 left) over the intermediate buffer from +0x10080 (progressive only); ≤ 50 blocks tracked at ctx `+0xe48` |
| 0x400941e2 | `JPEG_terminate` | set the error code, free the blocks, print "%s:ERROR= %d" |
| `JPEG_Thumbnail_Init` | — | `return 0` |

Absent compared with I2: EXIF / MPO parsing, CMYK / RGB conversion, software baseline decode (IDCT), thumbnail decoding
logic beyond the copy, the MJPEG helpers, all `MApi_JPEG_*` getters.

### 6.2 Marker handling

- `JPEG_locate_sof_marker`: SOI must be the first two bytes, else search ≤ 0x1000 bytes for FF D8 (fail → 0). If the
  device's DT `EOI` property is 1 (the camera's: 0), the last two input bytes must be FF D9 (else −217, printed "JPD not
  end with FF D9"). Then the marker loop; the SOF found decides: C0 / C1 → baseline; **C2 → progressive**: needs
  `progressive_support` ≠ 0, else "ERROR! Progressive mode Need Verify More…" and failure; C9 → −215 (arithmetic coding);
  any other SOF (C3, C5–C7, CA, CB, CD–CF) or SOI / EOI / SOS first → −218. The SOF parse also records the header length.
- Marker loop (`JPEG_process_markers`): skip to FF, skip fill FFs; FF00 → continue; `FF01` (TEM), C8, D0–D7 (RST) → −216; CC (DAC)
  → −215; **C4 DHT** → `read_dht` (§6.3; flag `+0xf14` bit 1); **DB DQT** → up to 4 tables at ctx `+0xc10 + id·0x82`
  (the precision bit selects 8- / 16-bit values; id bits 2–3 must be 0, else −204; short segment → −219; length < 2 →
  −203; flag `+0xf14` bit 0); **DD DRI** → length must be 4 (else −211), interval → `+0xe44`; **E0 APP0** → skipped (the
  JFIF-thumbnail branch needs ctx `+5`, never set); **E1 APP1** → payload copied to the start of the intermediate buffer
  VA (the condition "not MJPEG and no thumbnail" is always true here, so this runs for every JPEG with an Exif block;
  length < 2 → −235) — nothing reads the copy (its consumer, the thumbnail path, is dead, §7); DC, DE, DF, E2–FE →
  skipped; SOFn, SOI, EOI, SOS → returned to the caller.
- SOS (`JPEG_locate_sos_marker`): EOI before SOS → fail; another marker → −216; components in the scan 1–4 and length =
  2n + 6 (else −212); the component id must exist (−213); DC table id → `+0xe2e+c`, AC → `+0xe32+c` (= id + 4); 3
  components with different Cb / Cr tables → "3 Huffman tables" `+0xf7c` = 1; Ss / Se / Al / Ah → `+0xe36` / `+0xe37` /
  `+0xe38` / `+0xe39`, forced to 0 / 63 (Ss / Se) for baseline.

### 6.3 Huffman table records and DHT parse

DHT: per table: class / id byte, 16 counts (sum ≤ 255, else −200), symbols; index i = id + 4·class (≤ 7, else −201);
segment overrun → −202. Record i at ctx `+0x150 + i·0x158`:

| ctx offset (+ i·0x158) | record offset | I2 field | set by |
|---|---|---|---|
| +0x150 | +0x000 | `u8Huff_num[17]` (`[0]` = 0, `[l]` = codes of length l) | read_dht |
| +0x161 | +0x011 | `u8Huff_val[256]` (symbols in DHT order) | read_dht |
| +0x261 | +0x111 | `u8Symbol[17]` | read_dht |
| +0x272 | +0x122 | `u16Code[17]` | check_huff_tables |
| +0x294 | +0x144 | `u8Valid[17]` ("length used" flags) | read_dht |
| +0x2a5 | +0x155 | `u8SymbolCnt` (u8, total symbols) | read_dht |
| +0x2a6 | +0x156 | `bValid` (table present) | read_dht (set once) |

read_dht writes the last two bytes as `ctx + 0x158 + 0x14d / 0x14e + i·0x158`. The arithmetic of `Symbol`, `Valid` and
`Code` and the G / H tables built from them: §5.4.

### 6.4 SOF and table checks

- Precision must be 8 (−205). Height 0 or > limit → −206; width 0 or > limit → −207. Limit = baseline maximum (SetParam 0)
  × 8, clamped to 0xfff8 when the value ≥ 0x1fff — mi_jpd's 0x2000 gives **65528**. Progressive (with decode type 0):
  limit = progressive maximum (SetParam 1) × 8, and the image must fit the intermediate buffer:
  `((h + 7) & ~7) · ((w + 7) >> 3) >> 3 · 0x186 + 0x10130 ≤ buffer size` (else "Progressive image size too big…", −233).
- Components ≤ 4 (−208), segment length = 3n + 8 (−209); a component id 0 switches the id map to {0, 1, 2}; one
  component forces sampling 1×1.
- `JPEG_check_huff_tables`: every DC / AC table a scan component uses must be present (−223); builds the code tables
  (§5.4 step 2); for progressive also the software look-up tables at `+0x1094 + t·0x700` (§6.6).
- `JPEG_check_quant_tables` (only if a DQT was seen): every component's table present (−222).

### 6.5 Supported layouts, scale-down, output size

`JPEG_calc_block` (scan type `+0xf12`, MCU size `+0xf52/+0xf53`, blocks per MCU `+0xf56`):

| components / sampling (Y; Cb, Cr must be 1×1) | scan type | MCU | blocks |
|---|---|---|---|
| 1 (gray) | 0 | 8×8 | 1 |
| 3, Y 1×1 | 1 (4:4:4) | 8×8 | 3 |
| 3, Y 2×1 | 2 (4:2:2) | 16×8 | 4 |
| 3, Y 1×2 | 3 (4:4:0) | 8×16 | 4 |
| 3, Y 2×2 | 4 (4:2:0) | 16×16 | 6 |
| 3, Y 4×1 | 5 (4:1:1) | 32×8 | 6 |

Rejected: 2 or 4 components (CMYK / YCCK) → −225; Cb / Cr ≠ 1×1, other Y factors, and 3-component 1×1 files whose first
component id is 'G', 'B' or 'R' (RGB JPEG) → −226 (on the baseline path `JPEG_DealHdrInfo` then overwrites the code with
−215). Also `blocks per MCU × MCUs per row` ≤ 6144 (−229).

`JPEG_Get_ScaleInfo`: aligned width = width rounded up to 8·H0, aligned height to 8·V0; effective maximum = SetParam 0
(baseline) or 1 (progressive); ratio = 8 if width > 4·max or height > 4·max, else 4 if > 2·max, else 2 if > max, else 1
(code `+0xf11` 3 / 2 / 1 / 0). Output (`+0x148/+0x14a`, what `ExtractJpgInfo` returns) = aligned size / ratio; with a
ratio the pitch `+0xf60` is the aligned width rounded down to a multiple of 16 / 32 / 64 (ratio 2 / 4 / 8) and the ROI cuts
the picture to it (§4.3 step 7). With mi_jpd's maximum 8192, pictures up to 8192×8192 decode 1:1 and larger ones are
halved, quartered or cut to an eighth. The output is always YUV 4:2:2 packed (YUYV, §5.3).

### 6.6 Progressive JPEG (software decode; only with `progressive_support=1`, off by default — unused on the camera)

Reference: I2 `apiJPEG.c` [S] (a jpgd-derived decoder). Every statement of the functions below matches I2 except where a
difference is listed [R binary against I2].

**Context fields** (ctx = the MApi JPEG context): `+0x7` u8 tem flag (synthetic-marker toggle); `+0x1c` / `+0x20` read
pointers of input segments 1 / 2, `+0x28` bytes in segment 2, `+0x2c` bytes left; `+0x148/+0x14a` output (aligned,
scaled) size, `+0x14c/+0x14e` original SOF size; `+0xe18` components in the frame, `+0xe19+c` / `+0xe1d+c` H / V sampling,
`+0xe29` components in the scan, `+0xe2a+n` scan component list, `+0xe2e+c` / `+0xe32+c` DC / AC table of component c;
`+0xe36` Ss, `+0xe37` Se, `+0xe38` Al, `+0xe39` Ah; `+0xe3c` s16 bits left, `+0xe40` u32 bit buffer (`+0xe41` = its bits
8–15); `+0xe44` restart interval; `+0xf2c+2c` / `+0xf34+2c` component H / V blocks; `+0xf3c` blocks per MCU, `+0xf44` /
`+0xf46` MCUs per row / column, `+0xf48+n` MCU block → component; `+0xf54` / `+0xf58` frame MCUs per row / column
(coefficient-buffer size); `+0xf5a` restarts left, `+0xf5c` next RST number; `+0x1094 + t·0x700` software Huffman table t
{s16 look_up[256] +0x000, s16 tree[512] +0x200, u8 code_size[256] +0x600} (built by `check_huff_tables` when
progressive); `+0x4894+4c` DC coefficient buffer, `+0x48a4+4c` AC coefficient buffer, `+0x48b4+4c` last DC value,
`+0x48c4` EOB run.

**Bit reader:**
- `JPEG_get_char` (0x40093e3c): bytes left = 0 → toggle `+0x7` and return **0xFF when it was 0, 0xD9 when it was 1**
  (an endless synthetic FF D9) — as I2. Else the byte from segment 1 while left > the segment-2 length, then from segment
  2; left −1. I2 instead refills from its stream (`JPEG_fill_read_buffer`); there is no refill here — the whole input is
  in the two segments.
- `JPEG_get_octet` (0x40093e72) — I2's `get_charP` + `get_octet` + `stuff_char` folded together: left = 0 → toggle
  `+0x7`, return **0** (padding); byte ≠ 0xFF → it; 0xFF as the last byte → push 0xFF back, toggle `+0x7`, return 0xFF;
  `FF 00` → 0xFF (stuffing removed); `FF xx` (a marker) → push `FF xx` back, return 0xFF — so the bit reader then sees
  0xFF bytes until the scan loop consumes the marker. Equivalent to I2. **Difference**: it reads only through the
  segment-1 pointer (`+0x1c`) and ignores the segment-2 split that `get_char` honours, so a progressive input split in two
  segments would be read wrongly past segment 1 [R; I consequence]; mi_jpd's ring records never wrap, so segment 2 is
  empty there [I].
- `JPEG_get_bits` (0x40093ed0): result = (buf >> (16 − n)) & ((1 << n) − 1) of the buffer **before** the
  update; bits_left −= n; if bits_left ≤ 0: buf = rol(buf, n + bits_left), c1 = get_octet, c2 = get_octet, buf = (buf &
  0xffff) | c1 << 24 | c2 << 16, buf = rol(buf, −bits_left), bits_left += 16; else buf = rol(buf, n). (rol is written as
  ror by (32 − k) & 31.) Identical to I2.
- `JPEG_huff_decode` (0x40093f4a): s = look_up[(buf >> 8) & 0xff]; s ≥ 0 → `get_bits(code_size[s])`; s < 0
  → `get_bits(8)`, then repeat s = tree[−s − get_bits(1)] while s < 0 (binary: index 0x100 − s − bit into the look_up
  base = tree index −s − bit; I2: `tree[~s + (1 − bit)]`, the same). Identical.
- `HUFF_EXTEND(r, s)` = r < extend_test[s] ? r + extend_offset[s] : r, with extend_test[s] = 2^(s−1), extend_offset[s] =
  (−1 << s) + 1: the binary's two 16-entry int tables are byte-identical to I2's (one copy each in mhal.ko, file offset
  0x1025c4 and +0x40) [R byte search].

**Block decoders** (args `(ctx, comp, block_x, block_y)`, return 1 ok / 0 error):
- `JPEG_decode_block_dc_first` (0x40095296): p = coeff_buf_getp(DC buffer of comp, x, y) (NULL → print "%s [%d]" line
  0xaed, return 0); s = huff_decode(DC table); s ≠ 0 → r = get_bits(s), s = EXTEND(r, s); last_dc[comp] += s; p[0] =
  last_dc << Al. Identical.
- `JPEG_decode_block_dc_refine` (0x4009531c): if get_bits(1): p = getp(DC buffer) (NULL → print line 0xada, return 0),
  p[0] |= 1 << Al. Identical.
- `JPEG_decode_block_ac_first` (0x40095378): **added**: if block_x > ((original W + 7) >> 3) − 1 or block_y > ((original
  H + 7) >> 3) − 1 → return 1 at once (no bits read, EOB run not decremented). Then as I2: EOB run ≠ 0 → run −1, return
  1; p = getp(AC buffer) (NULL → print line 0xba2, return 0); for k = Ss … Se: s = huff_decode(AC table), r = s >> 4, s
  &= 15; s ≠ 0 → k += r (> 63 → error −227, return 0), p[k] = EXTEND(get_bits(s), s) << Al; s = 0, r = 15 → k += 15
  (> 63 → −227); s = 0, r < 15 → EOB run = (1 << r) + (r ? get_bits(r) : 0) − 1, stop. I2's CMYK / RGB zig-zag variant
  is compiled out.
- `JPEG_decode_block_ac_refine` (0x40095488): p = getp(AC buffer) first, then the **same added bounds check** (out of
  range → return 1, nothing read, EOB run unchanged; a NULL p matters only inside the range: print line 0xb0f, return 0).
  p1 = 1 << Al, m1 = −1 << Al, k = Ss. EOB run = 0: while k ≤ Se and k < 64: s = huff_decode, r = s >> 4, s &= 15; s ≠ 0:
  s must be 1 (else −227), s = get_bits(1) ? p1 : m1; s = 0 and r ≠ 15: EOB run = (1 << r) + (r ? get_bits(r) : 0), leave
  the loop; then walk: for each coefficient k (k ≤ Se, k < 64): non-zero → if get_bits(1) and (coef & p1) = 0: coef +=
  (coef ≥ 0 ? p1 : m1); zero → r −1, at r < 0 stop there; after the walk, s ≠ 0 and k < 64 → p[k] = s; k += 1. EOB run
  > 0: for the remaining k ≤ Se, k < 64, the same correction bit for every non-zero coefficient; EOB run −1. Identical to
  I2 apart from the check.

**Coefficient buffers:** `JPEG_coeff_buf_getp` (0x4009526c): x < bx and y < by → data + (y·bx + x)·block_bytes; else
`JPEG_terminate(−229)` (sets ctx `+0xc`, frees all blocks) and NULL. Buffers from `JPEG_coeff_buf_open`:
header 12 B {data, u16 bx, u16 by, u16 block_bytes = w·h·2, u8 w, u8 h}, data bx·by·block_bytes, both through
`JPEG_alloc` (`JPEG_alloc`: zeroed, ≤ 50 tracked blocks at ctx `+0xe48`, else −221; out of pool memory → −233). Per
component a DC buffer (1×1-block elements) and an AC buffer (8×8 × s16) of (H·MCUs per row) × (V·MCUs per column)
blocks, for the whole MCU-aligned picture.

**Block counts and the added bounds check** [I, from the arithmetic]: `JPEG_calc_mcu_block_order` computes the
component block counts from the **output size `+0x148/+0x14a`** (MCU-aligned), where I2 uses the original image size:
`blocks = ((size·samp + max_samp − 1) / max_samp + 7) / 8` [R binary against I2 `JPEG_calc_mcu_block_order`]. For the luma
component of a subsampled picture whose ⌈W/8⌉ is odd (e.g. W = 1000 in 4:2:0: aligned 1008 → 126 blocks, coded 125) a
non-interleaved scan would iterate one block more per row than the encoder wrote; the added check (original size) skips
exactly that block in AC scans, so the bit stream stays in step. Chroma counts agree in both computations (⌈W/16⌉). DC
scans have no such check, so a **non-interleaved DC scan** of such a picture would read the extra blocks from the bit
stream and lose sync; common encoders emit interleaved DC scans, which iterate whole MCUs and are not affected. With a
scale-down ratio > 1 (`+0x148` divided) the counts would be wrong for every scan; with mi_jpd's maximum 8192 and the
intermediate-buffer cap (§6.4) the ratio is 1 for every progressive picture that is accepted.

**Scan loop** (inlined in `JPEG_init_progressive`, 0x400956c6; = I2 `JPEG_init_progressive` + `JPEG_decode_scan`): four
components → −225. Allocate the coefficient buffers. Then per scan until `locate_sos` finds EOI: MCU order, init_scan
(`JPEG_init_scan`; a failure takes the EOI exit below and the function returns 1); Se < Ss or Se > 63 → −230; Ss ≠ 0 → AC
first (needs exactly one component, else −230); Ss = 0 → DC first (needs Se = 0, else −230); Ah ≠ 0 → refine, Al must be
Ah − 1 (else −231). For every MCU row and column: if restarts are on (`+0xe44` ≠ 0) and the count `+0xf5a` reached 0:
search ≤ 0x600 bytes for 0xFF, skip fill 0xFF, the marker must be RST(`+0xf5c`) (else −228); clear the DC predictors and
the EOB run, next RST = (n + 1) & 7, bits left 16, count = interval, refill the bit buffer (`get_bits(16)` twice). Then
one call per MCU block: non-interleaved → block_x = the column offset of the component (+1 per call); interleaved → x
within H, y within V of the component, the offsets advancing by H per MCU; after each MCU row the row offsets advance by
1 (non-interleaved) or by V (interleaved). After the last MCU row: bits left = 0, next scan. At EOI: components in scan :=
components in frame, list 0…n−1, MCU order recomputed, return 1.

**Feeding the engine (software VLD):** `MApi_JPD_Start_Decode` walks the MCU rows (`JPEG_load_next_row`) and feeds each
block to the hardware in software-VLD mode (§4.3 step 10) as 32-bit words (`JPEG_do_RLE`): first `0x8000 | ((DC −
previous DC of the component) & 0xfff)`, then per non-zero AC coefficient `(zero_run << 12) | (value & 0xfff)`, runs > 15
emitted as `0xf000`, and `0x10000` as end of block when coefficient 63 is zero; the last word of the picture carries the
"last buffer" handling (`JPEG_write_RLE`, §4.4). The read window is the input buffer itself (size rounded down to 32
bytes) used as two ping-pong halves.

### 6.7 Error codes left in ctx `+0xc` [R values; names from I2 `apiJPEG.h` [S]]

−200…−243 as `JPEG_ErrCode`; used here: −200 −201 −202 −203 −204 −205 −206 −207 −208 −209 −210 −211 −212 −213 −215
−216 −217 −218 −219 −221 −222 −223 −225 −226 −227 −228 −229 −230 −231 −233 −235 −240 −243 (−215 also stands for arithmetic coding — SOF9, DAC — and for a layout rejected
on the baseline path, §6.2, §6.5). Named ones: −227 `E_JPEG_DECODE_ERROR`, −233 `E_JPEG_NOTENOUGHMEM`; 0x5566 `E_JPEG_JPD_DECODE_ERROR` (hardware
timeout or error in software VLD, §4.4). The code is visible only in `/proc/jpd/histdec_param` (§2.7); MHAL returns its
own codes (§3.2).

## 7. Unused and unreachable parts [R unless tagged]

Reachability from the 15 exports, the module init, the ISR, the proc and sysfs hooks, the 17 folded tail-call thunks
included (a call graph of the block). 91 of 294 functions are never reached, and no relocation outside the JPD
text references them. Beyond that, whole paths are dead because their switch field is never written (no store to the
field in the block):

| dead path | why | what it would do |
|---|---|---|
| thumbnail decode | decode type `+0x38` never set (the descriptor's `eDecodeMode` is not copied) → `+0x134/+0x135` never set | decode the Exif thumbnail: input = intermediate buffer + offset `+0x140` (the APP1 copy, 64 KiB window 0x10080), read window in `NJPD_BufCfg.bThumbnailAccessMode` |
| MJPEG mode | `+4` (bIsMjpeg) only ever cleared | skip the APP1 copy; table base = the thumbnail buffer (I2 [S]) |
| JFIF APP0 thumbnail | `+5` never set | parse the APP0 thumbnail size into `+0x90/+0x92/+0x94` |
| write protection | SetParam 8 = 9 never sent by mi_jpd | §4.3 step 12 |
| FPGA hooks | SetParam 10 never sent | §3.4 |
| second device (`sstar,njpd1`, bank slot 1) | not in the camera's DT; mi_jpd uses dev 0 | everything per device id; frees that use slot 0 only (§3.2) |
| progressive JPEG | module parameter `progressive_support` = 0 by default | §6.6 |
| `MHAL_JPD_CheckDecodeStatus`, `SetDebugLvl`, `Decodetimes` | not imported by mi_jpd | §3.2 |
| SetParam 4, GetParam 0 / 1 / 2 / 4 / 6 / 7 | not used by mi_jpd | §3.4, §3.5 |
| debug dumps (`HAL_NJPD_Debug` prints, MDrv MRC print) | HAL flags / MDrv level never set | §5.5 (only the J:32 cycling runs) |

Never-reached functions, by group:
- **Utopia ISR path**: `MDrv_NJPD_EnableISR` / `DisableISR` (attach `_NJPD_ISRHandler` through the OSAL stubs, which
  return 0, so they would always fail), `_NJPD_ISRHandler` (sets a flag), `MDrv_NJPD_Get_ISR` / `Set_ISR`,
  `MApi_EnaleIrq` / `DisaleIrq` (`CamOsIrqEnable/Disable(dev+0x58)`), all eight `OSAL_NJPD_*` (stubs).
- **Power / clock / port select**: `HAL_NJPD_PowerOn` / `PowerOff` / `SetClock` / `GetClock` / `JPDSelect`,
  `MDrv_NJPD_PowerOff` (§2.2, §5.3), `HAL_NJPD_Create/DeleteMutex` (OSAL stub mutex).
- **Table read-back (TBC)**: `MDrv_NJPD_TableRead` — J:02[1] (TBC mode) off; for each entry: J:40 = `index << 8 | sel`
  (sel 0x10 H table, 0x12 G table — 0x40 or 0x60 entries, 0x14 Q table — 0x40 entries), J:08 = 0x40
  (`SetWriteOneClearReg_2`, TBC enable), wait for J:40 bit 4, read J:43 / J:44 (4 bytes, printed); J:02[1] back on, J:40
  restored. Accessors `HAL_NJPD_{Set,Get}RIUInterface`, `TBCReadData_L/H`, `GetHTableSize`.
- **Handshake row-count modes** (a direct JPD → consumer handshake without the write ring):
  `HAL_NJPD_Handshake_SWRowCountMode(dev, n)`: J:75 high byte |= 4, |= 2, J:75 low byte = n, J:08 = 0x200 (SW WOC);
  `Handshake_HWRowCountMode`: J:75 high [0] 0→1, [2] = 0, [1] = 1; `HAL_NJPD_GetHandshakeCnt` = J:74 high byte.
- **Other accessors never called**: `HAL_NJPD_ForceEventFlag` (J:2a), `GetMWCBuf_Start` / `WritePtr` (J:1a–1d) and
  `MDrv_NJPD_GetMWCStartAddr` / `GetWritePtrAddr`, `Get/SetSpare01/03–07` (J:49, J:4b–4f), `Get/Set_MARB06/07` (J:56/57),
  `SetMRBurstThd` (J:54[4:0]), `SetMTLBMode` (J:2f high byte), `SetCRCWriteMode` (J:6d = 0x300),
  `GetWriteOneClearReg`, `SetWriteOneClearReg_2` (only from TableRead), `SetDbgLevel` / `GetLibVer` (HAL; version string
  "NJPD.HAL.00117630"), `SetNJPDInstance`, `JPDCount`, `GetVerificationMode` (all trivial), `SetWriteProtect` and
  `SetMWBuffLineNum` (empty bodies), `MDrv_NJPD_SetReadBuffer0/1` (old read-buffer API: J:12–15 / 16–19 for one half
  each), `MDrv_NJPD_GetWRingBuf_ENABLE` (J:4a[15]), `MDrv_NJPD_GetCurVidx` (prints "Do not support"),
  `MDrv_NJPD_GetInfo` (lib version + 3 bytes of `.rodata`), `MDrv_NJPD_GetLibVer`, `MDrv_NJPD_GetCurRow/Col/MRCAddr`
  thunks (the used path calls the HAL ones through `MDrv_NJPD_GetStatus`), `MDrv_NJPD_GetSpare`, `MDrv_NJPD_JPDCount`,
  `MDrv_NJPD_SetDbgLevel` (§5.5).
- **Context helpers**: `JPD_CtxRelease` (unlock + destroy the mutex + deinit the semaphore + free — the variant that would
  have destroyed the mutex; the used hook is `JpdCtxRelease`), `MApi_JPD_Get/Set_GCtx` (global context slots
  `.bss+0x1e3a0`), timing helpers `_JpdTimespecDiffNs`, `JpdTimespecDiffNs`, `MAsm_CPU_Sync` (dsb), `printfMEM`,
  `_NJPD_PrintMem` (hex dump), `JPEG_Thumbnail_Init` (`return 0`).

