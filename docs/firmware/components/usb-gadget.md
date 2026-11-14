# usb-gadget — the USB device port: `udc-msb250x.ko`, `usb_f_uvc.ko`, `g_webcam.ko`
<!-- ev:usb-gadget -->

The camera's only USB connector is a USB 2.0 device port. Three vendor kernel modules make it a webcam and a network
adapter: the device-controller driver `udc-msb250x.ko` (Mentor MUSB-style "MSB250x" core with its UTMI PHY), the UVC
function `usb_f_uvc.ko` (f_uvc + uvc_video + uvc_queue + uvc_v4l2 + uvc_configfs) and the legacy composite gadget
`g_webcam.ko`, which carries the UVC descriptor tables and adds an RNDIS function. This document covers the three modules,
the descriptors and formats the camera offers to the host, and what was observed on the link.

Elsewhere: RIU addressing and the static I/O map, interrupt numbers and the device tree as a whole, the vendor change to
`struct usb_request` — `kernel-platform.md` (§3.1 "Static I/O map and RIU addressing", §15.3 "Vendor struct changes"); the USB3
PHY driver and the EHCI host module `ehci-hcd.ko`, which programs this port's PHY at boot — `kernel-drivers.md` (§4 "USB host
side and PHYs"); the inventory rows of the modules and of the stock gadget / V4L2 pieces — `firmware-image.md` §3.4; the
`demo.sh` load lines and the RNDIS network set-up — `rootfs-customer.md` §3.2, §3.3; the XU units and controls prog_uvc answers, its
format table and its use of the QBUF `reserved` word — `prog_uvc.md` §4.4, §4.6, §3.1, §3.5.

Tags: **[R]** read from a `.ko`, its disassembly, the DTB, a source file compiled and compared, a host or camera
log; **[S]** stated by a source (a vendor source tree, header or comment); **[I]** inference. Binaries:
`/config/modules/4.9.227/{udc-msb250x,usb_f_uvc,g_webcam}.ko` on the camera (vermagic `4.9.227 SMP preempt mod_unload
ARMv7 thumb2 p2v8`, built with gcc 9.1 per `.comment` and with stack protector) [R].
Addresses below load `.text` of these modules at 0x10000 + the section's file offset (usb_f_uvc `.text` at 0x10028).
Register notation: kernel virtual 0xFD28xxxx = phys 0x1F28xxxx (`kernel-platform.md` §3.1); `rN.lo` / `rN.hi` = the low /
high byte of 16-bit register N of a bank, at base + N·4 and base + N·4 + 1.

## 1. The stack at a glance

| module | role | source relation | on this camera |
|---|---|---|---|
| `udc-msb250x.ko` (§2) | USB 2.0 device controller + UTMI PHY bring-up | "5 June 2019" driver: tipoman9 4.9 core + the i6c 5.10 PHY-tuning additions + edits found in no public copy | loaded by `demo.sh`; high speed |
| `usb_f_uvc.ko` (§3) | UVC function (V4L2 output device `/dev/videoN`) | OpenIPC i6c 5.10 `function/` base with the 4.9-line video / queue path and open / release model | used by g_webcam, isochronous, single stream |
| `g_webcam.ko` (§4) | legacy composite gadget: UVC + RNDIS, descriptor tables | tipoman9 `legacy/webcam.c` with ODM-edited tables | insmodded by prog_uvc |

Public source copies used for the comparisons [R] (no public SigmaStar kernel tree for Mercury6 at 4.9.227 exists; the drivers
exist in other SigmaStar trees — 4.9.84 for I6E / I2M, 5.10.61 for I6C with M6 platform files):

| short name | tree | parts used |
|---|---|---|
| tipoman9 | tipoman9/sigmastar-infinity6e (I6E SDK, 4.9.84), github.com/tipoman9/sigmastar-infinity6e | `drivers/usb/gadget/{legacy,function}/`, the udc driver |
| TAKOYAKI | ZYCX8888/Democode-TAKOYAKI-BETA001-0312 (I6E demo SDK, 2020-04) | the udc driver; older f_uvc / webcam |
| i6c | OpenIPC/linux branch `sigmastar-infinity6c` (SigmaStar SDK kernel 5.10.61), github.com/OpenIPC/linux/tree/sigmastar-infinity6c | `drivers/sstar/usb/gadget/udc/usb20/`, `drivers/sstar/usb/gadget/function/`, `include/mercury6/msb250x_udc_common.h` |
| upstream | vanilla 4.9.227 | `drivers/usb/gadget/function/` |
| skyipc | skyipc_tiramisu, SSD268G / SSC8836Q UVC app | `backup/uvc_3formats_7resolutions/webcam.c` |

Method of the body comparison [R]: each candidate source copy was compiled as an out-of-tree module against a 4.9.227 tree with
the camera's module ABI (vendor headers that differ from vanilla laid over it: i6e 4.9.84 `linux/usb/{composite,gadget,video}.h`,
`uapi/linux/videodev2.h`; for the 5.10 copies the i6c `g_uvc.h` / `video.h`; one-line shims for 5.10 API names `__poll_t`,
3-argument `vb2_qbuf`, `VFL_TYPE_VIDEO`, `V4L2_PIX_FMT_HEVC`, `USB_TEST_*`; build options `CONFIG_SS_GADGET`, `ARCH_SSTAR`; udc
i6c with the `mercury6` header, udc i6e with `infinity6e`). Per function of the binary and of each build a fingerprint was taken —
direct calls, referenced external symbols and function pointers, strings, constants (immediates, `movw/movt` pairs, literal-pool
words outside switch tables) and effective MMIO addresses (base register + displacement, so `0xFD284C00 + 4` and a pooled
`0xFD284C04` compare equal) — and compared, dropping items one side reaches through a local callee (gcc 9.1 vs the host gcc 15
inline differently), matching constants 0x100–0xFFFF within ±0x40 (structure offsets move with header revisions), and ignoring
build-option calls (`__stack_chk_fail`, `memset`↔`__memzero`, `usb_ep_*` and spinlock helpers inline in one header revision,
`arm_heavy_mb`). Every function with a finding, and every function only one side has, was read in the disassembly against the
source. Blind spots [I]: changed constants < 0x100 and conditions with the same calls, strings and large constants; for the
udc these were checked by reading every function that touches registers differently.

The modules loaded at run time are listed in the `Modules linked in` line of the oops in §2.7 [R]: the three vendor modules on top
of `udc-core`, `libcomposite`, `u_ether`, `usb_f_rndis` and the videobuf2 modules, which with `usb-common` are the stock 4.9
gadget / V4L2 pieces (`firmware-image.md` §3.4).

Load path [R]: `/customer/demo.sh` loads `ehci-hcd.ko` (line 5, which programs this port's UTMI / USBC for the host side first —
`kernel-drivers.md` §4.2), `udc-core.ko` (line 66) and `udc-msb250x.ko` (line 71), all without parameters. `/customer/usb_otg.sh`
[R script]: option `device` loads the whole stack (media, videodev, v4l2-common, usb-common, videobuf2-*, udc-core, libcomposite,
usb_f_uvc, u_ether, usb_f_rndis, udc-msb250x without parameters at line 22, then `g_webcam.ko streaming_maxpacket=3072
streaming_maxburst=13 uac_function_enable=0 rndis_function_enable=0`); `mass_storage` loads `usb_f_mass_storage.ko`;
`rmmod_device` removes `g_webcam`. kjd_lvgl's USB-disk mode runs `usb_otg.sh rmmod_device` and then insmods
`usb_f_mass_storage.ko` and `g_mass_storage.ko file=/dev/%s removable=1` [R strings] (`kjd_lvgl.md` §9.1). prog_uvc runs
`insmod g_webcam.ko streaming_maxpacket=3072 uvc_function_enable=1 uac_function_enable=0 rndis_function_enable=1`;
`uac_function_enable` does not exist in this build, so the kernel logs "unknown parameter ignored" and loads the module anyway
[I: 4.9 `unknown_module_param_cb` behaviour]. The gadget appears on the bus only when prog_uvc opens `/dev/videoN` (§3.3).

## 2. `udc-msb250x.ko` — USB 2.0 device controller

### 2.1 Source relation [R]
<!-- ev:usb-gadget.2.1 -->

The driver of `drivers/sstar/usb/gadget/udc/usb20/` (files `msb250x_udc.c`, `_ep.c`, `_gadget.c`, `_dma.c`; `DRIVER_VERSION
"5 June 2019"`, the same string in tipoman9, TAKOYAKI and i6c). By name every binary symbol exists in tipoman9 except the
additions of §2.5, and in i6c without exception, together with an `include/mercury6/msb250x_udc_common.h` holding the same three
register bases (name check counted 93 functions; the fingerprint comparison counts 70 named functions + 2 without
symbols — `msb250x_udc_write_riu` at 0x11448 and `msb250x_udc_read_riu` at 0x11b84, matched by fingerprint). By body
the binary is **the 4.9 vendor line (tipoman9 core) with the i6c PHY-tuning additions, plus edits found in neither copy**:
against the i6c 5.10 build 41 of 72 fingerprints are identical, against tipoman9 33 of 61; the rest are mostly MMIO-address
artefacts (gcc 9 reaches registers through a base register the tracker loses across branches). Every function whose source
differs between the two copies (35 functions) and every function with a
non-artefact finding was read in the disassembly:

