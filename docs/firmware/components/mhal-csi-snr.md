# mhal-csi-snr — mhal sensor interface (SNR) and MIPI CSI-2 receiver (CSI)
<!-- ev:mhal-csi-snr -->

Two blocks of `mhal.ko` form the sensor end of the capture front end. **SNR** ("sensorif") sits between the loadable sensor
driver and the MI / ISP side: it owns the per-pad sensor handle, gives the driver its I2C and "ISP-for-sensor"
(`ISensorIfAPI`) tables, forwards every `MHal_SNR_*` call to a driver callback, runs the frame-event AE notify and a stall
monitor. **CSI** is the MIPI CSI-2 receiver HAL (MAC, D-PHY analog, D-PHY digital) for four receivers. This document owns
the sensor ABI — the sensor handle, `ISensorIfAPI` and `i2c_handle_t` as mhal builds and reads them. The vendor GC8613
driver that fills the handle is `sensor-gc8613.md`; the VIF block that SNR calls for pins, MCLK, masks and channel
mapping is `mhal-vif.md` §6, §3.3; the module as a whole (init order, ISRCB, character devices, DT properties per node) is
`mhal-core.md`.

Tags: **[R]** read from the binary, a decompile, a disassembly, the DT or a header; **[S]** stated by a source; **[I]**
inference. A heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mhal.ko` (`/config/modules/4.9.227/mhal.ko` on the camera).

**Image base** [R]: at Ghidra's default base the module occupies 0x10000–0x2xxxxx, the same range as RIU register
constants, which can then show up as pointers. Rebased to 0x40000000 and compared: no register address in these blocks
changed — they address registers as ioremap base + offset, and literal-pool words are resolved from the module's
relocations. The only differences were control-flow artefacts in the three functions named above.

**Register notation.** `BANK.RR[h:l]` = RIU bank (hex), 16-bit register index `RR` (hex), bit field. A driver byte offset
`o` inside an ioremapped 0x200-byte bank is register `o>>2`, bits `8·(o&1)…` (RIU addressing: `kernel-platform.md` §3.1). All
writes in these blocks are byte read-modify-writes of one field unless stated.

## 1. The two blocks [R]

### 1.1 Size, interface, reach

| block | code | functions | exports | reached from |
|---|---|---|---|---|
| SNR | 20 KB | 175 | 66 | the sensor driver (6 registration exports, §2.1), `mi_sensor` (16 `MHal_SNR_*`), `mi_isp` (`MHal_SNR_SwitchSensor` / `GetSwitchSensorId`), mhal's ISP block (the `DrvSensor*` set of §3.9), its own platform driver and `/dev/sensorif` |
| CSI | 17 KB | 220 | none | only (a) the sensor driver through the CSI function pointers SNR puts into `ISensorIfAPI` (§2.3), (b) SNR (`DrvSensorSetCsiClk → Drv_CSISetClk`, `DrvSensorSetHDRMode → Drv_CSISetHDRMode`, `DrvSensorQueryLaneNum → Drv_CSIQueryLaneNum`), (c) its own platform driver (probe, IRQ, debug misc device) |

SNR writes **no register of its own**: pins, reset, power-down and MCLK go to VIF HAL functions, lanes and clocks to CSI,
I2C to the kernel I2C core through CamOs. CSI writes the clock-generator bank 1038 and its receiver banks directly.
Which consumer imports which SNR export, and the call counts between blocks, are in `mhal-core.md` §2.3–§2.4.

### 1.2 Public relatives
<!-- ev:mhal-csi-snr.1.2 -->

- **SNR**: no public source of this build. Mercury5 `drv_SensorIF.o` is an older API (`DrvSensorIF*` names). The
  driver-facing structures match the public `drv_ms_cus_sensor.h` / `sensor_i2c_api.h` (OpenIPC) only in part (§2.5); the VCM handle matches the SSC37x SDK
  `drv_ss_cus_vcm.h` (§3.13).
- **CSI**: Mercury5 (`longyanjun2020/SDK_pulbic`, saved in): `hal_csi.o`, `drv_csi.o`
  with DWARF, register headers `kernel_csi_mac_4lane.h`, `kernel_csi.h`, `kernel_dphy_{ana,dig}[_4lane].h`,
  `kernel_clkgen.h` (and, for the VIF: `hal_vif.o` with DWARF, `kernel_vif.h`, `mercury5_reg_vif.h` — `mhal-vif.md` §1).
  The M6 CSI HAL has the **same function names** (`HalCsiMac4Lane_*`, `HalCsiAna4Lane_*`, `HalCsiDig4Lane_*`); the M5
  register layout matches the M6 one for the MAC and the digital PHY; the analog PHY matches up to register 0x16 and
  differs above it (§4.8). Register field names in this document are the M5 names unless stated.
- **Clock generator** (bank 0x1038): mercury6 camclk
  gives the names and parent lists of
  the clock fields of bank 1038 — it confirms the CSI and MCLK clock registers of §2.3 and §4.3.
- **Infinity2** (`jockyw2001/sdk mhal/i2/vif`, 2018): source of the older VIF driver / HAL (`drv_vif.c`, `hal_vif.c`,
  `mhal_vif_datatype.h`) and `mhal.o` with DWARF — the same HAL function family (`HalVif_Crop`, `HalVif_SensorMask`, …)
  with an older MHal API (dev / chn / sub-chn instead of the M6 group / dev / pipe / port). CSI and sensorif were outside
  mhal on I2.

### 1.3 Platform drivers and device nodes

Both platform drivers are registered by `camdriver_init` (steps 2 and 4, `mhal-core.md` §3.2), not by their initcall
entries. The character devices and their liveness are listed in `mhal-core.md` §4.2; the DT properties each probe reads
are in `mhal-core.md` §7, with their values on this camera in §2.4 (sensorif) and §4.1 (csi) below.

- `mstar_sensorif_probe` (DT `compatible = "sstar,sensorif"`): `misc_register("sensorif")` → `/dev/sensorif`; char device
  `sensorif0` (a CamDevice) with a sysfs file `info`; DT `sensorif_grpN_i2c` → `gSensorI2CMaster[N]` via
  `HalSensorSetI2CMaster`; `DrvSensorifInitCommon` = start the work-queue thread (§3.7); `DrvSensorCtxInit` zeroes the
  0x1d0-byte context; `DrvSensorSkipframeInit` zeroes the 32-byte skip-frame state.
- `csi_probe` (DT `compatible = "sstar,csi"`): banks, DT lane tables, misc device `csi`, device `csi0`, IRQ (§4.1).
- `ss_sensor_earlyinit_probe` (platform "sstar,sensor_earlyinit") is registered only from `.initcall4.init`, which a module
  never runs, so it never runs and `/dev/sensor_earlyinit` does not exist on this build [R code + I loader; the DT has no
  `sensor_earlyinit` node either].

## 2. Sensor ABI — what the driver and SNR exchange

### 2.1 Version handshake and registration exports [R]
<!-- ev:mhal-csi-snr.2.1 -->

A sensor driver's `init_module` calls the three version checks, allocates its private data and registers its
init-handle function; the vendor GC8613 driver (`gc8613_MIPI.ko`, module name `drv_ms_cus_gc8613_MIPI`) imports exactly the
six exports below (`sensor-gc8613.md` §3).

| export | behaviour |
|---|---|
| `DrvSensorHandleVer(maj, min)` / `DrvSensorIFVer` / `DrvSensorI2CVer` | major must be 2 / 2 / 1, else −1; a different minor (≠ 2 / 1 / 1) only logs (if logging is on) and returns 0. The version numbers do not prove the layout: builds with different handle layouts pass the same 2.2 / 2.1 / 1.1 handshake (§2.5) |
| `DrvRegisterSensorDriverEx(pad, init_fn, priv)` | pad < 4 and the slot free, else −1. `memset(handle, 0, 0xa1c)`; ctx+0x00 = `init_fn`; mutex init; zero the I2C table; `handle+0x88 = pad`, `+0x28 = priv`, `+0x40 = &i2c table`, `+0x44 = &IF table`; fill the IF table (§2.3); **call `init_fn(handle)`** (the driver fills its fields and callbacks); ctx+0x18 = handle, plane mode 0; `DrvSensorI2CAttach(pad, i2c table)` (§2.4). The handle is `g_SensorHandle + 0xa1c·pad` (`movw r2,#2588` right before the `mla`) |
| `DrvRegisterSensorI2CSlaveID(pad, id)` | id ≠ 0 and a handle registered → `handle+0x3c` (u16 `i2c_cfg.address`) = id; id 0 → no-op |
| `DrvSensorRelease(pad)` | destroy the mutex, clear `handle+0x40`, the handle pointers and the init function, plane mode 0 |

`DrvRegisterSensorDriver(pad, fn)` = `DrvRegisterSensorDriverEx(pad, fn, 0)`. Plane (HDR) drivers register through
`DrvRegisterPlaneDriver[Ex]` / `DrvRegisterPlaneI2CSlaveID` (§3.11), lens motors through `DrvRegisterVcmDriver` (§3.13);
of the camera's modules only the out-of-scope `imx307_MIPI.ko` imports the plane functions [R, nm], and none registers a
VCM. Symbol addresses in this mhal: `DrvRegisterSensorDriverEx` 0xabe44, `DrvSensorGetFPS` 0xad268, `DrvSensorHandleVer`
0xad0f8, `DrvSensorIFVer` 0xad140, `DrvSensorI2CVer` 0xad188 [R].

### 2.2 The sensor handle (`ms_cus_sensor`, 0xa1c = 2588 bytes)

One handle per (slot, pad) in `g_SensorHandle` (§3.1). Offsets are [R]: the ones mhal touches from the mhal code,
the rest from the stores of the vendor GC8613 driver (`sensor-gc8613.md` §4); our sensor driver's header pins them with
`_Static_assert`s. Field names are those of the public `drv_ms_cus_sensor.h` / the I6C header [S]; that a
field at an offset is the field of that name is [I] where neither side reads it.

| offset | field | mhal writes / reads |
|---|---|---|
| 0x000 | `version` | — |
| 0x004 | `model_id[32]` | [I] the sensor name `DrvSensorGetPlaneInfo` returns |
| 0x024 | `sen_handle` | — |
| 0x028 | `private_data` | written: `priv` of `DrvRegisterSensorDriverEx` |
| 0x02c | (one word, untouched) | — |
| 0x030 | `i2c_cfg` (16 B `app_i2c_cfg`: +0 mode, +4 fmt, +8 speed, +0xc u16 8-bit address) | written: address by `DrvRegisterSensorI2CSlaveID`; read by the I2C functions (§2.4) |
| 0x040 | `i2c_bus` (pointer to the 48-byte `i2c_handle_t`) | written at registration |
| 0x044 | `sensor_if_api` (pointer to the 68-byte `ISensorIfAPI`) | written at registration |
| 0x048–0x054 | `ae_gain_delay`, `ae_shutter_delay`, `ae_gain_ctrl_num`, `ae_shutter_ctrl_num` | read (`GetPlaneInfo`: AE delays) |
| 0x058–0x068 | `reset_POLARITY`, `pwdn_POLARITY`, `VSYNC_POLARITY`, `HSYNC_POLARITY`, `PCLK_POLARITY` | read (`GetInfo`) |
| 0x06c | `isp_type` | — |
| 0x070 | `bayer_id` | read (`GetPlaneInfo`) |
| 0x074 | `RGBIR_id` | read (`GetPlaneInfo`: 0 → 8) |
| 0x078 | `sif_bus` (`CUS_SENIF_BUS`) | read (`GetInfo`, `DrvSensorInit`) |
| 0x07c | `data_prec` | read (`GetPlaneInfo`: precision) |
| 0x080 | `data_mode` | — |
| 0x084 | `orient` | — |
| 0x088 | pad (`snr_pad_group` in the public numbering) | written: pad at registration and by `DrvSensorAeNotify` |
| 0x08c | (untouched word) | — |
| 0x090 | `video_res_supported`: +0 `num_res`, +4 `ulcur_res`, +8 `res[31]` × 72 B (`cus_camsensor_res`: width, height, …, crop start, output size, `strResDesc[32]` at +32, `nMinFrameLengthLine` +64, `nRowTime` +68) | read through the driver's resolution callbacks |
| 0x950 | `interface_attr` (44-byte union; `attr_mipi`: `mipi_lane_num`, `mipi_data_format`, `mipi_yuv_order`, `mipi_hsync_mode`, `mipi_sampling_delay`, `mipi_hdr_mode`, `mipi_hdr_virtual_channel_num`) | read (`GetInfo`: MIPI attributes +0x950…+0x960, HDR) |
| 0x97c | `mclk` (`CUS_MCLK_FREQ`) | read (`GetInfo`) |
| 0x980 | `sat_mingain` | — |
| 0x984, 0x988 | (untouched words) | — |
| 0x98c | `pCus_sensor_poweron(handle, pad)` | called by `DrvSensorInit` |
| 0x990 | `pCus_sensor_poweroff(handle, pad)` | called by `DrvSensorDeInit` |
| 0x994 | `pCus_sensor_init(handle)` | called by `DrvSensorInit` |
| 0x998 | `pCus_sensor_post_init(handle, pad)` | called by `DrvSensorInit` |
| 0x99c | `pCus_sensor_release` | — |
| 0x9a0, 0x9a4 | `suspend`, `resume` | — |
| 0x9a8 | `SetPatternMode` | — |
| 0x9ac | `GetSensorID(handle, *id)` | `DrvSensorGetID` |
| 0x9b0 / 0x9b4 / 0x9b8 | `GetVideoRes` / `GetCurVideoRes` / `SetVideoRes` | `MHal_SNR_GetRes` / `GetCurRes` / `SetRes` |
| 0x9bc / 0x9c0 | `GetOrien` / `SetOrien` | `MHal_SNR_GetOrien` / `SetOrien` |
| 0x9c4 | `GetSensorCap` | — |
| 0x9c8 | `AEStatusNotify(handle, status)` | `DrvSensorAeNotify` (§3.6) |
| 0x9cc / 0x9d0 | `GetAEUSecs` / `SetAEUSecs` | `DrvSensorGetUs` / `SetUs`; `GetPlaneInfo` |
| 0x9d4 / 0x9d8 | `GetAEGain` / `SetAEGain` | `DrvSensorGetGain` / `SetGain`; `GetPlaneInfo` |
| 0x9dc / 0x9e0 | `GetAEMinMaxUSecs` / `GetAEMinMaxGain` | `DrvSensorGetAEMinMaxGain` (0x9e0) |
| 0x9e4 / 0x9e8 | `GetFPS` / `SetFPS` | `MHal_SNR_GetFPS` / `SetFPS` |
| 0x9ec | `SetAEGain_cal` | — |
| 0x9f0 | `setCaliData_gain_linearity` | — |
| 0x9f4 | `GetShutterInfo` | `DrvSensorGetShutterInfo` |
| 0x9f8 | `GetVideoResNum` | `MHal_SNR_QueryResCount` |
| 0x9fc | `CustDefineFunction` | `DrvSensorCustFunction` |
| 0xa00 | `GetSrcType` [S I6C name] | — |
| 0xa04 / 0xa08 | get / set analog-decoder source attribute | `DrvSensorGet/SetAnadecSrcAttr` (§3.12) |
| 0xa0c / 0xa10 | `TryAEGain` / `TryAEShutter` | `DrvSensorTryGain` / `DrvSensorTryShutter` |
| 0xa14 / 0xa18 | switch sensor id / get current switch sensor id | `DrvSensorSwitchId` / `DrvSensorGetCurSwitchId` (§3.12) |

