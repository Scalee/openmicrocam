# ldc-1-stack — mi_ldc.ko, libmi_ldc, mhal LDC, the LDC config and calibration file formats
<!-- ev:ldc-1-stack -->

The lens-distortion-correction (LDC) stack of the SigmaStar media software on this camera, from the user library down to
the registers. Loaded at every boot, used by nothing the product runs (§1.3):

- **mi_ldc** (`mi_ldc.ko`, `/dev/mi_ldc`, MI module id 0x17) — one device (dev 0), up to 16 channels, 1 input and 1
  output port per channel, one mi_sys pass on its own worker thread. A channel is configured by a 0x54-byte attribute
  carrying a configuration buffer (JSON, text options, a text-file path or a container of ready tables) and optional lens
  calibration data; from these it builds one table ("bin") per output view and, per input frame, programs the engine
  block by block through mhal, directly or through a CMDQ (§2, §4–§7).
- **libmi_ldc.so** — 11 exports, ioctl wrappers plus the conversion of `GetChnParam` and a device-count-driven close of
  the fd (§3).
- **The configuration and calibration formats** — the channel attribute, the multi-view / single JSON, the text options,
  the DBIN container of ready tables, `CalibPoly_new.bin` (§4, §5.3).
- **mhal LDC** — the HAL in `mhal.ko` (89 functions, ≈15 KB): RIU banks 0x12F0 / 0x12F1, IRQ 0x4B, clock `ldcfeye`
  (CLKGEN 1038.53), per-block programming, trigger, fences and re-trigger, sysfs (§8).