| function (source file) | binary follows | what that means |
|---|---|---|
| `tx_swing_*`, `cm_current_*`, `de_emphasis_current_*`, `test_mode_store`, `msb250x_ep0_test_mode` | i6c (only there) | §2.5; the "… unknown" show strings are i6c's |
| `msb250x_ep_queue`, `msb250x_gadget_{map,sync}_request`, `msb250x_gadget_match_ep`, `msb250x_udc_resume`, `msb250x_ep_isr_handler` | i6c | `ep_queue` takes `dev->lock` around the checks (§2.6); RX bulk short-packet handling through `shortPkt` + `ok2rcv_for_packets(cfg, 0)` |
| `msb250x_udc_init_otg` | tipoman9 | no `bEnableSuspendM`, **suspend interrupt not enabled** (INTRUSBE untouched) — the i6c copy enables both |
| `msb250x_udc_init_usb_ctrl` | tipoman9 | USBC reg 0: `OTG_RST\|REG_SUSPEND` then `XIU_ENABLE`; no USBC reg 5 resume / reset interrupt enables (i6c has them) |
| `msb250x_gadget_pullup_i` | tipoman9 | print without `KERN_INFO` |
| `msb250x_ep_free_request` | tipoman9 | returns silently for NULL ep / req or an un-enabled non-ep0 endpoint |
| `msb250x_dma_setup_control`, `msb250x_dma_isr_handler` | tipoman9 | **32-bit DMA addresses only** — no USBC bit 4 / `USBC_CTRH` bits 7:4 address extension (i6c) |
| `msb250x_gadget_udc_start` | neither | `init_usb_ctrl` → `reset_otg` → `init_otg` → `mdelay(1)` (`__const_udelay(0x0CCCCB60)`) → "UDC start"; **no `init_utmi`** (tipoman9 calls `msb250x_udc_deinit_utmi()` + `msb250x_udc_init_utmi()` before `init_usb_ctrl`, i6c calls `init_utmi`) |
| `msb250x_gadget_udc_stop` | tipoman9 | clears the seven `ep.desc`, driver pointers, then `init_usb_ctrl`; no `udc_disable` / `deinit_utmi` (i6c) |
| `msb250x_udc_remove` | neither | `-EBUSY` while a driver is bound; free IRQ, remove the four sysfs files, `usb_del_gadget_udc`; no disable, no `deinit_utmi` |
| `msb250x_udc_deinit_utmi` | — | absent from the binary |
| `msb250x_udc_probe` | i6c without the DT read | no `"tx-swing,de-emphasis,cm-current"` property — tuning only from module parameters (§2.5) |
| `msb250x_udc_init_utmi` | i6c without UTMI2 write and OTP trim | §2.4 |
| `msb250x_udc_link_isr` (inlined in `msb250x_udc_isr`) | **neither** | bus reset **does not touch UTMI reg 0x16** — both copies rewrite it (tipoman9: 0x10 / 0x02 bytes at reset start, 0x0230 HS / 0x0030 FS after; i6c: the same values merged with the tuned bits) |
| `msb250x_ep_set_halt` | **neither** | IN endpoint, halt: refuses with "fifo busy, cannot halt" and `-EAGAIN` when TXCSR1 bit 1 (FIFONotEmpty) is set (tipoman9); IN, clear: writes TXCSR1 = old \| 0x40 (ClrDataTog) and then **0**, continues the request if TxPktRdy was clear (both copies: FlushFIFO when TxPktRdy, no ClrDataTog); OUT: SendStall / clear + ClrDataTog (both); ep0: CSR0 = 0x60 (ServicedRxPktRdy + SendStall) |
| `msb250x_udc_get_status` | i6c limit | §2.6 |
| `msb250x_ep0_isr_handler` (SET_FEATURE TEST_MODE) | i6c | §2.6 |
| all other functions (≈35) | both copies are equal there; binary equal in calls, strings, large constants and MMIO addresses | small constants and conditions not read [I: same code] |

`msb250x_ep0_clr_opr_set_ss` exists only in TAKOYAKI, not in the binary.

### 2.2 Registers and device tree [R]
<!-- ev:usb-gadget.2.2 -->

Three banks, compiled in as kernel-virtual addresses (literal pools), identical to tipoman9 `include/infinity6e/msb250x_udc_common.h`
and to i6c `include/mercury6/msb250x_udc_common.h` (mercury6 header: `RIU_BASE 0xFD200000`, UTMI bank written 0x42100, so
phys = 0x1F000000 + 0x142100·2):

| block | kernel virtual | phys | RIU bank | access |
|---|---|---|---|---|
| UTMI (PHY) | 0xFD284200 | 0x1F284200 | 0x1421 | 8- and 16-bit; the binary touches r00–r05, r0A, r16, r17, r1E, r3C only (literal-pool UTMI words 0xFD28420C / 10 / 58 / 5C / 78 / F0) |
| USBC (port control) | 0xFD284600 | 0x1F284600 | 0x1423 | r00.lo (`init_usb_ctrl`), r01.lo, r0A–r0C (`init_utmi`) — nothing else |
| OTG0 = MUSB core | 0xFD284A00 | 0x1F284A00 | 0x1425 | byte registers, FIFOs at +0x40…, DMA at +0x200…, CFG registers at +0x100… |

MUSB core registers the binary uses (names from `msb250x_udc_reg.h`):

| register | address (virtual) | use |
|---|---|---|
| FADDR | 0xFD284A00 | function address (`wValue & 0x7F` on SET_ADDRESS; 0 in `init_otg` and on bus reset) |
| POWER | 0xFD284A01 | bit 6 SoftConn (pull-up), bit 5 HSEnab, bit 4 HS mode (speed read on reset), bit 7 set on isoc ep enable if clear; `init_otg`: `(v & 0x9D) \| (max_speed == HIGH) << 5` |
| INTRTXE / INTRRXE | 0xFD284A0C / 0xFD284A10 | per-endpoint interrupt enables (`\|= 1 << ep`) |
| INTRUSB | 0xFD284A14 | bit 2 = reset; `init_otg` writes 0xE7 here (not INTRUSBE 0xFD284A15) and reads it back |
| INDEX | 0xFD284A1C | 0 in `init_otg` |
| TESTMODE | 0xFD284A1D | J = 0x02, K = 0x04, SE0_NAK = 0x01, PACKET = 0x08 |
| CSR0_FLSH | 0xFD284A25 | 0x01 = ep0 FIFO flush |
| FIFO0 / FIFOn | 0xFD284A40 + ep·8 | `__raw_writesb` / `__raw_readsb` |
| DEVCTL | 0xFD284AC0 | 0 in `init_otg` |
| CFG0_L | 0xFD284B00 | bit 0 = SRST_N (soft reset of the OTG core, low active) |
| CFG0_H | 0xFD284B01 | bit 7 bDMPullDown |
| CFG6_H | 0xFD284B19 | `\|= 0xE8` = INT_WR_CLR_EN 0x08 \| ShortMode 0x20 \| BusOPFix 0x40 \| MI_WDFIFO_CTRL 0x80 |
| TXMAP / RXMAP | 0xFD284A00 + (0x100 / 0x104 + ep·0x10)·2 | wMaxPacketSize (16-bit) |
| CSR0 | 0xFD284C04 (`GET_REG8_ADDR(OTG0, 0x102)`) | ep0 control / status |
| COUNT0 | 0xFD284C10 | ep0 received byte count (16-bit `ldrh`) |

DT [R: built-in DTB]: node `/soc/Sstar-udc`, `compatible = "sstar,infinity-udc"`,
`interrupts = <GIC_SPI 0x40 IRQ_TYPE_LEVEL_HIGH>` (`<0x0 0x40 0x4>`), `status = "ok"`, no `reg` (the bases are compiled in). The
separate `/phy` node (`sstar,sstar-usb-phy`, `io_phy_addr = <0x1f000000>`, `banks = <0x1523 0x1524 0x1525>`, `reg = <0x1f2a4600
0x300>`, `phy_type = "utmi"`) belongs to the built-in USB3 PHY driver (`kernel-drivers.md` §4.1), not to this port.

### 2.3 What the driver does on this camera, in order [R]

- **Load** (`demo.sh`, after `ehci-hcd.ko`, which already calibrated this UTMI and switched the port owner to the host side —
  `kernel-drivers.md` §4.2.3): `probe` → `udc_disable` (INTRUSBE / INTRTXE / INTRRXE = 0, read-clear INTRUSB / TX / RX,
  pull-up off), seven endpoints with their maxpacket limits, IRQ (DT SPI 0x40), `usb_add_gadget_udc`, 64-bit DMA mask, four
  sysfs files, **`init_utmi`** (§2.4: MIU select bytes, UTMI reg0 = 1, calibration, tuning 115 / 105 / 110), state POWERED,
  "complete porbe" (sic). The port owner byte that ehci-hcd set to host (`kernel-drivers.md` §4.2.2's "USBC 0x02" is an 8-bit RIU
  address = CPU 0xFD284604 = USBC r01.lo, bits 1:0 = 01) is set back to bits 1:0 = **10** (device / OTG side) by this
  `init_utmi` (§2.4 step 7).
- **Gadget bind** (g_webcam) → `udc_start`: `init_usb_ctrl` (USBC reg 0), `reset_otg`, `init_otg` (§2.4: DM pull-down, HSEnab,
  no soft-connect, 0xE7 = all bits but Conn / SOF written to INTRUSB, ep0 FIFO flush, ep0 TX / RX interrupts on), 1 ms. The PHY is **not** re-initialised here; it stays as `probe` left it. No other udc function touches the
  USBC bank except reg 0 in `init_usb_ctrl` [R: MMIO address sets of all 72 functions].
- **Connect**: `pullup(1)` (POWER bit 6) when prog_uvc opens `/dev/videoN` (§3.3).
- **Bus reset IRQ**: driver `disconnect` if state ≥ DEFAULT, FADDR 0, ep0 idle, speed from POWER bit 4 (HS / FS, printed),
  autoNAK configs 1–2 released, state DEFAULT — PHY untouched (§2.1, `msb250x_udc_link_isr`).
- **udc-core sysfs `soft_connect` "disconnect" → "connect"**: from udc-core's `usb_udc_softconn_store`, "disconnect" =
  `usb_gadget_disconnect` (pull-up off) → driver `disconnect` → `udc_stop` (which runs `init_usb_ctrl`); "connect" = `udc_start`
  → `usb_gadget_connect` (pull-up on) — again no PHY init. The only path in the driver that re-runs `init_utmi` after probe is
  `msb250x_udc_resume` (platform resume), in the order `init_utmi` → `init_usb_ctrl` → `reset_otg` → `init_otg` → `mdelay(1)` →
  `pullup(1)`.

### 2.4 PHY and core bring-up [R]
<!-- ev:usb-gadget.2.4 -->

