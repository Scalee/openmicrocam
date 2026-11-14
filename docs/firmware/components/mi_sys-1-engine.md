# mi_sys-1-engine — the mi_sys pipeline engine
<!-- ev:mi_sys-1-engine -->

`mi_sys.ko` is the core kernel module of SigmaStar's MI layer on the Mercury6 (M6) chip. Besides its user-facing side
(`/dev/mi_sys`, `/dev/mi_poll`, memory, configuration tables — `mi_sys-2-interfaces.md`) it holds the **pipeline engine**
every other MI kernel module registers with: the object model of devices, passes, channels and ports; binding and bind
capability negotiation; the frame flow between bound ports (buffer allocation, queue depths, frame-rate control, the user
FIFO); the 64-bit sideband message carried with bound buffers; the worker threads and their MAIN loop; the CMDQ objects;
task completion; the pass graph that decides which pass drives a bound chain; the SRAM (internal memory) path; the ring pools
of variable-size records; and the registration contract (info record, pass callback table). Module documents state only what
their module fills in; the contract itself is here. The char-device front end shared by all MI modules (`mi_common`,
`/dev/mi_<name>`, `/proc/mi_modules`) and the `config_tool` tables mi_sys serves are in `mi_sys-2-interfaces.md` §1, §10 as well.

Tags: **[R]** read from the binary, a decompile or disassembly; **[S]** stated by a source; **[I]** inference. A heading
tagged [R] means every statement under it is [R] unless marked otherwise.

Binary: `/config/modules/4.9.227/mi_sys.ko` (build 2022-07-21, `project_commit.7251436`). Address rule: a Ghidra address
in `mi_sys.c` = objdump `.text` address + 0x10028; Unnamed static functions are named by the `__func__` strings their
asserts print. An older source of the same code base (2018, github `jockyw2001/sdk`) has a redesigned engine and serves
for names only [S: 58-mi-sys].

## 1. Object model [R]

### 1.1 Hierarchy and limits
device (`'MSYS'`, one per `mi_sys_RegisterDev`) → passes (`'MPAS'`, ≤ 4) → channels (≤ 48 = 0x30) → per channel and pass a
channel pass (`'MCAS'`) → input ports (`'M_IN'`, ≤ 16) and output ports (`'MOUT'`, ≤ 8). User calls address a port by
`MI_SYS_ChnPort_t {eModId, u32DevId, u32ChnId, u32PortId}`; inside the kernel the same structure has five words
`{+0 module id, +4 dev, +8 chn, +0xc pass, +0x10 port}` (the form the bind-capability callbacks read, §3.3). The pass of a
user-addressed port is resolved through the module's `GetInterBindInputPortPassId` / `GetInterBindOutputPortPassId`
callbacks (registration +0x24 / +0x28) [I from the names]. A device takes 0xdc0 + 0x1b90 × channels bytes.

### 1.2 Object layouts
Sizes, magics and the fields the engine uses. Offsets of a channel *c*, a device pass *p* and a channel pass *p* are given
in the rows. The device-pass fields of the pass graph are in §8.2.