Other documents: the map-generation library inside mi_ldc.ko ("eptz lib": the complete byte layout of the table, tiling,
the projection and lens models with their derived constants, the full text-option table, the library's return codes, the
Simd and MVE parts) — `ldc-2-eptz-lib.md`; the user-side ioctl wrapper, fd handling, return convention, MI error-code
format and module ids — `mi-user-abi.md` (§2 "The device fd", §3 "The ioctl call", §4 "Return values and error codes");
the mi_common device frame that creates `/dev/mi_*`, the per-process hooks, the per-module procfs files and the private
heaps — `mi_sys-2-interfaces.md` (§1 "mi_common.ko — the device frame of every MI module", §7 "Memory"); the registration
contract, pass callbacks, bind types, frame flow and CMDQ objects — `mi_sys-1-engine.md` (§2 "Registration contract", §3
"Binding", §4 "Frame flow", §7 "CMDQ objects and task completion"); the CMDQ service, its engine ids and trigger-bus
events — `mhal-scl-jpe-cmdq.md` §2; mhal.ko as a module, its block table, character devices (`/dev/ldc`), module-parameter
list and DT reads — `mhal-core.md` (§2.1 "Block table", §4.2 "The devices", §5 "Module parameters", §7 "Device-tree
properties mhal reads, per node"); RIU addressing, the chip id, the clock tree (`ldcfeye`) and the DT node `ldc` —
`kernel-platform.md` (§3.1 "Static I/O map and RIU addressing", §3.2 "Chip ops, chip id and revision", §5.1 "The DT clock
tree", §14 "Device tree"); MIU bank 0x1012 — `kernel-platform.md` §12 "MIU: driver, arbiter, BIST, protect"; GOP 5 in LDC
mode on the same clock register — `rgn-gop.md` §4.5; the load line in `demo.sh` — `rootfs-customer.md` §3.2; the inventory rows —
`firmware-image.md` (§3.3 "SigmaStar media stack — `/config/modules/4.9.227`", §3.5 "`/config/lib` (SDK user
libraries)"); prog_uvc's `-L <json>` path (option, start-up, binds, stream stop) and its static copy of libmi_ldc —
`prog_uvc.md` §8 "The LDC path (`-L <json>`, unused)".

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source; **[I]** inference. A
heading tagged [R] means every statement under it is [R] unless marked otherwise. Names of MI structures and fields
(`MI_LDC_ChnAttr_t`, `eMode`, `u16UserSliceNum` …) are the ones the module's own log strings use [R]; function names in
`_MI_LDC_IMPL_*` form are the module's symbols or its `__func__` strings [R]; functions without a name are named here by what
they do [I], with their address.

Binaries (dump of 2026-10-16) [R]: `/config/modules/4.9.227/mi_ldc.ko` (file 733 220 bytes; inventory figure 276 KB /
587 functions, `firmware-image.md` §3.3; 22 exports), `/config/lib/libmi_ldc.so`, `/config/modules/4.9.227/mhal.ko`
block LDC, `/customer/prog_uvc`. Addresses of mi_ldc.ko are Ghidra addresses (= ELF `.text` symbol value − 1 + 0x10030);
addresses of mhal are Ghidra addresses of the rebased image (= `.text` offset + 0x40000028).

Register notation **bank.reg**: 16-bit registers; RIU byte address = bank·0x200 + reg·4 (`kernel-platform.md` §3.1); CMDQ
address = bank·0x100 + reg·2.

## 1. The component on this camera

### 1.1 What mi_ldc.ko is made of [R symbol table + section names; sizes = sum of symbol sizes]

| part | functions | code | origin | described in |
|---|---|---|---|---|
| MI_LDC glue: `MI_LDC_IOCTL_*` (11), `MI_LDC_IMPL_*` (35 exported / global), `_MI_LDC_IMPL_*` statics (29), JSON-param helpers (`MI_LDC_*Json*`, 7), `MI_LDC_EPTZ/DBIN/GENBIN_*` front ends (16) + unnamed statics among them | 98 named + ≈45 unnamed | ≈49 KB + | SigmaStar, no source | §2–§7 |
| map-generation library ("eptz lib": `eptz_*`, `mi_eptz_*`, `world2cam_*`, `ldc_bin_*`, `ldc_split_*`, `st_stitch_*`, `RSC_*`, `bypass_*`, `cylinder_*`, `donut_*`, `erp_*`, `perspective_*`, `map2bin_*`, `map_crop_*`, `region_*`, `out_set_*`, fixed-point `sll*` / `Q15*`) | ≈250 named + ≈45 unnamed | ≈86 KB | SigmaStar algorithm library, no source | `ldc-2-eptz-lib.md` |
| `MI_MVE_*` (9) — software image helpers (alloc / resize / alpha-blend) on Simd | 9 | 3.5 KB | SigmaStar | `ldc-2-eptz-lib.md` §11.1 |
| Simd library (`Simd::Neon::*`, `Simd::Base::*`, comdat sections) + stripped `.text` helpers of it | 100 + ≈50 | ≈175 KB + ≈45 KB | public: ermig1979/Simd (MIT) | `ldc-2-eptz-lib.md` §11.2 |
| cJSON | 74 | 7 KB | public: DaveGamble/cJSON **1.7.12** (`cJSON_Version` prints `"%i.%i.%i", 1, 7, 12`) | §4.4 (only as the JSON parser) |
| `MI_DEVICE_*` (6) | 6 | 0.1 KB | the MI per-module device template (same as every mi_*.ko) | `mi_sys-2-interfaces.md` §1.6 |

The public Simd library (≈220 KB with its helpers) and cJSON (7 KB) make up most of the module's code [R]. The front-end source files named in the
strings: `mi_ldc_eptz.c`, `mi_ldc_genbin.c`, `mi_ldc_dbin.c` [R].

### 1.2 Module facts [R `.modinfo`, `__param`, ksymtab, undefined symbols]
- Version string `Sigmastar Module mi_ldc version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223310`;
  author Sigmastar, licence "GPL v2", `depends=mi_common`, vermagic `4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8`.
- Parameters: `debug_level` (uint), `debug_file`, `debug_func` (string) — the usual MI debug filters; **`CMDQBUFFSIZE`**
  (uint, "set Ldc cmdq buffer size"); **`thread_priority`** (uint, "set ldc work thread priority") (§2.1).
- 22 kernel exports, all `MI_LDC_IMPL_*`: CreateDevice / DestroyDevice / CreateChannel / CreateChannelV3 / DestroyChannel /
  StartChannelExternal / StopChannelExternal / SetChnParam / GetChnAttr / SetOutputPortAttr / GetOutputPortAttr /
  GetInputRect / SetConfig / SetBatchViewConfig / Init / DeInit and the `*External` variants of Create / Destroy Device /
  Channel, Init / DeInit. **No other module on the camera imports any of them** [R: no `MI_LDC*` undefined symbol in any
  `.ko` of `modules/4.9.227`].
- Imports: CamOs (memory, semaphores, rw-semaphores, atomics, IRQ request / free, jiffies, monotonic time, `CamOsChipId`),
  CamFs (Open / Read / Write / Seek / Close — file I/O from the kernel), **12 `MHAL_LDC_*`** (CreateDevice, DestroyDevice,
  CreateInstance, DestroyInstance, GetDevConfig, IsrProc, Trigger, ReTrigger, SetFrmAttr, SetBlkAttr, SetBlkFence,
  SetWDmaAttr), mi_sys (RegisterDev / UnRegisterDev, Enable / Disable Channel / InputPort / OutputPort,
  PrivateDevChnHeapAlloc / Free, Vmap / UnVmap, VFlushInvCache, MemcpyPa, NotifyWakeupISREvent,
  Set{Input,Output}PortBufExtConf, SetOutputPortBufClearConf, RegistCommand), `kernel_neon_begin/end` (Simd and the
  library's floating point run in kernel context), wait-queue primitives, procfs.

### 1.3 Use on this camera
- **Loaded at every boot**: `customer/demo.sh` inserts `mi_ldc.ko` without parameters between mi_ao and mi_vif [R]. Its
  `init_module` (device registration, proc, §2.1) runs although no application opens `/dev/mi_ldc` with the shipped
  command line (`prog_uvc -H -A 0 -a 0 -q -i …`, no `-L`; `prog_uvc.md` §2.1). `init_module` touches no hardware and
  allocates no memory, so the loaded module costs only its own memory [R §2.1; I cost].
- The only user of the stack in the firmware is prog_uvc's `-L <json>` path: it creates channel 0 with eMode 1 (LDC),
  eInfoType 1 (sensor calibration) and the JSON file and `./CalibPoly_new.bin` as user buffers; the device attribute is not
  sent (§2.4). Its binds, depths and stream stop (which destroys the LDC channel that only start-up creates) —
  `prog_uvc.md` §8.
- What the hardware does with a real configuration (the tile layout shown in `/proc/mi_modules/mi_ldc/mi_ldc0`, the clock
  in mhal's `check_*` attributes, whether the table format of `ldc-2-eptz-lib.md` §2 is what the engine expects) is not known:
  every statement of this document comes from static analysis. Only readings on the running camera that change state
  (prog_uvc run with `-L`) show them: the table info (NumRuns, block sizes) of the proc file and mhal's `check_clk`,
  `check_inst_table_info` with a JSON; a stock table dumped with proc `dumptablebin` compared with the layout, then a DBIN
  container (§5.3) fed through `prog_uvc -L` (`docs/firmware/camera-checks.md` P14, P15).
- Whether the map-generation library and MVE run at all depends on the chip id (§4.6 "Chip check"); the chip id of this
  camera has not been read (`kernel-platform.md` §3.2).

## 2. mi_ldc.ko — load, module state, device, ioctls, sys registration [R]

### 2.1 Load and parameters [`init_module` 0x535e8, `cleanup_module`]
- `init_module`: creates `/proc/mi_modules/mi_ldc/` with `debug_level`, `debug_file`, `debug_func`, `module_version_file`
  (mode 0640, the shared mi_common proc ops), prints `module [ldc] init`, `MI_DEVICE_Register` of its device object
  (`.data+0x54`; name "ldc" → `/dev/mi_ldc`, 11 ioctls, connected / disconnected = `mi_ldc_process_init` / `_exit`).
  Nothing else: no hardware is touched at load, no memory allocated. `cleanup_module` removes the five proc entries and
  unregisters.
- Parameters (`__param`, all 0644 — perm word 0x1a4): `debug_func`, `debug_file` (strings), `debug_level` (uint,
  `.data+0x50`), **`CMDQBUFFSIZE`** (uint, `.data+0x88`, default **0x60000** = 384 KiB), **`thread_priority`** (uint,
  `.data+0x8c`, default **0x46** = 70). demo.sh passes none.
- Per-process hooks: `mi_ldc_process_init` (every open of `/dev/mi_ldc`) → `MI_LDC_IMPL_Init()`; `mi_ldc_process_exit`
  (every close) → `MI_LDC_IMPL_DeInit()`.

### 2.2 Module state [`ldc_module_init` (0x13b60) = Init, `ldc_module_deinit` (0x192f0) = DeInit]
`.bss` globals: init count (`_LANCHOR1`, 0x8b6c4), module magic (0x8b5f8): **0x56566666** after `Init` from user space,
**0x56533333** after `InitExternal` (kernel callers); a module semaphore (0x8b604); the device list head (0x8b5fc).
- `Init(ext)`: count 0 or magic invalid → count 1, semaphore init, magic by `ext`, empty device list; otherwise count + 1
  ("already inited, InitCnt") → 0.
- `DeInit(ext)`: count 0 → warning "MI_LDC already DeInit, return ok", 0. Count ≥ 2 → count − 1, 0. Count 1 (last user):
  destroy device 0 with all its channels (`ldc_destroy_device(0, ext)`; for `ext` only if that succeeded it recurses once more),
  count 0, semaphore deinit → 0; invalid magic → `0xA017201F`. Effect [I]: **the last close of `/dev/mi_ldc` destroys
  everything** (as mi_vdisp does) — a crash of the owning process cleans the LDC up.
- In "external" mode every device id is mapped to 0 when looked up (`if (magic == 0x56533333) dev = 0`).

### 2.3 Device (only dev 0) [`_MI_LDC_IMPL_CreateDeviceWrap` 0x16428, `ldc_create_device` (0x1623c)]
`CreateDevice(dev)`: module not initialised → `"Mod is No Init"` log; **dev ≠ 0 → "DEV %d, overrange >= 1"**; an existing
device → warning, 0. Then, under the module semaphore:
1. device record `CamOsMemAlloc(0xA0)` zeroed: +0x00 u8 created, +0x04 dev id, +0x08 magic **0x56577724**, +0x0c mi_sys
   device handle, +0x14 u8 IRQ requested, +0x18 IRQ number, +0x1c mhal device handle, +0x20 CMDQ interface in use [I]
   (0 = direct register mode [I]), +0x24 ISR count per mi_sys wake-up [I] (compared with +0x2c), +0x28 ISR count, +0x2c
   ISR count since the last wake-up, +0x30 wake-ups, +0x34 u8 frame-done flag, +0x38 wait queue
   (`&pstDevInfo->stCompletion`), +0x50 mem-cache `mi_ldc_taskinfo` (object size 4 [R argument], i.e. the per-task private
   data), +0x68 u64 time of the last ISR (µs), +0x70 device semaphore (count 1), +0x90 list head of channels, +0x98 node in
   the module device list.
2. user (non-external) path only: **`mi_sys_RegisterDev`** (§2.5); failure → `"Fail to register sys dev"`, `0xA017201F`.
3. `MHAL_LDC_CreateDevice(0, &rec+0x1c)`; failure → `"Create mhal_ldc dev %d failed"`, unregister, free.
4. `MHAL_LDC_GetDevConfig(h, 1, {0xC, 1, out})` → `out` = CMDQ buffer length mhal needs (0x38400, §8.1; "Mhal Need CmdqBuf
   %d", only logged); `MHAL_LDC_GetDevConfig(h, 0, …)` → IRQ number into +0x18. Either failure → destroy mhal dev.
5. `CamOsIrqRequest(irq, _MI_LDC_IMPL_IsrProc, "_LdcIsr", rec)`, wait-queue init, +0x14 = 1, device semaphore,
   `CamOsMemCacheCreate("mi_ldc_taskinfo", 4, 1)`; success → created = 1, appended to the module list,
   `"success to register dev%d"`, 0. Any failure undoes the earlier steps in reverse.

`DestroyDevice(dev)` (0x18f08): destroys every channel of the device (§5.2), frees the IRQ, mem cache,
`MHAL_LDC_DestroyDevice`, `mi_sys_UnRegisterDev` ("mi UnRegister Sys dev %d failed" continues), removes and frees the
record, "Dev%d has been destroyed".

**ISR** (`_MI_LDC_IMPL_IsrProc(irq, rec)`): `irq ≠ rec+0x18` → BUG. `MHAL_LDC_IsrProc(h)`; non-zero → "Mhal Isr Trigger
Failed" and return. Else ISR count + 1, count-since-wakeup + 1, last-ISR time = now. If +0x20 = 0 (register mode):
frame-done flag = 1 and `wake_up` the waiter (§6.6). If count-since-wakeup ≥ +0x24: reset it, wake-ups + 1 and
`mi_sys_NotifyWakeupISREvent(rec+0x0c)` (wakes the mi_sys pass thread so it polls `CheckOutputTaskStatus`).

### 2.4 ioctls of `/dev/mi_ldc` [table `.data+0x24`, handlers 0x10030–0x101b4]
Wrapper, size check and return value as every MI module (`mi_sys-2-interfaces.md` §1.3). Every handler takes the module
mutex (`.data+0`) and leaves the unlock to mi_common; the payload is unpacked to `(dev, chn, struct *)`.

| nr | cmd (libmi_ldc) | payload | IMPL |
|---|---|---|---|
| 0x0 | `0x40046900` | u32 dev | `CreateDevice(dev)` — **the `MI_LDC_DevAttr_t *` argument of the API is not sent** |
| 0x1 | `0x40046901` | u32 dev | `DestroyDevice(dev)` |
| 0x2 | `0x405C6902` | dev, chn, `ChnAttr` (0x54) | `CreateChannelV3` = create (§5.1) + set attr (§4.1) |
| 0x3 | `0x40086903` | dev, chn | `DestroyChannel` |
| 0x4 | `0xC0106904` | dev, chn, `OutputPortAttr` (8) | `GetOutputPortAttr` |
| 0x5 | `0x40106905` | dev, chn, `OutputPortAttr` (8) | `SetOutputPortAttr` |
| 0x6 | `0x40086906` | dev, chn | `StartChannel` |
| 0x7 | `0x40086907` | dev, chn | `StopChannel` |
| 0x8 | `0x40506908` | dev, chn, `ChnParam` (0x48) | `SetChnParam` |
| 0x9 | `0x40A86909` | dev, chn, `LutTaskAttr` (0xA0) | `DoLutDirectTask` |
| 0xA | `0xC05C690A` | dev, chn, `ChnAttr` (0x54) | `GetChnAttr` |

### 2.5 Registration with mi_sys [`ldc_create_device`; record layout `mi_sys-1-engine.md` §2.2, §2.3]
Info: module **0x17**, dev 0, **1 input port, 1 output port, 16 channels, 1 pass**, worker priority **0x46** (the decompile
shows the constant; it equals the `thread_priority` default [I: the parameter is read here]). Pass callbacks: +0x00
`OnPreProcessInputTask`, +0x08 `EnqueueInputTask`, +0x0c `BarrierInputTask`, +0x1c `CheckInputTaskStatus`, +0x20
`DequeueInputTask`, +0x24 `CheckOutputTaskStatus`, +0x28 `DequeueOutputTask`, +0x30 `NeedIndividualWorkThread` (always 1:
LDC runs on its own pass thread), +0x34 **`NeedCmdq`** (slot "RejectCmdq" of the pass table: returns 1 = no CMDQ when
`CMDQBUFFSIZE` is 0, else writes the size and returns 0 = take a CMDQ of that size — LDC owns CMDQ classes 0–3), +0x40 /
+0x44 bind capability (both write 1 at record +0xc), +0x48 `OnBindInputPort`, +0x50 `OnUnBindInputPort`, +0x58
`OnBindOutputPort`, +0x60 `OnUnBindOutputPort`; +0x90 = 0x32 (never read by mi_sys). Proc ops: the five
`_MI_LDC_IMPL_ProcOn*`. Commands registered for `/proc/mi_modules/mi_ldc/mi_ldc0` (§7) with their maximum argument counts:
`writeoutfile` 3, `dumptablebin` 3, `setout` 4, `loopbuf` 2, `debug` 2, `eisswitch` 1, `setbin` 2, `ignore` 4.

## 3. libmi_ldc.so (11 exports) [R `libmi_ldc.c`]

Common mechanics as every libmi_* (`mi-user-abi.md` §2, §3: lazy `/dev/mi_ldc` fd per process, 16-byte wrapper, MI code =
ioctl result, `-1` + "failed to ioctl" on syscall failure). Per function:

| function | ioctl | what the library does besides packing |
|---|---|---|
| `MI_LDC_CreateDevice(dev, MI_LDC_DevAttr_t *)` | 0x0, sends only `dev` | on result 0 increments a library-private device counter (u16) |
| `MI_LDC_DestroyDevice(dev)` | 0x1 | decrements the counter on 0; **when the counter is 0 (also after a failed ioctl) it closes the fd** under the fd semaphore, removes it from `g_mi_fd_table` if it is the top entry and resets the cached fd to −1 — the only libmi_* that closes its device; so the next MI_LDC call reopens and `mi_ldc_process_exit` → `DeInit` runs at that close (§2.2) |
| `MI_LDC_CreateChannel(dev, chn, ChnAttr *)` | 0x2, copies 0x54 bytes | — |
| `MI_LDC_DestroyChannel(dev, chn)` | 0x3 | **any non-zero result (ioctl −1 or an MI error) becomes `0xA017201F`**; debug level ≥ 5 prints entry / exit |
| `MI_LDC_Get/SetOutputPortAttr(dev, chn, attr *)` | 0x4 / 0x5, 8-byte attr | Get copies back on 0 |
| `MI_LDC_Start/StopChannel(dev, chn)` | 0x6 / 0x7 | — |
| `MI_LDC_SetChnParam(dev, chn, ChnParam *)` | 0x8, 0x48 bytes | — |
| `MI_LDC_GetChnParam(dev, chn, ChnParam *)` | **0xA (GetChnAttr)** | converts the returned 0x54 attr to the 0x48 param: param+0x00 (u16) ← attr+0x04 (the two flag bytes), param+0x04..+0x27 ← attr+0x08..+0x2b (matrix), param+0x28..+0x2f ← attr+0x30..+0x37 (focal lengths); the rest of param untouched |
| `MI_LDC_DoLutDirectTask(dev, chn, LutTaskAttr *)` | 0x9, 0xA0 bytes | — |

There is no library function for ioctl 0xA under the name GetChnAttr; `MI_LDC_GetChnParam` is its only user. A statically
linked copy of the same functions is in prog_uvc at 0x830f8… (`prog_uvc.md` §1.3).

## 4. Channel attribute and configuration formats

### 4.1 `MI_LDC_ChnAttr_t` (0x54, ioctl 2 / 0xA) [R `_MI_LDC_IMPL_SetChnAttr` 0x1bf7c, `GetChnAttr` 0x11408, prog_uvc]
Field names are the ones the module's own log strings use ("eMode … bBypass … bUseProjection3x3Matrix … u16UserSliceNum …
u32FocalLengthX/Y … u32ConfigSize … ConfigAddr … stCalibInfo … stDispMapInfo_X", "MI_LDC_WORKMODE_LUT") [R]; the grouping
is [I].

| off | type | field | used |
|---|---|---|---|
| 0x00 | u32 | `eMode`: **1 = LDC** (prog_uvc), **2 = LUT** (nothing else read at create; tasks would come through `DoLutDirectTask`, §6.7), **4 = DIS** (gyro / matrix stabilisation: "dis-gyro" in the proc view, §6.5); others → handled like 1 [R: only 2 and 4 are tested] | all |
| 0x04 | u8 | `bBypass` | mode 4 only |
| 0x05 | u8 | `bUseProjection3x3Matrix` | mode 4: if set, the matrix is copied |
| 0x08 | s32[9] | `as32Projection3x3Matrix` | mode 4 |
| 0x2C | u16 | `u16UserSliceNum`, must be 1..100 (else `0xA0172003`, "Invalid SliceNum") | mode 4 |
| 0x30 / 0x34 | u32 | `u32FocalLengthX / Y` | mode 4 |
| 0x38 | void * | `pConfigAddr` — the configuration (§4.3; user pointer) | modes ≠ 2 |
| 0x3C | u32 | `u32ConfigSize` | modes ≠ 2 |
| 0x40 | u32 | `eInfoType`: **0 = displacement map**, **1 = sensor calibration** (prog_uvc: 1); other → `0xA0172003` "no support such map mode" | modes ≠ 2 |
| 0x44 | void * | type 1: `pCalibPolyBinAddr`; type 0: `pXmapAddr` | |
| 0x48 | u32 / void * | type 1: `u32CalibPolyBinSize`; type 0: `pYmapAddr` | |
| 0x4C / 0x50 | u32 | type 0 only: X / Y map sizes | |

What the kernel does with it (create path; `SetChnParam` runs the same function with the 0x48 param, §4.2):
1. a temporary internal attribute (0x60 bytes; the channel keeps the current one at chn+0xF8) is copied from the current
   one; `eMode` taken from the caller. The channel's EIS flag (chn+0xED4, §6.5) is reset to 0.
2. mode 2 → stored as is, done.
3. **config buffer**: `MI_LDC_IMPL_MemAlloc(chn, "config_buf", size + 1)` (mi_sys private heap of the channel, §5.6), copy
   from the caller's pointer (`CamOsCopyFromUpperLayer` for user memory, `memcpy` for kernel callers), **a NUL is
   appended** — a JSON or text configuration is used as a C string. Size 0 or NULL → no config.
4. **map data**, only when both its pointer and its size are non-zero (otherwise the step is skipped without an error):
   type 1 → `MemAlloc("sensorcalib", size)` + copy. Type 0 → `MemAlloc("dispmap_x", xsize)` + copy, then
   `MemAlloc("dispmap_y", ysize)` **into the same buffer record** (internal +0x50) — the X map record is overwritten and
   leaks; the EPTZ front end rejects displacement maps anyway ("no support dispmap", §5.3).
5. **`_MI_LDC_IMPL_TryInitAlgo`** (§5.3): builds the per-view tables from config + calibration; on success the temporary
   attribute replaces the current one (`_MI_LDC_IMPL_InvalidateAllViewRes` (0x00017f10) frees the old buffers), on
   failure the new buffers are freed and the error is returned (CreateChannelV3 then destroys the channel). A NULL
   configuration fails here ("Invalid Config") [I: the path a missing JSON takes in prog_uvc].

`GetChnAttr` returns eMode, the two flags, matrix, slice number, focal lengths and eInfoType (+0x40); **pointers and sizes
come back 0**.

### 4.2 `MI_LDC_ChnParam_t` (0x48, ioctl 8) [R same function, param branch]
+0x00 u8 bBypass, +0x01 u8 bUseProjection3x3Matrix, +0x04 s32[9] matrix, +0x28 u32 FocalLengthX, +0x2C u32 FocalLengthY,
+0x30 pConfigAddr, +0x34 u32ConfigSize, +0x38..+0x47 the map-data block of §4.1 (+0x44..+0x50 there). The flags / matrix /
focal lengths are applied only when the channel is in mode 4; `eInfoType` and the slice number are not part of it (kept).
Then steps 3–5 of §4.1. Effect [I]: SetChnParam is the "reload the configuration / calibration at run time" call.

### 4.3 Configuration formats accepted in `pConfigAddr` (modes ≠ 2) [R `_MI_LDC_EPTZ_ParseCfg` 0x21714, `MI_LDC_EPTZ_Init` 0x24a70, `MI_LDC_DBIN_Init` 0x25a68]
Tried in this order; the first that parses wins:

| kind (internal type) | test | parser |
|---|---|---|
| 4 multi-view JSON | valid JSON **and** top-level `update_screen` ∈ {0, 1} and `views_config` array of ≥ 1 | `views_config[i]` each parsed as one view (§4.4), `update_view` per view ∈ {0, 1}, `screen_width` / `screen_height` (ints) stored for view 0, `multiview_check_param`; error "multi view mode no support 2P/4R" for the panorama / 4-region modes |
| 1 single JSON | valid JSON whose root is an array (or object) of view objects | `MI_LDC_ParseJsonChnParam`: each item → `_MI_LDC_LoadEptzConfig`; **the buffer holds one view (0x16F8), so only a 1-element array is safe** [R allocation size / I consequence]; out_x / out_y of view 0 forced 0 |
| 2 text configuration in the buffer | `mi_eptz_config_set` on the text, size < 0x800 | option lines, §4.5 |
| 3 path of a text configuration file | `MI_LDC_IMPL_GetFileSize(text)` > 0 | `mi_eptz_config_parse(path)` reads the file with CamFs from the kernel, then as type 2 |
| DBIN container of ready tables | anything for which the four EPTZ forms or the EPTZ environment set-up fail | §5.3 |

The calibration buffer (va, size) is attached to every view (view +0x1650 → `{cfg va, cfg size, calib va, calib size}`).
A DBIN container is never valid JSON, is larger than the 0x800 limit of the in-buffer text form and is not an existing file
name, so `_MI_LDC_EPTZ_ParseCfg` fails and the DBIN loader takes it — on **any** chip id [I from the control flow].

### 4.4 One view object in JSON [R `_MI_LDC_LoadEptzConfig` 0x2e298; keys read with cJSON 1.7.12]
Numbers are read as `valueint` (ints) or `valuedouble` → float; a missing key logs "Can not find key" and gives 0, a
non-number gives 0. **`ptconfig_para`, `rsc_region` and `m33` are mandatory** (missing → parse fails); `zoom_lens_param`
is mandatory when `calib_type` = 3. Offsets are in the 0x16F8-byte view parameter structure (`mi_eptz_para`, "para").
```text
{
  "ptconfig_para": {                       // int unless noted
    "ldc_mode":+0x1428  "in_width":+0x142c  "in_height":+0x1430  "out_width":+0x1434  "out_height":+0x1438
    "in_xc":+0x1444  "in_yc":+0x1448  "in_fisheye_radius":+0x144c  "grid_size":+0x1450  "grid_size_h":+0x1454
    "pan_min":+0x1458 "pan_max":+0x145c "tilt_min":+0x1460 "tilt_max":+0x1464 "zoom_min":+0x1468 "zoom_max":+0x146c
    "rotate_min":+0x1470 "rotate_max":+0x1474 "radius_min":+0x1478 "radius_max":+0x147c "hw_mode":+0x1480
    "in_xc_offset":+0x1484 "in_yc_offset":+0x1488 "ldc_eis_split":+0x1490 "table_width":+0x1494
    "xBaseBits":+0x1498 "yBaseBits":+0x149c "fov_out_1p":+0x15f0 "map_width":+0x15f4 "map_height":+0x15f8
    "output_offset_x":+0x163c "output_offset_y":+0x1640 "x_resize_ratio":+0x1644 "y_resize_ratio":+0x1648
    "calib_type":+0x164c "crop_en":+0x1654 "table_mode":+0x1658 "out_x":+0x1664 "out_y":+0x1668 "pixel_size":+0x166c
  },
  "view_index":+0x1670 "pan":+0x1674 "tilt":+0x1678 "rotate":+0x167c
  "zoom":+0x1680 (float) "zoom_h":+0x1684 (float) "zoom_v":+0x1688 (float) "fc":+0x1690 (float)
  "out_rot":+0x1698 "r_inside":+0x169c "r_outside":+0x16a0 "theta_start":+0x16a4 "theta_end":+0x16a8
  "deb_state":+0x16ac "distort_intensity":+0x16b4,
  "zoom_lens_param": { "fov_h":+0x16ec (float), "fov_v":+0x16f0 (float) },   // only read when calib_type == 3
  "rsc_region": { "x":+0x16b8, "y":+0x16bc, "w":+0x16c0, "h":+0x16c4 },
  "m33": [ up to 9 numbers ] → +0x16c8..+0x16e8
}
```
`m33` is read as floats and then converted in place [R, constants read from the literal pool]: elements 0, 1, 3, 4 ×16384
(Q14) → int; 2 and 5 ×8 → int; 7 ×2^25 → int; **6 ×2^25, rounded, but stored back as float** (a vendor slip [I]); 8 stays
float. Rounding is half away from zero. View-structure fields not from JSON: +0x143c / +0x1440 output tiles
(`out_width_tile`, `out_height_tile` = ceil(out / grid) + 1, computed in `eptz_get_buffer_info`), +0x148c `frame_cnt`,
+0x15fc / +0x1600 map tiles, +0x1604.. square / stereo / blending fields (text configuration only), +0x1650 calibration
pointer. A JSON view has no default for `table_width`: without the key it reads 0, every block then fails the split's
`table_width` test and table generation ends with 0x21E — a JSON configuration must set `table_width` [R control flow / I
consequence; the split is in `ldc-2-eptz-lib.md` §3].

`ldc_mode` values (name table at `.rodata+0x6d6c`, indexed by the mode [R]; model function per mode from
`eptz_lib_parameter_set` / `eptz_lib_map_gen` [R]; the models themselves: `ldc-2-eptz-lib.md` §4):

| ldc_mode | name | model | through mi_ldc |
|---|---|---|---|
| 0 / 1 / 2 / 7 | 4R_CM / 4R_WM / 1R / 1R_WM | `eptz_*` fisheye dewarp, out layout `out_set_CM_4R` / `WM_4R` / `1R` / `WM_1R` | yes |
| 3 / 4 / 8 / 10 | 2P_CM / 1P_CM / 2P_DM / 1P_DM | `donut_*` (fisheye ring → panorama) | yes |
| 5 | 1P_WM | `erp_*` (equirectangular) | yes |
| 6 | 1O | `bypass_*`: rotation about the output centre and a plain resize, **no lens model** | yes |
| 9 | RSC | `RSC_*` (rolling-shutter / matrix region warp, uses `rsc_region` + `m33`) | yes |
| 11 | cylinder | `cylinder_*` | yes |
| 12 | stitch | (stitch mask helpers); fails in the library too | **refused** |
| 13 | map2bin | `map2bin_*` (converts a given X/Y map) | **refused** |
| 14 | st_stitch | `st_stitch_*` (two-sensor stitching, `stitch_info.bin`) | **refused** |
| 15 | — | `perspective_*` (3×3 projective, `m33`) | **refused** |
| any other | — | `eptz_*` default (out init, then returns 0x204 "unsupported") | — |

"Refused": before generating, `_MI_LDC_EPTZ_LibarayGeneraBin` accepts only ldc_mode 0–11 and returns −1 for 12–15 [R
switch default] (§5.3).

### 4.5 Text configuration (types 2 / 3) [R `mi_eptz_config_set` 0x26ab0, strings]
Whitespace-separated options, each `-<letters>` followed by integers (`sscanf`-style with `%n`); defaults: `x_resize_ratio`
= `y_resize_ratio` = 100, `table_width` = 256. Options → fields: `-m` ldc_mode; `-s` in_width in_height out_width
out_height; `-c` in_xc in_yc in_fisheye_radius (sets the offsets to −1); `-co` in_xc_offset in_yc_offset … (sets xc / yc
to −1 = "centre of the calibration image"); `-cp` crop_en; `-d` grid_size grid_size_h (1–64, power of 2); `-e`
ldc_eis_split (0 ldc split, 1 eis split); `-f` frame_cnt; `-fv` fov_out_1p (0–360); `-h` hw_mode (0–4); `-ir` x / y resize
ratio; `-j` xBaseBits yBaseBits; `-k` calib_type; `-of` output offset x y; `-p` pan min max; `-t..` tilt min max; `-z` zoom
min max; `-r..` rotate min max; `-a..` radius min max; `-sm` map width height; `-sl` ln_percentage (0–100); `-st` the six
stereo fields; `-si` the four "square" fields; `-y` table width (multiple of 32); letters `b p q x y` with a path argument
(≤ 0x3FF chars) set file paths (+0x1404, +0x140c…) used by the file-based debug paths (`%s/mapx.bin`, `%s/Cali_LDCpoly.bin`,
`%s/trdu0to180.bin`, `%s/trdu180to220.bin`, `%s/CalibPoly_new.bin`, `%s/matrix_3x3.txt`, stereo `%s/stereo_%d/…`). Input
width must be a multiple of 32 and height even (`eptz_align_in`, else 0x22E and "[ERR] The input width should be multiples of
%d…"; `ldc-2-eptz-lib.md` §8.2). The per-option range checks and
the `-s` sub-variants: `ldc-2-eptz-lib.md` §10.2.

### 4.6 Calibration data (`eInfoType` 1, `calib_type` in the JSON) [R `world2cam_init` 0x35074, `world2cam_poly_new_init` 0x34b8c, `world2cam_init_trdu_new(_set)` 0x3428c / 0x344e8]
<!-- ev:ldc-1-stack.4.6 -->
`calib_type` selects the lens model: **1 = "poly new" (`CalibPoly_new.bin`)**, 2 = ideal (centre + radius from the JSON,
no file), 3 = zoom lens (`world2cam_poly_zoom_lens_init`, a list of per-zoom-step records, attached buffer only), anything
else = old poly (`world2cam_poly_init`, `Cali_LDCpoly.bin` + `trdu0to180.bin` + `trdu180to220.bin` from a folder path).
With an attached calibration buffer the data is parsed from memory; without it, from the file `<path>/CalibPoly_new.bin`
via CamFs (debug path). `CalibPoly_new.bin` layout (little-endian, all 4-byte unless noted):
```text
+0x00 s32 calib_width      +0x04 s32 calib_height      (default centre = in_xc_offset + w/2, in_yc_offset + h/2
                                                          when in_xc & in_yc are both -1)
+0x08 f32 scale            +0x0C f32 c3   +0x10 f32 c4   +0x14 f32 c5   +0x18 f32 c6
+0x1C curve 0 (incidence angle 0..180°), then curve 1 (180..220°), each:
      s32 n (segments; the code handles up to 4)
      s32 knots[n]            (samples per unit of the input variable, per segment)
      f32 bounds[n+1]         (segment limits; curve 0 ascending, curve 1 descending)
      then for each segment i: s32 count_i, u16 lut_i[count_i]   (radius LUT, Q15 normalised radius 0..32767)
```
Roles of the header words [R]: c3 is the radius scale (with JSON `pixel_size` = 0 the sensor radius M the LUT output is
scaled by equals R; with `pixel_size` it is c3·1000/pf); c4 and c6 give the focal factor k (c6 > 0: k =
⌊|c4 / tan(c6/2 in degrees)|·1000⌋ / 1000, else 1.0); c5 < 0 rescales the radius, R := −R/c5·c3; `scale` divides the
normalised value. At load slope_i = knots_i / (bounds_{i+1} − bounds_i) is precomputed. `world2cam_table` maps a 3-D ray
(x, y, z in Q15) to the sensor: φ = atan2(y, x); for the ideal model r is proportional to θ = atan2(√(x²+y²), z); for the
poly model the normalised value s = (1/scale)·k·fc·√(x²+y²) (clamped ≤ 1) picks the segment whose bounds contain it; on
curve 0 (`world2cam_interpolation_LUT_0to180`) the LUT is read at (s − bound)·slope with linear interpolation between
neighbours, on curve 1 (rays beyond 90° from the
axis, fisheye lenses wider than 180°; read inline — the separate `world2cam_interpolation_LUT_180to220` has no caller)
**without interpolation**; optional `distort_intensity` term r + d·r⁵; then
u = xc + R_px·cos φ, v = yc + R_px·sin φ (Q15). The derived constants, the zoom-lens record (calib_type 3) and the exact
arithmetic: `ldc-2-eptz-lib.md` §9.3, §9.5, §9.4.

**Chip check** [R]: every library entry (`mi_eptz_runtime_init`, MVE) first runs `Platform_Check()`: `CamOsChipId()` must be
**0xF4**, else "Unexpected IC version" and the library refuses (runtime init returns NULL → the EPTZ set-up fails → the
DBIN loader is tried on the same buffer, §5.3). The kernel `CamOsChipId` calls the platform chip-id callback
(0xc01f7968), i.e. 001E.00 bits 7:0, the same value user space reads from
the sysfs `CHIP_ID` (`/sys/devices/virtual/mstar/msys/CHIP_ID`) (`kernel-platform.md` §3.2). Whether this camera's SoC gives 0xF4 is not known
(not read; the IPL's load path does not decide it, `ipl.md` §4.5); with any other id the EPTZ library and MVE do not run and
`-L` can only work with ready tables (DBIN container, §5.3; proc `setbin` does not work, §7.2).

## 5. Channel life cycle and the tables [R unless marked; names in brackets are the module's own `__func__` strings]

### 5.1 Channel record and create (`_MI_LDC_IMPL_CreateChannelWrap` 0x15b68)
Channel id ≤ 16 is accepted (`chn > 0x10` → "Chn %d, overrange >= %d"; ids 0..16 pass the check although mi_sys registered
16 channels — chn 16 is an off-by-one [R check / I consequence]). Existing channel → warning, 0. Then
`CamOsMemAlloc(0xFF480)` — **≈1 MiB of kernel memory per channel**, zeroed — and `MHAL_LDC_CreateInstance(mhal dev,
&chn+0xD4)` (failure → free, `0xA017201F`); "alloc chninfo failed" → `0xA017200C`. Record fields used across the module:

| off | field |
|---|---|
| +0x00 | u8 created (= 1) |
| +0x04 | channel id |
| +0x08 | magic **0x56588888** |
| +0x0C | status (0 created, 1 started, 2 stopping) |
| +0x10 / +0x12 | input w / h of the last frame |
| +0x14 | input stride |
| +0x54 / +0x56 | output port width / height (set from the table, §5.4) |
| +0x58 | output pixel format |
| +0x90 | rw-semaphore (channel lock) |
| +0xB8 | atomic "tasks in flight" |
| +0xD4 | mhal instance |
| +0xD8 | semaphore (view-resource lock) |
| +0xF8 | the current internal attribute (0x60, §4.1) |
| +0x158, +0x7B0 | view-resource slots A and B (0x658 bytes each, §5.4) |
| +0xE34 | GENBIN object (§5.3) |
| +0xE50 | `writeoutfile` frame count (u16), +0xE54 its path (§7.2) |
| +0xED4 | u8 EIS flag (hard EIS, §6.5; tested in mode 4 before new tables are installed, §5.3) |
| +0xED8 | `loopbuf` frame count (§7.2) |
| +0xEE0 | per-channel buffer list |
| +0xF1C | atomic: proc handlers working on the channel (BUG_ON if negative) |
| +0xF20 / +0xF24 / +0xF28 | lock-debug counter; view-resource re-entrancy counter; trace word (proc `debug`) |
| +0xF40 / +0xF70 | time-trace records |
| +0xFB8.. | per-view per-block timing (0x28A0 per view) |
| +0xFF470 | device pointer |
| +0xFF474 | list node |

### 5.2 Start / stop / destroy
- **Start** (`_MI_LDC_IMPL_StartChannelWrap` 0x12f6c): status ≠ 1 → status 1; then (user path) `mi_sys_EnableInputPort`,
  `EnableOutputPort`, `EnableChannel`, input ext-conf **{strideAlign 0x20, lumaLineAlign 2, chromaLineAlign 1, flag 0}**,
  output ext-conf **{0x10, 2, 1, flag 1}** and `SetOutputPortBufClearConf(1)` (padding of LDC output buffers is cleared;
  `mi_sys-1-engine.md` §4.2). Already started → warning "channel %d, have start", 0. Unknown dev → `0xA0172301`, chn →
  `0xA0172003`.
- **Stop** (`_MI_LDC_IMPL_StopChannelWrap` 0x134cc): not started → warning, 0; status 2, then waits until the in-flight
  atomic is 0 (up to 50 × 5 ms, dropping the channel lock between tries; afterwards "try 50, mhal not return buffer …"),
  then the mi_sys Disable calls, status 0.
- **Destroy** (`ldc_destroy_channel` (0x1891c)): stop if needed, waits up to 3 s for mhal to give buffers back ("wait 3s, atom=%d"),
  `MHAL_LDC_DestroyInstance` ("Ldc Destroy instance Fail" logged, continues), frees the GENBIN library (§5.3), both view
  resources, the attribute buffers (`config_buf`, `sensorcalib` / `dispmap`), unlinks and frees the record.

### 5.3 Building the tables: `_MI_LDC_IMPL_TryInitAlgo` (0x1a380), the GENBIN front end, EPTZ and DBIN [`MI_LDC_GENBIN_LibInit` 0x208f4, `MI_LDC_GENBIN_UpdateCfg` 0x20d40, `MI_LDC_EPTZ_Init` 0x24a70, `MI_LDC_DBIN_Init` 0x25a68, `MI_LDC_DBIN_UpdateCfg` 0x2609c, `_MI_LDC_DBIN_GenBins` 0x2547c, `_MI_LDC_EPTZ_LibarayGeneraBin` / `_LibBinCheck` 0x22cd8]
Called at create and at every `SetChnParam` (mode 2 is a BUG_ON there). Allocates a 0xE20-byte "view config" list, then
`MI_LDC_GENBIN_UpdateCfg(chn, attr, list, &bins, &n, &updated)`:
- **Library object** (`mi_ldc_genbin.c`): the first configuration of a channel runs `MI_LDC_EPTZ_Init` (type 0) =
  `_MI_LDC_EPTZ_ParseCfg` (§4.3) **and** `_MI_LDC_EPTZ_InitEnv`; **if either fails, `MI_LDC_DBIN_Init` (type 2) is tried on
  the same configuration buffer**. Type 1 ("cv lib") is not built in ("no support cv lib"). The chosen type is kept in the
  channel's GENBIN object (chn+0xE34); each type has an ops table {GenBins, FreeBins, DeInit, Init, UpdateCfg} (tables at
  0x243F8 / 0x2487C / 0x24D6C, stride 0x14). A later UpdateCfg failure deinitialises the object so the next call starts
  over.
- **EPTZ** (`mi_ldc_eptz.c`): `_MI_LDC_EPTZ_ParseCfg` → per-view parameter structures (0x16F8 each, ≤ 16 views:
  `u8ViewId >= MI_LDC_MAX_VIEW_NUM` check); `_MI_LDC_EPTZ_InitEnv`: `mi_eptz_get_buffer_info` (size of the library work
  buffer: ≈ 7.2 MB + tiles·8 (+ tiles·16 with crop, + zoom-lens tables), `MemAlloc("ldc-lib-workbuf")` from the channel's
  private mi_sys heap), `mi_eptz_runtime_init` (chip check §4.6, `eptz_init`: clamps pan / tilt / rotate / zoom to their
  min / max, out_rot ≤ 360, fov_out default 180, lens model init, output layout, `ldc_split_init`; `ldc-2-eptz-lib.md` §1.5).
  The displacement-map form (`eInfoType` 0) is rejected ("no support dispmap"). "IniParam is Diff" / "poly bin has been
  update" / "No Need to Construct": UpdateCfg compares the new parameters with the old (`memcmp` of the calibration blobs)
  and rebuilds only what changed. With `deb_state` bits set the library also writes debug files
  (`%s/out_i%d_t%d_r%d_z%4.2f_rot%d.bin`).
- **EPTZ table per view** (`_MI_LDC_EPTZ_LibarayGeneraBin`): the ldc_mode is filtered first — only 0–11 are accepted,
  12–15 return −1 (§4.4); then `mi_eptz_runtime_bin_size` → `MemAlloc("ldc-loadbin", size)` → `mi_eptz_runtime_map_gen` /
  `mi_eptz_runtime_bin_gen` → **`_MI_LDC_EPTZ_LibBinCheck`**: header s16 mode must equal the configured ldc_mode, view
  index (table +0x20) < 100, input w / h (table +0x38 / +0x3A) must equal para in_width / in_height; a failure frees the
  table and returns −1.
- **Default configuration** (view-config type 2, used by DIS, §6.5): views get preset parameters before generation: tilt
  preset 60; modes 0 / 1 / 2 / 7: pan 0, rotate 0, out_rot 0, zoom 150, tilt −50 (mode 0) / 50 (1, 7) / 0 (2); donut modes:
  r_inside 10, r_outside 550, theta 0…360; ERP / cylinder: zoom_h = zoom_v = 100 (fc 1 for ERP); RSC (9): slice s of S =
  ⌈out_height/S⌉ rounded up to even rows → rsc_region {0, s·h, out_width, h}, out_y = s·h, zoom 135 (0x43070000), and the
  view's m33 copied into the per-slice matrix list.
- **DBIN** (`mi_ldc_dbin.c`) — the configuration buffer is a container of ready tables, a sequence of records:
  ```text
  +0x0 u32 magic = 0x66666666   +0x4 u32 type (0 = view table)   +0x8 u32 size (payload bytes)
  +0xC u16 count (number of records / views, ≤ 100; checked in every record)   +0xE u16 (printed as "index", not used)
  +0x10 payload: one view table (0x60 + n·0x16C0 bytes, §6.1)
  ```
  Validity: first word 0x66666666, every record's magic and count ≤ 100, and **Σ(0x10 + size) over `count` records equals
  the config size or the config size − 1** (the NUL mi_ldc appends to the config copy, §4.1); else 0xA017201F. The log
  prints "header : Magic … type … size … count … index". For view v = 0 … count − 1 the first record with type 0, size ≠ 0
  and **payload view index (table +0x20) = v** is copied into its own `ldc-loadbin` buffer (missing → 0xA017201F, no memory
  → 0xA017200C). No other field of the table is checked on this path [R: no compare after the copy]. The DBIN path needs no
  calibration data: a missing `./CalibPoly_new.bin` in prog_uvc only leaves the map-data step of §4.1 skipped.
- After UpdateCfg the new tables are installed as the channel's next view resource (`_MI_LDC_IMPL_UpdateViewRes`, §5.4:
  retried every 2 ms, up to 100 times, while it returns BUSY `0xA0172012`), then the old bins are released. In mode 4
  (DIS) the EIS flag (chn+0xED4) is checked first ("DIS matrix ready" [I]).

### 5.4 View resources (double buffer) and PTS selection [`_MI_LDC_IMPL_UpdateViewRes` 0x19698 (disassembly ), `_GetViewResByPts` 0x10fd0, `_GetNewViewRes` 0x10dcc, `_InvalidateAllViewRes` 0x17f10, `_GetAndSetViewResByConfig` 0x1e494]
<!-- ev:ldc-1-stack.5.4 -->
Two slots per channel, A = chn+0x158 and B = chn+0x7B0 (0x658 bytes each): +0x00 state (0 free, 1 "score_1", 2 "score_2"
[R strings "score"]), +0x04 …, +0x08 u64 PTS, +0x10 reference count, +0x14 number of tables, +0x18 + 16·view: {phys u64,
size, virt} per view (≤ 100 views; the array ends at +0x657, the end of the slot) [R disassembly of `UpdateViewRes`]. A
blending configuration at slot +0x44 is named by the reading of `_MI_LDC_IMPL_Process` / `GetAndSetViewResByConfig`
; that offset lies inside the entry array — where the slot keeps the
blending configuration is not settled. The channel's view-resource semaphore (chn+0xD8) and the
re-entrancy counter chn+0xF24 (BUG_ON when already set) guard all of them. Two helpers: **release(slot)** = code at 0x17d68
(Ghidra made no function for it): frees every table entry of the slot (unmap + `mi_sys_PrivateDevChnHeapFree`) and
`memset(slot, 0, 0x658)` (state 0, count 0, refcount 0); **free_buf** = `ldc_free_view_buf` (0x17800), frees one buffer.

**`UpdateViewRes(chn, bins[], n, flag)`** — callers: `MI_LDC_IMPL_SetBatchViewConfig` with flag 0; `_MI_LDC_IMPL_TryInitAlgo`
with flag = (a local byte == 0); the per-frame DIS path of `_MI_LDC_IMPL_Process` with flag 1, repeated while the result is
BUSY [R call sites].
- **flag = 0**: target = a slot in state 0 — **B when both are free**, otherwise the free one; old = the other slot when it
  is in state 2. No slot in state 0 → BUSY **0xA0172012**; target's reference count ≠ 0 → release(target), BUSY; old exists
  with reference count 0 → log "exist new view res, invalid first", release(old), release(target), BUSY (a repeated call
  then finds both slots free); otherwise fill the target (below), its state becomes 2. A slot in state 1 is never chosen
  and never released by this path.
- **flag ≠ 0**: cand = the slot in state 0 or 1, B taking precedence over A; old = the slot in state 2, B taking
  precedence (a state other than 0 / 1 / 2 counts as neither):

  | cand | old | action |
  |---|---|---|
  | none | exists | fill **old in place** (state stays 2) |
  | none | none | BUSY |
  | state 0 | exists | fill **old in place** (the free slot stays unused) |
  | state 0 | none | fill cand, state := 2 |
  | state 1 | none, or reference count ≠ 0 | fill cand in place (state stays 1; its reference count is not checked) |
  | state 1 | reference count 0 | "exist new view res, invalid first", release(old), release(cand), BUSY |

  So with flag ≠ 0 the newest tables (state 2) are overwritten in place whenever possible — the DIS loop's per-frame
  regeneration rewrites one slot instead of alternating [I: consequence of the table].
- **Fill** (n entries of 16 bytes {phys u64, size, virt}; n = 0 → table count 0): per entry, virt = 0 or size = 0 →
  0xA0172003; phys = 0 → the table is copied: a kernel pointer whose range fails the `addr_limit` test → "Invalid addr
  0x%llx, size:%d", 0xA0172003; else `MI_LDC_IMPL_MemAlloc(chn, "ldc-loadbin", size)` (failure 0xA0172006) and
  `CamOsCopyFromUpperLayer` (user pointer, `CamOsMemIsUserSpace`) or `memcpy`; phys ≠ 0 → the buffer itself is taken over
  and the caller's entry zeroed (phys, virt = 0). Then cache flush, view id = byte +0x20 of the table (the view header's
  first byte, BUG_ON when ≥ 100), and the slot entry of that **view id** gets the buffer (an old buffer there is freed
  first, `ldc_free_view_entry` (0x17918)). After all entries the table count (+0x14) is recounted over **entry indices 0 … n−1** (non-zero
  phys), not over the view ids used — tables with view ids ≥ n are stored but not counted [R; I effect]. State := 2 when
  flag = 0 or the slot was in state 0. Any error during the fill frees the temporary buffer and **releases the whole
  target slot**, then returns the error. Success returns 0.

`ldc_proc_setbin(chn, buf, flag)` (0x1d548, only caller proc `setbin`, §7.2): `UpdateViewRes` specialised for a single buffer —
the same slot choice for both flag values and the same fill of one entry (copy when phys = 0, take over otherwise; view id
from byte +0x20), except that the table count is set to 1 when **entry 0** holds a table and to 0 otherwise, whatever view
id was filled.

- **`GetViewResByPts(chn, pts)`**: of the two slots, the one with the **latest PTS that is ≤ the frame's PTS** (equal PTS →
  the higher state); none → NULL, "No Suitable Res: CurPts" and a dump of both slots (ResIndex, score, ResPts, Ref).
- **`GetNewViewRes(chn)`**: slot B when its state ≠ 0, else slot A when its state ≠ 0, else none.
- **`GetAndSetViewResByConfig(chn, _, w | h << 16, pts)`** (from `OnPreProcessInputTask`): picks the slot whose view-0
  table input size equals the frame size (stitch tables, modes 12 / 14: height a multiple), stamps it with the frame PTS,
  reference + 1, sets the output port size (chn+0x54 / 0x56) from the table screen size (+0x48 / +0x4A) and pixel NV12
  when unset; for tables with a blending configuration it (re)creates the MVE context and the "MVE_Input1" image and builds
  the alpha map (`Init_Alpha_Map`: a built-in 36×36 Gaussian table `G_gaussian_array_36` resized with `MI_MVE_Resize` to the
  overlap size).
- **`InvalidateAllViewRes`**: frees both slots' tables and the attribute buffers (`stConfigBuf`, `stCalibInfo`,
  `stDispMapInfo_X/Y`) — channel destroy / reconfigure.

### 5.5 Output port and input rectangle
- `SetOutputPortAttr(dev, chn, {u16 w, u16 h, u32 ePixelFormat})`: only **NV12 (0xB)** is accepted ("Not Support Fmt" →
  `0xA0172003`), stored at chn+0x58; **width and height are ignored** — the output size is the table's. Mode 2: ignored, 0.
- `GetOutputPortAttr`: w / h = the current view-0 table's screen size (table +0x48 / +0x4A), pixel = chn+0x58; no table →
  "Ldc on mode, not load table, cant get output attr" `0xA017201F`. Mode 2: 0, nothing filled.
- `MI_LDC_IMPL_GetInputRect` (export, no ioctl): the view-0 table's input rectangle (table +0x34..+0x3A) — for kernel
  callers (no importer on the camera).
- `SetConfig` / `SetBatchViewConfig` / `*External` (exports only): the same create / start / attribute operations for
  kernel callers that pass already built view tables; no importer on the camera.

### 5.6 Memory
`MI_LDC_IMPL_MemAlloc(chn, name, rec, size, map)` → `mi_sys_PrivateDevChnHeapAlloc(dev, chn, name, size)` (the channel's
private heap, `mi_sys-2-interfaces.md` §7) + optional `mi_sys_Vmap` (cached) → record {u64 phys, u32 size, void *va}; it
returns 0 on success. `MemFree` reverses it; `FlushCache` = `mi_sys_VFlushInvCache`. Names seen: `config_buf`,
`dispmap_x`, `dispmap_y`, `sensorcalib`, `ldc-lib-workbuf`, `ldc-loadbin`, `ldc-bin-proc`, `ldc-loopbuf`, `MVE_Input1`.
Tables are written by the CPU and read by the engine, so they are flushed before use [I from the FlushCache calls].

## 6. Per-frame processing (mi_sys pass callbacks) [R unless marked]

### 6.1 The table ("bin") as the per-frame path consumes it [`MI_LDC_EPTZ_BinDebug` 0x24010, `_MI_LDC_IMPL_Process` 0x1a9d0, `_MI_LDC_IMPL_HalLdcProcess` 0x16c6c = `_MI_LDC_IMPL_HalLdcProcess`]
One table per view, little-endian, **0x60 + n·0x16C0 bytes** (n blocks, "runs", ≤ 650). Offsets counted from the table
start; the complete layout with the content of each part and how the library fills it is in `ldc-2-eptz-lib.md` §2.
```text
+0x00 s16 ldc_mode (§4.4; only the proc view and _MI_LDC_EPTZ_LibBinCheck read it)   +0x02..+0x1F not written
view header:
+0x20 u32 view index         +0x24 u32 NumRuns (n blocks)     +0x28 u32 Fmt (always 0)
+0x2C u32 hw_mode (eOpMode, proc "OpMode": input line-buffer mode 0-4 = the engine's block-size code)
+0x30 u32 flip code (eFlipMode, proc "flip"): 0 or 3 (bit 0 h-mirror, bit 1 v-flip; 3 = 180° turn)
+0x34 s16 input  rect x, y, w, h   (= 0, 0, in_width, in_height)
+0x3C s16 output rect x, y, w, h
+0x44 s16 screen rect x, y, w, h   (w, h at +0x48 / +0x4A = the channel's output size)
per block b, at B = b * 0x16C0:
B+0x60  s16 InBlk  x, y, w, h      B+0x68 s16 OutBlk x, y, w, h      B+0x70 s16 SclBlk x, y, w, h
B+0x78  u8 scale x, y (always 1, 1; not sent to hardware)
B+0x7A  u8 delta base bits x, y (xBaseBits / yBaseBits: the delta precision)   B+0x7C u8 bOutCropEn (always 1)
B+0x80   0xA0 bytes   engine configuration block = the block's node grid: u16 xs[42], 12 bytes 0xFF, u16 ys[32]
                      (fetched by the engine itself, address in 12F1.10 / .11)
B+0x120  0x1500 bytes delta (displacement) table ("DeltaBase … size 0x540" words)
B+0x1620 0x100 bytes  mask table (always zero; fetched only when mhal's mask-table enable is on)
```
The meaning of the 12 × 0xFF bytes of the configuration block (constant; mhal never reads the block) and the encoding of
the mask table (no vendor code on the camera writes one) cannot be settled statically. Whether the engine reads the table
as laid out here has not been checked on hardware (§1.3).

### 6.2 Callbacks
- **`OnPreProcessInputTask`** (`_MI_LDC_IMPL_OnPreProcessInputTask` 0x1eab4; runs under the device semaphore): the input
  must be a frame buffer (type 1) of a created, started (status 1) channel not marked to drop, else result 2 (reject);
  with in-flight tasks and no output yet → 2; then `GetAndSetViewResByConfig` (§5.4) for the frame size and PTS — no
  matching slot → 2 (frame dropped).
- **`EnqueueInputTask`**: input must be a FRAME buffer (type 1), channel started; copies size / stride / PTS into the
  channel; takes two `mi_ldc_taskinfo` objects (input / output private data = the CMDQ in use); **loop-buffer debug mode**
  (proc `loopbuf`): the first N input frames are copied (`mi_sys_MemcpyPa`) into a private `ldc-loopbuf` and replayed;
  then `_MI_LDC_IMPL_Process` (§6.3). Success → in-flight + 1. On a CMDQ kick failure: polls the CMDQ status (`+0x48`)
  30 × 1 ms, then resets it (`+0x38`: "kick fail reset cmdq engine"); result 3 = drop the task ("channel %d, drop task").
- **`BarrierInputTask`**: nothing (returns 0).
- **`CheckInputTaskStatus`**: done when the engine finished the task's blocks; **no ISR for more than 2 s → "LDC NO ISR …
  force drop input task"**; a stuck block fence leads to `MHAL_LDC_ReTrigger` ("LDC ReTrig [%llu] [%llu], diff time",
  §8.6).
- **`CheckOutputTaskStatus`** (`_MI_LDC_IMPL_CheckOutputTaskStatus` 0x171bc) / **`DequeueOutputTask`**
  (`_MI_LDC_IMPL_DequeueOutputTask` 0x1ffc0): both decompile badly (`halt_baddata`); the disassembly shows no calls besides
  debug-level checks and one indirect call [R call list]; they test the task's done / error counters and update the
  statistics the proc view prints (GetOputBufCnt, FinishOputBufCnt, FailOBufCnt, fps) and the in-flight atomic (− 1). The output file dump of proc
  `writeoutfile` is `ldc_dump_outfile` (0x1fea0), called from the dequeue path [R string / I call site] (§7.2).
- **`DequeueInputTask`**: frees the per-task data and loop-buffer pieces.
- Bind hooks only log; capability = 1 for both ports.

### 6.3 `_MI_LDC_IMPL_Process` (per input frame)
1. (eMode 4) the DIS step (§6.5).
2. `GetViewResByPts` (§5.4) picks the view resource whose PTS window matches the frame ("No Suitable Res: CurPts").
3. The channel output size must equal the view-0 table's screen size (+0x48), else BUSY 0xA0172012 with both sizes
   printed.
4. For every view (≤ 100) not masked by proc `ignore` (§7.3; both its masks act on whole views) and every block it builds the frame / block / WDMA attributes
   (§6.4) and calls `_MI_LDC_IMPL_HalLdcProcess` (0x00016c6c); a failing block stops the view; per-block times
   (accumulated per view and block) go to the proc statistics.

### 6.4 Per-block programming (`_MI_LDC_IMPL_HalLdcProcess` (0x00016c6c), `_MI_LDC_IMPL_HalWdmaSetting` 0x16a60)
- Both the input and the output buffer must have pixel format **0xB (NV12)**, else the block is not started (0xA0172003).
- **Frame attr** (0x40 bytes): input Y / UV physical addresses, stride, block-size code (view +0x2C). For eMode 1 and 4
  the input address is offset by `in_w·y + x` (Y) and `in_w·(y/2) + x` (UV) with x, y, in_w from the view header input
  rect (+0x34, +0x36, +0x38) — **the table's input width is used as the stride**, and the programmed input stride is that
  width too; other modes use offset 0. This is exact only because mi_ldc's input ports align strides to 32 (§5.2), the
  library requires in_width to be a multiple of 32 and always writes input x = y = 0 (only stitch writes y ≠ 0) [R; I
  "exact only because"].
- **Block attr** (0x38 bytes): the first 0x20 bytes of the block record (B+0x60..+0x7C) are copied; the three table
  addresses are table physical address + b·0x16C0 + 0x80 / 0x120 / 0x1620 — except in hard-EIS mode (§6.5).
- **WDMA attr** (0x38 bytes): output Y / UV addresses = output buffer + (SclBlk y·stride + SclBlk x) (with mirror / flip
  from the flip code, the last line / column of the block area), stride, size of the block.
- Then `MHAL_LDC_SetFrmAttr`, `SetBlkAttr`, `SetWDmaAttr`, `SetBlkFence(b)`, `Trigger` (§8.4–§8.6). In direct mode it
  waits for the done interrupt up to 50 jiffies = 0.5 s ("wait completion timeout").

### 6.5 DIS (eMode 4), the "hard EIS" switch and blending [`_MI_LDC_IMPL_DisGetMatrix`, `_MI_LDC_IMPL_ProcEisSwitch` 0x13d00, `_MI_LDC_IMPL_AlphaBlending` 0x17a54, `_MI_LDC_IMPL_SetBlendingCfg`]
- **DIS**: the build has **no DIS / gyro algorithm** — `_MI_LDC_IMPL_DisGetMatrix` only logs "disalgo is no enable, use
  default cfg value: sliceNum" and uses the attribute's user slice number; every frame `MI_LDC_GENBIN_GetBins` regenerates
  the RSC slice tables (default configuration, §5.3) from the channel's static 3×3 projection matrix
  (`bUseProjection3x3Matrix` / `as32Projection3x3Matrix`, §4.1), `UpdateViewRes` with flag 1 (retried while BUSY) installs
  them and `PutBins` frees the temporaries.
- **Hard EIS**: the EIS flag (chn+0xED4) is only toggled by proc `eisswitch <chn>` ("chn%d use:%s eis", hard / soft) and
  reset to 0 by every `SetChnAttr`. When eMode = 4 and the flag is set, the three table addresses of the block attribute
  stay 0 and the slot normally holding the table address carries a pointer to the slice's 3×3 matrix.
  `HAL_LDC_SetBlockAttrEx` then sees zero addresses, prints "Invalid buf addr" and skips the 12F1.10 / .11, .03 / .04 (and
  mask) writes while `MHAL_LDC_SetBlkAttr` still returns 0, so the engine runs with the previous block's table addresses
  [R / I effect]: "hard EIS" is an unfinished path in this build — the per-slice matrices are passed down but nothing
  programs them into the engine.
- **Blending** (multi-view, overlapping views): only for outputs in NV12 ("only support yuv420nv12 blending"); overlapping
  view areas of the output are mixed with `MI_MVE_AlphaBlending` using the alpha map of §5.4, timed as "BlendT". The MVE
  context and `MVE_Input1` are freed at destroy. Not reached with one view [I].

### 6.6 Direct (register) mode vs CMDQ
With `CMDQBUFFSIZE` ≠ 0 (default) mi_sys hands the pass a CMDQ and every mhal call records its register writes into it
(mhal buffers ≤ 0xA0 entries per instance and replays them at Trigger, §8.5). With `CMDQBUFFSIZE=0` the pass has no CMDQ:
mhal writes registers directly and mi_ldc sleeps on the device wait queue until the ISR sets the frame-done flag (§2.3).

### 6.7 LUT mode and `DoLutDirectTask`
`MI_LDC_IMPL_DoLutDirectTask` (ioctl 9) **only returns `0xA0172012` (BUSY)** — the LUT direct path is a stub in this build
[R 0x20898]. A mode-2 channel can be created and started but `TryInitAlgo` is never run for it (no tables), so it produces
nothing [I from §5.3, §5.5].

## 7. Proc interface `/proc/mi_modules/mi_ldc/mi_ldc0` [R help text 0x70c3c…, handlers `_MI_LDC_IMPL_Proc*`]

### 7.1 Read
The mi_sys device / channel / port views filled by the five `ProcOn*` callbacks:
- dev: DevID, IRQ_Enable, IsrNum, IsrCnt, TotBlkNum, FramDonCnt, cmdqif, then mhal counters HalInFrmCnt, HalDoneFrmCnt,
  HalDoneBlkCnt, HalIsrDoneCnt, HalIsrErrCnt;
- chn: workmode (`dis-gyro` / `ldc` / `lut` / Unknown), status, InputWH, pixel, stride, InBufCnt, InTodoCnt, atom (in
  flight), EisBypass; EPTZ timing (EptzInit, OneBinSz, OneBin, TotalBin, LdcDone, EnqTDeq, BlendT); "LDC Table Info" per
  view (viewId, mode name from the §4.4 table, NumRuns, ImgFmt, OpMode, flip, InSize, OutSize, ViewSize, ViewTime) and per
  block (InBlk, OutBlk, SclBlk, XYScale, DeltaXY, Crop-En, BlkTime);
- output port: PortId, Pixel, OutputW/H, GetOputBufCnt, FinishOputBufCnt, FailOBufCnt, fps, OVTcnt.

### 7.2 Write commands [`_MI_LDC_IMPL_ProcDumpTaskFile` 0x14cbc, `_MI_LDC_IMPL_ProcDumpViewTableBin` 0x1f988, `_MI_LDC_IMPL_ProcSetOutputAttr` 0x14394, `_MI_LDC_IMPL_ProcLoopInputBuf` 0x13fa8, `_MI_LDC_IMPL_ProcDebug` 0x14838, `_MI_LDC_IMPL_ProcEisSwitch` 0x13d00, `_MI_LDC_IMPL_ProcSetBin` 0x1f3ec, `_MI_LDC_IMPL_IgnoreBlk` 0x144e0]
Written as `echo <words> > /proc/mi_modules/mi_ldc/mi_ldc0`; argv[0] is the command word, so "argc" counts it. Handler
arguments: (print callback, its handle, device id, argc, argv, user data); user data = NULL is a BUG_ON. The device is
looked up in the module's device list (magic 0x56577724; with the single-device marker 0x56533333 the id is forced to 0).
The channel id is `CamOsStrtoul(argv[1], 0, 10)` **truncated to u8** (`writeoutfile`, `setout`, `loopbuf`, `debug`,
`setbin`; `dumptablebin` keeps the full int; `eisswitch` and `ignore` are not covered by this reading); the channel is searched in the device's channel list (created byte = 1, id,
magic 0x56588888, §5.1); not found → "Chn %d not created." and **0xA0172304**. Wrong argc → "Invalid Param, argc:%d." and
**0xA0172003**. While a handler works on a channel it holds the channel rwsem chn+0x90 (read; write for `setbin`) and
counts itself in chn+0xF1C. Numbers are parsed base 10 with no error check (a non-number reads as 0).