`msb250x_udc_init_utmi` (`.text.unlikely` 0x114–0x2d8) = the i6c source sequence with
`ENABLE_OTG_USB_NEW_MIU_SLE` on, without i6c's UTMI2 write and OTP trim (source lines 1238–1244, 1295–1306) and without the DT
override of the three tuning values. Order as in the disassembly ("src" = line of the i6c `msb250x_udc.c`):

| # | address (reg) | width | operation | src |
|---|---|---|---|---|
| 1 | 0x1F284628 USBC r0A.lo | 8 | = 0x70 (MIU select) | 1156 |
| 2 | 0x1F28462C USBC r0B.lo | 8 | = 0xEF | 1157 |
| 3 | 0x1F28462D USBC r0B.hi | 8 | = 0xEF | 1158 |
| 4 | 0x1F284630 USBC r0C.lo | 8 | = 0xEF | 1159 |
| 5 | 0x1F284631 USBC r0C.hi | 8 | \|= 0x01 | 1160 |
| 6 | 0x1F284210 UTMI r04 | 16 | = 0x0C2F | 1211 |
| 7 | 0x1F284604 USBC r01.lo | 8 | & 0xFC \| 0x02 (port owner → device / OTG) | 1216 |
| 8 | 0x1F284200 UTMI r00 | 16 | read; if ≠ 0x0001: = 0x0001 (one `strh`), mdelay(3) | 1233–1236 |
| 9 | 0x1F284278 UTMI r1E.lo | 8 | \|= 0x01 (CA_START), mdelay(10) | 1247–1249 |
| 10 | 0x1F284278 → 0x1F2842F0 UTMI r3C.lo | 8 | read 0x…278, write (v & 0xFE) to 0x…2F0 | 1250 |
| 11 | 0x1F284278 | 8 | poll until bit 1 (CA_END) = 1, no timeout | 1252 |
| 12 | 0x1F28420C UTMI r03.lo | 8 | & 0x9F \| 0x40 | 1257 |
| 13 | 0x1F284205 UTMI r01.hi | 8 | \|= 0x28 | 1259 |
| 14 | 0x1F284205 UTMI r01.hi | 8 | &= 0xEF | 1261 |
| 15 | 0x1F28420D UTMI r03.hi | 8 | &= 0xFD | 1263 |
| 16 | 0x1F284211 UTMI r04.hi | 8 | \|= 0x81 | 1266 |
| 17 | 0x1F284229 UTMI r0A.hi | 8 | \|= 0x20 | 1268 |
| 18 | 0x1F284215 UTMI r05.hi | 8 | \|= 0x80 | 1270 |
| 19 | 0x1F284208 UTMI r02.lo | 8 | \|= 0x80 | 1292 |
| 20 | 0x1F284258 UTMI r16.lo | 8 | tx_swing 115: & 0x8F \| 0x30 | 1308 |
| 21 | 0x1F284258 UTMI r16 | 16 | dem_cur 105: & ~0x0380 \| 0x0200 | 1309 |
| 22 | 0x1F28425C UTMI r17.lo | 8 | cm_cur 110: & 0xC7 \| 0x10 | 1310 |

- The MIU select bytes 0x70 / 0xEF / 0xEF / 0xEF are printed at init.
- The delays are `arm_delay_ops.const_udelay(0x0CCCCB60)` = udelay(1000) at HZ = 100, called 3 and 10 times. Step 8 is a single 16-bit `strh`.
- Step 10 is a vendor typo (`GET_REG16_ADDR` where `GET_REG8_ADDR` is meant), compiled into the binary: CA_START in 0x1F284278
  is never cleared, and the cleared value lands in 0x1F2842F0 (UTMI r3C, "UTMI 0x3C" in 16-bit register numbering).
- UTMI registers 0x1C and 0x60 (as 16-bit index r1C / r60 or as 8-bit RIU address = r0E / r30): neither the tipoman9 nor the i6c
  udc sources and headers name or touch them, the binary does not write them, and ehci-hcd's bring-up list does not contain them.

The rest of the bring-up (binary vs tipoman9: no difference):
- `msb250x_udc_init_usb_ctrl`: USBC r00.lo 0xFD284600 = `(v & 0xF3) | 0x0C` (OTG_RST + REG_SUSPEND), then `(v & 0xBB) | 0x40`
  (OTG_RST off, OTG_XIU_ENABLE) — result: OTG out of reset, XIU register access on, bit 3 suspend still set.
- `msb250x_udc_reset_otg`: CFG0_L 0xFD284B00 bit 0 cleared then set (soft-reset pulse of the OTG core).
- `msb250x_udc_init_otg`: read CFG0_L (discarded); CFG0_H |= 0x80; FADDR = 0; CFG6_H |= 0xE8; POWER = `(v & 0x9D) | HSEnab`
  (SoftConn cleared, i.e. disconnected); DEVCTL = 0; INTRUSB 0xFD284A14 = 0xE7, read back; INDEX = 0; CSR0_FLSH = 0x01;
  INTRTXE = 0x01; INTRRXE = 0x01; read INTRTXE — same order as tipoman9.
- `msb250x_gadget_pullup` / `msb250x_gadget_pullup_i`: `soft_conn` ← POWER bit 6; when `is_on == 1` and the bit is clear, or
  `is_on != 1` and the bit is set, `pullup_i` writes POWER bit 6 = `is_on`; `soft_conn = is_on`. The device stays logically
  disconnected after `init_otg` until `pullup(1)`.

### 2.5 PHY tuning parameters and USB-IF test modes [R]
<!-- ev:usb-gadget.2.5 -->

Present in i6c 5.10, absent from the 4.9.84 copies:

| feature | interface | registers |
|---|---|---|
| TX swing | param `tx_swing` (uint) + sysfs `tx_swing` (show / store) | UTMI+0x58 byte (r16.lo), bits 6:4: 80 %→4, 85→5, 90→6, 95→7, 100→0, 105→1, 110→2, 115→3 |
| CM current | param `cm_cur` + sysfs `cm_current` | UTMI+0x5C byte (r17.lo), bits 5:3: 105 %→3, 110→2, 115→1, 120→0, 125→5, 130→4 |
| de-emphasis | param `dem_cur` + sysfs `de_emphasis_current` | UTMI+0x58 u16 (r16), bits 9:7: 100 %→0, 105→4, 110→5, 115→6, 120→7 |
| USB-IF test modes | sysfs `test_mode` ("TEST_J", "TEST_K", "TEST_SE0_NAK", "TEST_PACKET") + SET_FEATURE(TEST_MODE) on ep0 (`msb250x_ep0_test_mode`) | MUSB TESTMODE 0xFD284A1D (J = 2, K = 4, SE0_NAK = 1, PACKET = 8 + the 53-byte test packet written to FIFO0 0xFD284A40, then `set_de_in`) |

Values are percentages (e.g. `tx_swing=100`); other values print "unknown". Defaults from `.data` 0x40 / 0x44 / 0x48 (where
`__param` maps `tx_swing`, `dem_cur`, `cm_cur`) = **115 / 105 / 110**; `probe` reads no DT property for them. Neither `demo.sh`,
`usb_otg.sh` nor prog_uvc passes these parameters, so the PHY runs with these defaults. The 53-byte test packet sits at `.rodata`
0x82–0xB6, bytes identical to tipoman9 (`msb250x_ep.c:778–783`); `msb250x_ep0_test_mode` lives in `.text.unlikely` 0x48–0x113.

### 2.6 EP0 and endpoint handling [R]
<!-- ev:usb-gadget.2.6 -->

Read in the binary against tipoman9.
- **ep0 helpers** (`clear_opr`, `clear_sst`, `clear_se`, `set_ipr`, `set_de`, `set_ss`, `clr_opr_set_ss`, `set_de_out`,
  `set_sse_out`, `set_de_in`): each writes one byte to CSR0 0xFD284C04 — 0x40 / 0x00 / 0x80 / 0x02 / 0x08 / 0x20 / 0x60 / 0x48 /
  0xC0 / 0x0A — with `dsb st` before the store; as tipoman9.
- **`msb250x_ep_enable`**: TXMAP / RXMAP = wMaxPacketSize; TXCSR1 = 0x48, TXCSR2 = 0x20 (+0x40 isoc), RXCSR1 = 0x90, RXCSR2 =
  0x40 isoc else 0; bulk OUT → autoNAK configuration; `fifo_size = 1 << (FIFOSIZE & 0x0F)` for both directions; INTRTXE / INTRRXE
  |= 1 << ep; isoc → POWER bit 7 set if clear; maxpacket & 0x7FF; as tipoman9.
- **`msb250x_ep0_isr_handler` + `msb250x_ep0_handle_idle`** (inlined): CSR0 read, SentStall / SetupEnd handling, state switch,
  COUNT0 == 8 check, 8-byte `readsb` from FIFO0, `req_std` / `req_pending` / `delay_status`, SET_ADDRESS (FADDR = wValue & 0x7F,
  state ADDRESS, `clear_opr`), GET_STATUS, CLEAR / SET_FEATURE halt, SET_CONFIGURATION / SET_INTERFACE with `delay_status`,
  default `clear_opr`, direction / wLength → ep0 state, the gadget's `setup()` call, stall 0x60 on error — as tipoman9, with one
  difference: SET_FEATURE, recipient device, wValue = TEST_MODE calls `msb250x_ep0_test_mode(wIndex >> 8)` and then **does not
  return** — it falls through to the direction / wLength code and the gadget `setup()` call (= i6c); tipoman9 writes TESTMODE inline
  and returns without calling `setup()`.
