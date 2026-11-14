# sensor-gc8613 — the vendor GC8613 sensor driver `gc8613_MIPI.ko`
<!-- ev:sensor-gc8613 -->

The loadable kernel module that drives the camera's GalaxyCore GC8613 image sensor (MIPI CSI-2, RAW10) for SigmaStar's
cus-sensor framework in `mhal.ko`. It registers one sensor instance per enabled sensor pad, fills the sensor handle with the
sensor's fixed description (one 3840×2160 mode, 4 MIPI lanes, 24 MHz MCLK, I2C A16D8 at 0x20) and its callbacks, powers the
sensor up through mhal's sensor-interface functions, writes one 175-entry register table, and turns AE / frame-rate /
orientation requests into register values that it keeps in shadow arrays and sends only at the frame-active notify. The
sensor ABI it fills — the handle layout, `ISensorIfAPI`, `i2c_handle_t`, the registration exports and what mhal does with
each callback — is owned by `mhal-csi-snr.md` (§2 «Sensor ABI»; §3 for the SNR side); this document states what this
driver puts there and does. The boot script line that loads the module is in `rootfs-customer.md` §3.2; its place in the
firmware inventory is in `firmware-image.md` §3.3. What public sources state about the GC8613 chip itself — the part,
its documented registers, every public mode table — is §8.

Tags: **[R]** read from the binary, its disassembly or a file; **[S]** stated by a source; **[I]** inference. A heading
tagged [R] means every statement under it is [R] unless marked otherwise.

Binary [R]: `/config/modules/4.9.227/gc8613_MIPI.ko` on the camera, 11240 B, sha256
`1d0551c10751be89fbacc7e8a20f8e80080492fe037e4c9189e5295a0ca5d081`, GNU build-id `e01673e75051f89cb79cd78bec51c3c90698ecc1`,
ELF32 ARM relocatable, not stripped (local function symbols present, no DWARF).

Offsets are section-relative (`.text+0x…`, `.rodata+0x…`). Field names of the sensor handle are those of the reconstructed
sensor ABI (`mhal-csi-snr.md` §2.2) [I]; names of private-data fields, of unnamed `.bss`
variables and of the static `.rodata` tables are descriptive names given in this analysis [I] — the vendor's C source and
its names for them are not available.

## 1. Module identity and ELF layout

### 1.1 Identity [R]
<!-- ev:sensor-gc8613.1.1 -->

- `.modinfo`: `license=Proprietary`, `author=SigmaStar`, `description=Sensor_gc8613`, `depends=` (empty),
  `vermagic=4.9.227 SMP preempt mod_unload ARMv7 thumb2 p2v8 `. No `__versions` section (no symbol CRCs; the camera kernel
  is built without CONFIG_MODVERSIONS [I: the stock module loads without CRCs]).
- `.gnu.linkonce.this_module`: 512 B; module name `drv_ms_cus_gc8613_MIPI` at +0x0c — the name `lsmod` shows [R camera: `lsmod` size 7040]; relocations put `init_module` at +0xe8 and `cleanup_module` at +0x1dc. 1-byte `.plt` and
  `.init.plt` sections are present (the CONFIG_ARM_MODULE_PLTS module linker script [R sections; I cause]).
- `.comment`: `GCC: (GNU) 9.1.0` (twice). `.ARM.attributes`: Tag_CPU_name "7-A", Thumb-2, VFPv2, enum size int,
  `Tag_ABI_optimization_goals: Aggressive Size`, `Tag_CPU_unaligned_access: v6`. [I] built with `-Os`.
- Build string (`.rodata.str1.1+0x75`, the default value of the `sensor_module_version` parameter, §2):
  `MVX3M6__g2029d13af-dirty<dir>/Tiramisu_DLS00V012[sensor_module][Tiramisu_DLS00V012_8836_UVC<dir>/Tiramisu_DLS00V012 build.202509201040]XVM`,
  where `<dir>` (twice) is the build directory: a developer's home on the build host, then `8836/test/test_sensor_drvier`.
  Read as [I]: SDK `Tiramisu_DLS00V012`, product `8836_UVC`, git `2029d13af` with local changes ("-dirty"), built
  2025-09-20 10:40 in a directory named `test_sensor_drvier`, by the camera maker's build account «kjd».
- The source names of the init and exit functions are not in the symbol table; the string passed to printk as `%s` at load
  is `gc8613_init_driver` (`.rodata+0x56e`), i.e. `__FUNCTION__` of the init function [I].

### 1.2 Sections and symbols [R]

| section | size | content |
|---|---|---|
| `.text` | 0x864 | 24 functions (§1.3) |
| `.init.text` | 0x9c | `init_module` (156 B) |
| `.exit.text` | 0x44 | `cleanup_module` (68 B) |
| `.rodata` | 0x5c6 | register tables, two strings, parameter names (§6) |
| `.rodata.str1.1` | 0x174 | strings (§7) |
| `.modinfo` | 0x1c8 | §1.1, parameter descriptions (§2) |
| `__param` | 0x78 | 6 `kernel_param` entries (§2) |
| `.data` | 0x44 | `mirror_reg1` +0x00, `mirror_reg2` +0x24, parameter variables +0x34…+0x43 |
| `.bss` | 0x34 | module-global state (§3.4) |
| `.ARM.exidx` | 0xc0 | 24 entries, one per `.text` function, compact model 0 (compiler-generated unwind data); one more entry each for init and exit |

Imports (all undefined symbols): from mhal `CamOsMemAlloc`, `CamOsMemRelease`, `CamOsPrintf`, `CamOsUsSleep`,
`DrvRegisterSensorDriverEx`, `DrvRegisterSensorI2CSlaveID`, `DrvSensorHandleVer`, `DrvSensorI2CVer`, `DrvSensorIFVer`,
`DrvSensorRelease`; from the kernel `printk`, `msleep`, `memset`, `strcpy`, `param_ops_charp`, `param_ops_int`,
`__aeabi_uidiv`, `__aeabi_unwind_cpp_pr0`.

Global symbols: `init_module`, `cleanup_module`, `cus_camsensor_init_handle`, `__this_module`; data `mirror_reg1`,
`mirror_reg2`, `mclk`, `lane_num`, `hdr_lane_num`, `chmap`, `i2c_slave_id`; and two zero-size `.rodata` labels `PatternTbl`
and `TriggerStartTbl`, both at `.rodata+0x582` — empty tables that nothing references (no relocation names them); unused.

### 1.3 Function map (`.text`, Thumb; symbol, start, size in bytes) [R]

| function | off | size | | function | off | size |
|---|---|---|---|---|---|---|
| pCus_GetSensorID | 0x000 | 108 | | pCus_GetAEGain | 0x2fc | 84 |
| gc8613_SetPatternMode | 0x06c | 4 | | pCus_SetAEGain | 0x350 | 188 |
| pCus_GetVideoResNum | 0x070 | 10 | | pCus_GetAEMinMaxUSecs | 0x40c | 16 |
| pCus_GetVideoRes | 0x07a | 28 | | pCus_GetAEMinMaxGain | 0x41c | 16 |
| pCus_GetCurVideoRes | 0x096 | 32 | | gc8613_GetShutterInfo | 0x42c | 20 |
| pCus_GetOrien | 0x0b6 | 42 | | pCus_setCaliData_gain_linearity | 0x440 | 4 |
| pCus_SetOrien | 0x0e0 | 268 | | cus_camsensor_release_handle | 0x444 | 4 |
| pCus_GetFPS | 0x1ec | 6 | | cus_camsensor_init_handle (global) | 0x448 | 536 |
| pCus_SetFPS | 0x1f2 | 106 | | pCus_AEStatusNotify | 0x660 | 148 |
| pCus_GetAEUSecs | 0x25c | 44 | | pCus_init | 0x6f4 | 104 |
| pCus_SetAEUSecs | 0x288 | 116 | | pCus_poweroff | 0x75c | 64 |
| — | | | | pCus_poweron | 0x79c | 154 |
| | | | | pCus_SetVideoRes | 0x836 | 46 |

`init_module` = `.init.text+0`, `cleanup_module` = `.exit.text+0`.

## 2. Module parameters
<!-- ev:sensor-gc8613.2 -->

Six `kernel_param` entries in `__param`, 20 B each (name, module, ops, perm u16, level s8, flags u8, arg); all six have
level −1 (0xff) and flags 0. Names live in `.rodata+0x582…+0x5c5`, descriptions (`parm=`) in `.modinfo` [R].

| # | name | type (ops) | perm | variable | default | `parm=` description | read by code |
|---|---|---|---|---|---|---|---|
| 0 | `sensor_module_version` | charp | 0444 | `.data+0x34` (no symbol) | → build string (§1.1) | (none, only `parmtype`) | no |
| 1 | `mclk` | charp | 0660 | `mclk` `.data+0x38` | → `"use default parameter"` (`.rodata.str1.1+0x15e`) | "Assign MCLK" | no |
| 2 | `chmap` | int | 0660 | `chmap` `.bss+0x0c` | 0 | "VIF channel mapping" | init, exit |
| 3 | `i2c_slave_id` | int | 0660 | `i2c_slave_id` `.bss+0x10` | 0 | "sensor i2c slave id" | init |
| 4 | `hdr_lane_num` | int | 0660 | `hdr_lane_num` `.data+0x3c` | 4 | "sensor output lane number" | no |
| 5 | `lane_num` | int | 0660 | `lane_num` `.data+0x40` | 2 | "sensor output lane number" | no |