The slot names of 0x9fc–0xa18 are the I6C header's (`CustDefineFunction`, `GetSrcType`, `GetSrcAttr`, `SetSrcAttr`,
`TryAEGain`, `TryAEShutter`, `SwitchgSensorId`, `GetCurSwtichSensorId`) [S]; the uses are [R]. A NULL callback is not an
error for SNR (§3.2) except `GetSensorID` (−2).

### 2.3 `ISensorIfAPI` — the functions SNR gives the driver (68 bytes) [R]

Per (slot, pad) at `gDrvSensorCtx + 0x590 + slot·0x110 + pad·0x44` (§3.1). The registration fills 13 function pointers;
+0x00, +0x04 and +0x2c are not written (+0x00 / +0x04 are `version` / `pdata` in the header). The table offsets are those
of the public ssd_lcd `ISensorIfAPI` (PowerOff 8 … SetSkipFrame 60), not those of I6C (SetIOPad at 20) or of the
tiramisu user-space header (SetIOPad at 24). Register effects are for pad 0; "§4.x" points into the CSI part.

| off | slot (driver name) | mhal function | what reaches hardware (pad 0) |
|---|---|---|---|
| 0x08 | PowerOff(idx, v) | `DrvSensorPdwn` → `DrvVif_SensorPdwn` | `1304.00[3]` (M5 `vif_ch0_sensor_pwrdn`) = **(v == 0)**; channel = `HalVif_GetChnBySNRPadID` (pad 0 → 0, 1 → 8, 2 → 4, 3 → 12; only channels 0/4/8/12 have the pin, banks 1304–1307). Returns −1 ("skip pdwn") while early init is enabled (§3.14) |
| 0x0c | Reset(idx, v) | `DrvSensorReset` → `DrvVif_SensorReset` | `1304.00[2]` (`vif_ch0_sensor_rst`) = **(v == 0)**; same skip rule |
| 0x10 | MCLK(idx, on, freq) | `DrvSensorSetMCLK` → `DrvVif_SetMclk` → `HalVif_SetMCLK` | `CUS_MCLK_FREQ` → kHz (0 27000, 1 21600, 2 12000, 3 5400, 4 36000, 5 54000, 6 43200, 7 61700, 8 72000, 9 48000, **10 24000**, 11/12 37125) → clkgen source index (27000 0, 72000 1, 61700 2, 54000 3, 48000 4, 43200 5, 36000 6, **24000 7**, 21600 8, 12000 9, 5400 10, 37125 11). Writes `1038.62[3:0] = 0`, then `1038.62[13:8] = sel<<2 \| !on` (bit 9 = invert, forced 0; pad 1 `1038.65[13:8]`, pad 2 `1038.65[5:0]`, pad 3 `1038.66[13:8]`). 24 MHz on: `1038.62[13:8] = 0x1c` (camclk `sr00_mclk` parent 7 = `xtali_24m`); off: `0x1d` |
| 0x14 | QueryMCLK(idx, freq) | `DrvSensorQueryMCLK` → `HalVif_QueryMCLK` | none (37.125 MHz support flag from DT `vif_srN_mclk_37p125`) |
| 0x18 | QueryLaneNum(idx, *n) | `DrvSensorQueryLaneNum` → `Drv_CSIQueryLaneNum` | none; returns DT `csi_srN_lane_num` (§4.1) |
| 0x1c | SetIOPad(idx, bus, lanes) | `DrvVif_SetIOPad` (VIF) | `101e.50[15] = 0` (all-pad-in off), then for MIPI on pad 0: `103c.69[2:0]` = DT `vif_sr0_mipi_mode` (1), `103c.69[9:8]` = `…_pdn_mode` (0), `103c.69[7]` = `…_rst_mode` (0), `103c.69[5:4]` = `…_ctrl_mode` (1), `103c.69[6]` = `…_mclk_mode` (0); then `HalVifDataIeConfig(0, 0)`: while `103c.69[2:0] == 1`, bits 4 and 6 of `103e.1d` … `103e.26` = 0 [I: input enable of the parallel-sensor pads that share the balls]. `lanes` is not used |
| 0x20 | SetCSI_Clk(idx, clk) | `DrvSensorSetCsiClk` → `Drv_CSISetClk` | §4.2, §4.3 |
| 0x24 | SetCSI_Lane(idx, n, on) | `Drv_CSISetLane` | §4.4 |
| 0x28 | SetCSI_LongPacketType(idx, a, b, c) | `Drv_CSISetLongPacket` | §4.5 |
| 0x30 | csi clk / data skip | `Drv_CSISetClkDataSkip` | HS settle, §4.2 |
| 0x34 | hdr mode | `DrvSensorSetHDRMode` → `Drv_CSISetHDRMode` | HDR sensors only (§3.11, §4.2) |
| 0x38 | YUV order swap | `Drv_CSISetYUVOrderSwap` | YUV sensors only (§4.2) |
| 0x3c | SetSkipFrame(idx, fps, n) | `DrvSensorSetSkipFrame` | VIF input mask for n frames (§3.10) |

`i2c_handle_t` (48 bytes) and the transfers behind it: §2.4.

### 2.4 I2C: `i2c_handle_t`, bus selection, transfers [R]

**Bus per pad.** DT `sensorif_grpN_i2c` → `gSensorI2CMaster[N]` (u8[4], .data) at probe; this camera's DT: grp0 → I2C
bus 1, grp1 → 2, grp2 → 1, grp3 → 2. `HalSensorSetI2CMaster(pad, bus)` / `HalSensorGetI2CMaster(pad)` write / read the
array (callers: probe, `DrvSensorI2CAttach`).

**`i2c_handle_t`** (48 bytes; per (slot, pad) at `gDrvSensorCtx + 0x1d0 + slot·0xc0 + pad·0x30`, §3.1), filled by
`DrvSensorI2CAttach(pad, tbl)` with the function family of bus b = `gSensorI2CMaster[pad]` (b 0..3; another value leaves
the table untouched): `+0x08 = pad`; open = `DrvSensorI2COpen<b>`, close = `DrvSensorI2CClose<b>` (both return 0; at
+0x0c / +0x10 by the header [I]); `+0x14 i2c_tx` = `Write<b>`, `+0x18 i2c_rx` = `Read<b>`, `+0x1c i2c_array_tx` =
`ArrayWrite<b>`, `+0x20 burst_tx` = `BurstWrite<b>`, `+0x24 burst_rx` = `BurstRead<b>`, `+0x28 array_rx` =
`ArrayRead<b>` (names `DrvSensorI2C*<b>`). The header's slot names (`i2c_open`, `i2c_close`, …, a trailing
`i2c_array_rw`) are [S].
- `Write<b>(h, cfg, reg, data)` = `WriteRegisterPair(b, cfg->address, reg, data, cfg->fmt)`; `Read<b>(h, cfg, reg,
  *data)` = `ReadRegisterPair(…)`. `cfg` is the handle's `i2c_cfg` (+4 fmt, +0xc u16 8-bit address).
- `ArrayWrite<b>(h, cfg, arr, n)` / `ArrayRead<b>` loop over `{u16 reg, u16 data}` pairs (`I2C_ARRAY`) and stop at the
  first non-zero result.
- `BurstWrite<b>(h, cfg, {u16 reg, u16 *data}, n)` = `WriteRegisterBurst(b, address, reg, data, n, fmt)`; `BurstRead<b>` =
  `ReadRegisterBurst`.

**Transfers** (`fmt` = `ISP_I2C_FMT`: 0 A8D8, 1 A16D8, 2 A8D16, 3 A16D16). Each transfer is `CamI2cOpen(bus)`, one
`CamI2cTransfer` (`i2c_transfer`) with the 7-bit slave address `addr8 >> 1`, `CamI2cClose`. No retries, no repeated-start
handling beyond the kernel's.
- `WriteRegisterPair(bus, addr8, reg, data, fmt)`: one write of 2 / 3 / 3 / 4 bytes for fmt 0 / 1 / 2 / 3, register and
  data big-endian (A16D8 = `reg_hi reg_lo data`); another fmt → returns 0 without a transfer; failure → "i2c-%d timeout",
  −1.
- `ReadRegisterPair(…, *data, fmt)`: write the 1 (fmt 0 / 2) or 2 (fmt 1 / 3) register bytes, read 1 (fmt 0 / 1) or 2
  (fmt 2 / 3) bytes, in one combined transfer; 16-bit results big-endian.
- `WriteRegisterBurst(bus, addr8, reg, data, n, fmt)`: n ≤ 16 (else nothing, no error); builds register (1 or 2 bytes) + n
  data (fmt 0: n bytes; 1: 2 register bytes + n bytes; 2: 1 register byte + n big-endian u16; 3: 2 register bytes + n
  u16) in a 0x22-byte stack buffer (register word + 0x20 data bytes; fmt 3 with n = 16 fills it exactly) — one write.
- `ReadRegisterBurst(bus, addr8, reg, buf, n, fmt)`: fmt 0 → 1 register byte, 1 → 2 register bytes, then read n bytes;
  another fmt → nothing.

The GC8613 driver uses A16D8 with address 0x20, i.e. I2C bus 1, 7-bit slave 0x10 (`sensor-gc8613.md` §4.1).
The ad-hoc `DrvSensorI2cRead/Write(pad, req)` used by the ISP and the debug commands build a temporary table through
`DrvSensorI2CAttach` (§3.9, §3.15).

### 2.5 Layout families and the public headers
<!-- ev:mhal-csi-snr.2.5 -->

The same SNR code exists in other M6 products with different handle layouts; the camera's mhal is family **A**
(handle 2588 B, `GetFPS` read at `[handle+2532]`, pad-context stride 0x74, 4 pads) [R]:

| family | image (consumer mhal build) | res element × n | `interface_attr` | `mclk` | `poweron` | `GetFPS` | handle size | IF table stride |
|---|---|---|---|---|---|---|---|---|
| A | Anjoy MC-H12S (2021-11-02, ge0ecf5a); **this camera** | 72 × 31 | 2384 | 2428 | 2444 | 2532 | 2588 | 68 |
| B | SIYI A8 mini (2021-11-26, gce127ed) | 72 × 31 | 2384 | 2432 | 2448 | 2536 | 2592 | 64 |
| C | RunCam Split-H (2022-06-06, g7dfcf71) | 76 × 31 | 2508 | 2556 | 2572 | 2660 | 2716 | 64 |

(offsets decimal) [R]. B = A + 4 from `mclk` on; C = A + 124 at `interface_attr` and + 128 from `mclk` on.
The res[] element stride (72 in A / B, 76 in C) is [R]; the count 31 is a strong [I] from the post-array arithmetic
(152 + 31 × 72 = 2384), matching the I6C 2024 header's 31-entry list. All three families pass the same 2.2 / 2.1 / 1.1
version handshake, so a module of the wrong family is not rejected by it [I] and would be misread [I].