- **`msb250x_udc_get_status`**: endpoint recipient rejects `(wIndex & 0x78) != 0`, i.e. ep_num > **7** (= i6c
  `MSB250X_MAX_ENDPOINTS` in `include/mercury6/msb250x_udc_common.h`; tipoman9's `infinity6e` header has 4). Otherwise as tipoman9:
  recipient device → `devstatus` (+0x418), interface → 0, other → return 1; wLength > 2 → 1; ep0 CSR0 & 0x20, IN TXCSR1 & 0x10,
  OUT RXCSR1 & 0x20; `udelay(5)` (`__const_udelay(0x10624C)`); status byte then 0 to FIFO0.
- **`msb250x_udc_write_riu`** (0x11448, no symbol): ep_num = desc ? `bEndpointAddress & 0xF` : 0; `pld` + `__raw_writesb(0xFD284A40 +
  ep·8, buf + actual, bytes)` only if bytes > 0; `actual += bytes`; last = `bytes != maxpacket || actual == length` →
  `request_done(…, 0)`. **`msb250x_udc_read_riu`** (0x11b84, no symbol): length 0 → 1; `__raw_readsb(0xFD284A40 + ep·8, …)` if
  bytes > 0; last = `bytes != maxpacket || req.zero`, or `actual == length`. Both as tipoman9.
- **`msb250x_request_handler`, ep0 branch**: halted → return req; set halted; INTRUSB bit 2 (reset) → return req; CSR0 read before
  the state switch; IDLE + `delay_status` → `clear_opr`, `delay_status--`, done; IN_DATA + TxPktRdy clear → min(maxpacket,
  remaining) → `write_riu` → last: `set_de_in` (0x0A) + IDLE, else `set_ipr` (0x02); OUT_DATA + RxPktRdy → 16-bit COUNT0, warning
  if > remaining, `read_riu` → last: `set_de_out` (0x48) + IDLE, else `clear_opr` (0x40); other state → printk; halted cleared.
  As tipoman9.
- **`msb250x_ep_queue`** (= i6c): checks `driver` / `gadget.speed` (→ `-ESHUTDOWN`) **before** taking `dev->lock` (tipoman9 takes
  the lock first and checks inside `__msb250x_ep_queue`); checks `complete`, `buf` and `list_empty(&req->queue)` for **every**
  request (`-EINVAL`, so a zero-length request with `buf == NULL` is refused; tipoman9 checks them only when `length != 0`); **no
  ZLP add-on** (tipoman9 queues an extra zero-length request, `msb250x_zlp_complete` / `dev->zlp_buf`, after any request with
  `zero && length && length % maxpacket == 0`, on every endpoint including ep0).

### 2.7 DMA receive path and the interrupt-time panic
<!-- ev:usb-gadget.2.7 -->

**The panic [R]** (UART log, ANSI stripped; the same signature — PC, LR and faulting
address — on 2026-10-17). The camera hangs: on 2026-10-17 until a power cycle, on 2026-10-24 it rebooted by itself about a minute
later. The oops without the raw stack words:

```
Unable to handle kernel paging request at virtual address c6700000
pgd = c0003000
[c6700000] *pgd=80000020007003, *pmd=265ff003, *pte=00000200
Internal error: Oops: 207 [#1] PREEMPT SMP THUMB2
Modules linked in: g_webcam(O) udc_msb250x usb_f_rndis u_ether usb_f_uvc libcomposite udc_core videobuf2_vmalloc videobuf2_memops videobuf2_v4l2 videobuf2_core drv_ms_cus_gc8613_MIPI(PO) fbdev(O) mi_v
CPU: 0 PID: 0 Comm: swapper/0 Tainted: P           O    4.9.227 #44
Hardware name: SStar Soc (Flattened Device Tree)
task: c0428180 task.stack: c0422000
PC is at _set_bit+0x22/0x4c
LR is at __dma_page_dev_to_cpu+0x59/0x68
pc : [<c014fc96>]    lr : [<c0014d21>]    psr: 600e01b3
sp : c0423d38  ip : 00000000  fp : 00000200
r10: bf8a9cb8  r9 : 00000100  r8 : bf8a9ca8
r7 : c046df80  r6 : c0427e44  r5 : 00038181  r4 : 001258ce
r3 : 00000200  r2 : 00000001  r1 : c6700000  r0 : 00000000
Flags: nZCv  IRQs off  FIQs on  Mode SVC_32  ISA Thumb  Segment user
Control: 70c5383d  Table: 24578300  DAC: 1218fe1d
Process swapper/0 (pid: 0, stack limit = 0xc0422218)
Stack: (0xc0423d38 to 0xc0424000)      [words not copied; they include fd284c8c, inside the OTG0 DMA block [I]]
[<c014fc96>] (_set_bit) from [<c0014d21>] (__dma_page_dev_to_cpu+0x59/0x68)
[<c0014d21>] (__dma_page_dev_to_cpu) from [<bf8a603b>] (msb250x_gadget_sync_request+0xc/0xe [udc_msb250x])
[<bf8a603b>] (msb250x_gadget_sync_request [udc_msb250x]) from [<bf8a665d>] (msb250x_dma_isr_handler+0x1f8/0x23c [udc_msb250x])
[<bf8a665d>] (msb250x_dma_isr_handler [udc_msb250x]) from [<bf8a6e77>] (msb250x_ep_isr_handler+0x112/0x160 [udc_msb250x])
[<bf8a6e77>] (msb250x_ep_isr_handler [udc_msb250x]) from [<bf8a7951>] (msb250x_udc_isr+0x18e/0x1f2 [udc_msb250x])
[<bf8a7951>] (msb250x_udc_isr [udc_msb250x]) from [<c00463ef>] (__handle_irq_event_percpu+0x2d/0xa2)
[<c00463ef>] (__handle_irq_event_percpu) from [<c0046473>] (handle_irq_event_percpu+0xf/0x30)
[<c0046473>] (handle_irq_event_percpu) from [<c00464bb>] (handle_irq_event+0x27/0x3c)
[<c00464bb>] (handle_irq_event) from [<c00486b5>] (handle_fasteoi_irq+0x75/0xc0)
[<c00486b5>] (handle_fasteoi_irq) from [<c0045c29>] (generic_handle_irq+0xd/0x16)
[<c0045c29>] (generic_handle_irq) from [<c0045fa7>] (__handle_domain_irq+0x63/0x74)
[<c0045fa7>] (__handle_domain_irq) from [<c0009253>] (gic_handle_irq+0x3f/0xac)
[<c0009253>] (gic_handle_irq) from [<c000fc53>] (__irq_svc+0x53/0x94)
Exception stack(0xc0423e88 to 0xc0423ed0)
[<c000fc53>] (__irq_svc) from [<c001f95c>] (__do_softirq+0x44/0x158)
[<c001f95c>] (__do_softirq) from [<c001fbe7>] (irq_exit+0x37/0x78)
[<c001fbe7>] (irq_exit) from [<c0045f91>] (__handle_domain_irq+0x4d/0x74)
[<c0045f91>] (__handle_domain_irq) from [<c0009253>] (gic_handle_irq+0x3f/0xac)
[<c0009253>] (gic_handle_irq) from [<c000fc53>] (__irq_svc+0x53/0x94)
Exception stack(0xc0423f58 to 0xc0423fa0)
[<c000fc53>] (__irq_svc) from [<c005a676>] (tick_nohz_idle_enter+0x36/0x44)
[<c005a676>] (tick_nohz_idle_enter) from [<c003fc79>] (cpu_startup_entry+0x41/0xd0)
[<c003fc79>] (cpu_startup_entry) from [<c03f791f>] (start_kernel+0x27f/0x2e8)
Code: f8b1 f000 fa02 f303 (e851) 2f00
---[ end trace 39c89f408c5ff2d3 ]---
Kernel panic - not syncing: Fatal exception in interrupt
CPU1: stopping
CPU: 1 PID: 0 Comm: swapper/1 Tainted: P      D    O    4.9.227 #44
[<c0011ed9>] (unwind_backtrace) from [<c000f59f>] (show_stack+0xb/0xc)
[<c000f59f>] (show_stack) from [<c01503c1>] (dump_stack+0x61/0x78)
[<c01503c1>] (dump_stack) from [<c0011503>] (handle_IPI+0x87/0x100)
[<c0011503>] (handle_IPI) from [<c00092ad>] (gic_handle_irq+0x99/0xac)
[<c00092ad>] (gic_handle_irq) from [<c000fc53>] (__irq_svc+0x53/0x94)
Exception stack(0xc608df98 to 0xc608dfe0)
[<c000fc53>] (__irq_svc) from [<c000dbd6>] (arch_cpu_idle+0xc/0x1e)
[<c000dbd6>] (arch_cpu_idle) from [<c003fcd7>] (cpu_startup_entry+0x9f/0xd0)
[<c003fcd7>] (cpu_startup_entry) from [<20009351>] (0x20009351)
---[ end Kernel panic - not syncing: Fatal exception in interrupt
```

Occurrences [R]:
- 2026-10-17: unstable link after a host reboot — the host saw the camera re-enumerate every few seconds (`usb 3-2: USB disconnect`
  / `new high-speed USB device`), `can't set config #1, error -110` (control-transfer timeouts), `uvcvideo: Failed to set UVC probe
  control: -71`, and the first STREAMON after a fresh camera boot returned `Protocol error` (EPROTO). The camera logged `WARNING …
  usb_composite_setup_continue+0x28 [libcomposite] ← uvc_function_setup_continue ← uvc_v4l2_streamon` (STREAMON with no pending
  setup — the host's probe had failed) and then the panic. The link was attributed to the cable / port [I].
- 2026-10-24: cold boot over a combined USB + UART cable, high speed, vendor app running, no UVC stream open; a 5 000 000-byte
  download from the host over the RNDIS network (camera `wget -O /dev/null`, host HTTP server):

| MTU (both ends) | runs | outcome |
|---|---|---|
| 400 | 3 × 5 MB | all complete, 2.17 s each (≈ 2.3 MB/s); RX counters 16.8 MiB, 0 errors |
| 1500 | run 1 | kernel panic during the first download |

Same cable, same boot, 5 minutes apart. A later 15 MB upload at MTU 400 also survived (one run) [R].

**Reading [I]:** at MTU 400 an RNDIS frame (400 + 14 + 44 = 458 B) fits one 512-B packet, and the driver receives a one-packet
request without DMA (`pkt_num == 1` → `allowAck`, then `dma_setup_control` refuses < 512 B → `read_riu`); at MTU 1500 every frame
goes through the multi-packet DMA receive path, so the fault lives in that path, though the experiment does not prove which race.
`msb250x_gadget_sync_request` → `dma_sync_single_for_cpu` → `__dma_page_dev_to_cpu` → `set_bit(PG_dcache_clean)`: the per-page
loop runs only for a FROM_DEVICE sync of ≥ PAGE_SIZE bytes, while an RNDIS receive request is ~2048 B — so the length the ISR
passed (`bytesdone = DMA_ADDR − req->dma − req->actual`) was garbage: the DMA address register did not belong to the request at
the head of the endpoint queue, which is the only way the ISR identifies the request (`msb250x_dma_isr_handler`, tipoman9
`msb250x_dma.c`). Here it was reached through `msb250x_ep_isr_handler` (the short-packet / packet-ready path), not the DMA
interrupt line. The DMA ISR waits for the "last done bit" (`MSB250X_OTG0_USB_CFG7_H` bit 7) in a `while` loop without a
timeout [S: tipoman9 `msb250x_dma.c`].

### 2.8 Interface offered [R]

Standard `usb_gadget_ops` / `usb_ep_ops` via `udc-core` (`usb_add_gadget_udc`), high speed, ep0 + MUSB endpoints with DMA (seven
endpoints set up at probe); the gadget sees it as `msb250x_udc` [R strings]. Four sysfs files (`tx_swing`, `cm_current`, `de_emphasis_current`, `test_mode`) plus
udc-core's own. Consumers: g_webcam (UVC isoc IN up to 3072 B / µframe, RNDIS bulk) and `g_mass_storage` in kjd_lvgl's USB-disk
mode (§1).

[S] SigmaStar FAEs on the Comake developer chat (SSD268G / SSC339G, 2021–2022): the USB hardware raises no event when a
cable is plugged in — products add a GPIO interrupt on VBUS; with UVC and RNDIS in one gadget the UVC application must be
running before RNDIS works; host and device mode use different driver sets (rmmod, then insmod the other).

### 2.9 Not known
<!-- ev:usb-gadget.2.9 -->

- Whether Mercury6 needs UTMI tuning beyond the stock values (none is visible in the binary).
- Whether the missing `init_utmi` in `udc_start` and the missing UTMI 0x16 rewrite on bus reset contribute to the "full speed
  after reconnect" behaviour seen live [R camera, one observation]; the readings that would settle it — UTMI 0x16 / 0x17 (phys
  0x1F284258 / 0x1F28425C) before and after a re-enumeration (CAMERA-CHECKS K42, P7), together with a boot
  without `ehci-hcd.ko` (P6, `kernel-drivers.md` §4.2.4) — are not taken.

## 3. `usb_f_uvc.ko` — the UVC function

### 3.1 Source relation and build [R unless marked]
<!-- ev:usb-gadget.3.1 -->

Build options as compiled: `CONFIG_SS_GADGET=y`; **no** `CONFIG_SS_GADGET_UVC_MULTI_STREAM` (one `struct uvc_device` per function,
`video_get_drvdata` → uvc); **no** `CONFIG_USB_WEBCAM_UVC_SUPPORT_SG_TABLE` [I: no `sg_*` imports, `uvc_video_encode_isoc` is the
memcpy variant]; **no** `CONFIG_UVC_STREAM_ERR_SUPPORT` (no error-bit / keyframe logic in `uvc_video_encode_isoc`). The
`struct usb_request` the module uses has the vanilla layout (`kernel-platform.md` §15.3).

By name, every one of the binary's 178 non-configfs-macro symbols exists in the tipoman9 sources, and the strings match
(`"[E] Bytes used out of bounds."`, `"overriding streaming_maxpacket to %d"`, `"reset UVC Control"` …). By body, the **OpenIPC
i6c 5.10 copy `drivers/sstar/usb/gadget/function/` is the base**, with the 4.9-era video / queue path of the tipoman9 copy:

| copy | functions compared | identical fingerprint | with findings |
|---|---|---|---|
| i6c 5.10 (`drivers/sstar/usb/gadget/function/`) | 181 of 185 | 136 | 45 |
| tipoman9 I6E 4.9.84 (`drivers/usb/gadget/function/`) | 147 of 185 | 41 | 106 |

185 functions = 177 named + 8 static functions whose symbols the vendor's strip removed, identified by
fingerprint — `uvcg_config_create_children` at 0x14b74, `uvcg_config_remove_children` at 0x11f84, `__uvcg_iter_strm_cls` at
0x1499c, `uvcg_format_bma_controls_show/store` at 0x129b4 / 0x13e94, `uvc_copy_descriptors` at 0x10650 — and by reading —
`uvc_video_free_requests` at 0x117d4, `__uvcg_iter_frm_intrv` at 0x14830.

Of the 45 i6c findings, 22 are structure-offset shifts only (e.g. `uvc_device.func` at +0x3C0 in the binary, +0x2D4 in the build —
the vendor 4.9.227 `struct video_device` / `usb_function` sizes); 12 are gcc-15 artefacts checked by reading (`strcmp(name,
"fs"/"hs"/"ss")` inlined in the class link functions and `uvcg_*_class_create_children`, `strcpy(page, "0x")` in `bmaControls`
show, the framebase attribute stores folded into the identical uncompressed ones, descriptor-initialisation words packed
differently in `uvcg_frame_make` / `uvcg_framebase_make` with equal byte values, the `GFP_KERNEL` word in
`uvc_v4l2_ioctl_default`); 11 are real (§3.3). The source files whose bodies differ between the two copies were also read function
by function, which found further differences among the "identical" and offset-only functions (`uvc_v4l2_qbuf`,
`uvc_v4l2_release`, `uvc_queue_setup`, `uvc_video_complete`'s `-ENOSR` case, `uvc_video_encode_*`). `uvc_alloc_inst` was
reconstructed byte by byte from its literal pool: descriptor defaults equal to i6c's.

Per source file the binary is [R]:

| file | binary = |
|---|---|
| `f_uvc.c` | i6c, except: the unbind callback is `uvc_unbind` (= i6c `uvc_function_unbind` without its `uvcg_info` print); `uvc_set_trace_param()` (exported, tipoman9) present; `uvc_alloc_inst` ignores `uvcg_attach_configfs`' result (no `kfree` on error) |
| `uvc_configfs.c` | i6c (framebase format, isp / customer extension units, per-function attributes), except: control-header default `bcdUVC` **0x0100** (i6c 0x0110); `dwFrameInterval` store = the 4.9 upstream parser; no `bInterfaceNumber` attributes in `control/` and `streaming/` (i6c has them) |
| `uvc_v4l2.c` | i6c `qbuf` (whole `reserved` word) and formats; **open / release / event subscription = the 4.9 upstream / tipoman9 model** |
| `uvc_queue.c` | tipoman9 (no scatter-gather, vmalloc only, monotonic timestamps) |
| `uvc_video.c` | tipoman9 without SG and without the CMA request buffers; request count 40 (i6c `UVC_ISOC_NUM_REQUESTS`); header / encode as §3.5 |

Functions present only in the i6c build (SG path, absent from the binary): `uvc_video_encode_isoc_sg`, `uvcg_prepare_sg`,
`uvcg_complete_sg`, `uvc_video_encode_header` (inlined), `uvcg_default_{control,streaming}_b_interface_number_show`.

### 3.2 Interface to user space [R]

- Device: `/dev/videoN`, a V4L2 **output** device (`V4L2_BUF_TYPE_VIDEO_OUTPUT`), named by the function's `streaming_name`.
- Module parameter `interrupt_ep_enable` (int, default **1**, `__param` → `.data+0` = 1): the VideoControl interrupt (status)
  endpoint exists only when it is set.
- Events `V4L2_EVENT_PRIVATE_START + 0 … + 6` (0x08000000 … 0x08000006): CONNECT, DISCONNECT, STREAMON, STREAMOFF, SETUP, DATA,
  **FORCEIDR** (`UVC_EVENT_FORCEIDR`, meant for the STREAM_ERR build, which this is not).
- `UVCIOC_SEND_RESPONSE` = **0x40405501** = `_IOW('U', 1, struct uvc_request_data)`, the struct being `s32 length; u8 data[60]`
  = 64 bytes [R: usb_f_uvc literal at pool 0x11504 / 0x1150c; prog_uvc builds the same value with `movw r1,#0x5501; movt
  r1,#0x4040` at file offset 0x1D2CE]. Length clipped to the setup's wLength; negative length = stall. The only private ioctl
  (§3.3).
- Standard V4L2: `VIDIOC_S_FMT`, `REQBUFS` (at most 32 buffers), `QBUF` (with the `reserved` word, §3.5), `DQBUF`, `STREAMON` /
  `STREAMOFF`, event subscribe / unsubscribe / dequeue; vb2 vmalloc buffers, `MMAP | USERPTR | DMABUF`.
- prog_uvc's use (`prog_uvc.md` §4.1, §3.2): it subscribes events 0..5, answers probe / commit / XU / PU / CT requests with
  `UVCIOC_SEND_RESPONSE`, uses `REQBUFS` with its `g_maxbuf_cnt` and sets the `reserved` word on every QBUF.

### 3.3 Behaviour, function by function [R: binary vs sources]

**uvc_v4l2_open** — allocates the file handle (`kzalloc`), `v4l2_fh_init/add`, `handle->device = &uvc->video`, then
**`uvc_function_connect(uvc)`** (gadget activation = D+ pull-up through `usb_function_activate`). i6c connects only when the
application subscribes `UVC_EVENT_SETUP`. So the USB device appears on the bus when prog_uvc **opens** `/dev/videoN`.

**uvc_v4l2_release** — **`uvc_function_disconnect(uvc)`** (deactivate → pull-up off), then under `video->mutex`:
`uvcg_video_enable(video, 0)`, `uvcg_free_buffers(queue)`; `v4l2_fh_del/exit`, `kfree`. No `is_uvc_app_handle` test (i6c).

**uvc_v4l2_subscribe_event** — accepts the seven event types of §3.2, else `-EINVAL`; `v4l2_event_subscribe(fh, sub,
2, NULL)`. No `-EBUSY` second-subscriber rule, no connect. **uvc_v4l2_unsubscribe_event** — plain `v4l2_event_unsubscribe`.

**uvc_v4l2_qbuf** — `uvc->video.queue.reserved = b->reserved; b->reserved = 0; uvcg_queue_buffer(); if ok schedule_work(&video->pump)`
— always, without i6c's `state == STREAMING` test. The word sits at uvc+0x754 and lands per buffer at `uvc_buffer+0x170`.

**uvc_v4l2_ioctl_default** — only `UVCIOC_SEND_RESPONSE`, with `uvc_send_response` inlined: `length < 0` → `usb_ep_set_halt(ep0)`;
else `req->length = min(length, uvc->event_length)`, `req->zero = length < event_length`, copy, `usb_ep_queue(ep0, req,
GFP_KERNEL)`. Other commands `-ENOIOCTLCMD`.

**uvc_queue_setup** — `*nbuffers = min(*nbuffers, 32)`, one plane of `video->imagesize`; does not set a request count.
**uvc_buffer_prepare** — OUTPUT queue: `bytesused > length` → trace "[E] Bytes used out of bounds." and `-EINVAL`; disconnected
queue → `-ENODEV`; `state = QUEUED`, `mem = vb2_plane_vaddr()`, `length`, `bytesused` (0 for CAPTURE).
**uvcg_queue_init(queue, type, lock)** (three arguments, 4.9 form): `io_modes = MMAP|USERPTR|DMABUF` (0x13), `buf_struct_size =
0x178`, `mem_ops = vb2_vmalloc_memops`, `timestamp_flags = V4L2_BUF_FLAG_TIMESTAMP_MONOTONIC` (0x2000; i6c: COPY | TSTAMP_SRC_EOF),
`vb2_queue_init`, irqlock / list init, `flags = 0`. Called from `uvcg_video_init` with `V4L2_BUF_TYPE_VIDEO_OUTPUT`.

**uvcg_video_init** — `req_free` list and `req_lock`, `pump` work, `fcc = 'YUYV'` (0x56595559), `bpp = 16`, 320×240, `imagesize =
153 600`; with `bulk_streaming_ep` also `max_payload_size = imagesize`; then `uvcg_queue_init(…, OUTPUT, &video->mutex)`.

**uvcg_video_enable(video, enable)** — `video->ep == NULL` → info "Video enable failed, device is uninitialized." `-ENODEV`.
Disable: `cancel_work_sync(pump)`, `uvcg_queue_cancel(queue, 0)`, `usb_ep_dequeue` every non-NULL `req[0..39]`,
`uvc_video_free_requests` (free each request and its `kmalloc` buffer, re-init `req_free`, `req_size = 0`), `uvcg_queue_enable(queue,
0)`. Enable: `uvcg_queue_enable(queue, 1)`; `encode = bulk ? encode_bulk (payload_size = 0) : encode_isoc`; `BUG_ON(req_size !=
0)`; request size = bulk ? 16 384 (also `max_payload_size = 16 384`) : `ep->mult × ep->maxpacket × max(ep->maxburst, 1)` (`mult`
used as stored — with `streaming_maxpacket=3072` the UDC sets maxpacket 1024, mult 3 → 3072 B); **40 requests**, each a
`kmalloc(req_size, GFP_KERNEL)` buffer + `usb_ep_alloc_request`, `complete = uvc_video_complete`, `context = video`, added to
`req_free`; on any failure free all, `encode = NULL`, `-ENOMEM`; finally `schedule_work(pump)`.

**uvc_video_complete** — status 0: nothing; **`-ENOSR` (uncompleted isoc request): status forced to 0, buffer not cancelled**;
`-ESHUTDOWN`: `uvcg_queue_cancel(queue, 1)`; other: `dev_info` "VS request completed with status %d." + `uvcg_queue_cancel(queue,
0)`. Then the request goes back to `req_free` and the pump is scheduled.

**uvcg_video_pump** — loop: take the first free request (`req_lock`); under `queue->irqlock` take `uvcg_queue_head`; none → put the
request back and stop; else `video->encode(req, video, buf)` and queue it: `ep` missing or disabled → `-ESHUTDOWN`;
`usb_ep_queue(ep, req, GFP_ATOMIC)` failure → `dev_err` "Failed to queue request (%d)." and, for a **bulk** endpoint,
`usb_ep_set_halt`; on any error `uvcg_queue_cancel(queue, 0)` and stop (the request is returned to `req_free`).

**uvc_video_encode_isoc / uvc_video_encode_bulk** — §3.5.

**uvcg_frame_dw_frame_interval_store** — the 4.9 upstream parser, not i6c's: tokens are separated by **newline or NUL** (not spaces),
at most 34 characters each (buffer 35; longer → `-EINVAL`), each parsed `kstrtou32(…, 0)`; a first pass counts, a second fills a
new `u32` array, which is `sort()`ed ascending (`uvcg_config_compare_u32`) and replaces the old one; `bFrameIntervalType = count`.
Busy (format linked or function in use) → `-EBUSY`.

**uvcg_control_header_make** — default `bcdUVC = 0x0100`, `dwClockFrequency = 48 000 000`, `bLength = 13`.

**uvc_alloc_inst** — descriptor defaults as i6c: IT 1 bmControls 02 00 00, PU 3 wMaxMultiplier 0x4000 bmControls 01 00, XU 6 AIT
GUID 14 controls bmControls FF 6F source 3, XU 2 customer GUID 6 controls 3F 00 source 6, OT 7 source 2, colour matching 1/1/4;
`streaming_interval 1`, `maxpacket 3072`, `maxburst 13`, `bulk_streaming_ep 0`, `streaming_name` = 32-byte zeroed buffer with
"UVC Camera". These defaults serve configfs gadgets only; g_webcam overwrites the control / streaming descriptor pointers with its
own tables (§4).

**ep0 class requests** (`f_uvc.c`, vendor additions over upstream): `uvc_function_req_match` lets the composite core route class
requests to the right function; `uvc_function_setup_continue` gives the delayed status after STREAMON.

### 3.4 Vendor changes against the upstream 4.9 f_uvc stack [S: tipoman9 / i6c sources; R in the binary where marked]

| area | change |
|---|---|
| `f_uvc_opts` | per-function `streaming_name` (video device name and string), `bulk_streaming_ep`; `streaming_interval / maxpacket / maxburst / name` and `bulk_streaming_ep` at +0x68 / +0x6c / +0x70 / +0x74 / +0x78 [R] |
| endpoints | a **bulk** streaming endpoint option: no alt settings, stream on / off driven by `SET_CUR(VS_COMMIT)` ("reset UVC": cancel queue, re-enable EP); **isoc: alt 0 + up to 28 alt settings** from a static table (§3.6) [R]; upstream has one alt 1 sized from `streaming_maxpacket` |
| interrupt EP | the VC status endpoint only when `interrupt_ep_enable` (default on) [R] |
| ep0 | `uvc_function_req_match`, `uvc_function_setup_continue` (§3.3) |
| payload header | 12 bytes (upstream: 2) [R] (§3.5) |
| frame split | one frame may span several v4l2 buffers — EOF / FID only at the last [R] (§3.5) |
| buffers | vb2 vmalloc, OUTPUT, payload > plane size → `-EINVAL` [R] |
| events | `UVC_EVENT_FORCEIDR` added |
| multi-stream | compiled out [R] |
| configfs | extra items: `framebase` format, `isp_extension` / `customer_extension` XU items, per-function attributes, writable `bcdUVC` / `dwClockFrequency` [R] (§3.7) |

### 3.5 Payload header and the per-buffer flag word [R]

`uvc_video_encode_isoc`: header byte 0 = 12 (length); byte 1 = `FID | EOH` (EOH = 0x80), `| STI` (0x20) when the buffer's flag
word (`uvc_buffer+0x170`, copied from `v4l2_buffer.reserved` at `VIDIOC_QBUF`) has **bit 1**, `| EOF` (0x02) when the rest of the buffer
fits and **bit 0 is clear**; bytes 2–11 = 0 (no PTS / SCR); payload = min(rest, req_size − 12). At buffer end: `state = DONE`,
`uvcg_queue_next_buffer`, and FID toggles only when bit 0 is clear. `uvc_video_encode_bulk` applies the same flag rules per
payload of `max_payload_size`. No error bit (0x40) is ever set by this code.

| flag word bit | meaning (name in the SDK header `st_uvc_datatype.h` [S]) | header effect |
|---|---|---|
| bit 0 | `UVC_BUFFER_FLAGS_FRAME_NOEND` — the frame continues in the next buffer | no EOF, no FID toggle |
| bit 1 | `UVC_BUFFER_FLAGS_STILL_IMAGE` — still-image data | `bmHeaderInfo \|= 0x20` (STI) |

tipoman9 instead turns `reserved == 1` into a `bFrameEnd = false` bool; the binary keeps the whole word. prog_uvc sets bit 0 on
every non-final slice and bit 1 on still-image slices (`prog_uvc.md` §3.5); a buffer holding one complete frame carries 0.

### 3.6 Isochronous alternate settings [R]

The streaming interface offers alt 0 (zero bandwidth) plus bandwidth alts built from the static 28-entry table
`uvc_streaming_comp_descs[]` (macro `DECLARE_STREAMING_DESCS`, the same in tipoman9), decoded:

| alt | FS B / frame | HS B / µframe (wMaxPacketSize) | SS B / interval (maxburst, mult) |
|---|---|---|---|
| 1 | 64 | 64 (0x0040) | 64 (0, 0) |
| 2 | 64 | 192 (0x1040) | 192 (0, 2) |
| 3 | 192 | 192 (0x00c0) | 192 (0, 0) |
| 4 | 192 | 576 (0x10c0) | 576 (0, 2) |
| 5 | 384 | 384 (0x0180) | 384 (0, 0) |
| 6 | 512 | 512 (0x0200) | 512 (0, 0) |
| 7 | 640 | 640 (0x0280) | 640 (0, 0) |
| 8 | 800 | 800 (0x0320) | 800 (0, 0) |
| 9 | 944 | 944 (0x03b0) | 944 (0, 0) |
| 10 | 1024 | 1024 (0x0400) | 1024 (0, 0) |
| 11 | 640 | 1280 (0x0a80) | 1280 (0, 1) |
| 12 | 992 | 1984 (0x0be0) | 1984 (0, 1) |
| 13 | 1024 | 2048 (0x0c00) | 2048 (0, 1) |
| 14 | 896 | 2688 (0x1380) | 2688 (0, 2) |
| 15 | 1024 | 3072 (0x1400) | 3072 (0, 2) |
| 16 | 1024 | 3072 (0x1400) | 6144 (1, 2) |
| 17 | 1024 | 3072 (0x1400) | 9216 (2, 2) |
| 18 | 1024 | 3072 (0x1400) | 12288 (3, 2) |
| 19 | 1024 | 3072 (0x1400) | 15360 (4, 2) |
| 20 | 1024 | 3072 (0x1400) | 18432 (5, 2) |
| 21 | 1024 | 3072 (0x1400) | 21504 (6, 2) |
| 22 | 1024 | 3072 (0x1400) | 27648 (8, 2) |
| 23 | 1024 | 3072 (0x1400) | 30720 (9, 2) |
| 24 | 1024 | 3072 (0x1400) | 33792 (10, 2) |
| 25 | 1024 | 3072 (0x1400) | 36864 (11, 2) |
| 26 | 1024 | 3072 (0x1400) | 39936 (12, 2) |
| 27 | 1024 | 3072 (0x1400) | 43008 (13, 2) |
| 28 | 1024 | 3072 (0x1400) | 46080 (14, 2) |

`config_streaming_desc` (inlined in `uvc_function_bind` at 0x108bc; source `f_uvc.c`) filters the table by the
requested endpoint budget and emits one alt per surviving entry, for each of FS / HS / SS:
- **FS**: keep entries whose `fs_endp.wMaxPacketSize ≤ requested` **and** `hs_endp.wMaxPacketSize ≤ 1023`.
- **HS**: keep entries whose `(hs wMaxPacketSize & 0x7ff) ≤ requested maxpacket`, mult ≤ requested mult, and `ss_endp_comp.bMaxBurst
  == 0`.
- **SS**: keep entries whose maxpacket ≤ requested, mult ≤ requested mult, burst ≤ requested burst.

With the product load `streaming_maxpacket=3072` the bind derives `max_packet_size = 1024`, `max_packet_mult = 3` (the ≤ 3072 branch)
→ HS wMaxPacketSize 0x1400 (1024 | (2 << 11)), SS mult field 2, burst 0. The HS filter then keeps alts 1..15; entries 16..28 all
carry a SuperSpeed burst ≥ 1 and are dropped. So at high speed — the speed of this UDC — the interface exposes **alt 0 + 15 bandwidth alts,
the largest 3072 B / µframe (1024 × 3)** [R / I], and the host picks alt 15 for 4K MJPEG at both 30 and 40 fps requests [R: host
`uvcvideo` trace, 0x400]. The FS list keeps the smaller alts (fs ≤ requested and hs ≤ 1023).

### 3.7 configfs interface — unused [R: strings and code; semantics from the i6c source where the fingerprint is identical]

Unused on this camera: it loads the legacy `g_webcam`, which never creates a configfs instance. A configfs gadget
(`/sys/kernel/config/usb_gadget/<g>/functions/uvc.<n>/`) would see:

```
uvc.<n>/  streaming_interval streaming_maxpacket streaming_maxburst streaming_name bulk_streaming_ep
  control/
    header/<H>/            bcdUVC dwClockFrequency                      (mkdir creates a VC header)
    processing/default/    bUnitID bSourceID wMaxMultiplier bmControls iProcessing
    extension/isp/         bUnitID baSourceID bmControls iExtension    (XU 6, AIT GUID)
    extension/customer/    bUnitID baSourceID bmControls iExtension    (XU 2, customer GUID)
    terminal/camera/default/  bTerminalID wTerminalType bAssocTerminal iTerminal wObjectiveFocalLengthMin/Max
                              wOcularFocalLength bmControls
    terminal/output/default/  bTerminalID wTerminalType bAssocTerminal bSourceID iTerminal
    class/fs  class/ss     (symlink -> ../../header/<H>)
  streaming/
    header/<H>/            bmInfo bTerminalLink bStillCaptureMethod bTriggerSupport bTriggerUsage  (+ symlinks to formats)
    uncompressed/<F>/      bFormatIndex guidFormat bBitsPerPixel bDefaultFrameIndex bAspectRatioX/Y bmInterfaceFlags bmaControls
    mjpeg/<F>/             bFormatIndex bDefaultFrameIndex bmFlags bAspectRatioX/Y bmInterfaceFlags bmaControls
    framebase/<F>/         bFormatIndex guidFormat bBitsPerPixel bDefaultFrameIndex bAspectRatioX/Y bmInterfaceFlags bmaControls
    <format>/<F>/<frame>/  bFrameIndex bmCapabilities wWidth wHeight dwMinBitRate dwMaxBitRate dwMaxVideoFrameBufferSize
                           dwDefaultFrameInterval dwFrameInterval
    color_matching/default/ bColorPrimaries bTransferCharacteristics bMatrixCoefficients
    class/fs class/hs class/ss  (symlink -> ../../header/<H>)
```

Modes as compiled: writable attributes are **0666** (the five function-level ones, `bcdUVC`, `dwClockFrequency`, `guidFormat`,
`bBitsPerPixel`, `bDefaultFrameIndex`, `bmaControls`, all frame attributes except `bFrameIndex`); the rest are 0444 — including
every extension-unit, processing, terminal and colour-matching attribute (fixed by `uvc_alloc_inst`, §3.3).

Behaviour:
- Function level: `streaming_interval` / `maxpacket` / `maxburst` `kstrtouint` base 0, accepted ranges 0–16 / 0–3072 / 0–15
  (unsigned `< 0x11`, `< 0xC01`, `< 0x10`; 0 is not rejected); `bulk_streaming_ep` 0 / 1; `streaming_name` copies the written
  bytes **without length check or terminator** into the 32-byte buffer (allocated from kmalloc-64) — writes > 64 bytes overflow,
  and the newline of `echo` is kept [R code; I consequence]. All return `-EBUSY` while `opts->refcnt` (function bound).
- Descriptor attributes: decimal / hex via `kstrtou8/16/32` base 0; `guidFormat` takes 16 raw bytes (longer input truncated,
  returns 16); `bmaControls` show prints `0x` + hex of the 1-byte control field, store requires `0x` / `0X` + hex (`hex2bin`,
  1 byte); any store on a format already linked to a header, or while bound, `-EBUSY`. `dwFrameInterval`: the parser of §3.3.
- Formats: `uncompressed` default GUID YUY2 / 16 bpp; `mjpeg` default flags 0; `framebase` (functions `uvcg_framebase_*`) default GUID `H264` (`48 32 36 34 00 00
  10 00 80 00 00 AA 00 38 9B 71`), 16 bpp, `bVariableSize 1`. Frames: default 640×360, bit rates 18 432 000 / 55 296 000, buffer
  460 800, default interval 666 666; subtype by parent format (uncompressed 5, mjpeg 7, framebase 0x11). Frame-based frames are
  emitted without `dwMaxVideoFrameBufferSize` and with `dwBytesPerLine` (0; no attribute) and `bLength = 26 + 4n`.
- Links: `streaming/header/<H>` → a format directory adds it to the header's format list (`bFormatIndex` / frame indices assigned
  then); `control/class/{fs,ss}` → `control/header/<H>` puts that header first in the fs / ss control descriptor array;
  `streaming/class/{fs,hs,ss}` → `streaming/header/<H>` builds the whole streaming descriptor array (input header with one
  `bmaControls` byte per format, formats in link order with `bFormatIndex = n+1`, their frames, colour matching, NULL). A second
  link, or a link while bound, `-EBUSY`.
- At bind (`uvc_function_bind`, i6c) the control and streaming class arrays from these links are copied into the gadget
  descriptors (`uvc_copy_descriptors`). `uvc_alloc` checks nothing (it copies whatever pointers `opts` holds). What bind does with
  an unlinked (NULL) array is not traced [I: likely a NULL dereference or an empty descriptor set].

### 3.8 Observed on the link [R unless marked]

- **Throughput, isochronous, high speed:** 4K MJPEG is bandwidth-bound at the USB 2.0 isoc ceiling of 3072 B × 8000 / s ≈ 24.5 MB/s:
  at the stock quality 105 frames / 9.97 s = 10.5 fps, 2.33 MB / frame, 24.5 MB/s; with smaller frames 40.3 fps at 21.5 MB/s
  (521 KB / frame) and 40.6 fps at 18.4 MB/s (452 KB / frame); ~20 MB/s of MJPEG at 4K45 (45 fps × ~450 KB). 4K H.264 at a 30-fps
  request runs 29.1 fps at 8.2 Mbit/s, paced by the requested interval.
- **Bulk mode** (`bulk_streaming_ep = 1`, prog_uvc `-T 1`): one 4K45 MJPEG run delivered 41.6 fps against 40 in isoc.
- **Error bit on the host — sources disagree.** A host trace of a 4K45 stream (uvcvideo `trace=0x80`, 5 s) showed 214 "Frame
  complete (EOF found)", 19 "Marking buffer as bad (error bit set)", 36 "Dropping payload (out of sync)", 6 "FID bit toggled",
  i.e. payload headers with the error bit (`UVC_STREAM_ERR`) on ~4 frames / s; seen 19 / 5 s once and 0 in later runs (one later
  4K45 run had 0 "buffer bad" marks) [R: host log]. An earlier reading of ours attributed it to the gadget setting `UVC_STREAM_ERR` when a
  request completes with `-ENOSR` [I]; the binary has no error-bit logic and treats `-ENOSR` as success (§3.3, §3.5) [R]. What set
  the bit in that run is not known. The host's uvcvideo (kernel 6.17) defaults to `nodrop = 1`, so such frames reach the player.
- **First STREAMON after a fresh boot** returned `Protocol error` (EPROTO) on the host twice: once on the unstable link of §2.7,
  once with the stream starting normally on the next attempt.
- **Descriptors as the host sees them** (`lsusb -v`, stock gadget): every frame size offers 30 / 15 / 10 fps; MJPEG subtype 0x07 +
  two frame-based (0x11) groups with GUIDs …34363248 "H264" and …35363248 "H265" (§4.4).

## 4. `g_webcam.ko` — the legacy composite gadget

### 4.1 Source relation [R]
<!-- ev:usb-gadget.4.1 -->

**tipoman9 `drivers/usb/gadget/legacy/webcam.c`** (I6E revision), built with `CONFIG_USB_WEBCAM_UVC=y`, `CONFIG_USB_WEBCAM_RNDIS=y`, **no** UAC, **no** DFU, **no**
`CONFIG_SS_GADGET_UVC_MULTI_STREAM` [R: parameter set, imports], with ODM-edited
descriptor tables (§4.3, §4.4). Evidence:
- Functions = source functions: `webcam_bind` (inlined `video_bind` + `rndis_bind`), `webcam_bind_config` (`rndis_bind_config` +
  per-function `usb_add_function` loop), `webcam_unbind`, `video_unbind`, `rndis_unbind`.
- `video_bind`, non-multistream path, byte for byte in structure: control list `MAX_CONTROL_SIZE` = 10 pointers (kmalloc n × 0x28),
  streaming lists `MAX_FORMAT_COUNT` = 50 pointers (n × 200) for fs / hs / ss, 4 output-terminal copies with `bTerminalID = 7 + j`,
  input-header copy with `bTerminalLink = 7 + i`, per-function `streaming_interval / maxpacket / maxburst / name[i]` and
  `bulk_streaming_ep` into `f_uvc_opts` (offsets in §3.4), `uvc_set_trace_param(trace)`.
- `rndis_bind` quirk kept: `usb_get_function_instance("rndis")` is called **twice** (the first instance, with qmult / host_addr /
  dev_addr applied, leaks; the second is used) — present in the source too.
- Unit IDs as in tipoman9: IT 1 → PU 3 → XU 6 ("ISP", AIT GUID) → XU 2 ("customer") → OT 7.
- TAKOYAKI's webcam.c (older: no RNDIS, scalar params, OT 3 / PU 4) and upstream 4.9 webcam.c (one fixed YUYV / MJPEG table, no
  params beyond the composite options) are further away.