| object | size | magic | fields |
|---|---|---|---|
| device `pstModDev` | 0xdc0 + 0x1b90 × chn | `'MSYS'` 0x4D535953 | +0x04 module id, +0x08 dev id, +0x0c pass count, +0x10 chn count (≤ 0x30), +0x14 in-port count, +0x18 out-port count, +0x1c u8 = registration +0x18, +0x1d u8 = registration +0x20, +0x20 thread priority, +0x24 node in `mi_sys_global_dev_list` (the device pass *p* lies at node + 0x54 + 0x338·p), +0x60..+0x70 registration +0x24..+0x34 (+0x68 `ConfigSramBuf`, +0x6c `AllowPreemptionSram`), device pass *p* at +0x78 + 0x338·p, +0xd60..+0xdb8 SRAM state (§9), channel *c* at +0xdc0 + 0x1b90·c |
| device pass `pstDevPass` | 0x338 | `'MPAS'` | +0x08 pass id; +0x0c copy of the pass callback table (table slot *s* at +0x0c + *s*, §2.3); +0x98 concurrent-task limit (slot 0x8c); +0x9e u16 output-task timeout (slot 0x92); +0xa4 in-port count, +0xa8 out-port count; +0xb4 `directbuftask_waitqueue`, +0xcc `input_thread_waitqueue`, +0xe4 `output_thread_waitqueue`, +0xfc `input_buf_monitor_waitqueue`; +0x134 list of input tasks in flight; +0x184 list of output tasks; +0x194 / +0x1b4 semaphore / current working queue; +0x1bc lazy-rewind queue; +0x1c4 / +0x1e4 semaphore / `new_pulse_fifo_inputqueue`; +0x1ec / +0x20c semaphore / `next_todo_pulse_inputqueue`; +0x218 later-barrier list node; +0x228 bind time (set on every successful bind); +0x230 current CMDQ; +0x220, +0x234..+0x25c, +0x285 pass-graph fields (§8.2); +0x260 / +0x264 / +0x268 / +0x270 / +0x274 / +0x278 later-barrier entry (channel pass, runtime, task words, §6.4); +0x26c per-port sequence table for HW_AUTOSYNC (see the note below); +0x27c counter whose change ends a MAIN sleep; +0x280 pending-pulse flag; +0x2a0 fence of the last kicked task; +0x2a4 task info and +0x2a8 (0) of the later-barrier entry; +0x290..+0x318 (outside the fields above) per-second apply / kick statistics words, named only by their use, +0x2b4… among them the apply-latency averages |
| channel `pstDevChn` | 0x1b90 | `'MCHN'` 0x4D43484E | +0x04 u8 enabled; +0x08 chn id; +0x44 device; channel pass *p* at +0x58 + 0x6b0·p; +0x1b18 / +0x1b24 counters of `EnqueueInputTask` results; +0x1b60 + 4·(r mod 5) counters of output-task results r |
| channel pass `pstChnPass` | 0x6b0 | `'MCAS'` 0x4D434153 | +0x04 pass id, +0x08 in-port count, +0x0c out-port count, +0x10 user count, +0x14 channel, +0x18 device pass, +0x1c input ports (0x238 each, one block), +0x20 output ports (0x228 each), +0x24 `track_listhead`, +0x30 `u32RootChnIdx`, +0x40 and +0x150 cleared at every bind |
| input port | 0x238 | `'M_IN'` 0x4D5F494E | +0x04 u8 enabled, +0x08 port id, +0x10 / +0x14 u64 sideband message (§5), +0x18 / +0x1c src rate num / den, +0x20 / +0x24 dst rate num / den, +0x28 and +0x50 two frame-rate statistic blocks (0x28 bytes each, §4.4), +0x78 dispatch accumulator, +0x7c alloc accumulator (−1 = time mode), +0x80 / +0x84 measured source rate `stPrevBindPortFR` {frames·1000, ms} or −1 / −1, +0x88 / +0x8c offered / accepted counters of the dispatch side, +0x90 / +0x94 / +0x98 FinishCnt / RewindCnt / DropCnt, +0xa4 atomic buffers queued by user injection, +0xa8 atomic buffers queued from the bound output, +0xae u8 keep-last-buffer-for-repeat, +0xb0 that buffer, +0xb4 wait queue, +0xcc bind-list node (in the output's list +0x188), +0xd4 bind type, +0xd8 u8 bound, +0xf0 chn pass, +0xf4 dev pass, +0x130 / +0x138 the port's lists of queued / lazily rewound references, +0x140 bound peer output port, +0x148 HW_RING allocator, +0x14c HW_RING bind parameter, +0x150 ext-conf (§4.2), +0x156 ext-conf flag byte, +0x158 clear-conf, +0x16c infer type (§8.4), +0x174 / +0x1d4 two buffer queues (0x60 each; +0x1d4 = latest-buffer slot, keeps 1) |
| output port | 0x228 | `'MOUT'` 0x4D4F5554 | +0x04 u8 enabled, +0x08 port id, **+0x0c user depth**, +0x10 driver back-ref depth, **+0x14 `u32OutputPortBufCntQuota`** (queue depth, default 4), +0x18 u8 user-FIFO block mode, +0x1c atomic `usrLockedBufCnt`, **+0x20 atomic buffers alive**, +0x24 atomic buffers in tasks, +0x28 and +0x50 two frame-rate statistic blocks (§4.4), +0x40 / +0x48 time-FRC state (ns of the last accepted frame / carried remainder in ms; both lie inside block 0's 0x28 bytes, after its listed fields), +0x80 / +0x84 finished / dropped counters, +0x88 back-ref queue, +0xe8 **user FIFO** (count at +0x118), +0x148 / +0x150 u64 FIFO in / dropped counters, +0x158 user allocator, +0x15c private-heap allocator, +0x160 chn pass, +0x164 dev pass, +0x168 semaphore, +0x188 list of bound input ports, +0x1d0 wait queue `stTcondWaitBufClear`, +0x1e8 `s32Fd`, +0x1ec poll object, +0x1f0 sequence counter, +0x204 ext-conf (8 bytes; flag byte +0x20a), +0x20c u8 clear-conf, +0x20d u8 force-alloc, +0x20e u8 low-latency mode, +0x210 low-latency parameter, +0x214 name `"<mod><dev>-out<chn>-<port>"` |
| input task | — | — | +0x34 fence; +0x40 task info handed to the module; +0x8c fence (second copy); +0x90..+0xa0 SRAM address / size (§9); +0xa4 u8 lazy-rewind flag |
| output task | 0xd0 (kmem cache) | — | +0x08 dev pass, +0x0c chn pass, +0x18 / +0x20 / +0x28 copied from the input task (PTS etc.), +0x38 `stOutputTaskInfo` (handed to the module), +0x3c `u32MaxValidOutputPortNum`, +0x64 + i u8 "no buffer for port i (blocked by rate control)", +0x6c + 4i `pastOutputPortBufInfo[i]`, +0x8e + i u8 "rewind port i" (set by the module), +0xa0 per-port dispatch mask, +0xa4..+0xb0 SRAM fields, +0xb4 u8 direct-buffer task |
| buffer reference | — | `'BUFR'` 0x42554652 | +0x0c allocation (reference counted, `ops.OnRelease` at +0x10), +0x14 owner port, +0x18 `MI_SYS_BufInfo_t` (what modules and user space see; its +0x08 / +0x0c = bufref +0x20 / +0x24 hold the sideband word, §5), +0x120 chn id, +0x124 buf-conf flags, +0x128 queue node, +0x140 u64 enqueue time (monotonic ns, §6.3), +0x14c peer sequence word |

Device-pass offsets +0x268 and +0x26c: one reading of ours places the HW_AUTOSYNC per-port sequence table at +0x26c,
another finds task words of the later-barrier entry at +0x268 / +0x270; whether +0x268 is a 64-bit word covering +0x26c
is not settled.

## 2. Registration contract [R layout; slot names from the modules' symbols]

### 2.1 `mi_sys_RegisterDev`
`MI_SYS_DRV_HANDLE mi_sys_RegisterDev(info *, void *usrData, procfs_ops *, struct proc_dir_entry *dir)` returns the device
(the handle the other exports take) or NULL; `usrData` is passed back as the last argument of every callback. `procfs_ops` =
5 functions (`OnDumpDevAttr, OnDumpChannelAttr, OnDumpInputPortAttr, OnDumpOutPortAttr, OnHelp`), as in the 2018 header.
Registering the same (module id, dev id) twice returns the existing device. `MI_SYS_IMPL_RegisterDev` creates the worker
threads of every pass (§6.1) and then sets device pass +0x285 := 1; the pass-graph walks skip passes with 0 there.
`mi_sys_UnRegisterDev(h)` removes the device. Export signatures of the other registration-side calls are in
`mi_sys-2-interfaces.md` §6.1.

### 2.2 Info record (0x298 bytes)
Values for the four modules of this camera's capture path.

| off | field | VIF / ISP / SCL / VENC values |
|---|---|---|
| 0x00 | module id | 6 / 0x21 / 0x22 / 2 |
| 0x04 | dev id | |
| 0x08 | input ports (≤ 16) | 0 / 1 / 1 / 1 |
| 0x0c | output ports (≤ 8) | 2 / 3 / 6 / 0 |
| 0x10 | channels (≤ 48) | 1 / 32 / 34 / 33 |
| 0x14 | passes (≤ 4) | 1 / 1 / 1 / 1 |
| 0x18 | u8 → device +0x1c: channel-heap flag (below) | VENC 1 |
| 0x1c | worker thread priority (0 → 50) | module constant |
| 0x20 | u8 → device +0x1d: per-channel wake-up counting in `NotifyWakeupISREvent` [I] | |
| 0x24, 0x28 | `GetInterBindInputPortPassId`, `GetInterBindOutputPortPassId` | ISP, SCL |
| 0x2c, 0x30, 0x34 | `ConfigSramBuf`, `AllowPreemptionSram`, `AllowMoveableSram` (§9) | SCL, VENC, DISP |
| 0x38 + 0x98·p | pass p callback table (§2.3) | |

**Info +0x18**: marks a device whose channels take their MMA heap from mi_sys's channel heap setting: on platform `"I2M"`
each channel gets the heap name `"MMU_MMA"` at creation (a 7-character literal; "MMU_MMA" is the only such string), and
`SetChnMMAConf` (ioctl 0x0f) refuses such a device with `0xA0092008`. Not effective on M6. The platform table is in §9.

### 2.3 Pass callback table (0x98 bytes)
Offsets inside the table; in the device pass the table starts at +0x0c (slot *s* = device pass +0x0c + *s*). The task
callbacks are called `fn(cmdq service or NULL, task, …, usrData)`. "Kept chain" = VIF, ISP, SCL, VENC, DISP of this camera;
other modules are named where a source has read them.

| slot | callback / meaning | signature, when called | set by |
|---|---|---|---|
| 0x00 | `OnPreProcessInputTask` | `(cmdq, &inTask.info, &meta, usrData)`, §4.1 step 3 | ISP SCL VENC DISP |
| 0x04 | `OnPrepareOutputTaskMetaInfo` | `(cmdq, &inTask.info, &outTask.info, usrData)`, after buffer allocation | ISP SCL |
| 0x08 | `EnqueueInputTask` | `(cmdq, &inTask.info, &outTask.info or NULL, usrData)`, §4.1 step 7 | ISP SCL VENC DISP |
| 0x0c | `BarrierInputTask` | `(CMDQ object +0xc or 0, &task +0x40, 0, usrData)`, §6.4 | ISP SCL VENC DISP |
| 0x10 | `OnPrepareAsyncOutputTaskMetaInfo` — the output-task counterpart of slot 0x04 | | mi_jpd; mi_vdisp (`vdisp_OnPrepareAsyncOutputTaskMetaInfo`, returns 0) |
| 0x14 / 0x18 | `EnqueueOutputTask` / `BarrierOutputTask` | ASYNC thread, §6.5 | VIF, DISP write-back |
| 0x1c / 0x20 | `CheckInputTaskStatus` / `DequeueInputTask` | §7.2 | ISP SCL VENC DISP |
| 0x24 / 0x28 | `CheckOutputTaskStatus` / `DequeueOutputTask` | `CheckOutputTaskStatus(cmdq, &task.info, msSinceEnqueue, usrData)`, §7.2 | all |
| 0x2c | `OnPollingAsyncOutputTaskConfig` (its presence creates the ASYNC thread) | §6.5 | VIF, DISP write-back |
| 0x30 | `NeedIndividualWorkThread` | `(modDev, devPass + 0x238, usrData)`, every pass-graph run (§8.3) | SCL dev 1/3, VENC, DISP |
| 0x34 | `RejectCmdq` — 0 = the pass takes a CMDQ | `(devPass, passId, &u32, usrData)`; the third argument receives the module's CMDQ buffer size (`_MI_ISP_IMPL_RejectCmdq`, `_MI_SCL_IMPL_RejectCmdq` write it and return 0 when the device's CMDQ flag is 1, else return 1); called when the pass takes its CMDQ (§7.1) and in the pass-graph worker (§8.3) | ISP SCL VENC DISP |
| 0x38 | `BarrierByPass` | `(word, passId, usrData)`, §6.4 | DISP (`_MI_DISP_BarrierByPass` always returns 1) |
| 0x3c | never read by mi_sys (no load from device pass +0x48 other than the copy of the table) — reserved in this build | | none |
| 0x40 / 0x44 | `OnGetInputPortBindCapability` / `OnGetOutputPortBindCapability` | §3.2 | per module, §3.3 |
| 0x48 / 0x4c | `OnBindInputPort` before / after linking | `(modDev, &selfChnPort, &peerChnPort, &bindAttr, usrData)`; 0x4c once the input is linked into the output's list and the graph updated (last step of the bind) | 0x48: kept chain, mi_shadow; 0x4c: mi_vdisp (`vdisp_OnBindInputPort`) |
| 0x50 / 0x54 | `OnUnBindInputPort` before / after (VENC names its +0x54 `…Post`) | | 0x50: DISP, mi_shadow; 0x54: ISP SCL VIF VENC |
| 0x58 / 0x5c | `OnBindOutputPort` before / after linking | same signature as 0x48, self = output side | 0x58: kept chain, mi_shadow; 0x5c: mi_vdisp (`vdisp_OnBindOutputPort`) |
| 0x60 / 0x64 | `OnUnBindOutputPort` before / after | | 0x60: DISP VENC, mi_shadow; 0x64: ISP SCL VIF |
| 0x68 | `OnOutputPortBufRelease` | `(device, &ChnPort, &bufinfo)` when the last reference to an output buffer of the pass goes (§4.5) | mi_jpd (`OnOutputPortBufRelease`), mi_vdec (`ReleaseFrameCallBack`) |
| 0x6c / 0x70 | `OnGetPassInputPortNum` / `GetPassOutputPortNum` | | ISP SCL |
| 0x74 / 0x78 | inner-pass port selection | `(modDev, chn, pass, usrData)` → input port id to enable on pass p > 0 / output port id to enable on pass p < last, 0xFFFF = none (`MI_SYS_IMPL_EnableChannel`) | only multi-pass devices; none on this camera |
| 0x7c | `OnSupportOutputTaskReorder` — out-of-order completion of output tasks is allowed only when it says so | called in `MI_SYS_IMPL_DequeueInputOutputTasks` | mi_shadow (returns 1), mi_vdec; none of the kept chain |
| 0x80 | input-side frame-rate statistics hook (device pass +0x8c) | for statistic block 1 only (§4.4) | none of the kept chain |
| 0x84 | output-side frame-rate statistics hook (device pass +0x90) | `(modDev, &ChnPort, &portFrcStats)` from `MI_SYS_IMPL_CalcOutputPortFrameRate`, block 1 only | none of the kept chain |
| 0x88 | never called; mi_sys only tests it for presence: a pass with it stops the pass-graph tree walk and is accepted as a self-barrier node (§8.3) | | no module |
| 0x8c | u32 **concurrent-task limit** — number of input tasks in flight the pass accepts (device pass +0x98); ≤ 0 → 1 without a CMDQ, 2 with one; more than 4 × channels → log "has only %d devchns but required %d concurrent tasks, force it to be %d" and clamp | | ISP 0x80, SCL 1 or 0x88, JPD 1, VDEC 3, mi_shadow 2, mi_vdisp `vdisp_bufq_depth` × 20 |
| 0x90 | never read as a word (no load from device pass +0x9c); its high half is the u16 of slot 0x92 | | — |
| 0x92 | u16 output-task timeout in ms (device pass +0x9e; the timeout log fires after it or after 5 s, §7.2) | | JPD 200, VDEC 10000 |
| 0x94 | `ResetDevPass` (after a CMDQ reset, §7.1) | | ISP SCL |

The registration values of the other modules are in their documents (`mi-capture.md`, `mi_venc.md`, `mi_disp-mi_hdmi.md`,
`mi_jpd-mi_vdec.md`, `mi_vdisp-mi_shadow.md`).

## 3. Binding [R]

### 3.1 Bind types
| value | name | how a consumer gets buffers |
|---|---|---|
| 1 | FRAME_BASE | buffers allocated by mi_sys, handed over when the producer's task has finished (§4.6) |
| 2 | SW_LOW_LATENCY | never chosen by the negotiation (§3.2) |
| 4 | REALTIME | descriptor-only buffer (`mi_sys_create_realtime_bufref`, no memory), handed over before the producer's `EnqueueInputTask` |
| 8 | HW_AUTOSYNC | sequence numbers from the producer device pass's shared table |
| 0x10 | HW_RING | buffers from the consumer's allocator (`"HWRingBuf"`; the ring-heap allocator `HWRING%04x_%d` at input +0x148 is created by the pass graph, §8.4); keeps the bind parameter |

`bInternalBusExchgmode(t)` = (t == REALTIME); `bLowLatencyKickOff(t)` = (t != FRAME_BASE) [R disassembly]. With REALTIME
the hardware link between the two blocks is made by the modules themselves in their bind callbacks [I].

[S] (SigmaStar Tiramisu MI_SYS document `sys_zh` §1.5, §2.4.2, §3.2.21): the bind parameter is the latency in
ms for SW_LOW_LATENCY and the ring depth in lines for HW_RING; HW_RING exists only between VPE and VENC (H.264 / H.265), one
path; REALTIME is one path for JPE / IMI, one for VIF → ISP and several for ISP → SCL; FRAME_BASE (the default) and HW_RING keep
the frames in DRAM, REALTIME in SRAM; ports bound REALTIME or HW_RING cannot be used with GetBuf / PutBuf. Frame-rate control:
under FRAME_BASE the upstream output port reduces the rate, under REALTIME / HW_RING only the downstream input port does; mi_sys
measures the source rate itself, with 1–2 s of jitter after a large change, and the doc asks for a new BindChnPort when the rate
changes; only rate reduction is described. [R] mi_sys.ko carries the error `Not allowed call _MI_SYS_IMPL_MmapToUserVirAddr in
realtime mode!`. [S] (SigmaStar FAE, 2022): the PTS a frame carries is stamped by VIF when VIF → ISP is bound
FRAME_BASE, and by the ISP when VIF → ISP is REALTIME and ISP → SCL FRAME_BASE.

### 3.2 Bind procedure (`_MI_SYS_IMPL_BindChannelPort`, ioctls 0x02 / 0x03)
1. Find the destination input port and the source output port; refuse if both are in the same device, or if the input port
   is in user-picture mode → `0xA0092003`.
2. Input already bound: to the same output with a compatible type → only the frame rates are updated, return 0; else error.
3. Output already feeding other inputs → REALTIME and HW_RING are removed from the requested types; an existing REALTIME /
   HW_RING consumer blocks further binds.
4. Filter (`_MI_SYS_IMPL_FilterBindTypePossibilityMask_NoLock`): every channel of the destination device pass is checked; an
   input already bound REALTIME must come from the same source device pass, channel pass and port, else `0xA0092012`.
5. Negotiation. Both 0x14-byte capability records (output side, input side) are local and **zeroed** before the calls. First
   the **output** pass's slot 0x44 `fn(outModDev, &outChnPort, &inChnPort, &capOut, outUsr)` (device pass +0x50), then the
   **input** pass's slot 0x40 `fn(inModDev, &inChnPort, &outChnPort, &capIn, inUsr)` (device pass +0x4c). Both are called
   **without a NULL check** (`ldr r4,[sl,#0x50]; blx r4` at 0x3f20 and `ldr r4,[r6,#0x4c]; blx r4` at 0x3f3a, file `.text`
   0x3b04…0x469c, Ghidra 0x13b2c) and **their return values are ignored**. The record:

   | off | field | combination |
   |---|---|---|
   | +0x00 | u16 (in a u32) | a (input) / b (output): equal → that value; one of them 0 → the other; else lcm = (a·b) / gcd(a, b) truncated to 16 bits |
   | +0x04 | u16 | same |
   | +0x08 | u16 | same |
   | +0x0c | u32 bind-type mask (bits as §3.1) | capIn & capOut & requested & filter mask; 0 → `0xA0092008` |
   | +0x10 | u32 mask | capIn & capOut, reduced to its lowest set bit (0 when both are 0) |

   Chosen type by priority: REALTIME (4) > HW_RING (0x10) > FRAME_BASE (1) > HW_AUTOSYNC (8); SW_LOW_LATENCY (2) is never
   chosen. The **bind attribute** `{lcm0, lcm1, lcm2, chosen type, param}` is handed to the four bind callbacks (slots 0x48 /
   0x58 before linking, 0x4c / 0x5c after); `param` = the lowest common bit of +0x10, replaced by the caller's bind parameter
   when the chosen type is HW_RING (also stored at input port +0x14c).
6. `OnBindInputPort` (slot 0x48), `OnBindOutputPort` (slot 0x58), link the input into the output's bind list, input +0xd4 :=
   type, +0xd8 := 1 (bound), frame rates set (`MI_SYS_IMPL_SetFrameRate`, §4.4), device pass +0x228 := bind time,
   `MI_SYS_InferGraph_OnBind` under the graph lock (§8.1); then the two after-link callbacks (slots 0x4c / 0x5c).

**What the three u16 carry.** In this build no module writes +0x00 / +0x04 / +0x08 or the +0x10 mask: every fill function
writes only the type mask +0x0c (§3.3). The three lcm results are therefore always 0, and the bind attribute is
`{0, 0, 0, type, param}` with `param` = 0 for every type except HW_RING. The only consumers that copy the whole attribute —
ISP (`OnBindInputPort` / `OnBindOutputPort` → channel +0x204…+0x214 and port +0x254…+0x264), SCL (+0x10c…+0x11c and
+0x1d4…+0x1e4), vdisp (C+0x30) — only store and clear words 0–2; no other access to those offsets was found in their
decompiles [R offset search; I: no indirect access]. VENC and JPD read only +0x0c (type) and +0x10 (param). What the three
values were designed to carry is not known: nothing in this build writes or reads them; an alignment meaning (by their lcm
treatment, like the ext-conf of §4.2) is a guess with nothing to confirm it [I].

**UnBind** (`_MI_SYS_IMPL_UnBindChannelPort`) mirrors the bind with the unbind slots (before: 0x50 / 0x60, after: 0x54 /
0x64), resets the input's measured source rate (§4.4) and calls `MI_SYS_InferGraph_OnUnBind`. Modules unbind from inside with
`mi_sys_UnBindChnPort(0, &arg)` (ISP, SCL, VIF).

### 3.3 Capability fill code of every module
<!-- ev:mi_sys-1-engine.3.3 -->
Mask bits as §3.1. "BUG" = `MI_COMMON_Print("…BUG ON…")` + `udf` when the record pointer is NULL (all fill functions except
the stubs check it).

| module (id) | input side, slot 0x40 | output side, slot 0x44 |
|---|---|---|
| mi_vif (6) | not set (no input ports) | 1; **5** when the own port is 0 and the peer module is ISP 0x21 or SCL 0x22 |
| mi_isp (0x21) | 1; **5** when the peer is VIF (module 6) port 0 | **4** (REALTIME only) for own port 0, **1** for ports 1/2 |
| mi_scl (0x22) | by the channel's type (channel +0x18): 0 or 2 → **4**, 1 or 3 → **5**, 4 → **1**, any other value → nothing written (mask 0 → bind refused) + log; dev > 3, device not created, chn > 0x21 or channel not created → nothing written, log, returns `0xA0222000` (ignored by mi_sys; the empty mask refuses the bind) | **0x1D** (1·4·8·0x10) for every port |
| mi_venc (2) | 1; from SCL also 0x10 (+8 on chips > 4) when engine capability byte +8 is set; 4 when byte +0xa is set | 1 |
| mi_disp (0xF) | **0x11** (FRAME_BASE + HW_RING) | 0x11 |
| mi_disp WBC device | — | **0x11** (`_MI_DISP_WbcOnGetOutputPortBindCapability`) |
| mi_jpd (0x20) | stub `return 0`, writes nothing → any bind into JPD is refused (`0xA0092008`) | **5** |
| mi_vdec | stub `return 0`, writes nothing → refused | **1** |
| mi_ldc (0x17) | 1, after a device sanity check (usrData non-NULL with the module magic ≠ 0x56533333 at `.LANCHOR1`+4, else a walk of the device list for an entry `{+0 u8 1, +4 0, +8 0x56577724}`; none → BUG) | 1, same check |
| mi_vdisp (0xE) | 1 (no NULL check, no port check) | 1 |
| mi_shadow | **0x15** (1·4·0x10) | 0x15 |
| mi_mipitx (0x1e) | 1 (no NULL check) | stub `return 0`, writes nothing → binds **from** mipitx are refused |
| mi_ai (4), mi_sensor (0x1B), mi_ipu (0x1D) | **slots NULL** (registration info memset to 0, no capability callbacks) | **slots NULL** |
| mi_ao (5), mi_gfx (0xD), mi_panel (0x19), mi_hdmi (0xB), mi_rgn (3) | slots NULL; no ports or no channels registered | slots NULL |

NULL slots with real ports [I from the unchecked `blx`]: mi_ai (1 output port, 8 channels, 1 pass), mi_sensor (1 output
port, 1 channel, 1 pass) and mi_ipu (1 input + 1 output port per channel, 0x30 channels) register ports, so an
`MI_SYS_BindChnPort` that names one of their ports and passes the earlier checks of the bind would branch to address 0
(kernel oops) instead of returning an error. Nothing on this camera binds them.

### 3.4 Binds on this camera
The app's binds (`prog_uvc.md` §2.3, §5.2): VIF→ISP REALTIME, ISP→SCL REALTIME (FRAME_BASE in the YUV branch with `-H`), SCL→DISP
FRAME_BASE, SCL→VENC FRAME_BASE (HW_RING without `-H`). The resulting roles of the passes are in §8.5.

## 4. Frame flow [R unless marked]

Output buffers are allocated by mi_sys for each task of a producer pass, capped by the port's queue depth (quota), from the
buffer configuration the module fills in `OnPreProcessInputTask`; a finished buffer is referenced into every accepting bound
input port and into the port's user FIFO; buffers are reference counted and freed when the last holder releases them.


Functions [R]: `_MI_SYS_KickOff_InputTask` 0x2a458, `_MI_SYS_IMPL_PrepareOutputTaskBuf` 0x29e40,
`_MI_SYS_IMPL_GetOutputPortBuf` 0x29558, `MI_SYS_IMPL_FinishAndReleaseOutputTask` 0x19638,
`_MI_SYS_IMPL_OutputTaskDispatch_Func` 0x44494 (none of them has a symbol).
### 4.1 One task, step by step (`_MI_SYS_KickOff_InputTask`, run by the MAIN thread of the pass or of its root, §6, §8)
Buffers are configured by `OnPreProcessInputTask`, allocated right after it, and only then is `OnPrepareOutputTaskMetaInfo`
called.
1. **CMDQ check.** If the CMDQ the thread now holds differs from the one the pass used last (+0x230), wait (1 ms polls, log
   every second) until the pass has no input, output or finished tasks in flight, then switch.
2. Channel disabled → result 3 (drop the input).
3. **`OnPreProcessInputTask(cmdq, &inTask.info, &meta, usrData)`** (slot 0x00). `meta` is a zeroed 0x3d8-byte block the module
   fills: +0x04 + i u8 "port i wants a buffer" (8 ports), +0x0c u8 pass-through, +0x18 + 0x78·i the `MI_SYS_BufConf_t` for
   port i. Return 0 = go on, 1 = not now (result 2, the input stays queued), 2 = drop (result 3). Time is measured (log above
   4 ms / 200 ms).
4. **Output task** only if some port wants a buffer: `MaxValidOutputPortNum` = highest such port + 1; a 0xd0 object from the
   output-task cache gets the input task's PTS / flags; the SRAM fields (+0xa4..0xb0) are taken from the device (§9).
5. **`_MI_SYS_IMPL_PrepareOutputTaskBuf`** — for each port with its flag set:
   - channel or port disabled → skip;
   - pass-through: `meta+0x0c` and `+0x04` set, one input port, input buffer present → the output is a duplicate reference of
     the input buffer (no allocation);
   - the port has no bound input, user depth 0 and the pass has no input ports (device pass +0xa4 = 0, a source pass) → nobody
     would consume it: mark +0x64+i, no buffer;
   - buf-conf checked (type RAW / META need a size; FRAME / MULTIPLANE need width, height, scan ≤ 1, format ≤ 0x53); invalid
     → release what was taken, `0xA0092003`;
   - otherwise **`_MI_SYS_IMPL_GetOutputPortBuf`** (§4.3); the PTS and two task words are copied into the buffer's info.
   Result: `0xA009200C` if no port got a buffer.
6. Prepare failed → the output task is dropped, and the **input is still handed to the module without an output task** (the
   frame is consumed and lost for every output). Success → `OnPrepareOutputTaskMetaInfo(cmdq, &inTask.info, &outTask.info,
   usrData)` (slot 0x04) with the buffers in place, then `MI_SYS_Kickoff_RealTime_OutputTasks` (consumers bound REALTIME are
   given their buffers at once, §4.6).
7. **`EnqueueInputTask(cmdq, &inTask.info, &outTask.info or NULL, usrData)`** (slot 0x08), timed like step 3. Result 0 =
   queued on the hardware: the output task goes onto the thread's list and is completed later by
   `MI_SYS_IMPL_DequeueInputOutputTasks` (§7.2); 1 = done synchronously: dispatched and finished at once; 2 = retry later;
   3 = drop. Results are counted per channel (+0x1b18 / +0x1b24). The pass's `input_buf_monitor_waitqueue` (+0xfc) is woken
   unless the result is 2.

### 4.2 Buffer geometry: ext-conf and `mi_sys_buf_mgr_get_size`
**Ext-conf** (`mi_sys_Set{Input,Output}PortBufExtConf(h, chn, pass, port, &ext)`): 8 bytes `{u16 strideAlign; u16
lumaLineAlign; u16 chromaLineAlign; u8 flag}`. Stored as: strideAlign = lcm(value, 2) (0 → 2), line aligns 0 → 1, flag → port
byte (+0x20a out / +0x156 in). Port default `{2, 1, 1, 0}`. Pass −1 = last pass (output) / pass 0 (input). At allocation
time the producer's ext-conf is merged with every bound input port's: each u16 = lcm of the two; the flag byte (+6) gets bit 0
if any consumer set its ext-conf flag and bit 4 if the producer's clear-conf (+0x20c, `mi_sys_SetOutputPortBufClearConf`) or
any consumer's clear-conf (input +0x158, `mi_sys_SetInputPortBufClearConf`) is set. The MMA allocator clears the padding of a
FRAME buffer (`_mi_clear_framebuf_pading_area`, the area between the picture and the aligned stride / lines) whenever that
byte is non-zero. So a consumer can demand alignment and clean padding of the producer's buffers.
`mi_sys_GetOutputPortBufExtConf` returns the same merge (port value lcm-combined with all bound inputs).

Ext-conf values the modules of this camera pass (from their disassembly; the constant word 0x00020010 = {16, 2} is what
Ghidra rendered as "`{ptr, 1}`" / "`{"cCnt", 1}`"):

| caller | call | value `{strideAlign, lumaLineAlign, chromaLineAlign, flag}` |
|---|---|---|
| mi_vif `EnableOutputPort` | SetOutputPortBufExtConf | {16, 2, 1, 0} |
| mi_isp input / `EnableOutputPort` | SetInput/OutputPortBufExtConf | {16, 2, 1, 0} |
| mi_scl output (two sites) | SetOutputPortBufExtConf | {align, lines, 1, 0} with per-port values; or {0x20 for pixel formats < 2 else 0x10, 2, n, flag} |
| mi_scl input (two sites) | SetInputPortBufExtConf | {a, 2, c, 0} computed; {16, 2, n, …} |
| mi_disp input | SetInputPortBufExtConf | {0x10, 2, 0} or {0x80, 0x40, 0x40} with rotation (`mi_disp-mi_hdmi.md` §4.8.3) |

**Frame size** (A = strideAlign, H = lumaLineAlign, C = chromaLineAlign; `al(x, a)` = round x up to a multiple of a; w, h
from the buf-conf; output: plane offsets, strides, total):

| pixel format (MI_SYS_PixelFormat_e) | stride | total size |
|---|---|---|
| YUYV, UYVY, YVYU, VYUY (0, 0xe–0x10) | al(2·al(w,2), A) | stride · al(h, H) |
| ARGB8888, ABGR8888, BGRA8888, RGB101010 (1–3, 0x53) | al(4w, A) | stride · al(h, H) |
| RGB565, ARGB1555, ARGB4444 (4–6) | al(2w, A) | same |
| I2 / I4 / I8 (7 / 8 / 9) | al(⌈w/4⌉, A) / al(⌈w/2⌉, A) / al(w, A) | same |
| YUV semi-planar 422 (0xa) | al(w, A), both planes | 2 · stride · al(h, H) |
| NV12 / NV21 (0xb, 0xc) | al(w, A), both planes | stride · (Ly + al(Ly/2, C)), Ly = al(al(h,2), H); UV offset = stride · Ly |
| tile 420 (0xd) | al(3w/2, A) | stride · al(h, H) |
| YUV422 planar (0x11) | Y al(al(w,2), A), U/V half | stride · (L + al(L, C)), L = al(h, H) |
| YUV420 planar (0x12) | Y al(al(w,2), A), U/V half | stride · (L + al(L/2, C)), L = al(al(h,2), H); V offset = 1.25 · Y size |
| FBC 420 (0x13) | al(w, A) | (stride + al(stride/2, C)) · al(al(h,2), H) |
| Bayer 8 bit (0x14–0x1f) | al(w, 16), max 0xfff0 (A not applied) | stride · al(h, H) |
| Bayer 10 bit (0x20–0x2b) | 80 · ⌈w/64⌉ (packed), max 0xfff0; compress mode 4 (to 8 bit) → al(w, A) | stride · al(h, H) |
| Bayer 12 bit (0x2c–0x37) | 48 · ⌈w/32⌉, max 0xfff0; compress 4 → al(w, A) | same |
| Bayer 14/16 bit (0x38–0x4f) | al(2w, 16), max 0xfff0 | same |
| RGB888, BGR888, GRAY8 (0x50–0x52), anything else | not supported: logs and BUG_ON | |

Buf types: RAW (0) and META (2) take the size from the buf-conf (+0x18); MULTIPLANE (3) uses the same table with the frame
config shifted by 4 bytes; FBC (4) is not handled here. The buf-conf layout matches the M6 header `MI_SYS_BufConf_t` (type
+0x00, flags +0x04, u64 target PTS +0x08, bDirectBuf +0x10, union at +0x18: width, height, scan, format +0x08, compress
+0x0c).

### 4.3 Getting one output buffer (`_MI_SYS_IMPL_GetOutputPortBuf`)
1. Scan the port's bound inputs: any REALTIME (4) or HW_RING (0x10) consumer → *real-time port*; HW_AUTOSYNC (8) → sequence
   numbers come from the device pass's shared table.
2. **Quota** (not for real-time ports): if *buffers alive* (+0x20) ≥ `BufCntQuota` (+0x14) → no buffer (debug log "Total
   Used Buffer >= Total Output Port Buffer Quota"). This is the queue-depth cap: every allocation of the port counts until it
   is freed, wherever it is (in the hardware, queued at a consumer, in the user FIFO, held by user space).
3. **User demand**: room = user depth (+0x0c) − buffers held by user space (+0x1c). The user side wants this frame if room ≥ 1
   and — in block mode (+0x18) — the FIFO is not already holding `room` buffers.
4. `MI_SYS_IMPL_CalcOutputPortFrameRate(port, 0)` (statistic block 0, §4.4; two call sites, one on each branch of the
   user-demand test). No bound inputs: user wants → allocate; user does not → no buffer; the port is marked "blocked" if the
   pass has input ports (+0xa4 ≠ 0).
5. **Frame-rate control per bound input** (`MI_SYS_IMPL_RcAcceptBufAlloc`, §4.4). If no consumer accepts and the port is not
   real-time: every input's dispatch and alloc accumulators are stepped, no buffer, port marked blocked — **a frame no
   consumer wants is never allocated**. Otherwise the alloc accumulators are stepped and the frame is allocated. The sideband
   messages of the accepting consumers are merged into the buffer (§5).
6. **Allocation** by buf type: FRAME with a REALTIME consumer → `mi_sys_create_realtime_bufref` (descriptor only, no memory);
   FRAME with a HW_RING consumer → that consumer's allocator (`"HWRingBuf"`); META → the global meta allocator
   (`"sys-meta"`); otherwise size per §4.2 and the first present of: the port's private heap allocator (+0x15c, re-created
   through `ConfigPrivateMMAPool` when the size changed and MMU heaps are on), the user allocator (+0x158), else
   `_MI_SYS_IMPL_AllocBufDefaultPolicy` (the channel's allocator collection → MMA, owner name = port name). Allocator calls are
   `ops+0x0c alloc(allocator, name, bufconf, ext)`; the allocation gets debug tags (+0x18 = mod<<16 | chn<<9 | 0x8000, +0x1c
   = 0x4D4E0000 | mod>>16). The allocators and heaps themselves are in `mi_sys-2-interfaces.md` §7.2, §7.6.
7. After success: `MI_SYS_IMPL_CalcOutputPortFrameRate` with statistic block 1 (0x29a08); the allocation's `OnRelease` is
   wrapped (`_MI_SYS_IMPL_Ouputport_wrapper_allocation_on_release`), *buffers alive* +1, kernel mapping if buf-conf flag bit 31
   (`MI_SYS_MAP_VA`), sequence number = port counter +0x1f0 (wraps at 0xFFFFFFFE) or the shared table, chn id stored.

### 4.4 Frame-rate control


The bound-input loop of `MI_SYS_IMPL_Calc{Input,Output}PortFrameRate` is `.text` 0x3d278–0x3d3f0 [R].
Input port: s = declared src rate num / den at +0x18 / +0x1c, d = dst rate num / den at +0x20 / +0x24, both set at bind by
`MI_SYS_IMPL_SetFrameRate(port, src, dst)` (each as `num | den << 16`, den 0 → 1); m = the measured source rate pair at
+0x80 / +0x84 (below). The rate used as "source" is m when m is set, else s.
- **Alloc side** (`RcAcceptBufAlloc`, at allocation, §4.3 step 5): accept if channel / port enabled and (first frame — +0x88
  = 0 —, or s == d, or acc_a ≥ src.num · d.den, then acc_a −= src.num · d.den); `StepRcBufAlloc` adds src.den · d.num to
  acc_a (+0x7c) for every produced frame. Accepted / produced = (d.num/d.den) / (src.num/src.den).
- **Dispatch side** (`RcAcceptBufDispatch`, when a finished buffer is offered to the input): the same rule on acc_d (+0x78)
  with its own counters (+0x88 offered, +0x8c accepted); `StepRcBufDispatch` adds src.den · d.num. Two stages let several
  consumers of one output run at different rates.
- **Time mode**: src rate −1 with a real dst rate sets acc_a = −1. Then a frame is accepted when the elapsed time since the
  last accepted one (output port +0x40, ns) plus a carried remainder (+0x48, ms) reaches the dst interval d.den·1000/d.num ms,
  or when the producer forced it (`mi_sys_SetOutputPortBufForceAlloc`, byte +0x20d, cleared on use); dispatch always accepts.
  Time mode ignores the measured pair.

**Statistic blocks.** Output and input ports each have two blocks of 0x28 bytes at +0x28 (block 0) and +0x50 (block 1):
+0x00 frames·1000, +0x04 window ms, +0x08 frame count of the running window, +0x0c jiffies at window start, +0x10 u64 ns at
window start (`_MI_SYS_IMPL_CalcFrameRate`, inlined). Each call counts one frame; when more than `HZ` jiffies have passed the
window closes: frames·1000 and elapsed ms (0 → 1000) are stored and the count restarts. Output port: block 0 is counted for
every buffer request that passed the quota test, block 1 after a buffer was actually allocated (§4.3), so block 0 = the rate
at which the producer asks for output buffers and block 1 = the rate of allocated buffers [I from the call sites]; when block 1
closes, pass slot 0x84 (device pass +0x90) is called with it (the hook is guarded by `block == output + 0x50`). Input port:
block 1 is counted in `_MI_SYS_IMPL_OutputTaskDispatch_Func` for every offered buffer, block 0 in
`_MI_SYS_IMPL_SWDMAPinonBind_ProcFunc`; slot 0x80 (device pass +0x8c) is called for block 1 only. +0x90 / +0x94 / +0x98 are
the FinishCnt / RewindCnt / DropCnt columns of the procfs FRC dump (`MI_SYS_IMPL_DumpInputportFrcInfo`).

**Measured source rate (+0x80 / +0x84, `stPrevBindPortFR` — name from the log string).** Set by mi_sys itself; no module and
no user call writes it.
- Writers: `MI_SYS_IMPL_InitInputPortFRC` (from `_MI_SYS_IMPL_InitChannel`) and `MI_SYS_IMPL_ResetInputPortFRC` (from
  `_MI_SYS_IMPL_UnBindChannelPort` and `MI_SYS_IMPL_DisableInputPort`) store −1 to both words;
  `MI_SYS_IMPL_CalcOutputPortFrameRate` stores the measurement (`stmia` at node − 0x4c, 0x3d330). A scan of every `str` /
  `strd` with offset #128 / #132 or (node-relative) #−76 / #−72 in the disassembly finds only stack slots besides these; stores
  through register-computed addresses are not covered by that scan, and the reading of all FRC callers found none.
  `MI_SYS_IMPL_SetFrameRate` (at bind) does not touch the pair, so a new bind starts from the −1 left by the unbind or channel
  init. (Ghidra appends the next function's body to `MI_SYS_IMPL_ResetOutputPortFRC` / `InitOutputPortFRC` after the
  no-return assert stub `assert_noreturn` (0x4c540); the output-port reset itself does not touch any input port [R: function boundaries in
  the disassembly].)
- Update by `MI_SYS_IMPL_CalcOutputPortFrameRate(outPort, 0)`, only for output block 0 and only right after its window closed, for every input port on the output's bind list
  (output +0x188, node at input +0xcc) whose channel and port are enabled and whose src rate ≠ dst rate (num or den differ)
  [R 0x3d298–0x3d3e4]:
  - the pair counts as **set** when (+0x84 ∉ {−1, 0} and +0x80 ≠ 0) or (+0x84 = −1 and +0x80 ∉ {−1, 0});
  - set → overwrite: +0x80 = output +0x28 (frames·1000), +0x84 = output +0x2c (ms); no check, accumulators kept;
  - not set, input in time mode (+0x7c = −1) → the same overwrite;
  - not set, ratio mode: r = ⌊(ms · d.num) / (frames·1000 · d.den)⌋ (dst rate / measured rate, integer). r > 1 → the pair
    is left unset and, at debug level > 4 or a debug-filter match, INFO "InputPort(mod,dev,chn,pass,port), FrameRate
    tolerance is more than twice big,ignore update stPrevBindPortFR. RtSrc:(frames·1000 / ms) Dst:(d.num / d.den)" (line
    0x24a). r ≤ 1 → both accumulators are rescaled to the new unit, acc = ⌊acc / s.den⌋ · ms, for +0x7c (alloc) and +0x78
    (dispatch), then the pair is written as above.
- Use: when set, the threshold is `m.num · d.den` and the step `m.den · d.num` in all four FRC functions — the pair replaces
  the **whole** source rate, and the accepted share becomes (d.num/d.den) / (measured rate). Effect [I]: the declared source
  rate given at bind only drives the first second (and stays in force if the measured source was below half the destination
  rate at the first measurement); after that a consumer's frame-rate conversion follows the producer's real rate,
  re-measured every second. Unused corner: with +0x84 = −1 and +0x80 valid the step adds (−1)·d.num, i.e. decrements the
  accumulator; the writer never produces that combination (ms ≥ 1) [I].

### 4.5 Finished output: consumers, user FIFO, back-reference queue, freeing
`MI_SYS_IMPL_FinishAndReleaseOutputTask` per port with a buffer:
- module set "rewind" (+0x8e+i) → `_MI_SYS_IMPL_OutputPortRewindBuf`: the buffer is released without being shown, dropped
  counter +0x84 +1 (for RAW / FBC / direct buffers the quota count is given back at once, others when freed).
- else: allocation hook +0x24 if set; cache flush of the mapped part; consumers bound FRAME_BASE get the buffer first
  (`MI_SYS_InferGraph_Finish_Output_Task`, §4.6); then the **user FIFO** (`_MI_SYS_IMPL_DupBufrefIntoOutputFifoQueue`, unless
  the port is in low-latency mode +0x20e and the task's bit was already cleared by the early delivery of §4.6): if channel and
  port enabled and room = user depth − user-held > 0, a reference is appended; while the FIFO holds more than room, the
  **oldest is dropped** (u64 counter +0x150); the poll object, if any, is woken. No room → FIFO flushed.
- **driver back-reference queue** (`_MI_SYS_IMPL_DupBufrefIntoDrvBkRefQueue`): keeps the last N = +0x10 buffers referenced (N
  set by `mi_sys_EnsureOutportBkRefFifoDepth`, must be < quota; used only by mi_vdisp) — N = 0 keeps nothing.
- finished counter +0x80 +1; the task's own reference is released.

**Delivery to a consumer** (`_MI_SYS_IMPL_OutputTaskDispatch_Func` per bind): if the input is bound and `RcAcceptBufDispatch`
accepts → `MI_SYS_IMPL_EnqueueBufToInputPort`: channel / port disabled → dropped; else a duplicate reference, stamped with the
enqueue time, goes into the consumer pass's input queues and the consumer's thread is woken (§6.3); the input's latest-buffer
slot (+0x1d4) keeps the newest one. While the SRAM-change flag `g_bSramChange` is on (§9), a FRAME_BASE input that already has ≥
(producer's alive buffers − 1) queued drops the new one. Input byte +0xae keeps a duplicate of the last buffer (+0xb0) for
repeat.

**Freeing**: when the last reference to an allocation goes, the wrapper calls `_MI_SYS_IMPL_OnAllocationFreeCB`: the producer
pass's slot-0x68 callback `OnOutputPortBufRelease` gets `(device, &ChnPort, &bufinfo)` (a META buffer is mapped around the
call), *buffers alive* −1, and if the port was at its quota the producer pass's input and output thread wait queues (+0xcc,
+0xe4) are woken — a pass blocked on the quota restarts here. Buffers in tasks (+0x24) are counted separately; reaching 0 wakes
+0x1d0 (what `BlockingDisableOutputPort` waits for).

### 4.6 When a consumer gets a buffer — by bind type
The output graph iterator (`_MI_SYS_IMPL_OutputGraphIterator` (0x42fb4), unnamed `_MI_SYS_IMPL_OutputGraphIterator`) walks every output port of a channel
pass and every input port bound to it, calling one of three per-bind functions; each skips ports the task marked "no buffer".
"Infer type" is the consumer input's type set by the pass graph (§8.4).

| moment | caller | handler | binds handled | action |
|---|---|---|---|---|
| right after `OnPrepareOutputTaskMetaInfo`, before `EnqueueInputTask` | `MI_SYS_Kickoff_RealTime_OutputTasks` | `_MI_SYS_IMPL_RealtimeBind_ProcFunc` | REALTIME (4) | hand the (descriptor-only) buffer to the consumer at once; consumer infer type 2 → run the consumer pass now in this thread (`MI_IMPL_KickOff_SyncInferenced_DevPassInputBufs`); type 1 → `MI_SYS_IMPL_EnsureInputPortFifoEmpty` |
| `EnqueueInputTask` returned 1 (done synchronously), or delayed passes at the end of a MAIN round | `MI_SYS_Kickoff_DMABasedLowLatency_OutputTasks` | `_MI_SYS_IMPL_DMABasedLowLatencyBind_ProcFunc` | not FRAME_BASE and not REALTIME (HW_RING, HW_AUTOSYNC, SW_LOW_LATENCY) | dispatch; infer type 2 / 3 → run the consumer now |
| output task finished (`MI_SYS_IMPL_FinishAndReleaseOutputTask` → `MI_SYS_InferGraph_Finish_Output_Task`) | graph iterator | `_MI_SYS_IMPL_SWDMAPinonBind_ProcFunc` | FRAME_BASE (1) | `_MI_SYS_IMPL_OutputTaskDispatch_Func` → `RcAcceptBufDispatch` → `EnqueueBufToInputPort` (§4.5); infer type 2 → run the consumer now. For REALTIME / HW_RING binds the early references still queued at the consumer are dropped here; dispatch FRC is stepped |

So FRAME_BASE consumers see a buffer only when the producer's task has finished; the user FIFO is filled in the same finish
step, after the consumers.

**Low-latency early delivery** (output port low-latency mode +0x20e, set by ioctl 0x27): when `CheckOutputTaskStatus` (slot
0x24) returns **4** ("partly done"), `DequeueInputOutputTasks` immediately puts the buffers of the ports in low-latency mode
whose task bit is still set into the user FIFO and clears the bit; the final completion then skips them.

### 4.7 Depth settings
| call | effect |
|---|---|
| ioctl 0x15 / `mi_sys_SetChnOutputPortDepth(&ChnPort, user, queue)` | user > queue → `0xA0092003`; +0x0c = user, +0x14 = quota = queue; notifies the pass graph (`MI_SYS_InferGraph_OnUsrFifoDepthChanged`) |
| ioctl 0x16 GetChnOutputPortDepth | reads both back |
| `mi_sys_SetOutputPortUserFIFOBlockMode(h, chn, port, 0/1)` | +0x18 (other values BUG); last pass only |
| `mi_sys_SetOutputPortBufForceAlloc(h, chn, port, u8)` | +0x20d (time-mode FRC) |
| `mi_sys_EnsureOutportBkRefFifoDepth(h, chn, pass, port, n)` | n > quota − 1 → `0xA0092003`; else +0x10 = n, trims the back-ref queue |
| `/proc/mi_modules/mi_sys/mi_sys0` `set_ouputport_depth <mod> <dev> <chn> <pass> <port> <user> <queue>` | debug: writes +0x0c / +0x14 directly, no check |

Default for every output port: user depth 0, quota 4 (procfs shows the quota as `BufCntQuota`). With user depth 0 the user
FIFO never holds anything and ioctl 0x13 returns NOBUF.

## 5. Sideband message [R]

A u64 message a consumer leaves on its input port and a producer finds in the buffers it allocates for that consumer, and
reads before allocating. mi_sys stores it as two u32, low word first. 

### 5.1 Format
| bits | meaning | evidence |
|---|---|---|
| 63..61 | type (`MI_SYS_GET_SIDEBAND_MSG_TYPE` = hi >> 29) | assert string in mi_scl naming `MI_SYS_SIDEBAND_MSG_TYPE_PREFER_ALIGN` for 3 |
| 60 | ACKED — set by the producer that honoured the request (`MI_SYS_SIDEBAND_MSG_ACKED`, hi bit 28) | mi_sys assert in `_MI_SYS_IMPL_GetOutputPortBuf` (0x29de0) |
| 59..0 | payload of the type; 0 where the type has none | senders below |

| type | sender → where | payload | reader / effect |
|---|---|---|---|
| 0 | anyone (clear) | — | treated as "no message" by the merge |
| 1 crop window [I name] | mi_isp producer, written straight into each output `MI_SYS_BufInfo_t` +0x08 in `_MI_ISP_IMPL_OnPrepareOutputTaskMetaInfo` (three port sites) when the port-zoom table is loaded and running (+0x6b8 ≠ 0, +0x6a8 ≠ 0, +0x6bc = 1), ACKED set | x bits 0–12, y 13–24, w 25–37, h 38–49 (hi 6–17), pixel format 50–57 (hi 18–25); window = current zoom entry (+0x6d0 x, +0x6d2 y, +0x6d4 w, +0x6d6 h), format = BufInfo +0x24 | mi_scl `OnPreProcessInputTask` on its input buffer: for convertible input formats the window becomes the channel's input crop (+0xa8..+0xb6, +0xc90..+0xc96, flag +0xb8) and the format replaces the input format (+0xbc); then the word is cleared (hi = 0, lo = old ACKED bit) |
| 2 tile request ("deliver tiled frames") | mi_disp `MI_DISP_IMPL_SetVideoLayerRotateMode`, modes 1/3: `mi_sys_SetInputPortSidebandMsg(dev, chn, 0, 0, mode & 0x3ff, 0x40000000)` on its input port; mode 0 sends 0 | rotate mode (read by no producer) | mi_scl and mi_vdec as producers (§5.3); mi_disp checks the ACK (§5.4) |
| 3 PREFER_ALIGN | mi_venc `_MI_VENC_OnBindInputPort`, when bound HW_RING (0x10) or REALTIME (4) from SCL (module 0x22): `(…, align, 0x60000000)`, align also kept at VENC channel +0xf0 | lo bytes: [31:24] 422 width align, [23:16] 422 height align, [15:8] 420 width align, [7:0] 420 height align (0 → 1) — from a per-codec VENC table (+0x1e8 / +0x1ec / +0x200 / +0x204 for HW_RING, +0x1f4 / +0x1f8 / +0x20c / +0x210 for REALTIME, base + codec·100); the table's values are not resolved | mi_scl (§5.3) |
| 4 enqueue message [I name] | mi_isp and mi_scl on their input port: `{1, 0x80000000}` when the first real-time task is enqueued (`MI_ISP_PASS_SetEnqueueMsg`, `_MI_SCL_IMPL_SetEnqueueMsg`), `{0, 0x80000000}` at Start / StopChannel | bit 0 | mi_vif `EnqueueOutputTaskDev` / `CheckOutputTaskStatusDev` wait for type 4 with bit 0 on their output port; no reader of SCL's type 4 to ISP was found [R: `>> 0x1d` grep over all module decompiles — the only check made; I: none exists] |

### 5.2 mi_sys path
- `MI_SYS_IMPL_SetInputPortSidebandMsg(dev, chn, pass, port, lo, hi)`: pass < 0 → 0; stores the u64 at **input port +0x10 /
  +0x14**. No other processing.
- **Merge into a buffer** (`_MI_SYS_IMPL_GetOutputPortBuf`, §4.3 step 5): only consumers whose `RcAcceptBufAlloc` accepted
  this frame take part; zero messages are skipped; equal messages combine; a differing one sets the result to 0 and it stays 0
  for the rest of the list. A result with ACKED set is a BUG; it is stored to the buffer's `MI_SYS_BufInfo_t` **+0x08**
  (bufref +0x20 / +0x24). Consumers that rejected the frame do not contribute.
- `MI_SYS_IMPL_GetOutputPortSidebandMsg(dev, chn, pass, port, &u64)` (pass < 0 → last pass): the same combination over
  **all** bound inputs (no rate filter) under the output port's semaphore (+0x168), with three logs on conflict (lines 0x244b,
  0x2450, 0x2454). This is what a producer reads before allocating (mi_scl, mi_vif).