The public headers differ from all three [R]: ssd_lcd `ms_cus_sensor` is 1296 B (res 64 B × 15,
`interface_attr` 1112, `ISensorIfAPI` 76 B); I6C 2660 B (res 72 B × 31, `ISensorIfAPI` 116 B with SetIOPad at 20); the
tiramisu user-space header 1132 B with `sensor_if_api` embedded (52 B) and 12 res entries. The prefix checked at 40…148
(`private_data` … `ulcur_res`) is at the ssd_lcd offsets in all families; `i2c_handle_t` is 48 B and `app_i2c_cfg` 16 B
everywhere. Enums (4 bytes, ssd_lcd = I6C numbering): `CUS_SENIF_BUS` PARL 0, MIPI 1, BT601 2, BT656 3, BT1120 4;
`CUS_DATAPRECISION` 8 → 0, 10 → 1, 16 → 2, 12 → 3, 14 → 4; `CUS_SEN_BAYER` RG 0 … GB 3; `CUS_SEN_RGBIR` NONE 0 … I1 8;
`CUS_HDR_MODE` NONE 0, SONY_DOL 1, DCG 2, RAW8…RAW16 3–7, LI 8, MULTI_VC 9; `CUS_MCLK_FREQ` 27M 0 … 24M 10, 37.125M 11;
`CUS_CSI_CLK` DISABLE −1, 108M 0 … 216M 4 …; `CUS_CLK_POL` POS 0 / NEG 1; `ISP_I2C_FMT` as above;
`CUS_CAMSENSOR_AE_STATUS_NOTIFY` `CUS_FRAME_INACTIVE` 0, `CUS_FRAME_ACTIVE` 1.

[S] (SigmaStar «Sensor Porting» page, the same English text in the Tiramisu sets SSC35XG 2022-04 and SSD268
2023-05, written for Macaron / I5 — it names «I5/MACARON Support MCLK List»): the handle and IF fields only as names in tables,
no struct and no offsets; the power-on example writes `ISensorIfAPI *sensor_if = &handle->sensor_if_api;` (an embedded struct),
which does not match this mhal (pointer at +0x44, above). MCLK list 27, 21.6, 12, 5.4, 36, 54, 43.2, 61.7, 72, 48, 24,
37.125 MHz and `CUS_CMU_CLK_LPLL_DIV1/2/4/8`; `handle->mclk = UseParaMclk(SENSOR_DRV_PARAM_MCLK())` lets a module parameter
override it; I2C speed 60 000–320 000, 8-bit slave address, formats A8D8 / A16D8 / A8D16 / A16D16; `mipi_lane_num` default 4;
module parameters `chmap`, `lane_num`, `hdr_lane_num`, `i2c_slave_id`; the mode table is filled from a static array
`{mode, {w, h, min_fps, max_fps}, {crop_x, crop_y, preview_w, preview_h}, {"desc"}}`, and MI picks a mode by index through
`SetVideoRes`. [S] (SigmaStar FAE on the Comake developer chat, SSD268G, 2021-08): `chmap` is a bit mask, bit n = sensor pad n.
`SENSOR_DRV_ENTRY_IMPL_END_EX` (in `drv/inc/drv_sensor_common.h`) makes the three `DrvSensor*Ver` calls with the
`CUS_*_MAJ/MIN_VER` defines.

## 3. SNR — sensor interface

### 3.1 State: the context area [R]

All per-pad state is one `.bss` area addressed from `gDrvSensorCtx` (.bss+0x223f4); the linker split it into symbols, the
code indexes it from the first one. Slot 0 = main sensor, slot 1 + p = plane p.

| offset from `gDrvSensorCtx` | size | content |
|---|---|---|
| +0x000 + pad·0x74 | 4 × 0x74 | per-pad context: +0x00 main init function, +0x04…+0x10 plane 0–3 init functions, +0x18 main handle, +0x1c…+0x28 plane 0–3 handles (HDR only), +0x30 VCM init function, +0x34 VCM handle, +0x38 u8 plane mode (0 = use +0x18), +0x3c CamOs mutex, +0x64…+0x70 the four ISRCB handles of the frame-event callbacks |
| +0x1d0 + slot·0xc0 + pad·0x30 | 5 × 4 × 0x30 | `i2c_handle_t` per (slot, pad) (§2.4) |
| +0x590 + slot·0x110 + pad·0x44 | 5 × 4 × 0x44 | `ISensorIfAPI` per (slot, pad) (§2.3) |
| +0xae0 + pad·8 (`gDrvSensorInfo`) | 4 × 8 | skip-frame state: +0 u32 pad, +4 u8 first VIF channel (group·4), +5 u8 plane mode (1 = frame-synced pair), +6 u8 frame count, +7 u8 busy |
| +0xb60 (`gSensorIfWorkPool`) | 0x118 | work queue (§3.7) |
| +0xc78 (`g_VcmHandle`) | 4 × 0x54 | VCM handles (§3.13) |
| +0xdc8 + pad·0x30 | 4 × 0x30 | VCM `i2c_handle_t` |
| +0xe88 + pad·8 | 4 × 8 | VCM `VcmIfAPI` {PowerOff, Reset} — its pointer is stored in the VCM handle, the table is **never filled** (stays NULL) |
| +0xea8 | 4 | stall-monitor pad bitmask; +0xeac…: the monitor's previous frame-start (+4·pad) and line-count-0 (+0x14 + 4·pad) counts, then the CamOs timer (+0xecc) |
| +0xee4 | 4 | monitor timer running flag |
| +0xee8 (`g_SensorHandle`) | 5 × 4 × 0xa1c | sensor handles `[slot][pad]` (§2.2) |

Outside the area: `gSensorI2CMaster` u8[4] (.data, §2.4); `tTimer` (4 × 0x18, skip-frame timers); the log mask
(`SNRIFLogGetEn` reads the word at .data+0x1c438, initial value 3: bit 0 trace on, bit 1 errors on, bit 3 verbose off;
`SNRIFLogSetEn` has no caller, so it stays 3); the early-init status words (§3.14).

### 3.2 Call pattern, plane mode, locking [R]

Every `DrvSensorX(pad, …)`: pad ≥ 4 → −1; take the main handle (in plane mode: the plane handles, below); if the driver
slot is non-NULL call `slot(handle, …)`, else return 0 with the outputs untouched (a NULL slot is not an error, except
`GetSensorID`, §3.12); a driver error → −1.

**Plane mode** (ctx+0x38) is set only by `DrvSensorSetPlaneMode` (from `MHal_SNR_SetPlaneMode`, §3.3, and
`/dev/sensorif`). With mode ≠ 0 every `DrvSensor*` call goes to the plane handles instead of the main one:
- **setters** (`SetOrien`, `SetFPS`, `SetRes`, `CustFunction`, `SwitchId`, the `Init` steps, the `DeInit` poweroff) run on
  every present plane 0..3 in order and stop at the first error (−1);
- **getters** (`GetOrien`, `GetFPS`, `QueryResCount`, `GetRes`, `GetCurRes`, `GetCurSwitchId`, `GetID`) use the first plane
  whose slot is non-NULL;
- calls taking a VIF channel (`AeNotify`, `SetUs`, `SetGain`, `GetUs`, `GetGain`, `Try*`, `GetShutterInfo`,
  `GetAEMinMaxGain`, `GetPlaneInfoByChn`, VCM) resolve (pad, plane) from the channel through `DrvVif_GetDev2SNRPadID` and
  use handle[plane + 1].

**Locking** differs per function: `SetOrien`, `SetFPS`, `SwitchId`, `GetCurSwitchId`, `AeNotify` and `Init` take the pad
mutex (ctx+0x3c); `SetRes`, `GetFPS`, `GetRes`, `GetCurRes`, `QueryResCount`, `GetOrien`, `CustFunction`,
`Get/SetAnadecSrcAttr` and `GetID` call the driver without it.

### 3.3 `MHal_SNR_*` — the MI-facing exports [R]

All check pad < 4 and map 1:1 to `DrvSensor*`; the reordering into MI structures is done in `mi_sensor` (the output
layouts PadInfo, PlaneInfo, Res: `mi-capture.md` §2.3). 16 of them are imported by `mi_sensor`, 2 by `mi_isp`.

| export | → | driver slot / note |
|---|---|---|
| `SensorEnable` | `DrvSensorInit` | §3.4 |
| `SensorDisable` | `DrvSensorDeInit` | §3.5 |
| `GetInfo` | `DrvSensorGetInfo` + `GetFPS` + `GetCurRes` | from handle fields: bus +0x78 (`CUS_SENIF_BUS` 0 PARL, 1 MIPI, 2 BT601, 3 BT656, 4 BT1120 → MHal interface 1, 4, 0, 0, 2; another value → 5), MIPI attributes +0x950…+0x960, HDR, polarities +0x58…+0x68, `mclk` +0x97c |
| `GetPlaneInfo(pad, plane, out)` | `DrvSensorGetPlaneInfo` | name, current resolution crop / output (`res[cur]`), Bayer +0x70, RGBIR +0x74 (0 → 8), precision +0x7c, AE delays +0x48…+0x54; then `GetAEUSecs` +0x9cc, `GetAEGain` +0x9d4. Data-type code into PlaneInfo +8 ("mhal pixel fmt"): Bayer planes → 0x2e + precision index 0..7, or for index 8 → 0x2a + sub-index 0..3; YUV planes by YUV order 1 → 0xf, 2 → 0xe, 3 → 0x10, else 0. Bayer id 0..4 → 0, 1, 4, 2, 3 into +0xc |
| `GetFPS` / `SetFPS` | | +0x9e4 / +0x9e8; any non-zero result → −1 |
| `MHal_SNR_GetFPS_x1000` / `MHal_SNR_SetFPS_x1000` | | `GetFPS_x1000` is a 4-byte tail call to `MHal_SNR_GetFPS`; `SetFPS_x1000(pad, fps)` requires fps ≥ 1000, then `DrvSensorSetFPS`. Both exported, **no importer** |
| `QueryResCount` / `GetRes` / `GetCurRes` / `SetRes` | | +0x9f8 / +0x9b0 / +0x9b4 / +0x9b8; `QueryResCount`, `SetRes`: any non-zero result → −1 |
| `GetOrien(pad, *mirror, *flip)` / `SetOrien` | | +0x9bc / +0x9c0. Driver orientation 0 → (0, 0), 1 → (1, 0), 2 → (0, 1), 3 → (1, 1); another value leaves the outputs; `GetOrien` always returns 0 for pad < 4. `SetOrien` is the inverse |
| `SetPlaneMode(pad, en)` | `DrvSensorSetPlaneMode` + VIF dev ↔ pad map | linear MIPI: `DrvVif_SetDev2SNRPadMap(group, dev, pad, en ? 0 : 0xff)` |
| `CustFunction` | | +0x9fc with all arguments; any non-zero result → −1 |
| `GetAnadecSrcAttr` / `SetAnadecSrcAttr` | | +0xa04 / +0xa08, analog decoders only (§3.12) |
| `GetSwitchSensorId` / `SwitchSensor` (`mi_isp`) | | +0xa18 / +0xa14 (§3.12) |
| `GetEarlyInitSnrInfo` | | exported, no importer (§3.14) |

### 3.4 Bring-up: `MHal_SNR_SensorEnable(pad)` → `DrvSensorInit(pad)` [R]

1. `DrvSensorGetInfo`; register the frame-event callback `_FrameEndISR` with `ISRCB_RegisterCallback(id, _FrameEndISR,
   ch)` (`mhal-core.md` §3.4), argument = VIF channel = id − 0x18:
   - MIPI: four ids — pad 0 0x18–0x1b, pad 1 0x20–0x23, pad 2 0x1c–0x1f, pad 3 0x24–0x27;
   - parallel: one id — pad 0 0x18 (channel 0), pad 1 0x20 (channel 8); pads 2 / 3 none;
   - other buses: none.
   The handles go to ctx+0x64…+0x70.
2. Early-init wait: while `DrvSensorIsEarlyInitEnabled(pad) == −1` print "Check EarlyInit", read the early-init info, and
   after `info+0x2c` 1-ms sleeps print "EarlyInit %d ms timeout, change to Non-EarlyInit mode", `StopEarlyInit`,
   `ClearEarlyInitStatus`. Never entered on this build: the first `IsEarlyInitEnabled` call turns the status into 0
   (§3.14).
3. For the main handle (or every plane): under the pad mutex `poweron(handle, pad)` (+0x98c); then, if
   `IsEarlyInitEnabled == 0`, `init(handle)` (+0x994; < 0 → "Sensor initial fail!", unlock, −1), else
   `ClearEarlyInitStatus` (a sensor left running by the boot loader: the register table is skipped); then
   `post_init(handle, pad)` (+0x998).
4. VCM handle present → its `PowerOn(handle, pad)` (+0x3c) and `Init(handle)` (+0x44) (§3.13).
5. `DrvSensorMonitor(pad, 1)` (§3.8) **only if the bus is parallel or MIPI** (`GetInfo` bus < 2).

**Register-level result with the GC8613 driver** (its `pCus_poweron` call order: `sensor-gc8613.md` §5.1;
`post_init` is NULL, no VCM): SetIOPad → `101e.50[15] = 0`, the `103c.69` fields of §2.3, `103e.1d–26` bits 4 / 6 = 0;
SetCSI_Clk(216M) → CSI clocks on (§4.3); SetCSI_Lane(4) → D-PHY / MAC bring-up (§4.4); SetCSI_LongPacketType(0, 0x1c00,
0) → RAW8 / 10 / 12 accepted (§4.5); PowerOff(!POS = 1) → `1304.00[3] = 0`; 1 ms; Reset(NEG = 1) → `1304.00[2] = 0`; 1
ms; PowerOff(POS = 0) → `1304.00[3] = 1`; 1 ms; Reset(!NEG = 0) → `1304.00[2] = 1`; 1 ms; MCLK(1, 24 MHz) →
`1038.62[3:0] = 0`, `1038.62[13:8] = 0x1c`; 5 ms. Then `pCus_init` writes the sensor's own table over I2C.