- The skyipc "3formats_7resolutions" webcam.c lists 240p … 4K + 7200×5400 (48 MP) in size order with one MJPEG still; this table
  replaces 48 MP by 424×240 and reorders MJPEG — the ODM edited the table itself.

### 4.2 Module parameters [R: `.modinfo`, defaults from `.data`]

| param | type | default | meaning |
|---|---|---|---|
| `uvc_function_enable` | int | 1 | number of UVC functions (loop bound in `video_bind`; 0 = no UVC) |
| `rndis_function_enable` | int | 0 | add the RNDIS function (first interface) |
| `streaming_maxpacket` | uint[4] | 3072, 3072, 30192, 30064 | wMaxPacketSize of the video endpoint per UVC function |
| `streaming_interval` | uint[4] | 1, 1, 1, 1 | bInterval |
| `streaming_maxburst` | uint[4] | 0, 0, 0, 0 | SS only |
| `streaming_name` | charp[4] | **"4K Camera"** × 4 (source: "UVC Camera") | v4l2 device / interface name |
| `bulk_streaming_ep` | bool | 0 | 0 isoc, 1 bulk |
| `trace` | uint | 0 | uvc gadget trace mask |
| `qmult`, `host_addr`, `dev_addr` | | 5 [I: upstream default] / NULL / NULL | u_ether (RNDIS) |
| `idVendor`, `idProduct`, `bcdDevice`, `iManufacturer`, `iProduct`, `iSerialNumber` | | — | `USB_GADGET_COMPOSITE_OPTIONS` |