- After allocation the producer's `OnPrepareOutputTaskMetaInfo` may overwrite BufInfo +0x08 (mi_scl type-2 ACK, mi_isp
  type 1).

Users of the exports: `mi_sys_SetInputPortSidebandMsg` — disp, isp, scl, venc; `mi_sys_GetOutputPortSidebandMsg` — scl, vif.

### 5.3 Producer side of types 2 and 3
- **mi_scl** — `_MI_SCL_IMPL_OnPreProcessInputTask`, every task, for each of the 6 output ports that is enabled (port +0x120 =
  1): `mi_sys_GetOutputPortSidebandMsg(dev, chn, 0, port)`; debug flag bit 5 of channel +0xc44 prints "Dev %d PassPortId %d
  sidebandmsg %llx, type %llx". Then:
  - type 3: 420 or 422 alignment pair chosen by the port's output format (+0x148: 0xb NV12 → 420 pair, 0 YUYV → 422 pair,
    other → 1×1); if the port's output size (+0x140 w, +0x142 h) is not a multiple, it is **rounded up** to it; then
    `mi_sys_SetOutputPortBufExtConf(…, {wAlign, hAlign, 1, 0})`. The ACK is set in a local copy only and never stored.
  - type 2, bound peer module (+0x1c0, stored by `OnBindOutputPort`) = 0xf DISP, output format NV12: if the port's HW scaler
    bit (+0x12c, set by `CreateChannel` = 1 << scaler index, index at +0x130) is **0x20 (HWSCL5)** and channel +0x18 ≠ 4 →
    `SetOutputPortBufExtConf(…, {0x80, 0x40, 0x40, 0})` (128-byte stride, 64-line luma and chroma alignment) and port byte
    **+0x1bc = 1**; else ERR "chn%d port %d, scl %d, source %d not support tile mode, cannot use disp rot" (debug level ≠ 0
    or a filter match) and +0x1bc = 0. Channel +0x18 is printed as "source" [I: the SCL instance type, 4 = rotation].
  - type 2 with another peer or format, any other type → +0x1bc = 0, nothing else.
  - `_MI_SCL_IMPL_OnPrepareOutputTaskMetaInfo`: for every output buffer of the task whose port has +0x1bc set, BufInfo +0x08 =
    0, +0x0c = `0x40000000 | 0x10000000` (hi = 0x50000000: type 2 + ACKED; the mode in the low word is dropped).