- Read by code [R]: the only relocations against `mclk`, `lane_num`, `hdr_lane_num` and `.data+0x34` are those in
  `__param`; `.text`, `.init.text` and `.exit.text` reference `.data` only at +0x00 and +0x24 (the mirror tables). So
  `lane_num`, `hdr_lane_num`, `mclk` and `sensor_module_version` are accepted and shown in sysfs but change nothing; the lane
  count the driver reports is the constant 4 and the MCLK the constant 24 MHz enum (§4.1); unused.
- `chmap` [R]: bit i (i = 0..3) set → a driver instance is registered on sensor pad i (§3.1). Default 0 → no pad: the module
  loads and registers nothing.
- `i2c_slave_id` [R]: byte i (bits 8i..8i+7) is passed as the slave id of pad i (§3.1). Default 0; mhal's
  `DrvRegisterSensorI2CSlaveID` ignores id 0 (`mhal-csi-snr.md` §2.1), so with the default the address stays the 0x20 set by
  the handle (§4.1).
- Use at boot [R camera]: `/customer/demo.sh` loads the module as `insmod
  …/gc8613_MIPI.ko chmap=1 lane_num=4` (`rootfs-customer.md` §3.2) → one instance on pad 0; `lane_num=4` has no effect [I from
  the fact above]. Values on the running camera [R camera]: chmap=1 lane_num=4 hdr_lane_num=4 i2c_slave_id=0
  mclk="use default parameter".

## 3. Init, exit and registration with the sensor framework

### 3.1 `init_module` (`.init.text+0x00`) [R]
<!-- ev:sensor-gc8613.3.1 -->

1. `DrvSensorHandleVer(2, 2)`, then `DrvSensorIFVer(2, 1)`, then `DrvSensorI2CVer(1, 1)`; any of them returning −1 →
   return −1 (module load fails). mhal's side of the handshake (majors 2 / 2 / 1 required, a differing minor only logged):
   `mhal-csi-snr.md` §2.1.
2. For pad i = 0..3, if `(chmap >> i) & 1`:
   - `priv = CamOsMemAlloc(0xe8)` (232 B, §4.3); `memset(priv, 0, 0xe8)` — the result of the allocation is not checked;
   - `DrvRegisterSensorDriverEx(i, cus_camsensor_init_handle, priv)` — return value ignored;
   - `DrvRegisterSensorI2CSlaveID(i, (u8)(i2c_slave_id >> (8·i)))` — return value ignored;
   - `printk("\0016Connect %s linear to sensor pad %d\n", "gc8613_init_driver", i)` (KERN_INFO);
   - `priv_tab[i] = priv` (`.bss+0x14 + 4·i`) — stored after the printk.
3. Return 0.

`DrvRegisterSensorDriverEx` zeroes the 2588-byte handle, stores `priv` at `handle+0x28`, the I2C table at `+0x40`, the IF
table at `+0x44`, and calls `cus_camsensor_init_handle(handle)` at once (`mhal-csi-snr.md` §2.1) — so §4 runs inside
step 2. With the boot parameters (§2) the log line is `Connect gc8613_init_driver linear to sensor pad 0` [R camera].

### 3.2 `cleanup_module` (`.exit.text+0x00`) [R]

For pad i = 0..3, if `(chmap >> i) & 1` and (`priv_tab[i]` ≠ NULL or `aux_tab[i]` ≠ NULL): `DrvSensorRelease(i)`; if
`priv_tab[i]` → `CamOsMemRelease`, set NULL; if `aux_tab[i]` (`.bss+0x24 + 4·i`) → `CamOsMemRelease`, set NULL.

`aux_tab` is never written anywhere in the module (the only `.bss` relocations are +0x00 — SetFPS, SetAEUSecs, SetAEGain,
init — and +0x14 — init, exit), so it is always NULL; unused. [I] a second per-pad allocation of the SDK template (e.g. for
HDR) that this build does not use.

### 3.3 `cus_camsensor_release_handle` (`.text+0x444`) [R]

Returns 0, does nothing; installed as `pCus_sensor_release` (§4.2).

### 3.4 Module-global state (`.bss`, 0x34 B) [R]

| off | size | name | content |
|---|---|---|---|
| 0x00 | 4 | (none) `last_vts` | frame length last put into the VTS shadow (SetFPS, SetAEUSecs) |
| 0x04 | 4 | (none) `last_expo` | exposure lines last put into the exposure shadow (SetAEUSecs) |
| 0x08 | 4 | (none) `last_gain` | clamped gain last put into the gain shadow (SetAEGain) |
| 0x0c | 4 | `chmap` | module parameter |
| 0x10 | 4 | `i2c_slave_id` | module parameter |
| 0x14 | 16 | (none) `priv_tab[4]` | private-data pointer per pad |
| 0x24 | 16 | (none) `aux_tab[4]` | second per-pad pointer, freed in exit, never set; unused |

The three `last_*` caches are module-wide, not per pad [R]: with two pads enabled the instances share them [I
consequence]. They start at 0, so the first SetFPS / SetAEUSecs / SetAEGain after load always marks its shadow dirty [I
from the compare-and-skip code, §5.6–§5.8].

## 4. Handle fields — `cus_camsensor_init_handle` (`.text+0x448`)

The only global function in `.text`; it is what the driver tells the framework. Entry checks [R]: `handle == NULL` → −1;
`handle->private_data` (`+0x28`) NULL → `CamOsPrintf("[%s] Private data is empty!\n", "cus_camsensor_init_handle")` → −1.
Otherwise it fills the private data (§4.3), the handle (§4.1, §4.2) and returns 0.

### 4.1 Values written [R values; field names from `cus_sensor_m6.h`, I]
<!-- ev:sensor-gc8613.4.1 -->

| off | field | value | | off | field | value |
|---|---|---|---|---|---|---|
| 0x004 | model_id | `"gc8613_MIPI"` (strcpy) | | 0x084 | orient | 0 (M0F0) |
| 0x030 | i2c_cfg.mode | 1 (NORMAL) | | 0x090 | num_res | 1 |
| 0x034 | i2c_cfg.fmt | 1 (A16D8) | | 0x094 | ulcur_res | 0 |
| 0x038 | i2c_cfg.speed | 200000 | | 0x098 | res[0].width | 3840 |
| 0x03c | i2c_cfg.address (u16) | 0x20 | | 0x09c | res[0].height | 2160 |
| 0x048 | ae_gain_delay | 2 | | 0x0a0 | res[0].max_fps | 30 |
| 0x04c | ae_shutter_delay | 2 | | 0x0a4 | res[0].min_fps | 5 |
| 0x050 | ae_gain_ctrl_num | 1 | | 0x0a8 | res[0].crop_start_x | 0 |
| 0x054 | ae_shutter_ctrl_num | 1 | | 0x0ac | res[0].crop_start_y | 0 |
| 0x058 | reset_POLARITY | 1 (NEG) | | 0x0b0 | res[0].nOutputWidth | 3840 |
| 0x05c | pwdn_POLARITY | 0 (POS) | | 0x0b4 | res[0].nOutputHeight | 2160 |
| 0x060 | VSYNC_POLARITY | 1 | | 0x950 | mipi_lane_num | 4 |
| 0x064 | HSYNC_POLARITY | 1 | | 0x954 | mipi_data_format | 1 |
| 0x068 | PCLK_POLARITY | 0 | | 0x958 | mipi_yuv_order | 0 |
| 0x06c | isp_type | 1 | | 0x95c | mipi_hsync_mode | 0 |
| 0x070 | bayer_id | 0 (RG) | | 0x964 | mipi_hdr_mode | 0 (none) |
| 0x074 | RGBIR_id | 0 | | 0x968 | mipi_hdr_virtual_channel_num | 0 |
| 0x078 | sif_bus | 1 (MIPI) | | 0x97c | mclk | 10 (CUS_CMU_CLK_24MHZ) |
| 0x07c | data_prec | 1 (10 bit) | | 0x980 | sat_mingain | 1024 (1×) |
| 0x080 | data_mode | 5 (CUS_SEN_10TO12_9000) | | | | |

Not written, so 0 from mhal's memset (`mhal-csi-snr.md` §2.1): `res[0].strResDesc` (+0xb8 — the mode has an empty
description), `res[0].nMinFrameLengthLine` (+0xd8), `res[0].nRowTime` (+0xdc), `mipi_sampling_delay` (+0x960),
`res[1..30]`.

The I2C address 0x20 is the 8-bit write form, 7-bit 0x10 (`mhal-csi-snr.md` §2.4); the camera's sensor answers on I2C bus 1
at that address [R camera: `/customer/i2c_read_write -b 1 -a 0x20 -f A16D8`; buses 0 and 2 time out]. With the
boot parameters the camera's mi_sensor reports pad 0 as one mode `0,0,3840,2160 → 3840x2160, MaxFps 30, MinFps 5`, MIPI,
10-bit RG Bayer, HDR off [R camera].