prog_uvc loads it with one UVC function and RNDIS, `streaming_maxpacket=3072`, the rest at the defaults (§1).

### 4.3 Device and control-interface descriptors [R: `.data` bytes]
<!-- ev:usb-gadget.4.3 -->

| item | binary | tipoman9 source | note |
|---|---|---|---|
| device | class EF/02/01, **VID 0x0ac8, PID 0x3420**, bcdDevice 0x0010 | 0x1d6b / 0x0102 | ODM edit; strings stay "Linux Foundation" / "Webcam gadget" / configuration "Video" |
| VC header | bcdUVC **0x0100**, clock 48 MHz, 1 interface | same (UVC 1.0 build) | |
| camera IT (id 1) | bmControls **FF FF FF** | 02 00 00 | ODM edit: advertises every CT control |
| PU (id 3, source 1) | wMaxMultiplier 0x4000, bmControls **FF FF** | 01 00 | ODM edit |
| XU 6 | GUID {23E49ED0-1178-4F31-AE52-D2FB8A8D3B48} (AIT), **bNumControls 0x10, bmControls FF FF**, baSourceID **2** | 0x0E, FF 6F, source 3 (PU) | ODM edit |
| XU 2 | GUID **{63610682-5070-49AB-B8CC-B3855E8D221D}**, 6 controls, bmControls 3F 00, baSourceID **2** | GUID {2C49D16A-32B8-4485-3EA8-643A152362F2}, source 6 | ODM edit; source = own ID |
| OT (id 7 + j) | type 0x0101, source 2 | same | |