| command | argc | parsing and effect |
|---|---|---|
| `writeoutfile <chn> <cnt> <path>` | 4 | cnt = strtoul **truncated to u16**; 0xFFFF → "argv[2] is %d error[0~65535]." 0xA0172003 (so 65535 and every value ≡ 0xFFFF mod 65536 are refused, larger values wrap). cnt is stored at chn+0xE50 *before* the path check. path: strlen(path) + 30 must be ≤ 0x80 (path ≤ 98 chars), copied with its NUL to chn+0xE54; longer → "filepath %s lenth > %d." (prints the path from its 31st character, and 0x80) and 0xA0172003. Success prints "ChanID %d DumpTaskFileNum %d filepath %s" and returns 0. Use: the output-task completion path writes, while chn+0xE50 ≠ 0, each finished output frame to `<path>/ldc_chn<chn>_<w>x<h>_pixel_<fmt>_<u64>.yuv` (format `%s/ldc_chn%d_%dx%d_pixel_%d_%llu.yuv`; `ldc_dump_outfile`: vmap of the output buffer, `MI_LDC_IMPL_WriteFile`, unmap) and decrements the count; cnt = 0 stops a running dump. |
| `dumptablebin <chn> <view\|all> <path>` | 4 | view: "all" (`strcasecmp`, any case) → 0xFF, else strtoul. Length check: strlen(path) + strlen("/ldc_chn0_view100_table.bin") (27) must be ≤ 0x80, else "File Name Invalid: len (%d %d %d)." 0xA0172003. The slot is taken with `_MI_LDC_IMPL_GetNewViewRes` (§5.4; none → "Ldc Chn%d Cannot Find ViewCfg." **0xA017201F**); no reference is taken on the slot. For view ids 0…99 equal to the request (or all): an entry without a table → "Ldc Chn%d Cannot find view %d" only when that single view was asked (all: skipped silently); else the table (entry virt, size) is written to `<path>/ldc_chn<chn>_view<i>_table.bin` (`%s/ldc_chn%d_view%d_table.bin` — `%d`, not the `/ldc_chn%d_view%02d_table.bin` of the help text and the length check). Returns 0 even when the view was missing. |
| `setout <chn> <w> <h> <pixel>` | not checked (only argc = 0 is a BUG_ON) | chn u8, w and h **u16**, pixel u32 → `MI_LDC_IMPL_SetOutputPortAttr(0, chn, {w, h, pixel})` (§5.5); its result is ignored, the handler returns 0. No channel lookup of its own. With fewer than 5 words argv[2…4] beyond the given ones are read anyway [R; I: what they contain depends on the proc framework]. |
| `loopbuf <chn> <cnt>` | 3 | cnt **u16**, 0xFFFF refused as in writeoutfile; stored at chn+0xED8; prints "ChanID %d loopbuf Cnt: %d"; 0 = off (help text). Effect: loop-buffer replay of the first *cnt* input frames (§6.2). |
| `debug <chn> <value>` | 3 | value = `CamOsStrtoull(argv[2], 0, 10)` (printed "chn%d value:%llu str:%s"). If 1 ≤ value ≤ 0xFFFFFFFE: bit n = value & 0xFF; else the string is compared case-insensitively: "off" → 1, "ViewRes" → 2, "ViewLock" → 3 (the help text writes "Viewlock"), "ChnRdLock" → 4, "ChnWrLock" → 5, "DisGyro" → 6, "Eptz" → 7; no match → "argv[2] is %s match %d error." 0xA0172003. Effect: the channel trace word chn+0xF28 **^= 1 << n** (a toggle, not a set; n ≥ 32 changes nothing, "0" is refused). Prints "ChanID %d tracelevel: 0x%px Mask:%lld" (the word printed with `%px`). The trace tests elsewhere are `(word & 0x0A) == 8` (ViewLock), `(word & 0x12) == 0x10` (ChnRdLock), `(word & 0x22) == 0x20` (ChnWrLock): bit 1 ("off") suppresses them while set. |
| `eisswitch <chn>` | (registered with 1 argument) | toggles the channel's EIS flag chn+0xED4 between "hard" and "soft" ("chn%d use:%s eis"); the hard mode is unfinished (§6.5). |
| `setbin <chn> <path>` | 3 | (help text: "SetBin [chnid] [path (file/folder)]"; only a file is handled). `CamFsOpen(path, O_RDONLY, 0644)`, size = `CamFsSeek(end)`, close; open failure or size ≤ 0 → "Chn %d illegal param path:%s size:%d ." 0xA0172003. Then `MI_LDC_IMPL_MemAlloc(chn, "ldc-bin-proc", size, map 1)` — **its result test is inverted**: MemAlloc returns 0 on success (as `UpdateViewRes` tests it), but setbin treats 0 as failure: prints "Chn %d no mem.", returns **0xA017200C** and does not free the buffer it just got (0x1f544 → 0x1f7d2 → 0x1f6f0, past the free at 0x1f6e8). Only when the allocation fails does it go on: `MI_LDC_IMPL_ReadFile(path, virt, size, 0)` with the empty buffer (non-zero result → 0xA0172003), then under the channel write lock up to 100 tries, 2 ms apart, of `ldc_proc_setbin(chn, &buf, 0)` (§5.4) while it returns BUSY, prints "chn%d setBin: %s" (the module's strings "Set ViewBin: id …" and "Set Bin Successfully" belong to this install path [R strings; I attribution]) and frees the buffer if still owned. So on a working system `setbin` always fails with 0xA017200C and leaks one buffer of the file's size per call [R code path; I "always", since a successful allocation is the normal case]. |
| `ignore <chn> view\|blk <mask> <enable>` | 5 | §7.3 |

### 7.3 `ignore` [`_MI_LDC_IMPL_IgnoreBlk` 0x144e0, use in `_MI_LDC_IMPL_Process`]
`echo ignore <chn> view|blk <mask> <enable> > /proc/mi_modules/mi_ldc/mi_ldc0` (exactly 5 words, else "Invalid Param";
unknown channel → 0xA0172304). `<mask>` is parsed as a **decimal** u64: bits 0–55 = 56 views, bits 56–62 = the group of 56
views they belong to (group = view / 56, so views 56–99 need a second command); `<enable>` ≠ 0 sets bit 63. The `view`
mask makes Process skip those views. The `blk` mask is tested in the block loop **with the view's bit, not the block's**
(the bit index is computed from the view counter) — so `blk` also skips whole views, only counting them per block; there is
no per-block skip.

## 8. mhal LDC block (89 functions, ≈15 KB) [R: register list per function]
<!-- ev:ldc-1-stack.8 -->

No source is known for the block. Ghidra finds 90 functions in the block range (the 89 named ones and one unnamed sysfs
helper). The HAL keeps a RAM shadow of the registers (§8.3) and writes either directly (VA of bank 0x12F0, the second bank
0x12F1 at VA + 0x200) or as (address, value) pairs into the caller's CMDQ list (≤ 0xA0 entries per instance, `inst+0x58`,
count `inst+0x558`).