### 4.2 Callbacks installed [R]

| handle off | slot | function |
|---|---|---|
| +0x98c | poweron | `pCus_poweron` |
| +0x990 | poweroff | `pCus_poweroff` |
| +0x994 | init (`pCus_sensor_init`) | `pCus_init` |
| +0x99c | release (`pCus_sensor_release`) | `cus_camsensor_release_handle` |
| +0x9a8 | SetPatternMode | `gc8613_SetPatternMode` |
| +0x9ac | GetSensorID | `pCus_GetSensorID` |
| +0x9b0 / +0x9b4 / +0x9b8 | GetVideoRes / GetCurVideoRes / SetVideoRes | `pCus_GetVideoRes` / `pCus_GetCurVideoRes` / `pCus_SetVideoRes` |
| +0x9bc / +0x9c0 | GetOrien / SetOrien | `pCus_GetOrien` / `pCus_SetOrien` |
| +0x9c8 | AEStatusNotify | `pCus_AEStatusNotify` |
| +0x9cc / +0x9d0 | GetAEUSecs / SetAEUSecs | `pCus_GetAEUSecs` / `pCus_SetAEUSecs` |
| +0x9d4 / +0x9d8 | GetAEGain / SetAEGain | `pCus_GetAEGain` / `pCus_SetAEGain` |
| +0x9dc / +0x9e0 | GetAEMinMaxUSecs / GetAEMinMaxGain | `pCus_GetAEMinMaxUSecs` / `pCus_GetAEMinMaxGain` |
| +0x9e4 / +0x9e8 | GetFPS / SetFPS | `pCus_GetFPS` / `pCus_SetFPS` |
| +0x9f0 | setCaliData_gain_linearity | `pCus_setCaliData_gain_linearity` |
| +0x9f4 | GetShutterInfo | `gc8613_GetShutterInfo` |
| +0x9f8 | GetVideoResNum | `pCus_GetVideoResNum` |

Left NULL (no store): +0x998 post_init, +0x9a0 suspend, +0x9a4 resume, +0x9c4 GetSensorCap, +0x9ec SetAEGain_cal,
+0x9fc…+0xa18 (eight slots: CustDefineFunction … GetCurSwtichSensorId by the I6C names); unused. mhal's `DrvSensorX`
wrapper returns 0 with the output untouched for a NULL slot (`mhal-csi-snr.md` §3.2).

### 4.3 Private data (232 B, allocated in init, `handle+0x28`) [R offsets; names descriptive, I]

| off | type | name | init_handle sets | written by | read by |
|---|---|---|---|---|---|
| 0x00–0x37 | — | unused | — | nothing | nothing |
| 0x38 | u32 | vts (frame length for the current fps, before stretching) | 2250 | SetFPS | SetFPS, SetAEUSecs |
| 0x3c–0x4f | — | unused | — | — | — |
| 0x50 | u32 | requested gain (before clamping) | — | SetAEGain | nothing |
| 0x54 | — | unused | — | — | — |
| 0x58 | u32 | fps (as given: 5…30 or 5000…30000) | 30 | SetFPS | GetFPS |
| 0x5c | u32 | exposure lines | 100 | SetAEUSecs | SetFPS, SetAEUSecs |
| 0x60–0x6b | — | unused | — | — | — |
| 0x6c | u8 | (cleared, never used) | 0 | init_handle (one 32-bit store clears 0x6c–0x6f) | nothing |
| 0x6d | u8 | orient_dirty | 0 | SetOrien (1), AEStatusNotify (0) | AEStatusNotify |
| 0x6e | u8 | gain_dirty | 0 | SetAEGain, AEStatusNotify | AEStatusNotify |
| 0x6f | u8 | expo_dirty | 0 | SetAEUSecs, AEStatusNotify | AEStatusNotify |
| 0x70 | u8 | vts_dirty | 0 | SetFPS, SetAEUSecs, AEStatusNotify | AEStatusNotify |
| 0x71 | u8 | pad | — | — | — |
| 0x72 | I2C_ARRAY[2] | VTS shadow | copy of `vts_reg` | SetFPS, SetAEUSecs | AEStatusNotify |
| 0x7a | I2C_ARRAY[12] | gain shadow | copy of `gain_reg` | SetAEGain | GetAEGain, AEStatusNotify |
| 0xaa | I2C_ARRAY[2] | exposure shadow | copy of `expo_reg` | SetAEUSecs | GetAEUSecs, AEStatusNotify |
| 0xb2 | I2C_ARRAY[9] | orientation shadow 1 | copy of `mirror_reg1` | SetOrien | GetOrien, AEStatusNotify |
| 0xd6 | I2C_ARRAY[4] | orientation shadow 2 | copy of `mirror_reg2` | SetOrien | AEStatusNotify |
| 0xe6 | 2 B | unused | — | — | — |

`I2C_ARRAY` = `{u16 reg; u16 data}` [R: 4-byte stride, `strh` to `+2` of each entry]. The arrays start at 2-mod-4 offsets
and are copied with unaligned 32-bit `ldr/str` [R] (allowed by `Tag_CPU_unaligned_access: v6`, §1.1). The shadows start
with the table defaults (§6.2, §6.3): VTS shadow 0x05dc (1500 lines), exposure shadow 0x05d0 (1488 lines) — different from
what `Sensor_init_table` writes (VTS 0x08ca = 2250, exposure 0x0127 = 295) [R]; they reach the sensor only after a setter
marks them dirty (§5.5).

## 5. Callbacks of the sensor handle

`IF` = `handle->sensor_if_api` (`+0x44`), `I2C` = `handle->i2c_bus` (`+0x40`), `cfg` = `&handle->i2c_cfg` (`+0x30`). I2C
calls are `I2C->fn(I2C, cfg, …)`. Slot names of the IF and I2C tables and what mhal does behind them: `mhal-csi-snr.md`
§2.3, §2.4.

### 5.1 Power sequence [R]

`pCus_poweron(handle, idx)` (`.text+0x79c`), in order:

| # | call | arguments (values with the §4.1 fields) |
|---|---|---|
| 1 | `IF->SetIOPad` (+0x1c) | (idx, `sif_bus` = 1 MIPI, `mipi_lane_num` = 4) |
| 2 | `IF->SetCSI_Clk` (+0x20) | (idx, 4 = CUS_CSI_CLK_216M) — constant |
| 3 | `IF->SetCSI_Lane` (+0x24) | (idx, (u16)`mipi_lane_num` = 4, 1) |
| 4 | `IF->SetCSI_LongPacketType` (+0x28) | (idx, 0, 0x1c00, 0) — constants |
| 5 | `IF->PowerOff` (+0x08) | (idx, `pwdn_POLARITY == 0` → 1) |
| 6 | `CamOsUsSleep(1000)` | |
| 7 | `IF->Reset` (+0x0c) | (idx, `reset_POLARITY` = 1) |
| 8 | `CamOsUsSleep(1000)` | |
| 9 | `IF->PowerOff` | (idx, `pwdn_POLARITY` = 0) |
| 10 | `CamOsUsSleep(1000)` | |
| 11 | `IF->Reset` | (idx, `reset_POLARITY == 0` → 0) |
| 12 | `CamOsUsSleep(1000)` | |
| 13 | `IF->MCLK` (+0x10) | (idx, 1, `mclk` = 10 → 24 MHz) |
| 14 | `CamOsUsSleep(5000)` | return 0 |

Return values of the IF calls are ignored. The MCLK is switched on last, after reset is released. What each step does at
register level on pad 0 (pads and CSI set up, RAW8 / 10 / 12 long packets accepted by `153a.1e = 0x1c00`, power-down and reset pins
`1304.00[3]` / `1304.00[2]`, MCLK `1038.62[13:8] = 0x1c`): `mhal-csi-snr.md` §3.4.

`pCus_poweroff(handle, idx)` (`.text+0x75c`): `IF->PowerOff(idx, 1)`; `IF->Reset(idx, 1)`; `CamOsUsSleep(1000)`;
`IF->SetCSI_Clk(idx, −1 = CUS_CSI_CLK_DISABLE)`; `IF->MCLK(idx, 0, 10)`; return 0. No sleep after the MCLK off.
Register level: `mhal-csi-snr.md` §3.5.

mhal calls `poweron`, then `init` (§5.3), then `post_init` (NULL here) under the pad mutex in `DrvSensorInit`
(`mhal-csi-snr.md` §3.4).

### 5.2 `pCus_GetSensorID(handle, u32 *id)` (`.text+0x000`) [R]
<!-- ev:sensor-gc8613.5.2 -->

- Stack array `{ {0x03f0, 0}, {0x03f2, 0} }`; `*id = 0`.
- Up to 3 tries of `I2C->i2c_array_rx(I2C, cfg, array, 2)` (+0x28); a try is repeated only when the call returns non-zero;
  three failures → −1.
- After a successful read: `data[0]` (reg 0x03f0) ≠ 0x86 → −1; `*id = 0x86 << 8`; `data[1]` (reg 0x03f2) ≠ 0x23 → −1
  (with `*id` left at 0x8600); else `*id = 0x8623`, return 0. No retry on a value mismatch.