The XU source bytes as read make the chain IT1 → PU3, XU6 ← 2, XU2 ← 2 (itself), OT7 ← 2 — not a valid UVC topology [R bytes;
I: hosts tolerate it, the camera works on Linux and Windows]. The source chain is IT1 → PU3 → XU6 → XU2 → OT7. As the host
reads them (`lsusb -v` of the stock gadget): `bcdUSB 2.00`, configuration `MaxPower 2mA`
[R]. What prog_uvc
answers on these units: `prog_uvc.md` §4.2, §4.4, §4.6.

### 4.4 Streaming descriptors (identical for fs / hs / ss; stream 0) [R]
<!-- ev:usb-gadget.4.4 -->

Input header: bNumFormats **5** (the source value; only 3 formats follow), bTerminalLink 7, bStillCaptureMethod 2, bControlSize 1,
bmaControls 00 04 04 04 00. Sequence (31 descriptors with the input header):

| # | descriptor | content |
|---|---|---|
| 1 | FORMAT_MJPEG idx 1 | 7 frames, default frame 1 |
| 2–8 | FRAME_MJPEG idx 1..7 | 3840×2160, 640×480, 1280×720, 1920×1080, 2560×1440, 320×240, 424×240 |
| 9–11 | STILL_IMAGE_FRAME × 3 | 1920×1080, 3840×2160, 1280×720 (one size each, compression 3) |
| 12 | FORMAT_FRAME_BASED idx 2, GUID `H264` | 7 frames, bpp 16, default 1, bVariableSize 1 |
| 13–19 | FRAME_FRAME_BASED idx 1..7 | 320×240, 640×480, 1280×720, 1920×1080, 2560×1440, 3840×2160, 424×240 |
| 20 | STILL_IMAGE_FRAME | 1920×1080 |
| 21 | FORMAT_FRAME_BASED idx 3, GUID `H265` | the same 7 frames as H.264 |
| 22–28 | FRAME_FRAME_BASED idx 1..7 | as H.264 |
| 29 | STILL_IMAGE_FRAME | 1920×1080 |
| 30 | COLOR_MATCHING | 1/1/4 |