### 3.5 Shutdown: `DrvSensorDeInit(pad)` [R]

`DrvSensorMonitor(pad, 0)` (the timer is deleted when no pad is left), `poweroff(handle, pad)` (+0x990),
`ISRCB_UnRegisterCallback` for the four handles. With the GC8613 driver's `pCus_poweroff`: PowerOff(1) → `1304.00[3] = 0`,
Reset(1) → `1304.00[2] = 0`, 1 ms, CSI clock off (§4.3), MCLK off → `1038.62[13:8] = 0x1d`.

### 3.6 Frame event → AE notify [R]

**The event.** ISRCB id `0x18 + ch` is raised by the VIF interrupt handler on interrupt bit 1 of channel `ch` = **line
count 0 reached**, programmed at h − ⌊0.3·h⌋ for MIPI (h − ⌊0.1·h⌋ for parallel; for BT656 by height class), i.e. at about
70 % of the active frame, not at its end (`mhal-vif.md` §11). The SNR names keep "frame end" (`_FrameEndISR`,
`_DrvSensorifFrameEndProc`).

**The path.** `_FrameEndISR(ch)` (hard-IRQ context) → `SensorIfThreadWqAdd(&gSensorIfWorkPool, _DrvSensorifFrameEndProc,
ch)` → in the work-queue thread `DrvSensorAeNotify(ch, 0)` → the driver's `AEStatusNotify(handle, 0)` (+0x9c8) under the
pad mutex. Channel → pad via `DrvVif_ChnID2DevID` + `DrvVif_GetDev2SNRPadID`. `DrvSensorAeNotify(vif_chn, status)` stores
the pad into `handle+0x88` **before** checking the handle for NULL (a NULL dereference if a channel maps to a pad without a
sensor), then passes `status` unchanged.

**Status values** (`CUS_CAMSENSOR_AE_STATUS_NOTIFY`: `CUS_FRAME_INACTIVE` 0, `CUS_FRAME_ACTIVE` 1), by caller (from the
disassembly):

| caller | status | when |
|---|---|---|
| `_DrvSensorifFrameEndProc` (SNR work queue, from VIF bit 1 / ISRCB 0x18 + ch) | **0** | every VIF line-count-0 event of the pad's channels |
| `CameraIsp_SensorAeNotify(ch, ctx, 1)` ← `FrameStartProc` | **1** | ISP frame start, after the type-1 works due now |
| `CameraIsp_SensorAeNotify(ch, ctx, 1)` ← `CameraSetAeParam` (ioctl 0x2E06) | **1** | after a user AE result was queued (realtime) |
| `CameraIsp_SensorAeNotify(ch, ctx, 1)` ← `SensorFrameDoneProc` | **1** | sensor frame done |
| `CameraDebugSetSensorFPS` | **1** | after `DrvSensorSetFPS` |
| `/dev/sensorif` ioctl 0x70 | **1** | after `DrvSensorSetGain` |
| `/dev/sensorif` ioctl 0x71, write command `notify <ch> <stage>` | caller's value | debug |

`CameraIsp_SensorAeNotify(ch, ctx, status)`: ctx NULL or ctx byte +0x19778 = 0 → only a log line; a global flag word
(+0x338 of the ISP state, bit 0) set → nothing; HDR pipe enabled and `(ch & ~0xc) == 0` → notify `ch` and
`ch + DrvIsp_Vif_GetHdrChOffset()`; otherwise notify `ch`. The GC8613 driver acts only on `CUS_FRAME_ACTIVE` (it sends the
queued shutter / gain / frame-length registers then) and ignores 0, so with it the VIF path is a no-op
(`sensor-gc8613.md` §5.5).

### 3.7 Work queue [R]

Pool `gSensorIfWorkPool` (0x118): +0x00 CamOs mutex, +0x28 atomic count, +0x2c write index, +0x30 read index, +0x34 Tsem,
+0x54 thread, +0x58 ring of 16 × `{fn, arg, valid}`.
- `SensorIfThreadWqInit`: one CamOs thread "SensorIfThreadWq", priority 0x5f.
- `SensorIfThreadWqAdd(pool, fn, arg)`: count < 16 → store at the write index (no mutex), advance mod 16, count++,
  `TsemUp`; returns the count, or −1 (full or NULL pool) — a full ring drops the event.
- `…Get(pool, *rec)` under the mutex: count ≠ 0 and the read slot valid → copy, clear valid, advance, count−−.
- `…Proc`: loop `TsemTimedDown(5000 ms)`; 0 → drain with Get and call `fn(arg)`; a result ≠ 0 and ≠ −4 (timeout) → print
  and **exit the thread**; stop when `CamOsThreadShouldStop`.
- `…Deinit` (stop, Tsem / mutex destroy), `…Clean` (drain without calling).
- `SensorIfThreadWqFlush` operates on a different, never-initialised pool `_g_tSensorIfThreadWqPool` and has **no caller**.

### 3.8 Stall monitor `DrvSensorMonitor(pad, on)` [R]

One CamOs timer for all pads (bitmask at ctx+0xea8), period **1500 ms**. `_MonitorTimerCb`: for each enabled pad whose
VIF group is not stream-masked, read the channel's interrupt counters (`DrvVif_IntGet` = {interval, frame start, —, line
count 0, line count 1}; the counters are `VREF_FALLING`, `LINE_CNT_0`, `LINE_CNT_1`, `TOTAL_PIX_CNT` of `DrvVif_IntShow`)
and compare the frame-start and line-count-0 counts with the previous tick; if either is unchanged → queue
`_DrvSensorifMonitorProc(pad)` = print **"Sensor is abnormal @sensor pad %d"** and run `DrvSensorInit(pad)` again (full
power-on and init of the sensor). The string exists only in mhal, not in `mi_sensor`, and this monitor is its only printer
[R]; the line in the camera's boot log therefore means that no frame events arrived for 1.5 s at that moment [I]; what
stalled them is not known. `GetSensorID` is reached only from `/dev/sensorif` (§3.12), not on the bring-up path.

### 3.9 The ISP-facing `DrvSensor*` set [R]