- Register 0x03f1 (chip id low, 0x13) is not read.

The sensor's ID registers [S: from `rk_gc8613.c` and the thingino drivers]: 0x03f0 = 0x86,
0x03f1 = 0x13; 0x03f2 is the variant byte, 0x03 = GC8613 "YN", 0x23 = GC8613 "YA". The camera's sensor reads 0x03f2 = 0x03
(YN) [R camera], so this callback returns −1 on it [I]. mhal reaches `GetSensorID` only from `/dev/sensorif`, not on
the bring-up path, and the line «Sensor is abnormal @sensor pad 0» in the camera's boot log comes from mhal's stall monitor,
not from this callback (`mhal-csi-snr.md` §3.8).

### 5.3 `pCus_init(handle)` (`.text+0x6f4`) — sensor register initialisation [R]

- Walks the 175 entries of `Sensor_init_table` (`.rodata+0x2b2`, §6.4) in order.
- Entry with `reg == 0xffff` → `msleep(data)` (the table has one: index 165, `{0xffff, 0x14}` = 20 ms).
- Other entries → `I2C->i2c_tx(I2C, cfg, reg, data)` (+0x14), one register per call (A16D8 → one 3-byte I2C write,
  `mhal-csi-snr.md` §2.4). Non-zero return → retry; at most 10 attempts per register with `msleep(10)` before each retry;
  the 10th failure returns −1 at once (the rest of the table is not written).
- All entries written → return 0. It writes nothing else: no shadow flush, no stream-on outside the table (the stream-on
  is inside the table, §6.4).

### 5.4 Resolution callbacks [R]

- `pCus_GetVideoResNum(handle, *n)` (`+0x070`): `*n = num_res` (1); return 0.
- `pCus_GetVideoRes(handle, idx, **res)` (`+0x07a`): `idx < num_res` → `*res = &res[idx]` (`handle + 0x98 + 72·idx`), 0;
  else −1 (`*res` untouched).
- `pCus_GetCurVideoRes(handle, *cur, **res)` (`+0x096`): `*cur = ulcur_res` always; `ulcur_res < num_res` → `*res =
  &res[ulcur_res]`, 0; else −1.
- `pCus_SetVideoRes(handle, idx)` (`+0x836`): `idx ≥ num_res` → −1. `idx == 0` → `ulcur_res = 0`, `pCus_sensor_init =
  pCus_init` (the same pointer as before), `printk("===================9/20=============\n")` (no log level), return 0.
  `0 < idx < num_res` → return 0 without changes (unreachable with `num_res` = 1).
- The one mode: 3840×2160, 30 fps max, 5 fps min, no crop, output 3840×2160; [I] its line time and frame length are those
  of §5.6 (14814 ns, 2250 lines at 30 fps), which the handle does not report (`nRowTime` / `nMinFrameLengthLine` are 0).

### 5.5 `pCus_AEStatusNotify(handle, status)` (`.text+0x660`) — the only place runtime registers are sent [R]

`status ≠ 1` → return 0 at once. `status == 1` (CUS_FRAME_ACTIVE), in this order, each block only if its flag is set:

1. orientation: `I2C->i2c_array_tx(I2C, cfg, priv+0xb2, 9)` (+0x1c); `msleep(20)`; `i2c_array_tx(…, priv+0xd6, 4)`;
   `orient_dirty = 0`;
2. gain: `i2c_array_tx(…, priv+0x7a, 12)`; `gain_dirty = 0`;
3. exposure: `i2c_array_tx(…, priv+0xaa, 2)`; `expo_dirty = 0`;
4. frame length: `i2c_array_tx(…, priv+0x72, 2)`; `vts_dirty = 0`;
5. return 0.

Return values of the transfers are ignored: a failed transfer still clears its flag. The register order on the bus within
one notify is therefore orientation (9 writes, 20 ms, 4 writes) → gain block (§5.8) → 0x0203, 0x0202 → 0x0340, 0x0341.

Who calls the slot and with which status (`mhal-csi-snr.md` §3.6): the VIF line-count event passes 0 (ignored by this
driver, so that path is a no-op with it); status 1 comes from the ISP's frame start, the ISP's sensor frame done, the
user AE-parameter path, `CameraDebugSetSensorFPS` and the `/dev/sensorif` gain command. The shadows therefore reach the
sensor at the next of these ISP events after a setter [I].

### 5.6 Frame rate [R]
<!-- ev:sensor-gc8613.5.6 -->

`pCus_SetFPS(handle, fps)` (`.text+0x1f2`):
- `5 ≤ fps ≤ 30` → `priv.fps = fps`, `vts = 67500 / fps` (`__aeabi_uidiv`, literal 0x107ac);
- else `5000 ≤ fps ≤ 30000` (the ×1000 form) → `priv.fps = fps` (stored ×1000), `vts = 67500000 / fps` (literal
  0x0405f7e0);
- else → −1, nothing changed;
- `priv.vts = vts`; if `priv.expo_lines > vts − 4` the value sent becomes `expo_lines + 4` (`priv.vts` keeps the
  unstretched value); if the value differs from `last_vts` → `last_vts` = it, VTS shadow `0x0340 = (v >> 8) & 0x3f`,
  `0x0341 = v & 0xff`, `vts_dirty = 1`. Return 0.

`pCus_GetFPS(handle)` (`.text+0x1ec`): returns `priv.fps` as stored — 30 after init, or whatever form SetFPS last received
(×1000 if it was called with the ×1000 form). [R] mhal's `SetFPS` passes values ≥ 1000 as ×1000.

Timing constants [R literals; I meaning]: 67500 lines/s ↔ line time 14814 ns (`0x39de`, used for exposure and shutter
info); 2250 lines at 30 fps (also the init value of `priv.vts`), 13500 at 5 fps. Frame length field: 14 bits (6 + 8) [R
masks].

### 5.7 Exposure [R]

`pCus_SetAEUSecs(handle, us)` (`.text+0x288`):
- `ns = us · 1000` (32-bit product, wraps above 4 294 967 µs [I arithmetic]);
- `lines = ns > 14813 ? ns / 14814 : 1` (the division is compiled as `((ns >> 1) · 0x46c863d1) >> 43`);
- `priv.expo_lines = lines` — no upper clamp, no maximum;
- frame length to send = `priv.vts`, or `lines + 4` if `lines > priv.vts − 4`;
- if `lines ≠ last_expo` → `last_expo = lines`, exposure shadow `0x0203 = lines & 0xff`, `0x0202 = (lines >> 8) & 0x3f`,
  `expo_dirty = 1`;
- if the frame length ≠ `last_vts` → `last_vts` = it, VTS shadow as in §5.6, `vts_dirty = 1`; return 0.

So a shutter longer than the frame stretches the frame (the frame rate drops) instead of being clamped. Example [I
arithmetic]: 33333 µs → 2250 lines → frame 2254 lines → 29.95 fps. The exposure field is 14 bits (max 16383 lines
≈ 242.7 ms); the advertised maximum 200 000 µs (§5.10) is 13 500 lines, inside the field [I arithmetic].

`pCus_GetAEUSecs(handle, *us)` (`.text+0x25c`): `lines = ((shadow 0x0202 << 8) & 0x3f00) | low byte of shadow 0x0203`;
`*us = lines · 14814 / 1000`; return 0. It reports the shadow, not the sensor; after init (before any SetAEUSecs) it
reports the `.rodata` default 0x05d0 = 1488 lines → 22 043 µs, not the 295 lines the init table wrote [R values; I
arithmetic].

### 5.8 Gain [R]

Units: 1024 = 1×. `pCus_SetAEGain(handle, gain)` (`.text+0x350`):
1. `priv+0x50 = gain` (as requested);
2. clamp to [1024, 0xc2000] (0xc2000 = 794 624 = 776×); equal to `last_gain` → return 0; else `last_gain = g`;
3. pick gain_table entry k (§6.1): the first k in 0..28 with `gain_table[k].gain ≤ g < gain_table[k+1].gain`, else k = 29;
4. digital factor `d = (g << 6) / gain_table[k].gain`, then `d = min(d, 1024)`;
5. gain shadow (12 entries, layout of `gain_reg`, §6.2): `[0] 0x031d = 0x2d`, `[1] 0x0614 = v0`, `[2] 0x0615 = v1`,
   `[3] 0x031d = 0x28`, `[4] 0x0225 = v2`, `[5] 0x1467 = v3`, `[6] 0x1468 = v4`, `[7] 0x00b8 = v5`, `[8] 0x00b9 = v6`,
   `[9] 0x1447 = v7`, `[10] 0x0064 = d >> 6`, `[11] 0x0065 = d & 0x3f`; `gain_dirty = 1`; return 0.

[I arithmetic] Analog steps run from 1× to 132.92× (entry 29); between steps the digital factor `d/64` covers the gap
(consecutive entries differ by ≤ 1.233×, so `d` ≤ 78 below entry 29); above 132.92× the digital factor reaches at most
794 624·64/136 110 = 373 → 0x0064 ≤ 5. The `d = 1024` clamp is never reached inside the gain clamp.