- **mi_vdec** — `_MI_VDEC_EnqueueAsyncOutputTask`, per output buffer, by the channel's applied layout mode (+0x2f4, from the
  device's `SetOutputPortLayoutMode` value by `apply_layout_mode` (0x10b9c): 1 → linear, 2 → tiled, 0 → follow the consumer): mode 2 → ACK
  every buffer; mode 0 and the buffer's merged sideband type is 2 → channel +0x2f0 (tiled output) := 1 and ACK; the first such
  frame after linear logs "Linear to tile, realloc frame buffer" and leaves through the common exit; mode 0 without type 2 →
  +0x2f0 := 0 (linear again); mode 1 → nothing. Tiled layout = ext-conf `{0x80, 0x40, 0x40}`.

The producer only switches to the tiled layout and stamps the ACK; it does **not** rotate. The display rotates (MOP rotate from
SRAM, `mi_disp-mi_hdmi.md` §4.11.1).

### 5.4 Consumer side (mi_disp)
`_MI_DISP_EnqueueInputTask` (line 0x69a): when the layer's rotate mode (`_stModRes` + 0x838 + 0x10d0·layer + 0x24) is non-zero
and the input port has a bound source (port +0x54), a frame is rejected (return 3, ERR "[dev:%d][chn:%d] sideband msg is not
correct") unless BufInfo +0x0c (the high half of the sideband) has **bit 28 (ACKED) set and bits 31..29 = 2**. The test is on
any non-zero rotate mode, so it also applies to mode 2 (180°), which sends no request — a 180° layer would reject bound frames
unless the producer still stamps type 2 + ACK [I: consequence of the condition as read].

### 5.5 On this camera [I]
SCL dev 0 is created with mask 7, so its ports map to HWSCL0..2 (+0x12c ∈ {1, 2, 4}); a 90° / 270° layer fed by it would get
the ERR above, no ACK, and every bound frame rejected. Only an SCL port on HWSCL5 (dev 3, created with mask 0x20) or mi_vdec can
feed a rotating display layer. The app uses no rotation.

## 6. Worker threads and the MAIN loop [R unless marked]



`mi_sys_internal_main_worker_thread` is at 0x2e1f0 (13 KB); `MI_SYS_IMPL_EnqueueBufToInputPort` calls
`MI_SYS_IMPL_ApplyNextToDoPulseToCurWorkingQueue` (0x152ec, no symbol) [R].
### 6.1 Threads
Per device pass:
- **MAIN** `"<prefix><dev>_P<pass>_MAIN"` (`mi_sys_internal_main_worker_thread`) when the pass has input ports or an
  `OnPollingAsyncOutputTaskConfig` callback (slot 0x2c);
- **ASYNC** `"…_ASYNC"` (`mi_sys_internal_async_output_worker_thread`) for passes with slot 0x2c, i.e. pure sources like VIF
  (§6.5).

Priority = registration field +0x1c (default 50); `MI_SYS_IMPL_SetModuleDevWorkThreadPriority` changes it later. Stack 0x800
(VIF), 0x1000 (VENC), 0x1400 (ISP, SCL), 0xc00 (module ids 1, 4). Each worker has a 10-s CamOs timer
(`mi_sys_worker_thread_timeout_monitor`), re-armed every loop, that only logs if the loop stalls. Other kernel threads of mi_sys
exist only for user pictures and log-to-disk (`mi_sys-2-interfaces.md` §8, §9).

Work queues: `mi_wq` (`g_miSysWorkQueue`; the pass-graph root search, §8.1) and `mi_cmdq_wq` (CMDQ reset notification, §7.1).
IRQ: only `sys-hwtimer-handler`, requested on the first `mi_sys_RegisterHwTimer` (mi_vdec, mi_ai; `mi_sys-2-interfaces.md` §8).

### 6.2 MAIN loop
Each loop: take the graph read lock; if the pass's **role** (device pass +0x234, set by the pass graph, §8; non-zero =
"SelfDrive", a root) is 0 — the pass is driven from a producer's thread — release its CMDQ, then sleep on
`input_thread_waitqueue` in 10-jiffy slices until a role is assigned. A pass in role 0 whose input has waited more than 2 s is
logged as "NON-SelfDrive module … input buffer has not been processed for more than 2 seconds". Otherwise:
1. CMDQ reset pending (flag set by the reset ISR) → `MI_SYS_IMPL_CMDQ_Reset`, counters, fence update (§7.1).
2. Apply the pulse queues (`ApplyNextToDoPulseToCurWorkingQueue`, §6.3); if nothing is queued or in flight, sleep on the input
   wait queue (10-jiffy slices; the empty-queue wait is 100 ms).