### 8.1 Layers and objects
- **MHAL** (12 exports, all imported by mi_ldc):
  - `CreateDevice(id, &dev)` — 0x40-byte device {id, atomic instance count, mutex, …, +0x3C IRQ}; bank from DT
    `compatible = "sstar,ldc"` (id 0) / `"sstar,ldc%d"`, `reg` → base − 0x22000000 (RIU offset) and size; IRQ =
    `CamIrqOfParseAndMap(node, 0)` (0x4B; node `ldc`, `kernel-platform.md` §14); `DRV_LDC_DevInitModule` (§8.2); clears a
    0xA00-byte instance table. `DestroyDevice`.
  - `GetDevConfig(dev, id, {u32 size = 0xC, u16 ver = 1, u32 out})` (size ≠ 0xC or ver ≠ 1 → error 3, id ≥ 8 → 6): id 0 →
    IRQ, **1 → 0x38400 (CMDQ buffer bytes)**, 2–7 → current instance counters (+0x568 frame count, +0x56C done frames,
    +0x570 done blocks, +0x574 error ISRs, +0x578 / +0x57C totals).
  - `CreateInstance(dev, &inst)` — 0x584-byte instance (`DRV_LDC_InstAcquire`: refcounted, mutex, fences, counters; +0x54
    = its index = device instance count at creation), registered in the device table (`DRV_LDC_DevRegister`).
    `DestroyInstance`.
  - `SetFrmAttr`, `SetBlkAttr`, `SetWDmaAttr` (repack the mi_ldc structures of §6.4, then the DRV / HAL calls of §8.4; with
    a CMDQ the writes go into the instance list), `SetBlkFence(inst, b)` (instance +0x580 = block fence value to wait for),
    `Trigger(inst, cmdq)`, `ReTrigger(inst, …)`, `IsrProc(dev)`.