Called from mhal's ISP block, each taking a **VIF channel** and resolving the pad (and plane) through the VIF dev ↔ pad map:
`DrvSensorSetUs` (+0x9d0), `DrvSensorSetGain` (+0x9d8), `DrvSensorGetUs` (+0x9cc), `DrvSensorGetGain` (+0x9d4),
`DrvSensorTryShutter` (+0xa10), `DrvSensorTryGain` (+0xa0c), `DrvSensorGetShutterInfo` (+0x9f4), `DrvSensorGetAEMinMaxGain`
(+0x9e0), `DrvSensorGetFPS` / `SetFPS`, `DrvSensorGetPlaneInfoByChn`, `DrvSensorAeNotify` (§3.6), the VCM calls (§3.13),
and `DrvSensorI2cRead/Write` (ad-hoc I2C: a temporary `i2c_handle_t` for the pad's bus, then open / tx-or-rx / close;
the ISP's `CameraIsp_SensorI2cRead/Write` use them). Callers include `IspMidWqSetSensorShutter/Gain`,
`CameraIspAlgo_DoHdrLinearAeConversion`, `CameraIsp_SensorShutterGainCompensation`, `CameraGetAeInitStatus`,
`MhalCameraOpen` (call graph from `objdump -dr`). Also used by the ISP: `DrvSensorExist(pad)` = 0 if the main handle or
the plane-0 handle exists, else −1 (caller `IspMid_IsSensorBindExist`); `DrvSensorGetPlaneMode(pad, *m)` reads ctx+0x38
(callers `CameraGetAeInitStatus`, ShowInfo).

### 3.10 Skip frame: `DrvSensorSetSkipFrame(pad, fps, n)` (IF slot +0x3c) [R]

Busy flag set → "SNRPAD%d still skip frame", −1. Else store pad, n, busy = 1; `DrvVif_SNRPad2GroupID(pad)` → group g and
the plane-mode byte; first channel = 4g; frame time = 1000 / (fps / 1000) ms when fps ≥ 1000 (fps × 1000), else 1000 / fps.
Plane mode 1 → `DrvVif_InputMaskFrameSync(ch, 1)` for two channels (the loop bound is the stored channel + 1) then
`DrvVif_TriggerMaskChange(4g)`; otherwise `DrvVif_InputMask(4g, 1)`. Arm `tTimer[pad]` for n × frame time.
`_SkipTimerCb` undoes the mask the same way, deletes the timer ("Timer Delete fail!" on error) and clears busy. The GC8613
driver does not call it.

### 3.11 Plane (HDR) drivers and the HDR mode [R]
<!-- ev:mhal-csi-snr.3.11 -->

`DrvRegisterPlaneDriverEx(pad, plane, init_fn, priv)` (pad, plane < 4; slot = plane + 1; else −1, and −1 if the plane slot
already has a handle): store `init_fn` at ctx+0x04 + plane·4; `memset(handle[slot][pad], 0, 0xa1c)`; **re-initialise the
pad mutex** (the same +0x3c mutex as the main sensor); zero the slot's I2C table; `handle+0x88 = pad`, `+0x44 =
&IF[slot][pad]`, fill that table with the same 13 pointers as the main one (§2.3), `+0x40 = &I2C[slot][pad]`, `+0x28 =
priv`; call `init_fn(handle)`; store the handle at ctx+0x1c + plane·4; **plane mode := 0**;
`DrvSensorI2CAttach(pad, handle+0x40)`. `DrvRegisterPlaneDriver(pad, plane, fn)` = Ex with priv 0.
`DrvRegisterPlaneI2CSlaveID(pad, plane, id)`: id 0 → no-op; plane handle present → `handle+0x3c = id`; else −1.

`DrvSensorSetHDRMode(pad, cus_mode, en)` (IF slot +0x34) maps the driver's `CUS_HDR_MODE` to the CSI HDR type: 0 → 0 and
1 (SONY_DOL) → 1, 2 (DCG) → return 0 without touching CSI, 3 / 4 / 5 (EMBEDDED_RAW8 / 10 / 12) → 2 / 3 / 4, 7 → 5 (UD1
8 → 16), anything else (6, 8 LI, 9 MULTI_VC) → "HDR Mode not defined", −2; then `Drv_CSISetHDRMode(pad, type, en)`
(§4.7) [R: code; enum names from the public `drv_ms_cus_sensor.h` in [S]].

### 3.12 Analog decoder, sensor switch, sensor id, custom function, helpers [R]

- `DrvSensorGetAnadecSrcAttr(pad, plane, out)` / `Set…` (+0xa04 / +0xa08; plane mode → handle[plane + 1]): a 0x18-byte
  record `{u32 status, u32 transfer format, u32 format, u32 w3, w4, w5}`; the code range-checks status (0 / 1 / 2, else 3
  and "Error BT656 Src status err"), transfer format (0..3, else 4), format (0 / 1, else 2) and passes the last three words
  through. The `MHal_SNR` wrappers repeat the checks (identity maps on valid values). Used by analog-decoder (BT656)
  drivers only.
- `DrvSensorSwitchId(pad, id)` → +0xa14 (under the mutex); `DrvSensorGetCurSwitchId(pad, *id)` → +0xa18. Reached from
  `mi_isp`'s `MHal_SNR_SwitchSensor` / `GetSwitchSensorId` (sensor time-multiplex); NULL slot → 0, output untouched. The
  GC8613 driver leaves both NULL.
- `DrvSensorGetID(pad, *id)` → +0x9ac (`pCus_GetSensorID`); NULL slot → −2. Only caller: `/dev/sensorif` ioctl 0x74.
- `DrvSensorCustFunction(pad, cmd, buf, …)` → +0x9fc with all arguments.
- `HalSensorPatchCropStartX(x)` = `(x + 3) & ~3`; **no caller**.
- `EXT_log_2(x)` / `intlog10(x)` (`sensor_drv_math.c`): fixed-point log2 (Q8.24, 256-entry interpolation table at
  .rodata+0x1012c) and log10 = log2 × 646456993 >> 31 — the Linux `dvb_math.c` `intlog2` / `intlog10` algorithm [I: same
  constant and table scheme]. Exported for sensor drivers (`imx307_MIPI.ko` imports `intlog10`); x = 0 warns and returns 0.
  `DrvSensorTest` prints `intlog10(i)·200/3 >> 24` for i = 1..4999 (log bit 0) — a self-test, reached from ioctl 0x90.

### 3.13 VCM (lens motor) drivers — unused [R]
<!-- ev:mhal-csi-snr.3.13 -->

`DrvRegisterVcmDriver(pad, init_fn)`: pad < 4 and no VCM yet (else −1); ctx+0x30 = `init_fn`; `memset(g_VcmHandle[pad], 0,
0x54)`; zero the VCM I2C table; handle `+0x34 = &I2C`, `+0x38 = &VcmIfAPI[pad]` (never filled); call `init_fn(handle)`;
ctx+0x34 = handle; `DrvSensorI2CAttach(pad, handle+0x34 table)`. `DrvReleaseVcmDriver` returns 0 and does nothing.

VCM handle (0x54 B): model_id[32] +0x00, i2c_cfg +0x20, i2c_bus +0x34, vcm_if_api +0x38, PowerOn +0x3c (called as
`(handle, pad)` at the end of `DrvSensorInit`), PowerOff +0x40 (never called), Init +0x44 (`(handle)`, same place), SetPos
+0x48 `(handle, pos)`, GetCurPos +0x4c `(handle, *pos)`, GetMinMaxPos +0x50 `(handle, *min, *max)`. This is the public
`ss_cus_vcm` of the SSC37x SDK **without** its
`private_data` word [I: the offsets fit exactly; field names [S]].

`DrvVcmSetPos` / `DrvVcmGetCurPos` / `DrvVcmGetMinMaxPos(vif_chn, …)` resolve the pad from the channel and call the slot;
NULL handle or slot → 0 (outputs left at 0). Callers: the ISP AF code and the `/dev/sensorif` write commands `setpos`,
`getcurpos`, `getposrange`. No VCM driver exists on the camera.

### 3.14 Early init — a stub on this build [R]

Early init (a sensor started before Linux and handed over: `rtk_sensor_early_init_*`, `DrvEarlyInit*`,
`/dev/sensor_earlyinit`) is compiled as a stub. Status words: .data+0x1c43c, u32[4],
initial **0xffffffff** each.

| function | behaviour |
|---|---|
| `DrvSensorIsEarlyInitEnabled(pad)` | if status[pad] < 0 → status[pad] = 0; return status[pad] (so −1 is seen by nobody: the first call turns it into 0 = off) |
| `DrvSensorClearEarlyInitStatus(pad)` | status[pad] = 0 |
| `DrvSensorStopEarlyInit(pad)` | returns 0 |
| `DrvSensorGetEarlyInitInfo(pad, info)` | `memset(info, 0, 0x30)`, returns 0 |
| `DrvEarlyInitGetStatus`, `…GetStatusEx`, `…GetSnrInfo`, `rtk_sensor_early_init_check_ready` | return −1 |
| `rtk_sensor_early_init_ex2` / `ex3` / `stop`, `rtk_sensor_timing_gpio` | empty |
| `SensorEarlyInitGenTable` | returns 0 |
| `EarlyInitSysGetRegBase` / `EarlyInitGetRegBase` | lazily `ioremap(0x1F000000, 4 MiB)` into .bss+0x124e8 / +0x124ec and return it; `EarlyInitSysRegV2R(va)` = va − that base. No caller |
| `DrvSensorEarlyInitSync(pad)` | if enabled: group from VIF, info; if `info+0x29` ≠ 0: plane mode 1, `SetRes(info+0x28)`, `SetFPS(info+0x14)`, `SetUs(4g, info+0x18)`, `SetGain(4g, info+0x1c)`. **No caller** |

**Early-init info record** (0x30 bytes; from its readers `MHal_SNR_GetEarlyInitSnrInfo`, `DrvSensorShowInfo`,
`DrvSensorInit`, `EarlyInitSync`): +0x00 precision index 0..3, +0x04 Bayer id 0..4, +0x08 (unused by the readers), +0x0c
width, +0x10 height, +0x14 fps × 1000, +0x18 shutter µs, +0x1c gain × 1024, +0x28 u8 resolution index, +0x29 u8 valid,
+0x2c wait timeout in ms (used by `DrvSensorInit`, §3.4).

`MHal_SNR_GetEarlyInitSnrInfo(pad, out)` (exported, no importer; caller `DrvSensorIfUt "early_info"`): out[0] = CSI data
type 0x2a + precision (RAW8 / 10 / 12 / 14 for 0..3; other → −1), out[1] = Bayer id mapped 0 → 0, 1 → 1, 2 → 4, 3 → 2,
4 → 3 (other → −1), out[2..6] = width, height, fps, shutter, gain, out[10] = (enabled ≠ −1) — with the stub always 1.

`DrvSensorPdwn` / `DrvSensorReset` return −1 ("skip pdwn") while enabled ≠ 0 — never, on this build.

`/dev/sensor_earlyinit` (platform driver `ss_sensor_earlyinit_*`): **not created on this build** (§1.3). Its code:
`ss_sensor_earlyinit_probe` allocates a miscdevice (minor 0xff = dynamic, name "sensor_earlyinit",
`sensor_earlyinit_fops` at .data+0x1644) and registers it, `…_remove` deregisters and frees;
`sensor_earlyinit_open` / `fread` / `ioctl` / `release` return 0; **write** copies the text and runs `DrvEarlyInitUt`:
command `trigger <fps×1000> <shutter_us> <gain×1024>` allocates 32 KiB of DMA memory "EarlyInit", builds `{shutter, gain,
u16 fps, …, buffer phys / virt, 0x8000}`, calls `rtk_sensor_early_init_ex2` (empty), then spins while
`DrvEarlyInitGetStatusEx(0) == 0` (it returns −1, so no spin), prints "CMDQ takes %u ms", frees. Other commands are
ignored. [I: in builds with a real early init the RTOS / IPL side runs the sensor table through CMDQ before Linux; this
build has none.]

### 3.15 Debug surface: `/dev/sensorif`, its write commands, sysfs `info` [R]

`/dev/sensorif` (fops at .data+0x1c4b8) duplicates `MHal_SNR` for debugging; no user-space reference to it is known on the
camera [I: none found among the MI user libraries].

**ioctl** (`sensorif_ioctl`): type `'S'` (else −25), every command `_IOW('S', nr, 84)` = 0x405453nn; the 84-byte payload is
copied in and, for getters, the whole 84 bytes copied back. Payload: +0 u32 pad (or VIF channel), +4 …, +8 … . A bad
pointer → −14; a function returning −1 → −14 for most commands.

| nr | name (log string) | call |
|---|---|---|
| 0x01 | `IOCTL_SENSOR_GET_INFO` | `MHal_SNR_GetInfo(pad, buf+4)` (0x50-byte PadInfo, fills the payload exactly) |
| 0x02 | `IOCTL_SENSOR_GET_PLANE_INFO` | `MHal_SNR_GetPlaneInfo(pad, plane = buf+4, buf+8)` |
| 0x30 | `IOCTL_SENSOR_SET_PLANE_MODE` | `MHal_SNR_SetPlaneMode(pad, buf+4)` |
| 0x60 | `IOCTL_SENSOR_RUN_INIT_SNRID` | `MHal_SNR_SensorEnable(pad)` |
| 0x70 | (no log) | `DrvSensorSetGain(ch, buf+4)` then `DrvSensorAeNotify(ch, 1)` |
| 0x71 | `IOCTL_SENSORT_NOTIFY` | `DrvSensorAeNotify(ch, buf+4)` |
| 0x72 | `IOCTL_SENSOR_SET_FPS` | `MHal_SNR_SetFPS(pad, buf+4)` |
| 0x73 | `IOCTL_SENSOR_GET_FPS` | `MHal_SNR_GetFPS(pad, buf+4)` |
| 0x74 | `IOCTL_SENSOR_GET_SENSOR_ID` | `DrvSensorGetID(pad, buf+4)` |
| 0x75 | `IOCTL_SENSOR_QUERY_RES_NUM` | `MHal_SNR_QueryResCount(pad, buf+4)` |
| 0x76 | `IOCTL_SENSOR_SET_RES` | `MHal_SNR_SetRes(pad, u8 buf+4)` |
| 0x77 | `IOCTL_SENSOR_GET_RES` | `MHal_SNR_GetRes(pad, u8 buf+4, buf+8)` (0x40-byte Res) |
| 0x78 | `IOCTL_SENSOR_GET_CUR_RES` | `MHal_SNR_GetCurRes(pad, buf+4 (u8 index out), buf+8)` |
| 0x79 | `IOCTL_SENSOR_SET_MIRROR_FLIP` | `MHal_SNR_SetOrien(pad, u8 buf+4, u8 buf+8)` |
| 0x90 | (none) | `DrvSensorTest()` (§3.12) |
| 0x91 | (none) | `WriteRegisterPair(bus = u16 buf+0, slave = u16 buf+2, reg = u16 buf+4, data = u16 buf+6, fmt = buf+8)`: −1 → −14, −2 → −19 |
| 0x92 | (none) | `ReadRegisterPair(bus, slave, reg, &u16 buf+6, fmt = buf+8)`, payload copied back |

Any other number in 0x01…0x92 → "Invalid ioctl command" (log bit 1), returns 0. The log strings are printed with bit 1 of
the SNR log mask (§3.1).

**write** (`sensorif_fwrite` → `DrvSensorIfUt`, text): `planemode <pad> <en>`, `enable <pad>`, `disable <pad>`, `set_res
<pad> <idx>`, `fps <pad> <fps×1000>`, `early_info <pad>`, `shutter <ch> <us>`, `gain <ch> <×1024>`, `notify <ch> <status>`,
`i2c_read <pad> <slave hex> <addr bytes> <data bytes> <reg hex>` (prints `Data=0x%X`), `i2c_write <pad> <slave> <ab> <db>
<reg> <data>`, `mf <pad> <mirror> <flip>` (then falls through to the next compare), `setpos <ch> <pos>`, `getcurpos <ch>`,
`getposrange <ch>`. The i2c commands go through `DrvSensorI2cRead/Write(pad, {slave, addr bytes, data bytes, speed 200000,
reg, data})`: (addr, data bytes) (1, 1) → A8D8, (2, 1) → A16D8, (1, 2) → A8D16, (2, 2) → A16D16 (format codes 0 / 1 / 2 / 3
= the `i2c_cfg.fmt` values).

**sysfs** `info` of the CamDevice `sensorif0` created in probe (`/sys/class/mstar/sensorif0/info`, class name:
`mhal-core.md` §4.1): show = `DrvSensorShowInfo(buf, 4096)` — per pad "== SENSOR PAD %d ==", the early-init line
("EarlyInit : Enable=0" here), "Fps x1000", then per plane (1, or 2 in plane mode) "== Plane %d ==" with sensor output X
/ Y / W / H, shutter, gain, shutter delay, gain delay, VChnNum; store = no-op (returns the count).

### 3.16 Remaining file operations [R]

`sensorif_open` / `fread` / `release` return 0. `mstar_sensorif_remove` removes the `info` file, destroys the CamDevice and
the char region, calls `DrvSensorifDeinitCommon` (= `SensorIfThreadWqDeinit`), deregisters and frees.

## 4. CSI — MIPI CSI-2 receiver

### 4.1 Instances, banks, DT, interrupt [R]
<!-- ev:mhal-csi-snr.4.1 -->

Four receivers: two 4-lane (pads 0 / 1) and two 2-lane (pads 2 / 3). `csi_probe` maps `io_phy_addr` (0x1F000000) +
`banks` (15 entries) with one `ioremap(PA + bank·0x200, 0x200)` per bank into globals:

| sensor pad | HAL family, instance | MAC | D-PHY analog | D-PHY digital | clkgen fields (bank 1038): lptx, mac, ns |
|---|---|---|---|---|---|
| **0 (the camera's sensor)** | `HalCsi*4Lane_*(0, …)` | **153a** | **1538** | **1539** | **58[4:0], 58[12:8], 59[4:0]** (camclk `*_m00`) |
| 1 | `HalCsi*4Lane_*(1, …)` | 1542 | 1540 | 1541 | 5b[4:0], 5b[12:8], 5c[4:0] |
| 2 | `HalCsi*_*(0, …)` (2-lane family) | 153d | 153b | 153c | 59[12:8], 5a[4:0], 5a[12:8] |
| 3 | `HalCsi*_*(1, …)` | 1545 | 1543 | 1544 | 5d[4:0], 5d[12:8], 5e[4:0] |

Globals: MAC `g_ptCSI4Lane[2]` / `g_ptCSI[2]`, D-PHY analog `g_ptDPHYAna4Lane` / `g_ptDPHYAna`, D-PHY digital
`g_ptDPHYDig4Lane` / `g_ptDPHYDig`, `g_ptClkGen` = 1038, MIPI mux `g_ptMipi[0..1]` = 153e / 1546 (touched only by the
2-lane path, §4.4; the `HalCsiMipi4Lane_*` variants are empty stubs). Every `Drv_CSI*` and debug entry maps the pad to
the family and instance as in the table and tail-calls the HAL twin (`HalCsiMac4Lane_*` / `HalCsiMac_*`, …) with the
other arguments unchanged; pad ≥ 4 → return without effect.

DT (`mhal-core.md` §7 lists the properties; values on this camera):
- `sr0_mipi_mode` — absent → `g_MipiMode = 0`; the value 2 only changes the pad-0 2-lane branch (§4.4).
- per pad N: `csi_srN_lane_num` → `g_CSILaneSupportNum[N]` (u8[4]); `csi_srN_lane_select[lane_num + 1]` →
  `g_CSILaneSelect[N]`; `csi_srN_lane_pn_swap[lane_num + 1]` → `g_CSILanePNSwap[N]`. Both arrays are **u32[4][5]** (stride
  0x14 per pad, one word per D-PHY channel). **sr0: lane_num 4, lane_select `<2 4 3 1 0>`, pn_swap `<1 1 1 1 1>`**;
  sr1–3: 2 lanes, `<2 1 0>`, `<1 1 1>`.
  [S] (SigmaStar Tiramisu BSP page `sensor_zh` §2–3): `csi_srN_lane_select = <clk l0 l1 l2 l3>`; SR0 and SR1
  are 4-lane each and SR0 can be split 2 + 2. [S] (SigmaStar FAE on the Comake developer chat, SSD268G): the receiver has no fixed data-lane
  numbers, only channels — `lane_select` decides which channel is the clock lane and which is data lane n; the documented
  values are examples, set from the real wiring.
- `interrupts`: up to 4 `CamIrqOfParseAndMap(node, i)`; the DT has one, **GIC SPI 0x41** → `request_threaded_irq(irq,
  csi_ISR, 0, 0, "CSI interrupt")` (§4.6).
- `clocks` (6 phandles: csi_mac_lptx / csi_mac / ns × 2 ports) are **not** requested: the clock fields are written
  directly in bank 1038 (no `CamClk*` / `clk_*` import in the CSI block).

Then probe: `misc_register("csi")` (fops `csi_fops`, .data+0x1c0), `CamDeviceCreate(msys class, "csi0")` with the sysfs
files of §4.10, `Drv_CSIInit(0)` → `HalCsi_RegInit()` = **empty** (no register is written at probe). `csi0` is created with
dev_t 0, so it has no `/dev` node — sysfs only (`mhal-core.md` §4.2). `csi_remove`: free the IRQs, `iounmap` everything,
deregister.

Logging: `CSILogGetEn()` returns the word at .data+0x140, **initial value 3** (bit 0 = trace prints such as
`"csi_ioctl\n"` and `"IOCTL_CSI_SET_*"`, bit 1 = error prints such as `"[%s] err, over CSI pad number"`). `CSILogSetEn(mask,
on)` sets / clears bits; it has **no caller**, so the mask stays 3.

### 4.2 Entry points (all take the sensor pad 0..3 first) [R]

| function (who calls it) | what it does | registers (pad 0) |
|---|---|---|
| `Drv_CSISetClk(pad, kHz)` (driver `SetCSI_Clk` via `DrvSensorSetCsiClk`, which maps `CUS_CSI_CLK` 0..9 / −1 → 108000, 86000, 144000, 172000, **216000**, 288000, 40000, 24000, 12000, 123000 / 0 kHz) | `HalCsiConvertCsiClk`: 216000 → 0, 240000 → 1, 172000 → 2, 144000 → 3, 86000 → 4, 40000 → 5, 0 → "off"; anything else → −2 "Not supported". Then **only code 0 (216 MHz) and "off" are implemented**; codes 1–5 print "Not implemented" and return −2 without touching hardware | §4.3 |
| `Drv_CSISetLane(pad, lanes, on)` (driver `SetCSI_Lane`) | full D-PHY + MAC lane bring-up; **`on` is ignored** on every pad (always brings up) | §4.4 |
| `Drv_CSISetLongPacket(pad, dt0_15, dt16_31, dt32_47)` (driver `SetCSI_LongPacketType`) | `HalCsiMac4Lane_DtEnable`: the long-packet data types the MAC accepts | `153a.1d = dt0_15`, `.1e = dt16_31`, `.1f = dt32_47` (16-bit writes); `dt0_15` bit 2 (embedded data) → also `.3d[1] = 1` (§4.5) |
| `Drv_CSISetClkDataSkip(pad, a, b)` (driver slot +0x30) | `HalCsiAna4Lane_SkipHsInHSRXMode`: **arguments ignored**, writes the constant; the 2-lane `HalCsiAna_SkipHsInHSRXMode` writes its arguments (§4.8) | `1538.16 = 0x1313` (M5: clock skip [7:0] / data skip [15:8] in ns-clock cycles on entering HS-RX = HS settle) |
| `Drv_CSISetHDRMode(pad, type, en)` (SNR `DrvSensorSetHDRMode`, driver slot +0x34) | `HalCsiMac4Lane_HDRType`; HDR sensors only | `153a.4d`, `153a.1a`, `.3d[0]` by type: §4.7 `HDRType` |
| `Drv_CSISetYUVOrderSwap(pad, v)` (driver slot +0x38) | `HalCsiMac4Lane_YUV422_8bitOutMode`; YUV sensors only | `153a.00[14] = v` (M5 `uy_vy_sel`) |
| `Drv_CSIQueryLaneNum(pad, *n)` (SNR `DrvSensorQueryLaneNum`) | `*n = g_CSILaneSupportNum[pad]` (pad ≥ 4 → 0) | none |
| `Drv_CSISetVC0HSMode(pad, vc, mode)` (`/dev/csi` ioctl only) | `HalCsiMac4Lane_VirtualChannelHsyncMode(i, vc, mode)` — despite the name any VC | `153a.01` (§4.7) |
| `Drv_CSIInit` / `Drv_CSIUinit` (probe / remove) | `HalCsi_RegInit` / `HalCsi_RegUninit`, both empty | none |
| `Drv_CSIISR(pad)` (`csi_ISR`, for pads 0..3 on every interrupt) | error-interrupt status → counters (§4.6) | `153a.0c` read, `.03` / `.05` |

### 4.3 Clocks [R]

Each receiver has three clocks — `csi_mac` (MAC), `csi_mac_lptx` and `ns` (M5: the D-PHY digital side measures the HS
clock period in `clk_ns` cycles) — and each is gated in two places: the clock generator (bank 1038, source + gate) and a
local gate / invert / divider in the MAC bank (`153a.18[5:0]` mac, `.18[12:8]` lptx, `.19[4:0]` ns). Field layout in both
places: [0] gate, [1] invert, [4:2] source / divider.

- `Drv_CSISetClkDiv_Mac` / `_Lptx` / `_Ns(pad, sel, gate, inv)` → `HalCsiMac[4Lane]_CkgMac` / `CkgMacLptx` / `CkgNs(i, sel,
  gate, inv)`: `CkgMac` → `.18[5:0]` = `sel[3:0]<<2 | inv<<1 | gate` (4-bit select, M5 `clk_csi_mac_sel` 0 = undivided,
  1 = /2, 2 = /4, 3 = /8 …); `CkgMacLptx` → `.18[12:8]` = `sel[2:0]<<2 | inv<<1 | gate`; `CkgNs` → `.19[4:0]`, same layout.
  Called only by `Drv_CSISetClk` (always sel 0, gate 0 / 1, inv 0) and `Drv_CSISetLane` (all 0).
- `HalClkGen_SelectMacLptxFreq` / `SelectMacFreq` / `SelectNsFreq(pad, src, gate, inv)`: one byte RMW of bank 1038, value
  `src[2:0]<<2 | inv<<1 | gate` into the 5-bit field of the pad (§4.1 table); pad ≥ 4 prints the error and returns.

`Drv_CSISetClk(pad, kHz)` order: MAC-local mac → lptx → ns (`Drv_CSISetClkDiv_*`), then clkgen lptx → mac → ns
(`Drv_CSISelectFreq` → `HalClkGen_Select{MacLptx,Mac,Ns}Freq`). **On (216 MHz)**: all six fields = 0 — gate open, no invert,
source 0 = `mpll_216m`, local divider /1; for pad 0 `153a.18[5:0] = 0`, `153a.18[12:8] = 0`, `153a.19[4:0] = 0`, then
`1038.58[4:0] = 0`, `1038.58[12:8] = 0`, `1038.59[4:0] = 0`. **Off**: the same six fields = `0b00001` (gate closed, source
0). No delay, no read-back, no lock poll. Only 216 MHz and off exist; every other rate the driver ABI can ask for returns −2
with the hardware untouched (§4.2), so the receiver clocks do not follow the sensor's lane rate — the HS bits are clocked
by the sensor's clock lane [I].

`Drv_CSISetLane` writes the three MAC-local fields to 0 again (§4.4 step 4) but not the clkgen ones, so a receiver whose
clkgen gates are closed stays dead after lane bring-up: `SetCSI_Clk` must come before `SetCSI_Lane` [I from the register
writes]. The GC8613 driver calls them in that order (§3.4) and closes the clocks in its poweroff (§3.5). Pads 1–3 use the
same functions with their clkgen fields.

### 4.4 Lane bring-up: `Drv_CSISetLane(pad, lanes, on)` [R]

**Pad 0 with 4 lanes** — the camera's path. Inputs from the DT (§4.1): `sel[0..4] = <2 4 3 1 0>` (`g_CSILaneSelect[0]`),
`pn[0..4] = <1 1 1 1 1>` (`g_CSILanePNSwap[0]`). D-PHY analog channel c (0..4) has three registers: LP / control `.04 .08
.0c .20 .23`, HS data `.05 .09 .0d .21 .24`, select `.06 .0a .0e .22 .25`. In call order, all byte RMWs, nothing read back:

1. D-PHY digital reset pulse: `1539.40[0]` 0 → 1 (M5 `sw_rstz`, active low).
2. D-PHY analog on: `1538.00[0] = 1` (`sw_rstz` released — written 1 only, no low phase), `.00[6] = 0` (LPTX VREG on),
   `.00[1] = 0` (dynamic `clk_ns` gating off), `.01[0] = 0` (HS mode on), `.01[1] = 0` (whole analog on).
3. Analog defaults: `1538.18[11:0] = 0` (M5 `ckg_rx_hs_short` / `ckg_rx_hs_nor` / `ckg_bist_tx`); the five 3-bit fields
   `1538.19[4:2]`, `.19[12:10]`, `.1a[4:2]`, `.1a[12:10]`, `.1b[4:2]` = 0; `1538.14 = 0x4000` (`TestModeEnable(0, 1)`; M5
   calls register 0x14 `dummy0`, meaning unknown).
4. MAC: local clocks open (`153a.18[5:0]`, `.18[12:8]`, `.19[4:0]` = 0, §4.3); `153a.20[2] = 0` (`dont_care_dt` off,
   §4.5); hsync mode of VC0..3 = 3 (M5: packet footer edge) → `153a.01[11:0] = 0x6db`.
5. HS settle: `1538.16 = 0x1313` (§4.2).
6. Per channel c = 0..4, select register: `[0] = pn[c]` (M5 `ds_pol_chN`; 1 on every channel of this board), `[14:12] =
   sel[c]` (M5 `chN_dat_sel`), `[1] = 0`, `[4:2] = 0` (software lane override off), `[15] = 0` (no MSB / LSB swap). Here:
   `[14:12]` of `.06 / .0a / .0e / .22 / .25` = 2 / 4 / 3 / 1 / 0.
7. MAC data lanes all off: `153a.00[5:2] = 0`.
8. The five fields of step 3 = `sel[0..4]`: `1538.19[4:2] = 2`, `.19[12:10] = 4`, `.1a[4:2] = 3`, `.1a[12:10] = 1`,
   `.1b[4:2] = 0` [I: one LP-side clock / source select per D-PHY channel; M5 has a different layout here, §4.8].
9. Every receiver forced off by software: for c = sel[0..4], LP / control `[7] = 1, [6] = 0` (M5 `en_sw_lprx_en`,
   `sw_lprx_en`, active low) and HS data `[11] = 1, [10] = 0` (`en_sw_hsrx_en`, `sw_hsrx_en`).
10. `lanes == 4`: the same channels handed back to the hardware FSM: `[7] = 0`, `[11] = 0`.
11. MAC data lanes 0..3 on: `153a.00[5:2] = 0xf`.

Return 0. No delay between steps, no poll of the lane FSMs or `mac_idle`. Steps 9–10 index channels by the **value**
`sel[i]`, steps 6 / 8 by position; on this board the same five channels either way (sel is a permutation of 0..4).

**The same sequence as HAL calls, every pad** (instance i, family F = 4-lane for pads 0 / 1, 2-lane for 2 / 3; `sel =
g_CSILaneSelect[pad]`, `pn = g_CSILanePNSwap[pad]`):
1. `HalCsiDig*_SoftwareReset(i, 1)` — digital `.40[0]` 0 → 1 (4-lane, bank 1539 / 1541); 2-lane: `.00[0]` of 153c / 1544.
2. `HalCsiAna*_SoftwareReset(i, 1)` (`.00[0] = 1`), `PowerDown_LPTX_VREG(i, 0)`, `DynGatingClkNsEnable(i, 0)`, `PdHsMode(i,
   0)`, `PdWholeDphyAnalog(i, 0)`, `HsRxTxAttribute(i, 0, 0, 0)` (`.18[11:0] = 0`), `LpAttribute(i, 0, 0, 0, 0, 0)`,
   `TestModeEnable(i, 1)` (`.14 = 0x4000`).
3. MAC: `CkgMac` / `CkgMacLptx` / `CkgNs(i, 0, 0, 0)`, `DontCareDt(i, 0)`, `VirtualChannelHsyncMode(i, vc, 3)` for vc 0..3.
4. `SkipHsInHSRXMode(i, 0x13)` (§4.8: a constant on 4-lane, the arguments on 2-lane; the decompile shows `(0, 0x13)`, the
   third register argument is the reused 0x13 [I]).
5. `SetHsTxPolarity(i, c, pn[c])` and `SetLaneSelect(i, c, 0, 0, sel[c], 0)` for c = 0..4.
6. `DataLaneEnable(i, lane, 0)` for lane 0..3 (MAC `.00[5:2] = 0`).
7. `LpAttribute(i, sel[0..4])`.
8. Every receiver forced off: `SetLP_RXEnable(i, sel[c], 0, 1)` and `SetHsRxData(i, sel[c], 0, 1)` for c = 0..4.

Then by `lanes` — a channel is released to the hardware FSM by `SetLP_RXEnable(i, ch, 0, 0)` + `SetHsRxData(i, ch, 0, 0)`,
a MAC lane enabled by `DataLaneEnable(i, n, 1)`:

| pad | lanes = 4 | lanes = 2 | lanes = 1 | other (0, 3, ≥ 5) |
|---|---|---|---|---|
| 0 | release sel[0..4]; MAC lanes 0–3 | release sel[0..2]; MAC lanes 0–1; then if `g_MipiMode == 2`: re-map (below) | release sel[0..1]; MAC lane 0 | return 0 after step 8 (every receiver off) |
| 1 | as pad 0 | as pad 0 without the re-map | `HalCsiMipi4Lane_MipiRx11Sel(1, 1)`, `SelRx11Clk(1, 2)` (both empty stubs), release sel[0..1], MAC lane 0 | as pad 0 |
| 2, 3 | release sel[0..4]; MAC lanes 0–3 (the 2-lane MAC has a 4-bit `lane_en` field too; whether lanes 2–3 exist is unknown [I]) | `HalCsiMipi_MipiRx22Sel(i, 1)` → `153e/1546.0e[0] = 1`, `HalCsiMipi_SelRxClk(i, 2)` → `.11[1:0] = 2`; release sel[0..2]; MAC lanes 0–1 | the same two MIPI-mux writes; release sel[0..1]; MAC lane 0 | as pad 0 |

**`g_MipiMode == 2` re-map** on pad 0 with 2 lanes (from DT `sr0_mipi_mode`; absent here → 0, never taken):
`LpAttribute(0, 2, 1, 3, 0, 0)`; `SetLaneSelect(0, c, 0, 0, s, 0)` with (c, s) = (0, 2), (1, 1), (2, 3); channels 0 and 4
forced off (`SetLP_RXEnable(0, ch, 0, 1)`, `SetHsRxData(0, ch, 0, 1)`), channels 1–3 released. So the 4-lane receiver #0
takes a 2-lane sensor on D-PHY channels 1–3 instead of the DT selection [I: a board option for a 2-lane sensor on the
4-lane balls].

**MIPI-mux bank** (153e / 1546): touched only on pads 2 / 3 with 1 or 2 lanes — `153e.0e[0]` "MipiRx22Sel" = 1 and
`153e.11[1:0]` "SelRxClk" = 2 (1546 the same for pad 3). No header names these bits; their meaning is unknown [I: they
select the 2-lane receiver's data / clock source pins, pairing the 2-lane receivers with the 4-lane balls]. A register
dump with and without a 2-lane sensor would show it; the camera has only the 4-lane sensor.

**`DataLaneEnable(i, n, on)`** (objdump addresses 0xebd0 / 0xdb9c): the table at .rodata+0x225c holds the three bytes **02 04
08**; mask = table[n − 1] for n = 1..3, and 1 for any other n (0 or ≥ 4). `on` → `.00[5:2] |= mask`; off → `.00[5:2] &=
~mask`. So the argument is a lane index and lane n sets bit n of M5 `lane_en`; the four `DataLaneEnable(i, 0..3, 1)` calls
of the 4-lane path give `.00[5:2] = 0xf`.

Decompile note: the decompile drops an argument where the compiler reused a register (e.g. `PowerDown_LPTX_VREG(0)`,
`VirtualChannelHsyncMode(0,3)`, `DataLaneEnable(0,1)`); the values above come from the pad-1 twin of the same switch, which
shows them in full (`(1,0)`, …) [I for VC3 = 3 and lane 1 on].

### 4.5 Long packets, pixel format, virtual channels [R unless tagged]

- **Accepted data types**: the GC8613 driver's `SetCSI_LongPacketType(0, 0x1c00, 0)` → `153a.1d = 0`, `153a.1e = 0x1c00`,
  `.1f = 0`. The three words are one 48-bit enable indexed by a bit number, not by the CSI-2 data-type code (M5 `dt_en` /
  `reg_dt_en`): 0 null, 1 blanking, 2 embedded, 14 YUV422 8-bit, **26 / 27 / 28 RAW8 / RAW10 / RAW12** (CSI-2 codes 0x2a /
  0x2b / 0x2c), 32… user-defined. Embedded data (bit 2 of the first word) would also set `.3d[1]` (set only, never
  cleared); this driver's value does not. With `dont_care_dt = 0` (§4.4 step 4) packets of any other type are dropped [I:
  from the M5 name].
- **No pixel format in the MAC**: bit depth and Bayer order are not set here; the MAC passes the payload on and the VIF
  is told the format (format table 0x10 = 10-bit, `sensor_rgb_in`; `mhal-vif.md` §3.1). RAW8 / 10 / 12 are enabled at once,
  so the CSI side would not notice a bit-depth mismatch with the VIF setting [I].
- **Virtual channels**: hsync of VC0..3 at the packet footer edge (§4.4 step 4); the vsync mode (`.02`) and `vc_en` are
  never written, so which VCs are passed is the reset default — unknown [I]. The frame / line number registers (`.12` /
  `.13`) and the short-packet capture (`.14` / `.15`) are never read.
- Not on the camera's path: YUV byte order (`.00[14]`), UD1 8 → 16-bit (`.3d[0]`), HDR types (`.4d`, `.1a`) — §4.7.

### 4.6 Interrupt and error counters [R unless tagged]

- One line for all four receivers, **GIC SPI 0x41** → `csi_ISR` (hard-IRQ handler, no thread function): calls
  `Drv_CSIISR(pad)` for pads 0..3 in turn and always returns handled.
- Per pad only the MAC **error** interrupt is serviced: status = `153a.0c[14:0]` (M5 `mac_err_int_src`); mask those bits
  (`.03 |= status`); count each set bit; clear (`.05 |= bits`, then `.05[14:0] = 0`); unmask (`.03 &= ~bits`). Pads 2 / 3 do
  the same in 153d / 1545.
- Counters `csi_err_int_count[pad][15]` (u32, 0x3c per pad), bit → name: 0 `ERR_INT_DT`, 1 `PA_LENS`, 2 `PH_LENS`,
  3 `ECC_ONEBIT`, 4 `FRAME_START`, 5 `FRAME_END`, 6 `ECC_TWOBIT`, 7 `CRC`, 8 `PA_WC_EQ0`, 9 `RAW10_LENS`, 10 `CON_FE`,
  11 `CON_FS`, 12 `LE`, 13 `LS`, 14 `OVERRUN` (bits 10–14 agree with M5 `mac_err_int_mask`; printed as `ERR_INT_DT` …
  `ERR_INT_OVERRUN`). They are shown only by the `csi_ints` sysfs file of `csi0` (§4.10) and never reset. Nothing acts on
  an error: no recovery, no log line, no callback into SNR or VIF. They are the only CSI-side view of a broken link [I].
- Which bits can fire is not set by the code: `153a.03` is written only by the ISR's mask / unmask and by the
  `csi_dbg_mask` sysfs store, so the enabled set is the reset default — unknown [I]. On the running camera `csi_dbg_mask`
  shows it ("sensor 0 csi dbg mask = 0x…") and `csi_ints` the counts (`/sys/devices/virtual/mstar/csi0/csi_dbg_mask`,
  `…/csi_ints`).
- Report interrupts (frame done per VC, `.06 / .08 / .0e`), PHY-layer ones (`.09–.0b`, `.48–.4a`) and the lane-FSM ones
  (`.28–.47`) have HAL functions (§4.7) but no caller: they are never enabled or serviced. Frame timing for SNR and
  `mi_vif` comes from the VIF (§3.6), and the SNR stall monitor (§3.8) watches VIF counters, not these.

### 4.7 MAC HAL — every function [R]

4-lane banks 153a / 1542, 2-lane 153d / 1545; identical unless noted. Field names from Mercury5
`kernel_csi_mac_4lane.h` (the same layout as M6 where checked). "none" = no caller in mhal outside `csi_ioctl` / sysfs.

| function (arguments after the instance) | register effect | callers |
|---|---|---|
| `ECCOff(v)` | `.00[1] = v` (`ecc_off`) | none |
| `DataLaneEnable(n, on)` | `.00[5:2]`, §4.4 | SetLane |
| `RemoveErroneousSOT(v)` | `.00[6] = v` (`rm_err_sot`) | none |
| `MACFuncMode(m)` | `.00[8:7] = m` (`fun_md`: 0 normal, 1 one-frame, 2 / 3 stream bypass) — written as two bit writes, `.00[7] = m&1`, `.00[8] = (m>>1)&1` | none |
| `VirtualChannelEnable(mask)` | `.00[12:9] = mask & 0xf` (`vc_en`) | none |
| `RemoveErroneousFS(v)` | `.00[13] = v` (`rm_err_fs`) | none |
| `YUV422_8bitOutMode(v)` | `.00[14] = v` (`uy_vy_sel`) | `Drv_CSISetYUVOrderSwap` |
| `VirtualChannelHsyncMode(vc, m)` | 3-bit field per VC in `.01`: vc 0 `[2:0]`, 1 `[5:3]`, 2 `[8:6]`, 3 `[11:9]` (M5 0 header edge, 1 line end, 2 line start, 3 footer edge); vc ≥ 4 ignored | SetLane (m 3 for all four), `Drv_CSISetVC0HSMode` (ioctl) |
| `VirtualChannelVsyncMode(vc, m)` | the same layout in `.02` (M5 0 frame start edge, 1 frame end edge) | none |
| `DebugEnable(v)` | `.02[12] = v` (`debug_en`: the short-packet registers `.14 / .15` show the MAC debug bus instead) | none |
| `MaskErrorInt`, `ForceErrorInt`, `ClearErrorInt`, `ErrIntMaskGet` / `ErrIntMaskSet`, `GetErrorIntStatus` | error interrupts, 15 bits: mask `.03[14:0]`, force `.04`, clear `.05`, status `.0c` (bits: §4.6). `MaskErrorInt(bits)`: mask \|= bits (returns the old low byte); `ForceErrorInt(bits)`: force \|= bits; `ClearErrorInt(bits)`: clear \|= bits, then clear = 0 (pulse), then mask &= ~bits (unmask); `ErrIntMaskSet(m)`: mask = m; `ErrIntMaskGet` / `GetErrorIntStatus`: read 15 bits | ISR (§4.6), `csi_dbg_mask_store` / `show` |
| `MaskRptInt(bits)`, `ClearRptInt(bits)`, `GetRptIntStatus()` | report interrupts, 7 bits: mask `.06[6:0]`, clear `.08[6:0]`, status `.0e[6:0]`. Bits (M5): 6 VC0 frame done, 5 VC1, 4 VC2, 3 VC3, 2 generic 8-bit short packet, 1 frame number, 0 line number. Mask: \|= bits; Clear: clear \|= bits, clear = 0, mask &= ~bits | none |
| `PhyIntMask(grp, bits)`, `PhyIntForce`, `PhyIntClear`, `PhyIntMaskGet(grp)` | D-PHY error interrupts, 10 bits per group: 4-lane group 0 = mask `.09`, force `.0a`, clear `.0b` (lanes 0 / 1: bits 0–4 `err_control`, `err_sync_esc`, `err_esc`, `err_sot_sync_hs`, `err_sot_hs` of lane 0, bits 5–9 the same for lane 1); group ≠ 0 = `.48 / .49 / .4a` (lanes 2 / 3). 2-lane MAC: one group only (`.09 / .0a / .0b`, no group argument). Clear = pulse then unmask, as for the error interrupts. No status getter exists (M5 `.10 / .11`, `.4b / .4c` are never read) | none |
| `FSMIntMask(n, m)`, `FSMIntForce`, `FSMIntClear(n, bits)`, `FSMIntMaskGet(n)`, `FSMIntSrcGet(n)`, `FSMIntRawSrcGet(n)` | lane-FSM status interrupts, 16 bits per FSM n: 4-lane n 0 = clock lane `.28` mask / `.29` force / `.2a` clear / `.2b` source / `.2c` raw; n 1..4 = data lanes 0..3 at `.30–.34`, `.38–.3c`, `.3e–.42`, `.43–.47`; any other n behaves as 0 in the getters. 2-lane: n 0..2 only (clock, data 0, data 1). Mask writes the whole word; Clear writes bits, then 0, then mask &= ~bits | none |
| `CkgMac`, `CkgMacLptx`, `CkgNs` | §4.3 | SetClk, SetLane |
| `DtEnable(w0, w1, w2)` | `.1d = w0`, `.1e = w1`, `.1f = w2`; if `w0 & 4` (embedded data type enabled) `.3d[1] = 1` (set only, never cleared) | `Drv_CSISetLongPacket` |
| `SoftwareReset(v)` | `.20[0] = v` (`sw_rst`) | none |
| `MacEnable(v)` | `.20[1] = v` (`mac_en`) | none |
| `DontCareDt(v)` | `.20[2] = v` | SetLane (0) |
| `SOT_EOT_Sel(lane, sot, eot)` | `.21[1:0] = lane` (`raw_l_sot_sel`), `.21[2] = sot` (0 from D-PHY, 1 auto), `.21[3] = eot` | none |
| `ReportFrameNum(v)` / `ReportLineNum(v)` | `.21[4]` / `.21[5]` (`rpt_fnum_cond` / `rpt_lnum_cond`: 0 at start, 1 at end) | none |
| `UD1_8TO16BitOutputMode(v)` | `.3d[0] = v` (`8to16_mode`) | `HDRType` type 5 only |
| `HDRType(type, en)` | type 1 (Sony DOL, M5 `sony_hdr`): `.4d[0] = en`; en → `.1a[3:0] = 1`, `.1a[7:4] = 2` (byte 0x21); off → `.1a[7:0] = 0`. Types 2 / 3 / 4 (`raw8_ed` / `raw10_ed` / `raw12_ed`): **4-lane** `.4d[1]` / `[2]` / `[3]` = en; **2-lane** `.4d[2]` / `[3]` / `[4]` = en (shifted by one versus the 4-lane MAC and M5); type 5: `.3d[0] = en` (UD1 8 → 16). Types 2–5 also clear `.1a[7:0]`; other types: nothing. `.1a` is `csi_mac_reserved` in M5 [I: on M6 the DOL VC / ID configuration] | `Drv_CSISetHDRMode` |

### 4.8 D-PHY analog HAL — every function [R]

4-lane banks 1538 / 1540: five channels c = 0..4 with LP / control register `.04 .08 .0c .20 .23`, HS data `.05 .09 .0d .21
.24`, select `.06 .0a .0e .22 .25`. 2-lane banks 153b / 1543: three channels c = 0..2 at `.04 .08 .0c` / `.05 .09 .0d` /
`.06 .0a .0e`. A channel index outside the range does nothing. Names from M5 `kernel_dphy_ana_4lane.h`.

| function | register effect (4-lane; 2-lane differences in brackets) | callers |
|---|---|---|
| `SoftwareReset(v)` | `.00[0] = v` (`sw_rstz`, active low) | SetLane (1) |
| `DynGatingClkNsEnable(v)` | `.00[1] = v` (`en_gate_clk_ns`) | SetLane (0) |
| `ClockDivide4Dot5XEnable(v)` | 4-lane: **empty**; [2-lane: `.00[2] = v` (`clk4p5_diven`)] | none |
| `LPTX_VREG_TSET_EN(v)` | 4-lane: empty; [2-lane `.00[3]`] | none |
| `LPTX_VREG_select(v)` | 4-lane: empty; [2-lane `.00[5:4]`] | none |
| `PowerDown_LPTX_VREG(v)` | `.00[6] = v` (`pd_ldo`) | SetLane (0) |
| `Sw_LPCD_Enable(m)` | `.00[12:8] = m & 0x1f` [2-lane `.00[10:8] = m & 7`] (`en_sw_lpcd`) | none |
| `PdHsMode(v)` | `.01[0] = v` (`pd_ib_dphy`) | SetLane (0) |
| `PdWholeDphyAnalog(v)` | `.01[1] = v` (`pd_dphy`) | SetLane (0) |
| `HsrxPreTest(v)` | 4-lane: empty; [2-lane `.01[3:2]`] | none |
| `HSRXClockSkew(en, dly)` | `.01[6:4] = dly`, `.01[7] = en` (`rxclk_dly`, `rxclk_skewen`) | none |
| `LprefTest(v)` | `.01[9:8] = v` | none |
| `TrimBit(v)` | 4-lane: empty; [2-lane `.01[11:10]` (`trim_rterm`: 0 100 %, 1 91 %, 2 111 %, 3 125 %)] | none |
| `UlpsSetting(dis_cd, sw, en, pd_ib)` | `.01[12]` … `[15]` (`dis_cd_ulps`, `sw_ulps`, `en_ulps`, `en_pd_ib_dphy_ulps`). **The 4-lane version ignores the instance and always writes 1538** (no index on `g_ptDPHYAna4Lane`) | none |
| `ClockLaneSetting(cken, en_cken, dly, lane_en, en_lane)` | `.03[0] = cken`, `.03[1] = en_cken`, `.03[15:8] = dly` (`pd_dphy_delay`), `.03[6:2] = lane_en & 0x1f` [2-lane `.03[6:4] = lane_en & 7`], `.03[7] = en_lane` | none |
| `SetLP_TXp(c, v, en)` / `SetLP_TXn` / `SetLP_TXEnable` / `SetLP_RXEnable` | LP register of channel c: `[0]/[1]`, `[2]/[3]`, `[4]/[5]`, `[6]/[7]` = (value, software-override enable). `SetLP_RXEnable(c, 0, 1)` = receiver forced off, `(c, 0, 0)` = the FSM controls it | `SetLP_RXEnable`: SetLane; the others none |
| `SetOutconf(c, v, en)` | LP register `[9:8] = v`, `[11] = en` (HS-TX output configuration) | none |
| `SetHsTxData(c, d, en)` | HS register `[7:0] = d`, `[8] = en` | none |
| `SetHsRxData(c, v, en)` | HS register `[10] = v`, `[11] = en` (`sw_hsrx_en`, `en_sw_hsrx_en`) | SetLane |
| `SetHsTxPolarity(c, p)` | select register `[0] = p` (`ds_pol`) | SetLane (DT pn_swap) |
| `SetLaneSelect(c, en_sw, sw_sel, dat_sel, swap)` | select register `[1] = en_sw`, `[4:2] = sw_sel`, `[14:12] = dat_sel`, `[15] = swap` [2-lane: `[3:2] = sw_sel & 3`, `[13:12] = dat_sel & 3`] | SetLane (DT lane_select) |
| `HsRxTxAttribute(a, b, c)` | `.18[3:0] = a`, `[7:4] = b`, `[11:8] = c` (`ckg_rx_hs_short`, `ckg_rx_hs_nor`, `ckg_bist_tx`) | SetLane (0, 0, 0) |
| `LpAttribute(f0..f4)` | 4-lane: `.19[4:2] = f0`, `.19[12:10] = f1`, `.1a[4:2] = f2`, `.1a[12:10] = f3`, `.1b[4:2] = f4` (3 bits each); [2-lane: `.19[3:2] = f0`, `.19[7:6] = f1`, `.19[11:10] = f2` (2 bits each), f3 / f4 ignored]. M5 has `ckg_rx_lp_clk/d0/d1` 5-bit fields here; M6 places one LP clock select per D-PHY channel [I] | SetLane (0s, then DT lane_select) |
| `TestModeEnable(on)` | on: `.14 = 0x4000`; off: `.14 = 0` (M5 `dummy0`) | SetLane (1) |
| `SkipHsInHSRXMode(clk, data)` | 4-lane: `.16 = 0x1313` constant; [2-lane `.16[7:0] = clk`, `.16[15:8] = data`] | SetLane, `Drv_CSISetClkDataSkip` |
| `SkewDelay(lane, dly, en)` | 4-lane: **empty**; [2-lane lane 0: `.17[2:0] = dly`, `[3] = en`; lane 1: `.17[6:4]`, `[7]`] | none |
| `HSRXSetting(auto_skip, ofs)` | `.1d[0] = auto_skip`, `.1d[1] = ofs` | none |
| `SpacedOneHotDecodeErrorClear(v)` | `.1d[2] = v` | none |
| `LpBistSetting(mode, en, pat)` | `.1d[6] = mode`, `.1d[7] = en`, `.1d[15:8] = pat` | none |
| `DmuxClear(clr, en)` | 4-lane `.26[0] = clr`, `.26[1] = en`; [2-lane `.1e[0]`, `.1e[1]`] (M5 4-lane: `.27[6:5]`) | none |

**Versus Mercury5** `dphy_ana_4lane`: registers 0x00–0x01, the per-channel 0x04–0x0e / 0x20–0x25, 0x14, 0x16 and 0x18
match; 0x19–0x1b differ (M5: `ckg_rx_lp_clk/d0/d1` at [4:0] / [9:5] / [14:10]; M6: five fields at [4:2] / [12:10] across
three registers); `DmuxClear` is `.26[1:0]` in M6 vs M5 `.27[6:5]`; and the M6 4-lane HAL has empty stubs for skew delay,
trim, HS-RX pre-test, clock-divide-4.5× and LPTX VREG test / select, which the 2-lane HAL implements.

### 4.9 D-PHY digital HAL [R]

4-lane (1539 / 1541), registers `.40` and `.46`: `SoftwareReset(v)` writes `.40[0] = 0` then `.40[0] = v` (a reset pulse
when v = 1; caller SetLane); `Enable8b9bDecode` `.40[1]`; `HsDataParallelToSerialMsb_LsbSwap` `.40[2]`;
`HsRxDelayForRetiming` `.40[5:4]`; `ClearDataLaneSyncSkew` `.40[7]`; `EnableSyncPatternWith1BitError` `.40[8]`;
`LpRxDelayForRetiming` `.40[10:9]`; `EnableEscapeMode` `.40[11]`; `ClearSkewOverFlag` `.46[12]`; `SampleSkewIndexOnce`
`.46[13]`; `EnableReportRuntimeSkewIndex` `.46[14]`; `EnableDataChannelDeskew` `.46[15]` (M5 names identical). Only
`SoftwareReset` has a caller.

2-lane (153c / 1544): `SoftwareReset` = the same pulse on `.00[0]`; **all ten other `HalCsiDig_*` functions are empty**
(2-byte `bx lr`).

### 4.10 Remaining CSI functions and the debug surface [R]

| function | behaviour |
|---|---|
| `HalCsiSetLaneNum(pad, n)` / `HalCsiGetLaneNum(pad)` | write / read `g_CSILaneSupportNum[pad]` (pad ≥ 4: no-op / 0). No caller; probe writes the array directly |
| `HalCsi_SetIoPad`, `HalCsiMipi4Lane_MipiRx11Sel`, `HalCsiMipi4Lane_SelRx11Clk`, `HalCsi_RegInit`, `HalCsi_RegUninit` | empty |
| `HalCsi_Dummy` | returns 0 |
| `Drv_CSIShowInts` | empty |
| `Drv_CSIIntShow(pad, buf, len)` | `CamOsSnprintf` of the 15 counters `csi_err_int_count[pad][0..14]` as `"%s            : %d\n"` with the names `ERR_INT_DT` … `ERR_INT_OVERRUN`; returns the length |
| `csi_open` / `fread` / `fwrite` / `release` / `suspend` / `resume` | return 0 |
| `csi_ioctl(f, cmd, arg)` | `(cmd >> 8) & 0xff != 'C'` → −25 (ENOTTY). cmd − 0x40104301 = 0..5 (i.e. `_IOW('C', 1..6, 16)`): copy 16 bytes `{u32 pad, a, b, c}` (bad pointer → −14); 1 → `Drv_CSISetClk(pad, a)`; 2 → `Drv_CSISetLane(pad, a & 0xffff, b & 0xff)`; 3 → `Drv_CSISetVC0HSMode(pad & 0xffff, b, a & 0xffff)`; 4 → `Drv_CSISetLongPacket(pad, a, b, c)` (each & 0xffff); 5 → `Drv_CSISetClkDataSkip(pad, a & 0xff, b & 0xff)`; 6 → `Drv_CSISetHDRMode(pad & 0xffff, b, a & 0xff)`; other numbers print `"Invalid ioctl command"` (log bit 1) and return 0. Each command traces its arguments when log bit 0 is set (it is): `IOCTL_CSI_SET_CLK`, `IOCTL_CSI_SET_LANE`, `IOCTL_CSI_SET_VC0HSMODE`, `IOCTL_CSI_SET_LONG_PKT`, `IOCTL_CSI_SET_CLK_DATA_SKIP`, `IOCTL_CSI_SET_HDR_MODE` with their arguments |
| `csi_ints_show` | for pads 0..3: `"== MIPI Sensor-%d ==\n"` + `Drv_CSIIntShow` + `"\n"` into a 4 KiB page |
| `csi_dbg_mask_store(buf)` | `sscanf("%d %x")` with defaults pad 0, mask 0xffff; prints `"Sensor Pad %d new mask 0x%X"`; `HalCsiMac*_ErrIntMaskSet(i, mask)` for pads 0–3 (other pads: nothing) |
| `csi_dbg_mask_show` | `"sensor N csi dbg mask = 0x%x\n"` for N = 0..3 from `ErrIntMaskGet` |
| `csi_create_bin_file(dev)` / `csi_remove_bin_file(dev)` | create / remove the sysfs attributes `csi_dbg_mask` and `csi_ints` on the `csi0` device; NULL dev → −22 |

The sysfs files are at `/sys/devices/virtual/mstar/csi0/{csi_ints,csi_dbg_mask}` on the camera (boot-log and shell captures
read `csi_ints` there). The `/dev/csi` misc device exists, but no user-space code on the camera is known to open it [I].

### 4.11 Registers never written
<!-- ev:mhal-csi-snr.4.11 -->

No function on any path writes these (no caller in, which holds every CSI function) [R]:
MAC `sw_rst` / `mac_en` (`153a.20[1:0]`), `vc_en` (`.00[12:9]`), `ecc_off`, `fun_md`, `rm_err_sot` / `rm_err_fs`, the SOT
/ EOT source (`.21`), the vsync mode (`.02`), the error / report / PHY / lane-FSM interrupt masks (the error mask only by
the ISR and `csi_dbg_mask`); everything in 1539 except the reset bit (8b9b decode, escape mode, deskew); and in 1538 skew,
ULPS, LPCD, clock-lane override, LP-TX / HS-TX test drive, BIST. They run at their reset defaults, whose values are not
known [I]. They are readable on the running camera as a dump of banks 1538 / 1539 / 153a (`riu_r 1538`, `riu_r 1539`,
`riu_r 153a`), which would also show which VCs are passed. The vendor sequence relies on these defaults for MAC enable and
VC enable: writing exactly the registers of §4.3–§4.5 reproduces the vendor state [I].