3. Get or keep a CMDQ (`MI_SYS_IMPL_GetCMDQ`, switching CMDQs only after draining, §4.1 step 1); `DequeueInputOutputTasks`
   (§7.2); finish completed output tasks.
4. One round of kick-offs (§6.4), ending with `MI_SYS_IMPL_KickOffCMDQ`.

The thread is woken by new input buffers (§4.5), by `mi_sys_NotifyWakeupISREvent(h, pass)` — called from the modules'
interrupt paths (disp, isp, jpd, scl, venc, vif; `…ByChn` from vdec), it wakes the pass's MAIN and output waits — and by
`mi_sys_DoDirectBufTask(task, timeoutMs)` (scl, for `MI_SCL_StretchBuf`).

### 6.3 Input queues and the target time
- The target time of a queued input set is the **monotonic time (ns) at which the buffer was queued to the input port**,
  stored in the queued buffer reference at **bufref +0x140** (u64) by `MI_SYS_IMPL_EnqueueBufToInputPort` — both for a buffer
  from a bound producer (release hook `_MI_SYS_IMPL_InputPortOnPipeInBufRefRelFunc`, counted at input +0xa8) and for a
  user-injected one (`…OnUserInjectBufRefRelFunc`, counted at +0xa4). `MI_SYS_IMPL_DoDirectBufTask` stamps its task the same
  way. It is not derived from the frame's PTS. The same call stores bufref +0x14c := the producer's sequence word (`RTInfo`
  +0x20) when the consumer pass is driven (role 0), −1 when it is a root or the buffer is injected.
- Path of the reference: appended to the pass's `new_pulse_fifo_inputqueue` (+0x1e4, semaphore +0x1c4, pending flag +0x280 :=
  1); if the consumer pass is a **root** (+0x234 ≠ 0) or the bind is FRAME_BASE, the queues are applied at once
  (`apply_input_queues(pass, 0)` (0x18ce8), then `ApplyNextToDoPulseToCurWorkingQueue`); then the pass's input wait queue (+0xcc) is woken.
  `apply_input_queues` (new pulse → next-todo step) was not read line by line.