- **DRV** (`DRV_LDC_*`): device / instance bookkeeping, clocks, sysfs, ISR accounting (per instance: done / err ISR counts,
  frames complete when done + err ≥ the frame's block count). `DRV_LDC_InstSetWdmaMiuPrioAttr` and `…CscAttr` exist with
  HAL bodies but **nothing calls them** (no MHAL entry) — unused (§8.4).
- **HAL** (`HAL_LDC_*`): registers.

### 8.2 Init and clocks [`DRV_LDC_Init_ClockSetting` 0x40099b1c, `DRV_LDC_DevClkSetByRate` 0x40099c18, `DRV_LDC_DevClkSetByIndex` 0x40099cb8]
`DRV_LDC_DevInitModule` → `DRV_LDC_Init_KernelModule` (sysfs, §8.8; char device `/dev/ldc`, `mhal-core.md` §4.2),
`DRV_LDC_Init_ClockSetting`, `HAL_LDC_InitModule` (§8.3).
- **Clock choice at init**: module parameter `drv_ldc_module.m_u32PreSetLdcClkRate` (Hz, default 0) ≠ 0 → that rate ("Use
  Preset Rate"); else `drv_ldc_module.m_s32PreSetLdcClkIdx` (default −1 [I: the "−1 = unset" test]) ≠ −1 → the index's
  rate ("Use Preset Idx"); else the HAL default for the OD level ("Use Default Clk"): **432 MHz** (OD level 0) or 480 MHz.
  The OD level `m_u32LdcOdLevel` is a plain global, **not** a parameter. The other LDC parameters of mhal.ko are
  `drv_ldc_module.m_u32LdcMaskTblEn` (mask-table enable, read by `DRV_LDC_GetMaskTblEn`, default 0) and
  `drv_ldc_sysfs.m_u32LdcIdleClockDiv`, `drv_ldc_sysfs.m_u32LdcWorkClockDiv` (§8.8); all are `int`, mode 0644, written on
  the insmod line with the object prefix as shown (table of mhal's parameters: `mhal-core.md` §5).
- `DevClkSetByRate` looks the rate up in the clock-parent table (DT parents of `sstar,ldc` clock 0, the `ldcfeye` mux of
  `kernel-platform.md` §5.1) and fails with "Can't find clock index" for a rate not in the table; the parent is selected,
  the rate set, the clock prepared and enabled (`CamClk*`). With the HAL flag it writes the register directly instead:
  `HAL_LDC_SetClockRate` → **CLKGEN 1038.53** bits [4:2] (bit 0 = gate): 432 MHz → 0, 480 MHz → 0, 384 MHz → 2, 320 MHz → 3,
  288 MHz → 4, 216 MHz → 5, 86 MHz → 6, anything else → 1; rate 0 → gated. The same register is GOP 5's clock in LDC mode
  (`rgn-gop.md` §4.5).
- Working / idle clock dividers: 12F0.00 bits [7:4] / [11:8] (`HAL_LDC_SetWorkingClockDiv` / `HAL_LDC_SetIdleClockDiv`).

### 8.3 `HAL_LDC_InitModule` (0x4009ad68): shadow map and register effect
Shadow (u16 per register, at `m_pvLdcDevHandle`+4) is filled from the hardware in this order: +0x00 12F0.00, +0x02
12F0.01, +0x04…+0x36 12F0.02–.1B, +0x38…+0x76 12F0.20–.3F, +0x78…+0xA2 12F0.40–.55, +0xA4…+0xBC 12F0.60–.6C, +0xBE…+0xC6
12F0.71–.75, +0xC8…+0xEE 12F1.00–.13; further entries used by other functions: +0x108 / +0x10A / +0x10C = 12F1.40 / .41 /
.42 (SRAM dump). 12F1.30–.3B (interrupts), .4B / .4C, .53, .54 are never shadowed (read / written directly).
Writes, in order: 12F0.00 = (old & 0x800A) | 0x0FF5; 12F0.1B low nibble = 0xC; **12F0.3D = 0x4000, then 0x3800** (the
shadow's high byte is overwritten with 0x38 before the second write); 12F0.40 = 0x2020; **12F0.41 low byte = 0x40**;
12F0.20 bits [1:0] = 0; 12F0.51 = 0x3838; **12F1.2F = 0, 12F1.53 = 0, 12F1.54 = 0, 12F0.36 = 0, 12F0.38 = 0** (instance id
and fences reset); interrupts **12F1.3B = 0xFFFF, 12F1.35 = 0xFFFF, 12F1.36 = 0xFEFF, 12F1.30 = 0xFEFF, 12F1.3A = 1,
12F1.34 = 1** (two interrupt groups: mask / clear / force; bit 8 is the enabled source [I]). `HAL_LDC_UnInitModule` masks
and clears both groups (0xFFFF) and zeroes 12F1.3A / 12F1.34.

### 8.4 Per-block programming [`HAL_LDC_SetFrameAttr` 0x4009b1dc, `HAL_LDC_SetBlockAttrEx` 0x4009b5a0, `HAL_LDC_SetWdmaAttr` 0x4009c4e0]
Addresses go to the engine in units: frame input >> 5 (32 B), block tables >> 5, WDMA output >> 3 (8 B); "lo" = low 16
bits, "hi" = the next 11 (input / tables) or 12 (WDMA) bits. Because the table addresses are in 32-byte units, every block
record must be 32-byte aligned, which holds when the table buffer is (all part sizes are multiples of 0x20) [I from the
>> 5].

| step | registers |
|---|---|
| frame (input picture) | 12F0.01 bits [6:4] = block-size code c (hw_mode); 12F0.12 [12:0] = 0x10 << c (16…256); 12F0.1A = (old & 0x6000) \| 0x8080; 12F1.02 bit 4 = input mode ≠ 0; **Y** 12F1.05 lo / 12F1.06 hi; **U(V)** 12F1.07 / 12F1.08; **V** 12F1.09 / 12F1.0A; **stride** 12F1.0B [12:0]; direct mode pulses 12F1.00 bit 0 |
| block | delta base bits x / y → 12F0.13 bits [12:10] / [15:13], bit 0 = 1 (12F0.13 = (x & 7) << 10 \| y << 13 \| 1); input block x, y → 12F1.0C, 12F1.0D; input w, h → 12F0.0B, 12F0.0C and 12F1.0E, 12F1.0F; output x, y → 12F0.0D, 12F0.0E; output w, h → 12F0.0F, 12F0.10; bOutCropEn → 12F0.0A bit 0 (bit 1 always set); 12F0.11 low byte = 6; 12F0.02–09 [12:0] = 0; 12F0.14 / 16 / 18 = 0, 12F0.15 / 17 / 19 = 0xFF; **engine configuration block** (B+0x80) → 12F1.10 / 12F1.11; **delta table** (B+0x120) → 12F1.03 / 12F1.04 and 12F1.02 bit 1; **mask table** (B+0x1620, only with the mask enable) → 12F1.12 / 12F1.13, enable → 12F1.02 bit 0; 12F0.01 bit 2 cleared; 12F0.0E / 0F upper bits. A zero table address → "Invalid buf addr", the address writes are skipped (§6.5). The per-block scale bytes are not written |
| WDMA (output) | format f (0–6; others → 0): 12F0.21 bit 6 (f = 2), bit 4 (f = 5 / 6), bit 7 = enable, sub-mode bits 0x21 (f = 5) / 0x1E (f = 6); 12F0.22 = 0x8400 \| (f=1) \| (f=3)<<1 \| (f=4)<<2 \| (f<3)<<11; **Y** 12F0.24 lo / 12F0.25 hi; **UV** 12F0.26 / 12F0.27; **V** 12F0.28 / 12F0.29; width 12F0.3E, height 12F0.3F; 12F0.3C bits [7:6] and 12F0.50 bits [1:0] = mode field; 12F0.43 bit 15; **stride Y** 12F0.4C (halved for f = 2), **stride C** 12F0.4D (0 for f 2 / 5 / 6); 12F0.50 bit 2 / bit 3 = the two flag bits (h-mirror / v-flip [I]), bit 4 = f 5 / 6; 12F0.20 = control shadow |
| MIU priority (unused) | 12F0.20, .22, .23, .3C, .3D, .40, .41, .42, .50 bit 5 |
| CSC (unused) | 12F0.60 bit 0, 12F0.61 bit 0, 12F0.61–.62 bytes, 12F0.63–.6B 14-bit coefficients |

mi_ldc always passes format 0 [R: WDMA struct +0 left 0] — NV12 [I, the only output format mi_ldc accepts].

### 8.5 Trigger and fences [`MHAL_LDC_Trigger` 0x4009ab20, `HAL_LDC_TrigDeWrap` 0x4009c9e4]
- **Direct mode** (`cmdq` NULL), `HAL_LDC_TrigDeWrap`: 12F1.3B = 0xFFFF (clear), **1012.33 bit 9 cleared** (MIU bank 0x1012,
  outside the LDC banks; the bit's meaning is not traced — MIU registers: `kernel-platform.md` §12), 12F1.00 bit 0 set and
  written five times, **12F1.01 bit 0 pulsed 1 → 0 = start**. Done is reported by the IRQ (§2.3, §8.7).
- **CMDQ mode**: the instance list is checked (≥ 0xA0 entries → "Overflow"), the CMDQ is polled for space (`+0x0C`, up to
  1000 × 2 ms; status `+0x3C` bits 0x1700 = error → reset `+0x38`), then: the recorded writes (`+0x28`), **12F1.53 =
  instance id**, **12F0.36 = block fence**, wait on 12F1.38 (mask 0x400 / 0x400, `+0x2C`), **wait for CMDQ event 0x1A** (LDC
  done, `+0x30`), 12F0.38 = fence + 1, then the IRQ clear sequence 12F1.36 = 0xFFFF, 12F1.3B = 0xFFFF, 12F1.36 = 0xFEFF.
  The `+0x..` offsets are those of the CMDQ service interface (`mhal-scl-jpe-cmdq.md` §2.3).
- Readbacks: `HAL_LDC_GetCurExecInstId` = 12F1.53; `HAL_LDC_GetFrameFence` = 12F1.54; `HAL_LDC_GetBlockFence(&a, &b)` =
  12F0.36 (block fence written by the CMDQ path) and 12F0.38 (fence + 1 written after the done event).

### 8.6 Re-trigger [`HAL_LDC_ReTrigDeWrap` 0x4009cba4]
`MHAL_LDC_ReTrigger` reads the frame fence and the block fence (HAL), counts per block in `m_u32LdcReTrigCnt[0x280]`, and if
the hardware block fence equals the expected one re-executes (`DRV_LDC_InstReExecDeWarp` → `HAL_LDC_ReTrigDeWrap`).
`HAL_LDC_ReTrigDeWrap`: saves MIU **1012.33**, **sets its bit 9** (the normal `TrigDeWrap` clears it), writes 12F1.00 with
bit 0 cleared five times, then with bit 0 set five times, restores 1012.33, then writes 12F1.01 with bit 0 = 1 (start) and
clears the bit only in the shadow (no second write).

### 8.7 ISR [`HAL_LDC_ISR` 0x4009cd64]
Status = 12F1.33; 12F1.30 = 0xFFFF, 12F1.35 = 0xFFFF, 12F1.30 = 0xFEFF (clear + re-mask); status bit 8 set → done (0); else
bits 0x1868 → "Error INT" (2). `DRV_LDC_DevIsrFunc` accounts done / err per current instance and counts a frame when done
+ err reach the instance's block count.

### 8.8 sysfs (`DRV_LDC_Init_Sysfs`) [`check_*_show` / `check_*_store`, `DrvLdcOsSplitString`]
- **Show**: `check_clk` ("Current LDC clock [%d] Hz, Enable [%d]"), `check_idle_clk_div`, `check_wrok_clk_div` (sic),
  `check_dump_dbg_bus` (`HAL_LDC_ProbeDebugBus`), `check_dump_sram` (walks `HAL_LDC_DumpSram`), `check_inst_table_info`
  (instances, "FbMode (0:2048, 1:1024, 2:512, 3:256, 4:128)", "MaskEn"), `check_inst_statistic`,
  `check_retrig_statistic`.
- **Store**: input is split on the separator (`strsep`), the trailing newline of the last token removed, the first token
  read with base 10:

  | attribute | store effect |
  |---|---|
  | `check_clk` | 0 → clock closed; N → `DevClkSetByRate(N, HAL)` and N becomes the preset rate |
  | `check_idle_clk_div` / `check_wrok_clk_div` | value saved and written to 12F0.00 bits [11:8] / [7:4] (`HAL_LDC_SetIdleClockDiv` / `SetWorkingClockDiv`) |
  | `check_inst_table_info`, `check_inst_statistic` | both set the same "selected instance" global used by the two show functions |
  | `check_retrig_statistic`, `check_dump_dbg_bus`, `check_dump_sram` | accepted, no effect |

- **`HAL_LDC_DumpSram(sel, addr, width, &lo, &hi)`**: 12F1.41 low nibble = sel (bits 4:0 masked 0xE0, i.e. a 4-bit SRAM
  select), 12F1.42 = addr, 12F1.40 bit 0 = 1 (read strobe); then lo = 12F1.4B for width ≤ 16, lo / hi = 12F1.4B / 12F1.4C
  for width ≤ 32, else 0 / 0.
- **`HAL_LDC_ProbeDebugBus(&a, &b, &c, &d)`**: 12F0.00 = (old & 0x800A) | **0x1FF5** (bit 12 on, vs 0x0FF5 at init),
  12F0.00 bits [14:12] = 2 → read 12F0.54 → a, 12F0.55 → b; bits [14:12] = 0 → read 12F0.54 → c, 12F0.55 → d (bit 12 stays
  set afterwards).

## 9. The same stack in sibling firmwares of the SoC [R version strings, symbol tables, function sizes, immediates of the disassembly]

Compared: the `mi_ldc.ko` of the RunCam Split-H and SIYI A8 mini firmware,
the Anjoy MC-H12S firmware (`config/modules/4.9.227/mi_ldc.ko`) and their libmi_ldc.so (all different builds, md5 differ).

| firmware | mi_ldc.ko build | project / sdk commit | size |
|---|---|---|---|
| ours | 2022-07-21 | 7251436 / 80327e1 | 733 220 |
| runcam-splith | 2022-06-06 | 0940dba / b03a7d4 | 670 572 |
| siyi-a8 | 2021-11-26 | b60f184 / 50c6ec9 | 669 168 |
| anjoy-mch12s | 2021-11-02 | 93c66ce / 8f57177 | 698 976 |

All four are the same SigmaStar module for kernel 4.9.227 (same vermagic); libmi_ldc.so of each carries the same commits as
its module.
- **Only in ours** (vs runcam and siyi): the zoom-lens model (`eptz_zoom_lens_calculate`, `zl_*`,
  `zoom_lens_step_calculation`, `world2cam_poly_zoom_lens_init` — calib_type 3) and `Init_Alpha_Map`. **Only in theirs**:
  `MI_LDC_ALPHABLEND_*` and `MI_LDC_BLENDING_*` (Init / DeInit / UpdateCfg / Blend — the blending path as a separate front
  end; in ours it is folded into `_MI_LDC_IMPL_SetBlendingCfg` / `AlphaBlending`, §6.5) and `mi_ldc_insmod`.
- **anjoy** (the oldest) additionally lacks the whole GENBIN / DBIN front end (`MI_LDC_GENBIN_*`, `MI_LDC_DBIN_*` — its
  tables come only through `MI_LDC_EPTZ_GetBin / PutBin / AllocCfgBuf`), multi-view JSON (`*_mulview`,
  `multiview_check_param`), perspective (mode 15), `Platform_Check` (so no chip-id check by that name [I: none found]),
  `MI_LDC_IOCTL/IMPL_GetChnAttr`, `region_map_find_inside_refine_Q15` and the stitch autocrop.
- **libmi_ldc.so**: the same 11 exports in runcam; siyi and anjoy lack `MI_LDC_GetChnParam` (and the module's GetChnAttr
  ioctl).
- **Table format and tiling are the same in all four**: `ldc_bin_DeltaYX_convert` (424 bytes), `ldc_split_map_downsample`
  (1980), `ldc_split_find_vertical` (1258), `ldc_split_yshift` (420), `ldc_split_process` (802) and `world2cam_table` (588)
  have identical sizes; `ldc_bin_gen` differs in size (790 vs 628–644) but uses the same constants (0x16C0 record, 0x54 /
  0x74 / 0x78 / 0x80 / 0xC0 / 0x15C0 offsets, 0x100 mask memset, `vmov.i8 #0xff`); `ldc_split_init` has the same limits
  (42, 32, 0x800, 650, 16…256) with the split-state offsets shifted by 8 bytes (two more fields in ours). anjoy's
  `ldc_split_bounding_box` (0x16ea4, 648 bytes) differs from ours (0x29b6c) only in which nodes count — a raw in-picture
  test without clipping (`ldc-2-eptz-lib.md` §3.4). So tables generated to the same layout suit all four builds [I: block layout
  and limits identical; header use not compared].