On the bus the 12 writes go in shadow order: 0x031d = 0x2d, 0x0614, 0x0615, 0x031d = 0x28, then 0x0225, 0x1467, 0x1468,
0x00b8, 0x00b9, 0x1447, 0x0064, 0x0065 — only 0x0614 / 0x0615 are between the two 0x031d writes [R order]. What 0x031d
does is not stated in the binary or in any source held; that it is a group hold is not established [I].

`pCus_GetAEGain(handle, *gain)` (`.text+0x2fc`):
- analog: the gain of the first table entry whose `(v0, v1)` equals the shadow `(0x0614, 0x0615)`, else 1024;
- digital: `64·(shadow 0x0064 & 0x0f) + (shadow 0x0065 >> 2)`;
- `*gain = (analog · digital) >> 6`; return 0. It reports the shadow, not the sensor.

Two properties of the binary [R table data and code, I consequence]:
- (a) `(v0, v1)` is not unique in the table — entries 5–9 and 13–20 share their pair with an earlier entry (5↔0, 6↔1,
  7↔2, 8↔3, 9↔4, 13↔0, 14↔1, 15↔2, 16↔3, 17↔4, 18↔10, 19↔11, 20↔12), so for gains between 2.25× and 33.1× GetAEGain
  reports the analog part of a lower entry;
- (b) SetAEGain writes 0x0065 as `d & 0x3f` but GetAEGain reads it as `0x0065 >> 2`.

So GetAEGain returns what SetAEGain was given only when the gain equals the gain of one of entries 0–4, 10–12, 21–29
exactly (then d = 64: 0x0064 = 1, 0x0065 = 0). The datasheet's register list (§8.3) puts the 10-bit `auto_pregain` in
0x0064 bits 3:0 (pregain[9:6]) and 0x0065 bits 7:2 (pregain[5:0]) [S] — the layout GetAEGain reads; SetAEGain's unshifted
`d & 0x3f` lands in 0x0065 bits 5:0 [I]. On this camera's sensor (YN) 0x0065 holds bits 5:0 only — 0x80 written reads
back 0, 0xfc reads 0x3c, 0x20 brightens the picture — so SetAEGain's write is the right one; 0x0064 alone does not take
effect, the write of 0x0065 takes the pair [R camera, `reg_poke` / `reg_peek`, 2026-11-12]. The same list names 0x0614
/ 0x0615[3:0] / 0x0225[0] the 13-bit analog PGA gain and 0x00b8[5:0] / 0x00b9[5:0] the 12-bit column gain [S]. mhal reads GetAEGain in `GetPlaneInfo` (`mhal-csi-snr.md` §3.3); whether the
values of (a) and (b) affect the ISP's AE is not known.

### 5.9 Mirror / flip / orientation [R]
<!-- ev:sensor-gc8613.5.9 -->