- The apply step moves every entry of `next_todo_pulse_inputqueue` (+0x20c, semaphore +0x1ec) into the **current working
  queue** (+0x1b4, semaphore +0x194), kept **sorted by bufref +0x140, oldest first**, and into the input port's own list
  (+0x130) in the same order (a channel-id mismatch there is a BUG), and finally into the input's bind-in or inject queue
  (`_MI_SYS_IMPL_DupBufrefIntoBindInQueue` / `…IntoInjectQueue`). It also keeps apply-latency statistics (pass +0x2b4…
  words, averaged per second).
- A rewound reference (`_MI_SYS_IMPL_InputPortRewindBuf(ref, lazy)`) goes back, by the same time, into the working queue
  (lazy 0) or the **lazy-rewind queue** (+0x1bc, lazy 1), and into the input port's list +0x130 / +0x138 respectively; a
  reference of a disabled channel or port is released instead.

### 6.4 One MAIN round (between `DequeueInputOutputTasks` and the CMDQ kick)
Read from `mi_sys_internal_main_worker_thread`; constants read from the literal pool.
1. In-flight input tasks (list +0x134) are counted against the pass's concurrent-task limit (+0x98, slot 0x8c). Limit reached
   → the round only sets the wait to **100 ms** (0x5f5e100 ns).
2. Head entry = first of the lazy-rewind queue, else first of the working queue; wait = head time − now:
   - wait ≤ 0 → kick-off phase (below);
   - wait > ≈2 s (1 999 999 999 ns) → the head's time is reset to now (with a log) and the round sleeps;
   - otherwise the round sleeps for *wait*.
   Since the stamp is the enqueue time, a queued head is always due [I]; the sleep is on the input wait queue (+0xcc) for
   ⌈wait ms⌉ jiffies, ended early by a new pulse (+0x280) or a change of the counter at +0x27c. A stall of 5 s (0x12a05f1ff
   ns) since the last processed task is logged.
3. Kick-off phase. `BarrierByPass` (slot 0x38) is asked first (below). Entries are scanned in order; a 64-bit per-round
   channel mask skips channels that already got a task this round; an entry whose channel has an input of infer type 3
   (HW_RING) must be bound HW_RING (else BUG) — the second scan pass made when the first entry's channel has an infer-type-3
   input (two flags of the main worker) was not read line by line. For the chosen entry: the fence = CMDQ software
   fence + 1 (−1 without CMDQ) goes to the task (+0x34, +0x8c) and to pass +0x2a0, the task is put on the round's
   **later-barrier list** (below), `BarrierByPass` is asked again, and `_MI_SYS_KickOff_InputTask` runs (§4.1). Its result:
   0 queued → channel bit set, task onto the in-flight list +0x134; 1 done / 3 drop → the pass's later-barrier entry for this
   task is removed, the input buffers are finished (`_MI_SYS_IMPL_InputPortFinishBuf`), the task freed; 2 retry → the same
   removal, channel bit set, buffers rewound.
4. End of round: input sets on the lazy list of the round are rewound (`_MI_SYS_IMPL_InputPortRewindBuf(ref, task +0xa4
   byte)`) and freed; DMA-low-latency consumers (`MI_SYS_Kickoff_DMABasedLowLatency_OutputTasks`) and the delay node
   (`MI_IMPL_KickOff_SyncInferenced_DevPassInputBufs(rt, node, −1)`, §8.4) are kicked; then
   **`_MI_SYS_ProcessDelayTasks_n_KickOff`**: for every pass on the later-barrier list its slot 0x0c `BarrierInputTask(CMDQ
   object +0xc or 0, &task +0x40, 0, usrData)` is called on the last registered task (timed; INFO above 2 ms, WRN at 200 ms or
   more, "BarrierInputTask"), the action is recorded, and the task's fence is written (`MI_SYS_IMPL_UpdateFence`, pass
   +0x2a0); finally `MI_SYS_IMPL_KickOffCMDQ` and the graph read unlock.

Input sets that cannot run are rewound and kept on the lazy-rewind list.

**Later-barrier list** (`_MI_SYS_AddDevPassToLaterBarrierList`): one entry per device pass (node +0x218, channel pass +0x260,
runtime +0x264, task words +0x268 / +0x270 / +0x274 / +0x278, task info +0x2a4, 0 at +0x2a8). If the pass is already on the
list, its pending barrier is issued first (same call, same timing logs, line 0x29f4) and the entry is replaced by the new
task — so a pass's `BarrierInputTask` for task *n* is written into the command queue just before task *n*+1 of the same pass,
or at the end of the round.

**The branch chosen by `BarrierByPass`** (slot 0x38, device pass +0x44). Called `fn(word, passId, usrData)` at the start of
the kick-off phase (word = CMDQ object +0x10) and again before every kick-off (word = CMDQ object +0xc; 0 without a CMDQ or
when CMDQ object +0x08 is 0). Ghidra shows the round body twice, once per answer:

| | slot absent or returns 0 | returns ≠ 0 |
|---|---|---|
| before a kick-off | `MI_SYS_ProcessChnPassInqueueTasks(rt, chnPass)`: completed tasks of the pass are dequeued, recursively for every consumer whose input has infer type ≥ 2 and whose pass has +0x235 = 0, output tasks are finished and the descendants' output tasks kicked | not done |
| tasks per round | **one**: after a queued (0) result the round ends | **several**: the scan continues after a queued result (a counter starts at −1 and ends the round at 0, i.e. no limit besides the channel mask, the in-flight limit and the queue) |
| barrier | every kicked task is registered on the later-barrier list | only while the latest answer was 0: the first task of the round is registered (the flag starts true), a non-zero answer before a kick-off clears it, so later tasks get **no** `BarrierInputTask`; a 1/2/3 result of the round's first task sets the flag again |
| CMDQ | one kick at round end | one kick at round end for the whole batch |

Only mi_disp sets the slot, and `_MI_DISP_BarrierByPass` **always returns 1** [R `mi_disp.ko` 0x1c04: `movs r0, #1`] — the
display pass therefore always batches all due layers / ports into one round and issues its `BarrierInputTask` only for the
first task of a round [I: DISP synchronises on vsync itself].

### 6.5 ASYNC thread
`mi_sys_internal_async_output_worker_thread` polls the module for output configuration (slot 0x2c
`OnPollingAsyncOutputTaskConfig`) and runs `_MI_SYS_IMPL_TryProcessAsyncOutputTask`: `PrepareOutputTaskBuf` for the polled
configuration, `EnqueueOutputTask` (slot 0x14) / `BarrierOutputTask` (slot 0x18), dispatch as in §4.6. It waits on
`output_thread_waitqueue`.

## 7. CMDQ objects and task completion [R]

The mhal CMDQ interface itself (service table, engine ids) is in
`mhal-scl-jpe-cmdq.md` §2.2, §2.3; mi_sys imports only `MHAL_CMDQ_{InitCmdqMmapInfo, DeinitCmdqMmapInfo, GetCmdqHWNumber, GetFreeId,
GetSpecificId, ReleaseId, GetSysCmdqService, ReleaseSysCmdqService}` (`mi_sys-2-interfaces.md` §11).

### 7.1 CMDQ objects (`MI_SYS_IMPL_GetCMDQ` / `DupCMDQ` / `PutCMDQ`)
- **Object** (0x158 bytes, magic `'QDMC'` 0x434D4451): +0x04 CMDQ id, +0x08 the mhal CMDQ service, +0x0c / +0x10 words handed
  to `BarrierByPass` and `BarrierInputTask` (§6.4), +0x14 u16 software fence counter, +0x16 u8 kicked once, +0x18 u16 reset
  generation, +0x1a u8 reset pending, +0x1c atomic refcount (> 100 is a BUG), +0x20 kick-off count, +0x24 busy count, +0x28
  and +0xc0 two copies of the service's 0x98-byte function table.
- **`GetCMDQ(devPass)`**: slot 0x34 `RejectCmdq(devPass, passId, &cfg, usrData)` must return 0 (else the pass runs without
  CMDQ); only module ids 2 (VENC), 0xf (DISP), 0x17 (LDC), 0x21 (ISP), 0x22 (SCL), 0x23 may own one (others → BUG). Id choice
  (`MI_SYS_IMPL_GetFreeCMDQ`): with ≥ 5 hardware CMDQs, VENC / DISP / 0x23 take a specific id in 4–7 and ISP / SCL / LDC in
  0–3; with fewer, 0–3; else any free id (`MHAL_CMDQ_GetFreeId`). Then `MHAL_CMDQ_GetSysCmdqService(id, &cfg, 0)`, service
  +0x10(0) (fence 0), +0x40 (kick), +0x90(`MI_SYS_IMPL_CMDQ_ResetIsrNotify`). **Put** at refcount 0: service +0x94, release
  service and id.
- **Fences**: with `bSelfFence` (default 1) each task writes its fence (`MI_SYS_IMPL_UpdateFence` → service +0x10) and
  `KickOffCMDQ` only kicks (+0x40); without it the kick-off increments the fence and writes it first.
  `CMDQ_IsSWFenceDone(cmdq, fence)` compares the hardware fence (service +0x14) with 16-bit wrap, consulting the busy state
  (service +0x48).
- **Reset** (also after a CMDQ timeout): the CMDQ ISR calls `ResetIsrNotify(id)` → work item on `mi_cmdq_wq` → every device pass (≤ 4 per device) using
  that id gets its reset flag set and both thread wait queues woken; the MAIN thread then calls `CMDQ_Reset` (service +0x38)
  and, through `DequeueInputOutputTasks`, the pass's slot 0x94 `ResetDevPass` and `_MI_SYS_IMPL_ForceDropAllBufInQueue` for
  tasks of the old generation (`bEnableCmdqReset` gates the reset).

A pass that takes a CMDQ and is driven from a producer's thread (role 0, §8) has its tasks programmed into the producer's
command queue (§8.4).