Every frame: bmCapabilities 0, dwDefaultFrameInterval 333333, 3 discrete intervals **333333 / 666666 / 1000000** (30 / 15 / 10 fps).
Frame descriptors in `.data` (frame-based frames carry dwBytesPerLine 0, MJPEG frames
dwMaxVideoFrameBufferSize = w × h × 2):

| size | MJPEG idx | MJPEG `.data` | H.264 / H.265 idx | H.264 / H.265 `.data` | dwMinBitRate | dwMaxBitRate | MJPEG max buffer |
|---|---|---|---|---|---|---|---|
| 3840×2160 | 1 | +0x81f | 6 | +0x6d8 / +0x5a7 | 1 327 104 000 | 1 327 104 000 | 16 588 800 |
| 640×480 | 2 | +0x8b7 | 2 | +0x770 / +0x63f | 49 152 000 | 147 456 000 | 614 400 |
| 1280×720 | 3 | +0x891 | 3 | +0x74a / +0x619 | 147 456 000 | 442 368 000 | 1 843 200 |
| 1920×1080 | 4 | +0x86b | 4 | +0x724 / +0x5f3 | 331 776 000 | 995 328 000 | 4 147 200 |
| 2560×1440 | 5 | +0x845 | 5 | +0x6fe / +0x5cd | 589 824 000 | 589 824 000 | 7 372 800 |
| 320×240 | 6 | +0x8dd | 1 | +0x796 / +0x665 | 12 288 000 | 36 864 000 | 153 600 |
| 424×240 | 7 | +0x7f9 | 7 | +0x6b2 / +0x581 | 16 281 600 | 16 281 600 | 203 520 |

The other class-specific descriptors of the streaming list (pointer array at `.data` 0x1b4; a second array at 0x27c repeats the
list with its header entry at +0x0ca): input header +0x0b8, FORMAT_MJPEG +0x903, its stills +0x7ee / +0x7e3 / +0x7d8,
FORMAT H.264 +0x7bc, its still +0x6a7, FORMAT H.265 +0x68b, its still +0x576, COLOR_MATCHING +0x570.

The table matches prog_uvc's own format table `uvc_formats[]` index for index — MJPEG frame order 4K first, H.264 / H.265 4K at index 6, 424×240
last, the three MJPEG stills = the app's still table (1080p, 4K, 720p) (`prog_uvc.md` §3.1, §3.4).

### 4.5 Not known
<!-- ev:usb-gadget.4.5 -->

- Whether a host sees the XU source IDs (both 2) and `bNumFormats` 5 as the `.data` bytes say, or anything corrects them at run time
  (nothing in the binary writes them: no write found in `video_bind`). Settled only by `lsusb -v` on a host (CAMERA-CHECKS K41).