`pCus_SetOrien(handle, ori)` (`.text+0x0e0`): `ori > 3` → return 0 with nothing changed. For 0..3 it rewrites every data
field of both orientation shadows and sets `orient_dirty = 1`; return 0. It does not update `handle->orient` (+0x84).
Values written (register: value), per `ori` (CUS_ORIT_M0F0 = 0, M1F0 = 1, M0F1 = 2, M1F1 = 3 by the header's enum):

| shadow entry | reg | M0F0 | M1F0 | M0F1 | M1F1 |
|---|---|---|---|---|---|
| mirror_reg1[0] | 0x022c | 0x00 | 0x00 | 0x01 | 0x01 |
| mirror_reg1[1] | 0x0063 | 0x00 | 0x05 | 0x02 | 0x07 |
| mirror_reg1[2] | 0x0a67 | 0x80 | 0x80 | 0x80 | 0x80 |
| mirror_reg1[3] | 0x0a98 | 0x04 | 0x04 | 0x04 | 0x04 |
| mirror_reg1[4] | 0x05be | 0x00 | 0x00 | 0x00 | 0x00 |
| mirror_reg1[5] | 0x05a9 | 0x01 | 0x01 | 0x01 | 0x01 |
| mirror_reg1[6] | 0x0a70 | 0x03 | 0x03 | 0x03 | 0x03 |
| mirror_reg1[7] | 0x0a73 | 0x60 | 0x61 | 0x62 | 0x63 |
| mirror_reg1[8] | 0x0a5a | 0x80 | 0x80 | 0x80 | 0x80 |
| mirror_reg2[0] | 0x05be | 0x01 | 0x01 | 0x01 | 0x01 |
| mirror_reg2[1] | 0x0a70 | 0x00 | 0x00 | 0x00 | 0x00 |
| mirror_reg2[2] | 0x0080 | 0x02 | 0x02 | 0x02 | 0x02 |
| mirror_reg2[3] | 0x0a67 | 0x00 | 0x00 | 0x00 | 0x00 |

Only 0x022c, 0x0063 and 0x0a73 depend on `ori`; every other value equals the `.data` default (§6.3), and the M0F0 column
is the `.data` default itself. Pattern [I]: 0x0063 bit 1 and 0x022c bit 0 follow the flip, 0x0063 bits 0 and 2 follow the
mirror, the low two bits of 0x0a73 are the orientation index. The datasheet's table "Mirror and Flip Information" gives
0x0063 / 0x022c[1:0] = 00 / 00 normal, 05 / 00 horizontal mirror, 02 / 01 vertical flip, 05 / 01 mirror + flip (the driver
writes 07 / 01 for the last), first pixel "GR" in every row [S datasheet V1.0 p.14]. What 0x0a73, 0x0a70, 0x05be and 0x0a67 select inside the
sensor is not stated by any source held; the reading of 0x0a73 as an "OTP select" has no source [I]. The registers are sent
at the next frame-active notify (§5.5): mirror_reg1 (9 writes), 20 ms, mirror_reg2 (4 writes). On the running camera
mi_sensor reports pad 0 with mirror = 1, flip = 1 [R camera].

`pCus_GetOrien(handle, *ori)` (`.text+0x0b6`): reads the low byte of the 0x0063 shadow value (`priv+0xb8`) and maps
0 → M0F0, 5 → M1F0, 2 → M0F1, 7 → M1F1; any other value leaves `*ori` untouched; always returns 0. It reports the shadow,
not the sensor (no I2C read).

### 5.10 Limits, shutter info and stubs [R]

- `pCus_GetAEMinMaxUSecs(handle, *min, *max)` (`.text+0x40c`): 30, 200 000 µs; return 0.
- `pCus_GetAEMinMaxGain(handle, *min, *max)` (`.text+0x41c`): `*min = handle->sat_mingain` (1024), `*max = 0xc2000`;
  return 0.
- `gc8613_GetShutterInfo(handle, *info)` (`.text+0x42c`): `info->max = 200 000 000`, `info->min = 14814`,
  `info->step = 14814` (CUS_SHUTTER_INFO `{length, max, min, step}`: `length` at +0 untouched); return 0. [I] units ns.
- `gc8613_SetPatternMode(handle, mode)` (`+0x06c`), `pCus_setCaliData_gain_linearity(handle, arr, n)` (`+0x440`),
  `cus_camsensor_release_handle(handle)` (`+0x444`): return 0, no effect. No test-pattern support (`PatternTbl` is empty,
  §1.2).
- OTP / AWB / AE hooks: none — no callback reads sensor OTP, there is no GetSensorCap, no calibration-data handling, and
  the only sensor reads in the module are the two ID registers of §5.2.

## 6. Register tables

All tables are `I2C_ARRAY {u16 reg; u16 data}` except the gain table [R].

| table | location | entries | bytes | sha256 |
|---|---|---|---|---|
| gain_table | `.rodata+0x000` | 30 × 20 B | 600 | `7a0d475011eed2a6d2dd93321c57e6761764fa9c30b235c741c30ed4594c9469` |
| gain_reg | `.rodata+0x272` | 12 | 48 | `587176b67bd72990b74c16c102124f5c4c1c99777ab40acfa0e213411015d4f9` |
| expo_reg | `.rodata+0x2a2` | 2 | 8 | `e5b96087ded56b8e4a298ca1f2d6b42828f5cf5f1daa5955208cf8443abee356` |
| vts_reg | `.rodata+0x2aa` | 2 | 8 | `589029e1cc3517b5658385671912c050044e874cf646ec423b95b3f6dd59b199` |
| Sensor_init_table | `.rodata+0x2b2` | 175 | 700 | `06b5221612d141a608f8b37f93b6acad0eceaaf13c106aae17bd3d2eeeb6ab82` |
| mirror_reg1 | `.data+0x00` (symbol) | 9 | 36 | `35fb18030c378a7b4f0a82c032425ce001dde3b3a2c7b0983fe2aae7842c337f` |
| mirror_reg2 | `.data+0x24` (symbol) | 4 | 16 | `dec1def634be4fa0748481ef6a37b074496a66d941a4c61fe81febdeb307b1fd` |

The `.rodata` tables have no symbols (static); their names are descriptive [I]. Their bounds follow from the code (loop
counts 30, 12, 2, 2, 175; copy lengths 0x30, 8, 8) and from the strings that sit between them [R]. `.rodata` layout end to
end [R]: gain_table 0x000–0x257, string `"cus_camsensor_init_handle"` 0x258–0x271 (the `__func__` of §4), gain_reg
0x272–0x2a1, expo_reg 0x2a2–0x2a9, vts_reg 0x2aa–0x2b1, Sensor_init_table 0x2b2–0x56d, string `"gc8613_init_driver"`
0x56e–0x580, pad 0x581, `PatternTbl` / `TriggerStartTbl` labels at 0x582 (size 0), parameter names 0x582–0x5c5.

### 6.1 gain_table — `{u32 gain; u16 v[8]}`, gain in 1/1024 [R]

v0…v7 go to 0x0614, 0x0615, 0x0225, 0x1467, 0x1468, 0x00b8, 0x00b9, 0x1447 (§5.8).

| k | gain | v0 v1 v2 v3 v4 v5 v6 v7 | | k | gain | v0 v1 v2 v3 v4 v5 v6 v7 |
|---|---|---|---|---|---|---|
| 0 | 1024 | 00 00 00 07 07 01 00 77 | | 15 | 12214 | 01 00 01 0d 0d 0b 3b 75 |
| 1 | 1171 | 90 02 00 07 07 01 09 77 | | 16 | 14390 | 91 02 01 0e 0e 0e 03 75 |
| 2 | 1431 | 01 00 00 08 08 01 19 77 | | 17 | 16978 | 02 00 01 0f 0f 10 25 75 |
| 3 | 1703 | 91 02 00 08 08 01 2a 77 | | 18 | 20315 | 92 02 01 10 10 13 35 75 |
| 4 | 2014 | 02 00 00 09 09 01 3d 77 | | 19 | 24329 | 03 00 01 11 11 17 30 75 |
| 5 | 2305 | 00 00 00 07 07 02 10 75 | | 20 | 28775 | 93 02 01 13 13 1c 06 75 |
| 6 | 2717 | 90 02 00 07 07 02 29 75 | | 21 | 33905 | 04 00 01 14 14 21 07 75 |
| 7 | 3261 | 01 00 00 08 08 03 0b 75 | | 22 | 40836 | 94 02 01 15 15 27 38 75 |
| 8 | 3825 | 91 02 00 08 08 03 2f 75 | | 23 | 48525 | 05 00 01 17 17 2f 18 75 |
| 9 | 4547 | 02 00 00 08 08 04 1c 75 | | 24 | 57487 | 95 02 01 19 19 38 09 75 |
| 10 | 5401 | 92 02 00 09 09 05 11 75 | | 25 | 67450 | 06 00 01 1a 1a 41 37 75 |
| 11 | 6658 | 03 00 00 0a 0a 06 20 75 | | 26 | 80814 | 96 02 01 1c 1c 4e 3b 75 |
| 12 | 7771 | 93 02 00 0b 0b 07 25 75 | | 27 | 94464 | b6 04 01 1d 1d 5c 10 75 |
| 13 | 8686 | 00 00 01 0c 0c 08 1e 75 | | 28 | 114104 | 86 06 01 1e 1e 6f 1b 75 |
| 14 | 10164 | 90 02 01 0d 0d 09 3b 75 | | 29 | 136110 | 06 08 01 20 20 84 3a 75 |

Patterns [R data; I reading]: v3 = v4 in every row; v7 = 0x77 for k 0–4 and 0x75 from k 5; v2 = 0 for k 0–12 and 1 from
k 13; `(v0, v1)` cycles through 00 00 / 90 02 / 01 00 / 91 02 / 02 00 … (the repeats of §5.8 (a)). The last entry, 136110,
is 0x213ae.

### 6.2 gain_reg, expo_reg, vts_reg — shadow templates [R]

- gain_reg: `031d=2d 0614=00 0615=00 031d=28 0225=00 1467=0e 1468=12 00b8=01 00b9=00 1447=75 0064=01 0065=00`.
  Its data at init (0614 / 0615 = 00 00, 0064 = 01, 0065 = 00) is what GetAEGain reads as 1× before the first SetAEGain;
  its analog part (0x0225, 0x1467, 0x1468, 0x00b8, 0x00b9, 0x1447 = 00 0e 12 01 00 75) is the v2…v7 of no gain_table row.
- expo_reg: `0203=d0 0202=05` (0x05d0 = 1488 lines). vts_reg: `0340=05 0341=dc` (0x05dc = 1500 lines).
  Both values are overwritten by the setters before they are ever sent (§5.6, §5.7).

### 6.3 mirror_reg1, mirror_reg2 — orientation templates (global symbols in `.data`) [R]

- mirror_reg1: `022c=00 0063=00 0a67=80 0a98=04 05be=00 05a9=01 0a70=03 0a73=60 0a5a=80` (= the M0F0 column of §5.9).
- mirror_reg2: `05be=01 0a70=00 0080=02 0a67=00`.
- Relation to the init table: mirror_reg2 equals `Sensor_init_table[167..170]`, and in the init table it also follows the
  20 ms pause (index 165) — the same "first part, 20 ms, second part" shape the orientation flush uses (§5.5). The first
  part of the init table's tail (indices 139–164) contains 0a67=80, 0a98=04, 05be=00, 05a9=01, 0a70=03, 0a73=60, 0a5a=80
  but not 0x022c or 0x0063, and adds further registers (0a54, 0a65, 0089, 0aa0, 0317, 0a82, 0a83, 0a71, 0a72, 0a75 …).

### 6.4 Sensor_init_table — 175 entries, written by `pCus_init` [R]
<!-- ev:sensor-gc8613.6.4 -->

Full content (index of the first entry on the line, then `reg=data` in hex):

```
  0: 03fe=f0 03fe=00 03fe=10 0a38=01 0a20=19 061b=17 061c=50 061d=06
  8: 061e=87 061f=05 0a21=10 0a31=fb 0a34=40 0a35=08 0a37=46 0314=50
 16: 0315=00 031c=ce 0219=47 0342=03 0343=20 0259=08 025a=96 0340=08
 24: 0341=ca 0351=00 0345=02 0347=02 0348=0f 0349=18 034a=08 034b=88
 32: 034f=f0 0094=0f 0095=00 0096=08 0097=70 0099=0c 009b=0c 060c=06
 40: 060e=20 060f=0f 070c=06 070e=20 070f=0f 0087=50 141b=03 0907=d5
 48: 0909=06 0901=0e 0902=0b 0904=08 0908=09 0903=c5 090c=09 0905=10
 56: 0906=00 072a=7c 0724=2b 0727=2b 072b=1c 073e=40 0078=88 0618=01
 64: 1466=12 1468=07 1467=07 0709=40 0719=40 1469=80 146a=c0 146b=03
 72: 1480=02 1481=80 1484=08 1485=c0 1430=80 1407=10 1408=16 1409=03
 80: 1434=04 1447=75 1470=10 1471=13 1438=00 143a=00 024b=02 0245=c7
 88: 025b=07 02bb=77 0612=01 0613=26 0243=66 0087=53 0053=05 0089=02
 96: 0002=eb 005a=0c 0040=83 0075=54 0205=0c 0202=01 0203=27 061a=02
104: 03fe=00 0106=78 0136=00 0181=f0 0185=01 0180=46 0106=38 010d=c0
112: 010e=12 0113=02 0114=03 0100=09 0004=0f 0219=47 0054=98 0076=01
120: 0052=02 021a=10 0430=10 0431=10 0432=10 0433=10 0434=6d 0435=6d
128: 0436=6d 0437=6d 0704=03 0706=02 0716=02 0708=c8 0718=c8 071d=dc
136: 071e=05 031f=01 031f=00 0a67=80 0a54=0e 0a65=10 0a98=04 05be=00
144: 05a9=01 0089=02 0aa0=00 0023=00 0022=00 0025=00 0024=00 0028=0f
152: 0029=18 002a=08 002b=88 0317=1c 0a70=03 0a82=00 0a83=e0 0a71=00
160: 0a72=02 0a73=60 0a75=41 0a70=03 0a5a=80 ffff=14 0089=02 05be=01
168: 0a70=00 0080=02 0a67=00 024b=02 0220=80 0058=00 0059=04
```

Values that tie it to §5 [R]: HTS 0x0342 / 0x0343 = 0x0320 (800), VTS 0x0340 / 0x0341 = 0x08ca (2250), exposure
0x0202 / 0x0203 = 0x0127 (295 lines), 0x0114 = 0x03, 0x0100 = 0x09, one pause `ffff=14` (20 ms). Registers written more
than once [R count]: 0x03fe (×4); 0x0087, 0x0219, 0x0106, 0x031f, 0x05be, 0x024b (×2 each); 0x0089, 0x0a70 (×3 each).

What the table is [S]: GalaxyCore's 4K30 linear RAW10 setting (HTS 0x0320 / VTS 0x08ca,
14.81 µs row time, 24 MHz MCLK, 4 lanes). The binary does not give the meaning of single registers; from public drivers
[S]: 0x0100 = 0x09 is "streaming" (0x00 standby; `rk_gc8613.c`), 0x0340 / 0x0341 are VTS, 0x0342 / 0x0343
HTS, 0x0202 / 0x0203 the (long) exposure. 0x0114 = 0x03 is the 4-lane setting [I]. The
stream-on is thus inside the table (index 115, followed by 59 more entries incl. the 20 ms pause) and `pCus_init` has no
separate stream-on step [R position; I meaning]. The datasheet's abridged register list (§8.3) names the analog gain
0x0614 / 0x0615 / 0x0225, the column gain 0x00b8 / 0x00b9, the pre-gain 0x0064 / 0x0065 and the readout direction 0x0063 /
0x022c that the driver writes at runtime [S]. The meaning of the other registers it writes — in the init table and at
runtime (0x031d, 0x1467 / 0x1468, 0x1447, 0x0a73, 0x0a70, 0x05be, 0x0a67, 0x0a98, 0x05a9, 0x0a5a, 0x0080) — is not known: no
public document names them.