### 7.2 Task completion (`MI_SYS_IMPL_DequeueInputOutputTasks(rt, devPass)`, 8.7 KB, read for its control flow and calls)
- **Output tasks** (+0x184 list) are taken in fence order (16-bit wrap check) once their CMDQ fence passed (or without CMDQ):
  `CheckOutputTaskStatus(cmdq, &task.info, msSinceEnqueue, usrData)` (slot 0x24): 0 = still running (logged as timeout after
  the pass's u16 +0x9e ms — slot 0x92 — or 5 s), 4 = partly done (low-latency early delivery, §4.6), others →
  `DequeueOutputTask` (slot 0x28) and finish (§4.5); results counted per channel (+0x1b60 + 4·(r mod 5)). Out-of-order
  completion is allowed only if slot 0x7c `OnSupportOutputTaskReorder` says so.
- **Input tasks** (+0x134 list) the same with `CheckInputTaskStatus` / `DequeueInputTask` (slots 0x1c / 0x20): finished input
  buffers are released (`_MI_SYS_IMPL_InputPortFinishBuf`), others rewound for retry.

## 8. The pass graph [R unless marked]

The pass graph ("inference graph", `MI_SYS_InferGraph_*`) decides, for every device pass, whether it is a **root** that runs
its own MAIN loop or is driven synchronously from its producer's thread, and for every bound input port an **infer type**.
The root worker is at 0x467ac–0x48294; 0x45b3c (no symbol) is `_MI_SYS_Deliate_Infer_OneBind_Descendants` with
`_MI_SYS_IMPL_CapableOfBeRootDev` and `_MI_SYS_IMPL_AppendToPendingRootDevPassList` inlined [R]. Names in `code font` are the
source names printed by the asserts; offsets are from the code.

### 8.1 When the worker runs
`MI_SYS_InferGraph_OnBind` (bind count +1, input +0x16c := 0, both channel passes' +0x40 and +0x150 := 0),
`MI_SYS_InferGraph_OnUnBind` (count −1), `MI_SYS_InferGraph_OnUsrFifoDepthChanged` and the first user GetBuf on an unbound port
set the dirty byte (`.bss` 0x82d78) and queue `_MI_SYS_Determine_InferenceRootPassWorker` on `g_miSysWorkQueue` (`mi_wq`). The
worker does nothing unless the byte is set, and clears it under the graph write lock and the device-list semaphore; several
queued runs therefore collapse into one [I]. The graph write lock is a counter with −1 = writer; a writer waits in 1-jiffy
slices, with a stack dump after 1000 tries.

### 8.2 Device-pass inference fields (`stInferDAGInfo`)
| off | field | meaning |
|---|---|---|
| +0x220 | — | jiffies of the first bind (`OnBind`, bind count 0 → 1) |
| +0x234 | `bFinalRenderRootFlag` | **role**: 1 = root ("SelfDrive", runs its own MAIN loop), 0 = driven by a producer's thread |
| +0x235 | — | result of pass slot 0x30 `NeedIndividualWorkThread(modDev, devPass + 0x238, usrData)`, refreshed every run; "notify instead of run": `MI_IMPL_Notify_PulseBufReady` wakes the consumer's thread instead of running it |
| +0x23c | `bDelayInferenceNode` (final) | driven by a root but kicked after the producer's own kick-off |
| +0x23d | `bHasHWRingInInput` (final) | some input of the pass is bound HW_RING |
| +0x240 | `bindCnt` | atomic count of bound inputs over all channels (`OnBind` +1, `OnUnBind` −1, < 0 is a BUG) |
| +0x244 | `track_listhead` | list node for the worker's root lists |
| +0x24c | `bTmpDelayInferenceNode` | working copy of +0x23c |
| +0x24d | `bTmpHasHWRingInInput` | working copy of +0x23d |
| +0x24e | `bTmpRenderRootFlag` | working copy of +0x234 |
| +0x24f | `bTmpDevPassRoleDetermined` | set once per run when the pass's role is final |
| +0x250 | — | the pass is the producer of some HW_RING bind |
| +0x254 | `s32InferencedCnt` | number of this pass's channel passes reached from the current root |
| +0x258 | `s32InferenceScore` | self-barrier score |
| +0x25c | `u32InferencePhase` | 0 / 1 / 2 visit marks of the selection steps |
| +0x285 | — | set to 1 by `MI_SYS_IMPL_RegisterDev` after the pass's threads are created; passes with 0 are skipped by the walks |

The channel pass's `track_listhead` (+0x24) and `u32RootChnIdx` (+0x30, index of the root channel the channel pass descends
from) and the input port's HW_RING allocator, bind parameter and infer type (+0x148 / +0x14c / +0x16c) are in §1.2.

### 8.3 The algorithm of `_MI_SYS_Determine_InferenceRootPassWorker`, in order
"Takes a CMDQ" = the pass has slot 0x34 and `RejectCmdq(...)` returns 0. The worker reaches `RejectCmdq` through the
device-list node: its two indirect calls (`.text` 0x36a80, 0x36bf0) load `[node + 0x94]`, where *node* is the module's entry in
`mi_sys_global_dev_list` (device +0x24) and the device pass sits at node + 0x54, i.e. device pass +0x40 = slot 0x34. A scan of
every `ldr rX, [rY, #0x94]` followed by `blx rX` in the disassembly finds only these two and one in `MI_SYS_IMPL_PutCMDQ` (the CMDQ
service table). A channel pass is *used* when its user count (+0x10) > 0, or an input is bound, or an output port has a bound
input or a user depth ≠ 0 (`_MI_SYS_IMPL_IsChnPassUsed`).
1. **Reset.** Every device pass: +0x250 := 0; +0x24c..+0x24f := 0; +0x235 := slot 0x30 `NeedIndividualWorkThread` result (0
   if absent). Every input bound HW_RING (0x10): its pass +0x24d := 1, the producer pass +0x250 := 1.
2. **Seed roots.** For every device pass D not yet a root, with *cond* = (bind count > 0 and exactly 1 input port and D takes
   a CMDQ):
   - not *cond* → D is a root (unbound passes, sources without inputs, passes without `RejectCmdq` or refusing a CMDQ);
   - *cond*, no HW_RING input (+0x24d = 0) and (+0x235 ≠ 0, or some channel's input 0 is bound to a producer P that does not
     take a CMDQ, or P has input ports and a slot-0x2c async poller) → root;
   - otherwise (*cond* and a HW_RING input, or *cond* with a producer that can share its CMDQ and no individual thread) → not
     a root now; it can only be reached from a producer.
   A new root goes on the pending-root list, +0x24e := 1, and its +0x235 is cleared unless it refuses a CMDQ and has a HW_RING
   input. `_MI_SYS_IMPL_CapableOfBeRootDev` only logs HW_RING inputs (debug level > 4) and never refuses [R disassembly
   0x36f52–0x37012].
3. **Grow each root's tree** (roots are taken from the pending list until it is empty; a root goes to the processed list after
   step 7). `_MI_SYS_Deliate_Infer_OneBind_Descendants(R)`:
   - every used channel pass *k* of R gets `u32RootChnIdx` := *k* and starts the set;
   - breadth-first over bound outputs (`_MI_SYS_IMPL_OutputGraphIterator` with `_MI_SYS_IMPL_InferOneBindDevPass_ProcFunc`): a consumer
     pass D is skipped if it is not registered (+0x285 = 0) or is a root; it **joins** when it has a HW_RING input, or when the
     producer takes a CMDQ and (has no input ports or no slot-0x2c poller) and D has fewer than 2 input ports; any other bind
     here is a BUG (ERR "Pre output port(mod, dev, pass, chn, outputport, reject_cmdq)" / "Next input port(…)", line 0x536).
     A joining D with slot 0x88 present is **not** added (the walk stops there); else its channel pass inherits the producer
     channel's `u32RootChnIdx`, a cycle (`_MI_SYS_IMPL_IsDescendantOf_NoBindLock(in, D, TRUE)`) is a BUG, and it is appended;
   - counting: `s32InferencedCnt` of each reached pass = its channel passes in the set. A pass reached through only part of
     its binds (count ≠ `bindCnt`) whose producer is R itself or is fully counted is cut out with its subtree
     (`_MI_SYS_IMPL_RemoveAllSubTreeOf_NoBindLock`, `_MI_SYS_IMPL_RemoveAllSubTreeOf_NoBindLock`) and becomes a **root** (appended, +0x24e := 1); the count
     is redone. A pass still partial after that is a BUG (log "(mod, dev, pass, reject_cmdq, chn) bindcnt, infercnt", line
     0x5ee);
   - a reached pass with one input port and two channels whose `u32RootChnIdx` are equal (two channels fed from the same root
     channel, "MutiRender") is cut out with its subtree but not made a root.
4. **Self-barrier selection** (`_MI_SYS_Deliate_InferenceTree_Select_One_SelfBarrierPass`, inlined): consumers just outside the
   set that are bound from the set (`_MI_SYS_IMPL_InferSelfBarrier_ProcFunc`, same join conditions as step 3, same BUG otherwise) are
   candidates. Per candidate pass: count (+0x254) and score (+0x258) over its candidate channel passes by the bind type of
   input 0 [R `tbh` table at 0x37548]: FRAME_BASE +1, REALTIME +1000 (with INFO "Is fatal error to find
   E_MI_SYS_BIND_TYPE_REALTIME bind here…", no BUG), HW_RING +10000 (`HW_RING_SCORE_BASE`; the pass must have a HW_RING input,
   else BUG), **HW_AUTOSYNC +2 and then the HW_RING arm as well** (no `break`: +10002, same BUG check) [I: a missing `break`
   in the source], SW_LOW_LATENCY or any other type → BUG. A candidate with no "MutiRender" pair and no slot 0x88 is a BUG
   ("MutiRender not found for non-self_barrier Node…", line 0x730) — so **slot 0x88's presence marks a pass as a
   self-barrier node** [I name]. The highest score wins (strictly greater replaces, first wins ties) among candidates that
   have no HW_RING input or score > 9999, and that produce no HW_RING bind (+0x250 = 0). The winner's used channel passes are
   all added if it has a HW_RING input (score < 10000 is then a BUG); the winner is moved **into R's set as a delay node**
   (+0x24c := 1); every other candidate pass without a HW_RING input becomes a **root**.
5. **Real-time descendants** (`_MI_SYS_IMPL_ScanFor_RealTime_DevPass_ProcFunc`, repeated until nothing is added): a
   REALTIME-bound consumer outside the set (registered, not a root, exactly one input) joins the set with the inherited
   `u32RootChnIdx`; it must descend, through REALTIME binds, from a delay node — reaching a root or the chain's end first is a
   BUG.
6. **External consumers** (`_MI_SYS_IMPL_Mark_ExternalNodeAsRoot_ProcFunc`): every other bound consumer outside the set that is
   registered, not a root and has no HW_RING input becomes a **root**; one that is already a root must be on the pending or
   processed list (else BUG, line 0x850); bind count < 1 is a BUG (line 0x82f).
7. Every pass of the set gets `bTmpDevPassRoleDetermined` (twice is a BUG); a root whose set is empty gets it directly.
8. **Commit** (every device pass; one without `bTmpDevPassRoleDetermined` is a BUG): +0x234 := +0x24e, +0x23c := +0x24c,
   +0x23d := +0x24d, temporaries cleared; every input port gets its infer type (§8.4).

The bodies of `_MI_SYS_IMPL_RemoveAllSubTreeOf_NoBindLock` (0x00042340) and of the output-graph iterator (`output_graph_iterate` (0x44a24))
are known from their assert names and call sites only.

### 8.4 Infer types and the synchronous kick-off
| case | infer type (+0x16c) |
|---|---|
| not bound | 0 |
| bound HW_RING (the pass must have `bHasHWRingInInput`) | **3**; a ring-heap allocator `"HWRING%04x_%d"` (global counter, bind parameter +0x14c) is created into +0x148 if none (`mi_sys_ringheap_allocator_create`) |
| other bind, consumer pass not a root and without HW_RING input | **2** — sync: the consumer is run in the producer's thread (§4.6) |
| other bind, consumer pass is a root or has a HW_RING input | **1** — the consumer's own thread takes the buffers (drain only) |

For types 1 / 2 a ring allocator left at +0x148 is released (allocator ops +4) and cleared.

`MI_IMPL_KickOff_SyncInferenced_DevPassInputBufs(rt, devPass, chn)` applies the consumer's pulse queues, builds its input task
and runs `_MI_SYS_KickOff_InputTask` for it **in the producer's thread with the producer's CMDQ** — so a chain like VIF→ISP→SCL
bound REALTIME / FRAME_BASE is programmed into one command queue; a delay node is only recorded and run later by the caller
(end of the MAIN round, §6.4).

Debug: `MI_SYS_InferGraph_RecAction` keeps up to 16 named actions per channel pass (11 action names, table at `.rodata`
`PTR__LC61_0005c11c`; the names and the 16-entry record layout are not listed) for `_MI_SYS_DEBUG_Dump_InferActions`. The graph
is shown by `/proc/mi_modules/mi_graph_contrl` (`mi_graph_contrl`: bind-dump switch) and `/proc/mi_modules/mi_infer_graph.py` (`mi_infer_graph.py` is a Python
script generated from the graph; `mi_sys-2-interfaces.md` §10.1).

### 8.5 Applied to this camera's chain [I: the rules above + the slot sets of §2.3]
VIF has no input port and no `RejectCmdq` → root. ISP: one input, takes a CMDQ, no slot 0x30, but its producer VIF has no
`RejectCmdq` → **root**. SCL dev 0: one input, takes a CMDQ, no slot 0x30 (only dev 1/3 register `NeedIndividualWorkThread`),
producer ISP takes a CMDQ and has no slot-0x2c poller → **not a root**: joins ISP's tree, its input from ISP gets type **2** (SCL
is programmed in ISP's thread with ISP's CMDQ). VENC and DISP register `NeedIndividualWorkThread` → roots when bound FRAME_BASE
(input type 1); a VENC bound HW_RING from SCL is not a root and its input is type 3. So SCL dev 0 runs inside ISP's thread, while
VIF, ISP, VENC (FRAME_BASE bind) and DISP run as roots. These roles have not been read from the running camera; the two procfs
views above show them.

## 9. SRAM (internal memory) — active only on platform "M6", i.e. this camera [R]

`mi_sys_GetPlatFormIdx` table: 0 I5, 1 I6, 2 I2M, 3 I6E, 4 I6b0, 5 M6, 6 P3. In
`_MI_SYS_KickOff_InputTask` the SRAM step runs only when the platform is M6 and the device has a `ConfigSramBuf` callback
(registration +0x2c, device +0x68): `ConfigSramBuf(dev, devId, &bNeed, &size, &bKeep)`; bNeed → `MI_SYS_AllowPreemptionSram`
(registration +0x30, device +0x6c) → `MI_SYS_PreAllocSramBuf(dev, size)`; not needed and still holding → `MI_SYS_FreeSramBuf`.
The SRAM address / size go into the input task (+0x90..+0xa0) and output task (+0xa4..+0xb0) for the module.
- **Device SRAM state** (device +0xd60..+0xdb8): +0xd60 lock, +0xd80 state (0 settled, 1 moved — waiting for the module to
  acknowledge, 2 resizing), +0xd84 u8, +0xd85 u8 owns a block, +0xd88 u32, +0xd90 u64 block address, +0xd98 size,
  +0xda0..+0xdb0 a second (shared) block, +0xdb8 partner device.
- **`PreAllocSramBuf`**: VENC (module 2, dev 0) and VDEC (module 8) can **share** one block (`_MI_SYS_SyncShareSram`: the
  partner's block is reused if large enough; else `0xA009201F`). A size change frees the old block only when no task of either
  device is in flight. Allocation from `imi_mma_heap` (`_MI_SYS_IMPL_MmaAlloc_ByDev`); when it is full, mi_sys **evicts** another
  device's movable block: allocates DRAM of the same size (`MI_SYS_IMPL_MmaAlloc_ByChn`), records the new address in that
  device (state 1, `g_bSramChange` = 1) and waits up to 1000 jiffies (uninterruptible) for that device to acknowledge
  (`MI_SYS_CheckSramFlagDone`); 13 failed rounds → BUG. The IMI allocator itself compacts with `mi_sys_merge_sram_chunk` and the
  same acknowledge protocol.
- While `g_bSramChange` is set, a FRAME_BASE input with ≥ (producer's alive buffers − 1) queued refuses new buffers (§4.5).

Which modules ask for SRAM is decided by their `ConfigSramBuf` callbacks: mi_disp (`mi_disp-mi_hdmi.md` §4.11.3), mi_venc (device
+0x5f8 / +0x5fc, `mi_venc.md` §2.1, §3.3) and mi_jpd (REALTIME binds only, `mi_jpd-mi_vdec.md` §1.8) [R]. The size actually requested on the
running pipeline is not known; it is a camera reading.

## 10. Ring pools (`mi_sys_RingPool_*`, source file `impl/sys/mi_sys_internal_ring_pool_utility.c`) [R]
<!-- ev:mi_sys-1-engine.10 -->

A ring pool is one MMA block split into 1–4 equal rings; writers reserve free space at a ring's tail, fill it (usually by
hardware), then commit it as a *record*; readers take records from the head in commit order and release them. Used by mi_venc
(bitstream), mi_jpd (JPEG input) and mi_vdec (ES input); in the full SDK also by ldc / vdisp [I]. Field names
in the tables come from the assert strings and the dump printk of the binary.

### 10.1 Objects
**Pool** (`pstRingPoolRes`, `CamOsMemAlloc(0x168)`; the handle given to the modules is this kernel pointer):

| off | field | notes |
|---|---|---|
| 0x00 | `u32MagicNumber` = 0x524E4750 (`MI_SYS_RINGPOOL_MAGIC`) | every entry point BUG_ONs on NULL or a wrong magic |
| 0x04 | `u32RingPoolNum` | from config +0x18; the object has room for 4 rings and **the count is not range-checked** (5+ would overrun the 0x168-byte object) |
| 0x08 | owner channel (Create arg 2) | value −1 disables the waiting of `GetHeadRecordTimeout` / `GetHeadRecordUnrelease` |
| 0x0c | u8 flag (config +0x1c) | when set, `GetRecordUnreadNum` counts only closed records and `GetHeadRecordTotalNum` reports 0 if no closed record exists; all three callers pass venc's chn byte +0x804 or 0 |
| 0x10 | `wait_queue` | one queue for both "space freed" and "record committed" |
| 0x28 | list of **published** records (global read order, linked through record +0x08) | |
| 0x30 | pool spinlock | |
| 0x48 | `stConfig` copy (0x20 bytes, below) | size and min size rewritten by Create |
| 0x68 + 0x40·i | `astRingPoolInfo[i]`, i < 4 | |

**Config** (8 × u32, passed to Create, returned by `GetRingPoolConfig`):

| off | field | callers |
|---|---|---|
| 0x00 | `u32SizeOfPool` — bytes **per ring**; must be > alignment and > min size; Create rounds it up to the alignment, then to 4 KiB | jpd attr[0], vdec max(attr, 128 KiB), venc chn buffer size |
| 0x04 | min record size (`MinSize`); 0 → alignment; rounded up to the alignment | jpd 0x400, vdec 0x1000, venc min(size / chn `u32MaxStrmCnt` [I: the divisor register is the field the preceding BUG_ON checks], size / 9) |
| 0x08 | `u32Alignment`, power of two; 0 → 0x20 | jpd 0x20, vdec 1, venc 0x20 |
| 0x0c, 0x10, 0x14 | stored, never read by mi_sys | all callers 0 |
| 0x18 | ring count | jpd 1, vdec 1, venc chn byte +0x1670 |
| 0x1c | u8 flag → pool +0x0c | jpd 0, vdec 0, venc chn byte +0x804 |

venc's ring count comes from a channel byte that is 1 unless multi-ring encoding is configured [I]; what sets it to more than 1
was not traced.

**Ring** (`pstRingPoolInfo`, 0x40 bytes):

| off | field |
|---|---|
| 0x00 | not used |
| 0x04 | atomic reference count, 1 after Create |
| 0x08 | ring spinlock |
| 0x20 | list of all records of this ring in write order (record +0x00) |
| 0x28 | `record_end_offset` — aligned end of the last committed piece; where writing continues when the ring has no records |
| 0x30 | u64 `PA` (MIU address) = pool base + i × size |
| 0x38 | `VA` (kernel mapping, uncached, `mi_sys_Vmap(…, 0)`) |
| 0x3c | u8 `bBufLock` — a writer holds reserved tail space in this ring (set by EnsureTailSpace, cleared when a piece is committed closed) |

**Internal record** (`pstInternalRecord`, 0x70 bytes from the kmem cache `g_miSysRingpoolRecordCachep`; freed records are
poisoned with the source line number):

| off | field |
|---|---|
| 0x00 | ring-list node |
| 0x08 | pool-list node (valid once published) |
| 0x10 | u8 `bContentClosed` — last piece of a unit (frame) |
| 0x11 | u8 `bContentRead` — handed to a reader |
| 0x12 | u8 not yet published (sits only in the ring list) |
| 0x18 | `stContentRecord`, 0x50 bytes — the caller-visible record, below |
| 0x68 | channel id the record belongs to |

**Content record** (0x50 bytes, exchanged with the modules; `GetRingPoolTotalSpace` uses the same shape):

| off | field |
|---|---|
| 0x00 | u64 `extFlags` |
| 0x08 | u64 `pts` |
| 0x10 | u64 `seqNum` |
| 0x18 | u8 `bEOS` |
| 0x20 | u64 `PA` |
| 0x28 | `VA_In_Kernel` |
| 0x2c | u32 `u32Length` |
| 0x30–0x4f | not interpreted by mi_sys (copied through) |

### 10.2 Calls
All calls take `(pool, …)`; "ISR flag" = the caller says it runs in interrupt context; the code BUG_ONs if that does not match
`CamOsInInterrupt()` and uses plain spinlocks in ISR mode, IRQ-saving ones otherwise. Lock order: pool lock, then ring lock.
Exported names are `mi_sys_RingPool_<call>` (and `mi_sys_Ringpool_…` where written so below); their importers are listed after
the table.

| call | args | behaviour |
|---|---|---|
| `Create` | `(h, chn, unused, owner, &cfg, &pool)` | checks cfg (align power of 2, align < size, min < size → else `0xA0092003`); allocates one block of count × size from the device / channel private heap (`mi_sys_PrivateDevChnHeapAlloc(h, chn, 0, owner, …, 1)`, failure `0xA009200C`), maps it into the kernel (failure → free, `0xA009201D` NOVASPACE); fills the rings; returns the pool |
| `Destroy` | `(h, chn, pool)` | decrements every ring's count; when the sum is 0: frees all records, unmaps, frees the heap block, poisons and frees the pool. With the counts at 1 the first Destroy frees (nothing else takes references) |
| `EnsureTailSpaceTimeout` | `(pool, ms, &out, hintRec, u8 resetWhenEmpty)` | reserves tail space, process context. Tries once; on failure with `ms ≠ 0` sleeps **uninterruptibly** on the pool queue until a retry succeeds or `ms` runs out (`ms` < 0 ⇒ effectively forever). Returns the last error (`0xA009200C` no room, `0xA009201F` every ring locked) |
| `EnsureTailSpaceInISR` | `(pool, &out, hintRec, resetWhenEmpty)` | the same, one try, interrupt context only |
| `AppendTailContentTailRecord` | `(pool, chn, isrFlag, bClose, &content)` | commits a piece (§10.4) |
| `GetHeadRecordTimeout` | `(pool, chn, &out, bMarkRead, ms)` | first published record of `chn` with `bContentRead == 0` (closed or not); copies the 0x50 content; marks it read if asked. Not found → `0xA009201F`; with `ms ≠ 0` and pool owner ≠ −1, sleeps uninterruptibly on the pool queue and retries |
| `Ringpool_GetHeadRecordInISR` | `(pool, chn, &out, bMarkRead)` | the same lookup in ISR mode, no wait. No importer |
| `GetHeadRecordUnrelease` | `(pool, chn, &out, unused, ms)` | first published record of `chn` **whatever its read flag** (the oldest not yet released); waiting, if any, uses the unread lookup (as GetHeadRecordTimeout with mark 0). Used by vdec |
| `ReleaseHeadRecord` | `(pool, chn, isrFlag)` | removes the first published record of `chn` from both lists, frees it, wakes the queue. Not found → `0xA009200E` BUF_EMPTY. It does not check the read flag |
| `ReleaseTailRecord` | `(pool, isrFlag)` | removes the newest published record (any channel); its ring's `record_end_offset` = that record's start; wakes. Empty → `0xA009200E`. No importer |
| `CancelUnFinishedTailRecord` | `(pool, &content, isrFlag, u8 bBufLock)` | ring = the one holding `content.PA`; sets its `bBufLock`; drops from the ring tail every unpublished record that is not closed, then from the published tail every record of this ring that is neither closed nor read; `record_end_offset` = start of the last one dropped; wakes if something was dropped. Used by venc to discard a partly written frame |
| `Ringpool_GetRecordTotalNum` | `(pool, chn, isrFlag, &n)` | n = published records of `chn` with `bContentClosed` [R disassembly; the Ghidra text is wrong here] |
| `Ringpool_GetRecordUnreadNum` | `(pool, chn, isrFlag, &n)` | n = published, unread records of `chn`; with pool flag +0x0c only closed ones |
| `Ringpool_GetHeadRecordTotalNum` | `(pool, chn, isrFlag, &n)` | n = records of `chn` from the head up to and including the first closed one; no closed one → all of them, or 0 with pool flag +0x0c [R disassembly] |
| `GetRingPoolTotalSpace` | `(pool, &rec0x50)` | zeroes the record, fills `PA` (+0x20), `VA` (+0x28) of ring 0 and total length count × size (+0x2c) |
| `GetRingPoolConfig` | `(pool, &cfg)` | copies the 0x20-byte config (as rewritten by Create). No importer |
| `SetRecordMinAllocSize` | `(pool, size)` | 0 or ≥ ring size → `0xA0092003`; else min size = size rounded up to the alignment |
| `DumpRecordInfo` | `(pool)` | printk of every ring (`PA VA Size MinSize Align bBufLock`) and its records (`index bContentClosed bContentRead PA VA length extFlags pts seqNum bEOS`) |

Importers: `mi_sys_RingPool_{Create, Destroy, AppendTailContentTailRecord, EnsureTailSpaceTimeout, GetHeadRecordTimeout,
ReleaseHeadRecord, GetRingPoolTotalSpace, SetRecordMinAllocSize}` and `mi_sys_Ringpool_GetRecordTotalNum` — jpd, vdec, venc;
`…EnsureTailSpaceInISR`, `…CancelUnFinishedTailRecord`, `…DumpRecordInfo`, `mi_sys_Ringpool_GetHeadRecordTotalNum` — venc;
`…GetRecordUnreadNum` — vdec, venc; `…GetHeadRecordUnrelease` — vdec.

### 10.3 Free-space rule (`_MI_SYS_IMPL_GetRingPoolFreeSpace` (0x49848), unnamed `_MI_SYS_IMPL_GetRingPoolFreeSpace`)
Ring choice: with a hint record, the ring that contains the hint's `PA` (continuing a unit in the same ring, lock ignored);
without one, the first ring whose `bBufLock` is 0 (none → `0xA009201F`). On success the ring's `bBufLock` is set. The result is
written into the caller's content record: `PA`, `VA_In_Kernel`, and `u32Length` = the size of the free region found (the writer
may use less; the committed length decides).

Within the ring (S = ring size, M = min size, offsets relative to the ring's PA):
- **no records**: with `resetWhenEmpty` → whole ring from offset 0. Without it → from `record_end_offset` if S − end ≥ M, else
  the whole ring from 0.
- **records present**: `beg` = start of the oldest record of the ring, `end` = end of the newest rounded up to the alignment.
  - `beg < end` (data not wrapped; free space is the tail [end, S) and the head [0, beg)): if tail × 4 < head, prefer the head
    (take [0, beg) if beg ≥ M, else the tail if ≥ M); otherwise prefer the tail (tail if ≥ M, else head if beg ≥ M).
  - `beg ≥ end` (wrapped): [end, beg) if its size ≥ M.
  - nothing fits → `0xA009200C`.

A record never wraps around the ring end: each reservation is one contiguous region, so a reader always gets one contiguous
buffer per record.

### 10.4 Commit and publication (`AppendTailContentTailRecord`)
- Length 0: no record is made; the ring's `bBufLock` = !bClose, and with bClose the ring's newest record is marked closed (this
  is how venc closes a frame whose last slice was already committed). The spare record object is freed.
- Otherwise a record is created from the 0x50 content (must lie inside the ring, else BUG_ON), `chn` and `bContentClosed =
  bClose` are set, it is appended to the ring list, marked unpublished, unread; ring `bBufLock` = !bClose;
  `record_end_offset` = aligned end of the content.
- Publication: if the newest published record belongs to another ring and is not closed, nothing is published now (a unit
  being written in another ring must finish first). Otherwise every unpublished record of this ring is moved, in order, to the
  pool's published list. Unpublished records of a ring are only flushed by a later Append on that same ring; with one ring
  (all users on this camera) records are published immediately.
- bClose == 1 → wake the pool queue (readers waiting in GetHeadRecordTimeout). An unclosed piece does not wake anyone, but it is
  already visible to `GetHeadRecord*`.

### 10.5 Use on this camera (from the callers)
venc reserves in ISR with the previous slice as hint, appends each slice from the ISR with bClose 0 and closes the frame from
process context with bClose 1; jpd and vdec reserve with a timeout (the app gives −1 / its own value) and append the whole input
unit with bClose 1; readers release strictly from the head. One ring per pool is in use for all three modules. mi_venc's
`ResetChn` (stopped channel only, else 0xA002201F) releases every record of the current ring and, if a replacement pool exists,
drains and destroys that one; no pool is created there. mi_venc's ioctl 0x44 `RingPool_GetFreeSpace` is a stub returning 0x1F
(like 0x46 / 0x47) (`mi_venc.md` §5.4).

## 11. Engine features this camera does not use

Present in the binary and specified (the ioctl handlers and exports in `mi_sys-2-interfaces.md` §3, §6), but not reached by the
modules and the app of this camera: user pictures (ioctls 0x29–0x2b; an input port in user-picture mode refuses binds, §3.2),
input-port injection (0x11 / 0x12 / 0x1b, used only by the IQ server), register access (0x0d / 0x0e), private pools and
per-channel heaps from user space (0x0f / 0x10 / 0x20 / 0x25 / 0x26), BDMA ioctls (0x21–0x24), low latency (0x27, §4.6), buffer
dup (0x28), UUID, the HW timer (mi_vdec / mi_ai), PQ tables of config.ini (DISP PQ), log-to-disk, the debug proc views,
suspend / resume. In the engine itself: multi-pass devices (slots 0x74 / 0x78), slot 0x88 self-barrier nodes, the
back-reference queue (mi_vdisp only), the SW_LOW_LATENCY bind type (never chosen), and every binding module of §3.3 that is not
on the app's path. The MMU path, by contrast, is active on this camera (`mi_sys-2-interfaces.md` §7.3).