## 7. Strings [R]

`.rodata.str1.1` (0x174 B), every string and its user:

| off | string | used by |
|---|---|---|
| 0x000 | `[%s] Private data is empty!\n` | init_handle, `CamOsPrintf` |
| 0x01d | `gc8613_MIPI` | init_handle, `strcpy` to `model_id` |
| 0x029 | `===================9/20=============\n` | SetVideoRes, `printk` (no level prefix) |
| 0x04f | `\0016Connect %s linear to sensor pad %d\n` | init_module, `printk` (KERN_INFO) |
| 0x075 | build string (§1.1) | default of `sensor_module_version` |
| 0x15e | `use default parameter` | default of `mclk` |

The two `%s` arguments and the parameter names are `.rodata` strings (layout in §6). `.modinfo` strings: §1.1, §2. [I] "9/20" in the SetVideoRes banner matches the 2025-09-20 build
date.

## 8. The GC8613 chip — what public sources state

The driver holds register values only (§6); what the chip is and what its registers mean comes from GalaxyCore's documents
and from other platforms' drivers. Everything in this section is [S] — a datasheet, a vendor statement or public driver
code — unless tagged [I]. Register-list lengths and table diffs are our counts over those public tables. The camera's
sensor is the YN variant (§5.2).

### 8.1 The part
<!-- ev:sensor-gc8613.8.1 -->
- 3840 H × 2160 V active array, on-chip 10-bit ADC; 1.5 µm BSI pixel, 1/2.7" optical format, 55 nm process; rolling
  shutter; raw Bayer over MIPI CSI-2, RGGB (Rockchip `MEDIA_BUS_FMT_SRGGB10_1X10`; the driver's Bayer id `CUS_BAYER_RG`,
  §4.1); DAG single-frame HDR and staggered two-frame HDR; "60fps@全尺寸" (60 fps at full size); GalaxyCore: "GC8613支持多种
  分辨率，最高可输出4k 60fps" (several resolutions, up to 4K60); announced 2023-06-07 [16rd sensor page, asmag 2023-06, gcoreinc.com news detail-84 (Wayback 2023-07-24)].
- Supplies AVDD28 2.7–2.9 V (2.8 typ), DVDD 1.15–1.25 V (1.2), IOVDD 1.7–1.9 V (1.8); CSP package [16rd; Rockchip supply
  names `avdd`, `dvdd`, `dovdd`].
- Variants: **YA** (0x03f2 = 0x23) and **YN** (0x03); Rockchip and thingino keep separate tables per variant; 10-bit and
  14-bit variants also exist [statement of an FPV-community driver author].
- Documents: "SHSAE-8M-3001 GC8613-C43YA CSP Datasheet V1.0" — the YA variant, 33 pages, file dated 2022-11-14 (§8.3);
  "SHSAE-8M-3002 GC8613 CSP Assembly Guide V1.0" (reference circuit, "MIPI 4 通道典型应用电路"); a V1.3 for "GC8613-C43YN"
  dated 2023-11-30 is known only from a third-party transcription (pdapply.com).
- Datasheet V1.0 figures: frame rate "60fps@Full Size"; power 240 mW at full size, 30 fps, "TBD mW @60fps"; features PLL,
  frame sync (master / slave), windowing, mirror and flip, binning, OTP; sensitivity, dynamic range and SNR "TBD".
- V1.3 per the transcription: sensitivity 1.74 V/lux·s, dynamic range 73 dB, max SNR 39 dB, dark current 49 e⁻/s at 80 °C;
  Table 5 currents IAVDD / IDVDD / IIOVDD: 4K30 51.37 / 72.07 / 6.20 mA (≈ 240 mW), 4K60 60.20 / 99.51 / 7.70 mA
  (≈ 302 mW), 1920×1080@120 56.87 / 86.10 / 7.21 mA (≈ 275 mW); the `I2C_ID_SEL` pin selects 8-bit address 0x62 / 0x63 or
  0x20 / 0x21; I2C up to 400 kHz.

### 8.2 Interface
- I2C: 16-bit register address, 8-bit data (Ingenic `TISP_SBUS_MASK_ADDR_16BITS | …_SAMPLE_8BITS`). 7-bit address 0x31 (8-bit
  0x62) in Rockchip, thingino and the RV1126B EVB device tree; 0x10 (8-bit 0x20, this driver's address, §4.1) in an Ingenic
  header; Aimorelogy lists 0x31 and 0x7f.
- Chip id 0x03f0 / 0x03f1 = 0x86 / 0x13, variant byte 0x03f2 (§5.2).
- MCLK 24 MHz (Rockchip; Aimorelogy 8M60 / WDR) or 27 MHz (Ingenic, XMedia, Aimorelogy linear); thingino's clock code
  accepts 24, 27 and 37.125 MHz.
- Control pins reset and pwdn (RV1126B EVB: `reset-gpios`, `pwdn-gpios`; its -dv board drops pwdn and uses reset active-low).
- 4 lanes (Rockchip, XMedia, Aimorelogy, the SigmaStar drivers) or 2 lanes (Ingenic T41 / T32, XMedia 2-lane); the RV1126B
  EVB wires `data-lanes = <1 2 3 4>` with `CLK_MIPI0_OUT2IO` as the clock.

### 8.3 Registers named in datasheet V1.0 [S datasheet V1.0 §7, §8]

| register | name / meaning | default |
|---|---|---|
| 0x03f0 / 0x03f1 | Sensor_ID high / low (RO) | 0x86 / 0x13 |
| 0x0202 / 0x0203 | Exposure[13:8] (bits 5:0) / [7:0]; set frame length before the shutter | 0x00 / 0x10 |
| 0x0342 / 0x0343 | CISCTL_hb (line length) [7:0] / [11:8]; "internal set, not recommended to be modified" | 0x80 / 0x07 |
| 0x0346 / 0x0347 | CISCTL_row_start [10:8] / [7:0] | 0x00 / 0x02 |
| 0x060c / 0x070c | CISCTL_col_start [10:8] / [7:0] (the datasheet's §7.6 calls them left / right) | 0x00 / 0x00 |
| 0x034a / 0x034b | CISCTL_win_height [11:8] / [7:0] | 0x05 / 0x18 |
| 0x0348 / 0x0349 | CISCTL_win_width [10:8] / [7:0] | 0x09 / 0x0c |
| 0x0340 / 0x0341 | Framelength [13:8] / [7:0], in lines | 0x07 / 0x0d |
| 0x029d | min_vb | 0x10 |
| 0x0213 / 0x0214 | CISCTL_vs_st / vs_et | 0x18 / 0x04 |
| 0x0218 bit 4 / 0x0077 bit 3 | row binning enable / column binning enable | — |
| 0x0100 | bit 3 Lane_Ena, bit 2 ULP_Ena, bit 1 Line_sync_mode, bit 0 MIPI_Ena (0x09 streaming, 0x00 standby in Rockchip) | 0x20 |
| 0x0180–0x0186 | DPHY_analog_mode1–7 (mipi_en, lane enables, LP drive, HS phase, clock lane) | 0x06, 0, 0, 0x05, 0x05, 0, 0x53 |
| 0x0111 | LDI_set: MIPI data type (bits 7:6 virtual channel) | 0x2b |
| 0x010d / 0x010e | LWC_set: line word count [7:0] / [15:8] (bytes per line; 0x12c0 = 4800 = 3840 px RAW10 in §6.4) | 0x40 / 0x0b |
| 0x0114 | SYNC_set (public tables write 0x03 in 4-lane, 0x01 in 2-lane modes) | 0xb8 |
| 0x0115 | DPHY_mode (lane gating, all-lane-open, MSB switch, clock-lane mode) | 0x10 |
| 0x0116 | LP_set | 0x29 |
| 0x0120–0x012b | MIPI timing: T_init, T_LPX, T_CLK_HS_PREPARE, T_CLK_zero, T_CLK_PRE, T_CLK_POST, T_CLK_TRAIL, T_HS_exit, T_wakeup, T_HS_PREPARE, T_HS_Zero, T_HS_TRAIL | 0x80, 0x10, 0x05, 0x20, 0x02, 0x20, 0x08, 0x10, 0xa0, 0x06, 0x0a, 0x08 |
| 0x008c | test image (bit 2) | 0x10 |
| 0x0094 / 0x0095 | out_win_width [11:8] / [7:0], a multiple of 8 in RAW10 | 0x0b / 0x90 |
| 0x0096 / 0x0097 | out_win_height [10:8] / [7:0] | 0x06 / 0x82 |
| 0x0098 / 0x0099, 0x009a / 0x009b | out_win_y1, out_win_x1 (output window offsets) | 0 |
| 0x009c | Win_mode | 0x01 |
| 0x008e | Out_win_offset (bits 3:2 for auto up-down, 1:0 for auto mirror) | 0x05 |
| 0x0060 | WB_offset (OB offset) | 0x40 |
| 0x0064 / 0x0065 | auto_pregain [9:6] in bits 3:0 / [5:0] in bits 7:2 | 0x01 / 0x00 |
| 0x0614 / 0x0615 / 0x0225 | analog PGA gain [7:0] / [11:8] in bits 3:0 / [12] in bit 0 | 0 |
| 0x00b8 / 0x00b9 | col_gain [11:6] / [5:0], bits 5:0 each | 0 |
| 0x0063 / 0x022c[1:0] | readout direction (table in §5.9) | — |

The bit layouts of 0x0100 and 0x0180 are read from the PDF's text extraction, whose columns are shifted [I].

Rules stated in the text: the 3840×2160 array "can be windowed by output size control … it will affect field angle"
(§7.6); "GC8613 has Binning mode which support a lower resolution output with high frame rate. The row or col can be
independent controlled. However, only the row binning can increase frame rate" (§7.8); minimum frame length = window
height + TBD + VB, VB_min = 0; when the shutter exceeds the minimum frame length the frame grows to shutter + 8; frame
blank = frame length − output-window height (§7.9). The power-on defaults (win 2316 × 1304, frame length 1805) belong to
no public mode [I]. The datasheet has no PLL register table, no line-time formula and no mode tables; these exist only in
GalaxyCore's setting files (§8.4).

### 8.4 Timing in the public tables [values S; arithmetic I]
The line time is HTS × 4 / wpclk in every table whose vendor header states a row time:

| table | HTS | wpclk | line | VTS | frame rate |
|---|---|---|---|---|---|
| Rockchip YA linear 4K30 (= this driver's table, §6.4) | 0x0320 = 800 | 216 MHz | 14.815 µs | 2250 | 30.00 |
| Rockchip YA DAG 4K30 | 0x0310 = 784 | 216 MHz | 14.519 µs | 2296 | 30.00 |
| Rockchip YA 4K40 | 0x03b4 = 948 | 351 MHz | 10.803 µs | 2314 | 40.00 |
| Rockchip YA HDR_X2 | 0x0278 = 632 | 351 MHz | 7.202 µs (14.404 per HDR row pair) | 2314 | 30 |
| Aimorelogy "8M60" | 0x0278 = 632 | 351 MHz | 7.202 µs | 2322 | 59.80 |

- The 4K40, HDR_X2 and 8M60 tables share the PLL / MIPI registers (0x061c–0x061f, 0x0a31 / 0x0a37, 0x0122–0x012b) at
  1458 Mbps per lane; the 4K30 header reads "mclk 24MHz, mipiclk 1004Mbps, wpclk 216MHz, rpclk 200.8MHz, rowtime 14.81us,
  vts 2250" (Rockchip's link-frequency menu says 729 MHz). 27 MHz-MCLK tables differ from 24 MHz ones in 0x061e (0x78 vs
  0x87, the ratio 24 / 27) and 0x0a31. HiSilicon developer comments confirm the scaling: "0x320=800 30fps 0x4b0=1200 20fps",
  "0x8ca=2250 30fps 0xd2f=3375 20fps", HDR "0x3b4=948 20fps 0x278=632 30fps".
- Every public table reads the array window from (2, 2), 3864 × 2184 (3856 in the 1080p table); none shrinks it — smaller
  modes crop the output window or decimate.
- GalaxyCore setting files are multi-mode text files; driver comments copy their headers: `version v2.2 / <MODE_2
  type="MODE_2_GC8613_3840*2160_30fps_raw10_4lane_1000_YA002">`, `<MODE_8 type="…raw12_4lane_DAG_1001_YA002">`, `<MODE_2
  type="0A_GC8613_YA_MIPI4L_24M_3840x2160_30fps_raw10_HDR">`, `version 1.6 … <MODE_12 …YN002>`, `<MODE_20 …DAG_1001_YN002>`;
  single-mode release files are named like `release_liner_v2.2.1_04_4lane_raw10_3840x2160_40fps_24mhz_GC8613YA.txt`. The YN
  file numbers its modes up to at least 20, public drivers carry one to four [I].

### 8.5 Modes in public drivers
<!-- ev:sensor-gc8613.8.5 -->

| source | mode | variant | lanes, rate | HTS / VTS | note |
|---|---|---|---|---|---|
| Rockchip `drivers/media/i2c/gc8613.c` (develop-6.1) | 3840×2160@30 linear RAW10 | YA, YN | 4, 1004 Mbps | 0x0320 / 0x08ca | the YN table as shipped writes VTS 0x1194 (15 fps), its 30 fps value commented out |
| same | 3840×2160@40 linear | YA | 4, 1458 Mbps | 0x03b4 / 0x090a | |
| same | 3840×2160@30 DAG RAW12 | YA, YN | 4, 1188 Mbps | 0x0310 / 0x08f8 | single-frame HDR; a 15 fps VTS 0x11f0 commented out |
| same | 3840×2160@30 HDR_X2 RAW10 | YA | 4, 1458 Mbps | 0x0278 / 0x090a | two-frame HDR |
| Aimorelogy / CVITEK SDK `gcore_gc8613` | 8M30 (3848×2168 sensor, 3840×2160 window), 8M25 (VTS 0x0a8c), 8M30 WDR | YA [I] | 4 | 0x0320 / 0x08ca; WDR 0x0278 / 0x090a | MCLK 27 MHz (8M30, 8M25, 2M30), 24 MHz (8M60, WDR) |
| same | "8M60" | YA [I] | 4, 1458 Mbps | 0x0278 / 0x0912 | mode table `f32MaxFps = 60`, AE `VMAX = VtsDef × MaxFps / fps`; init comment "24M 60fps setting changed vts(0x340 0x341) to output 30fps to increase vblanking time(17ms)" |
| same | "2M30" | YA [I] | 4 | 0x0320 / 0x08ca | outputs 2560×1440 (window 0x0a00 × 0x05a0) — an output crop, no binning register changed |
| HiSilicon Hi3516CV610 (gitee shumjj/3516cv610_app) | 1920×1080, full field of view; header "00_GC8613_YN_MIPI2L_27M_1920x1080_30fps_raw10_linear", v1.6, "rowtime 27.78us, vts 1200" | YN | 2, 1003.5 Mbps | vendor 0x05dc / 0x04b0 (committed HTS 0x08ca → 20 fps) | full-array read, 1920-px lines (word count 2400 B); writes 0x0218 = 0x10 and 0x0077 = 0x08 (row and column binning, §8.3), 0x0902 0x1b, 0x060c / 0x070c 0x0a |
| same | 3840×2160@20 linear (header "06_…YN_MIPI4L_27M_…20fps", rowtime 14.81 µs) and 3840×2160@20 HDR 2-to-1 (1458 Mbps) | YN / — | 4 | developer-edited | |
| Ingenic T41 (OpenIPC openingenic, thingino), T32 (thingino) | 3840×2160 @15 / 20 / 25 / 30 | YN (`gc8613.c`), YA (`gc8613a.c`) | 2, 1100–1584 Mbps | VTS 2250 and others | 27 MHz (T32 YA 24 MHz); versions H20230427a, H20250211a |
| OpenIPC openhisilicon gk7206v100 (XMedia API) | 3840×2160@30, 4-lane (VMAX 0x8f8, data width declared 14-bit) and 2-lane (VMAX 0x8ca) | — | 4 / 2 | | 8-bit I2C address 0x62 |
| Novatek NT98560 `sen_gc8613.c` | 3840×2160@30 | — | 4, 792 Mbps | VTS 2250, row 14.8 µs | 27 MHz |
| SigmaStar I6C (Maruko, kernel 5.10) community drivers | `drv_ms_cus_gc8613_MIPI.ko`: 4K30 linear (167 entries); `drv_ms_cus_gc8613_YA_v2.2_MIPI_14bit_20240705.ko`: DAG 4K30 (203 entries) | YA (name) | 4 | 0x0320 / 0x08ca; 0x0310 / 0x08f8 | built 2025-08-22, with debug info; shared in an OpenIPC community chat |
| Realtek Ameba Pro2 `ameba_sensor.h` | `SENSOR_GC8613_FHD = {1920, 1080, 30}` | — | — | — | header entry only |

Not public, stated only: a YA 1920×1080@120 table on RV1126B (`gc8613ya_linear_10bit_1920x1080_120fps_regs`, `hts_def
2656`, `vts_def 1142`; 7.30 µs per line by arithmetic [I]; "可以找sensor厂要" — obtained from the sensor maker; ~120 fps
measured on the MIPI clock) [CSDN yongheng6/156996403]; a binned 1080p90 SigmaStar driver on SSC378QE / SSC338 [FPV-group
author]. Product statements above 30 fps below 4K: Rollei Compactline 10x (NT96660) 1080p60 and 720p120,
AKEEYO AKY-710Pro 2K60 and 1080P60, the 泓嘉 MIPI module AS-AB318A68M4-106 "4K：60 fps … 1080P：120fps".
