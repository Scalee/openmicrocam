# mi_sys-2-interfaces — mi_common and the interfaces of mi_sys
<!-- ev:mi_sys-2-interfaces -->

`mi_common.ko` is the character-device front end of every MI kernel module of SigmaStar's MI layer on the Mercury6 (M6)
chip: one dynamic major `mi`, a node `/dev/mi_<name>` per module, the unpacking of the 16-byte ioctl wrapper, the open /
close callbacks with the caller's process id, the PM hooks and `/proc/mi_modules`. `mi_sys.ko` is the core module. This
document holds its interfaces: load, parameters, init and exit; `/dev/mi_sys` with 44 ioctls (18 of them reached by
prog_uvc, libcus3a and libmi_iqserver) and their in-kernel mirror; `/dev/mi_poll`; the user library libmi_sys's side of the
ioctls not in the shared table; the exports the other modules import; memory (address spaces, MMA heaps, the MMU path,
private heaps, allocators, MIU protect, BDMA, the CMDQ buffer); the HW timer; the MI log; procfs; the configuration tables
config_tool feeds. The pipeline engine (object model, registration contract, binding, frame flow, sideband, worker threads,
CMDQ objects, pass graph, SRAM, ring pools) is `mi_sys-1-engine.md`.

Tags: **[R]** read from the binary, a decompile, disassembly or a file; **[S]** stated by a source; **[I]** inference. A
heading tagged [R] means every statement under it is [R] unless marked otherwise.

Binaries: `/config/modules/4.9.227/mi_common.ko` and `mi_sys.ko` (build 2022-07-21, `project_commit.7251436`);
`libmi_sys.so` of the same dump. Address rule: a Ghidra address in `mi_sys.c` = objdump `.text` address + 0x10028;
Unnamed static functions are named by the `__func__` strings of their asserts. An older source of the same code base
(2018, github `jockyw2001/sdk`) has a `device.c` close to the 2022 mi_common binary; its mi_sys serves for names only
[S: 58-mi-sys]. Its internal API header is `interface/include/sys/mi_sys_internal.h` there; the 2022 binary has a larger
registration record (0x298 bytes, `mi_sys-1-engine.md` §2.2).

Shared facts owned elsewhere: the user-side ioctl wrapper, `'i'` numbering, payload packing, return codes, the MI error-code
format and module ids — `mi-user-abi.md`; the physical memory map (DRAM size, `mma_heap=` reservation, reserved regions) and
the MIU protect / MMU hardware — `kernel-platform.md` §11, §12.6; the mmap.ini / config.ini grammar and the meaning of the
`/proc/mi_modules/common/*` records — `config_tool.md`; the mhal CMDQ interface — `mhal-scl-jpe-cmdq.md` §2.3.

Header of the user library: the public skyipc_tiramisu `mi_sys.h` (its prototypes match the binary).

## 1. mi_common.ko — the device frame of every MI module [R]

### 1.1 Load
No module parameters. Init: wait queue `g_miTaskInExitingEvent`; class `mi` (`CamClassCreate`, with the PM hooks of §1.5);
`__register_chrdev(0, 0, 256, "mi", fops)` (dynamic major, 256 minors); `/proc/mi_modules/` and
`/proc/mi_modules/mi_common/module_version_file`.

### 1.2 Registered device object (one static object per module)
Each module keeps one object in its `.data` and calls `MI_DEVICE_Register(obj)` from `init_module`.

| off | field |
|---|---|
| 0x00 / 0x04 | list links |
| 0x0c | `const char *name` ("sensor" for mi_sensor, "hdmi", "gfx", "disp", "venc", "sys", …) → node `/dev/mi_<name>` |
| 0x10 | minor (assigned: 63 − first free bit of a 64-entry bitmap) |
| 0x14 | `struct device *` |
| 0x18 | ioctl handler table, indexed by `_IOC_NR`; entry = `s32 fn(ctx *, void *payload)` |
| 0x1c | table size |
| 0x20 | `connected(client *)` (the module's ProcessInit / `process_init`) — on every open |
| 0x24 | `disconnected(client *)` (ProcessExit / `process_exit`) — on every release |
| 0x28 / 0x2c | resume / suspend |
| 0x30 | `struct mutex *` (the module mutex), held around the four callbacks above |

`MI_DEVICE_Register(obj)`: takes a free bit (0..63), minor = 63 − bit, `CamDeviceCreate(class, NULL, MKDEV(major, minor),
obj, "mi_%s", name)` (device number = major << 20 | minor; mdev makes the node `/dev/mi_<name>`), links the object. Minors are
handed out from 63 downwards, so a module's minor depends on the load order; the libraries open the nodes by name only.
`MI_DEVICE_Unregister` undoes it. What `connected` / `disconnected` do is per module (mi_hdmi: Init / DeInit of the whole
module — `mi_disp-mi_hdmi.md` §2.1; mi_sys: §2.4).

### 1.3 File operations
- **open** (`MI_DEVICE_Open`): only `O_RDWR` is accepted (else −EACCES). Allocates a client `{device *, tgid, pid}` as
  `file->private_data`. If a previous MI process is still exiting (flag set in release, below) it waits on
  `g_miTaskInExitingEvent`, up to 200 jiffies. Then, under the object mutex, `connected(client)` and the wake-lock helper
  (§1.5).
- **release**: under the object mutex `disconnected(client)` and the matching `wake_unlock` helper. If the closing task is
  exiting, the first MI fd closed sets the flag; closing `mi_sys` clears it and wakes the waiters. Effect [I]: a new process
  opening MI devices waits until the dead process's mi_sys clean-up (§2.4) is done.
- **unlocked_ioctl** (`MI_DEVICE_Ioctl(file, cmd, arg)`), the kernel half of the wrapper of `mi-user-abi.md` §3:
  1. `_IOC_TYPE` must be `'i'` and `_IOC_NR` < table size, else −EIO (−5, `0xfffffffb`).
  2. context `{const char *name = "common", u32 from = 1 (ioctl), struct mutex *held (ctx+8, `mutex_to_unlock`) = NULL,
     client * (`file_ctx`)}`.
  3. `arg == 0` → `handler(ctx, NULL)` with no payload (the `_IO` calls, e.g. `MI_HDMI_Init` / DeInit).
  4. copy the 16-byte wrapper `{u32 len; u32 pad; u64 ptr}`; `len > _IOC_SIZE(cmd)` → −EINVAL; `len > 4096` (0x1000) only
     logs.
  5. `_IOC_WRITE` set: `len == 0` → −EINVAL, else `memdup_user(ptr, len)`; only `_IOC_READ`: `kmalloc(len)` (not zeroed); no
     direction → −EINVAL.
  6. `ret = table[nr](ctx, buf)`; for `_IOC_READ` the buffer is copied back (`copy_to_user(ptr, buf, len)`) **whatever `ret`
     is** (the libraries copy to the caller only on 0); `kfree`.
  7. if the handler stored a mutex in `ctx->held`, it is unlocked here, after the copy-back.
  8. return `ret` unchanged — the MI error code reaches user space as the ioctl result.
- **Handler pattern** (every MI module): copy the scalar fields out of the payload, `mutex_lock(module mutex)`, store it in
  `ctx->held`, tail-call `MI_<MOD>_IMPL_<Func>(fields…, pointers into the payload)`; the IMPL function writes its outputs
  straight into the payload, which mi_common copies back. **One mutex per module serialises all its ioctls** (a consequence for
  mi_venc's ioctl 0x45 is in `mi_venc.md` §5.4).

### 1.4 Exports (8) and users
| export | users | what |
|---|---|---|
| `MI_DEVICE_Register`, `MI_DEVICE_Unregister` | all 20 mi_* | §1.2 |
| `MI_DEVICE_GetProcModuleDir` | all mi_*, fbdev | returns `/proc/mi_modules` |
| `mi_device_number_ops` | all mi_* | proc file: read / write one u32 (`debug_level`; write = `kstrtouint`, base 0) |
| `mi_device_version_ops` | all mi_* | proc file: show a string (`module_version_file`) |
| `mi_device_debug_file_ops`, `mi_device_debug_func_ops` | all mi_* | proc file: 256-byte string set by write (log filters `debug_file`, `debug_func`, §1.6) |
| `MI_DEVICE_NotifyMiSysExit` | mi_sys | empty in this build |

Modinfo `depends=` lists only mi_common (plus mi_gfx for mi_disp, mi_sys for mi_venc), but mi_hdmi, mi_gfx and mi_disp also
import mi_sys exports (`mi_sys_RegisterDev`, `MI_SYS_LOG_IMPL_PrintLog`, `MI_COMMON_Print`, …) and all MI modules import
mhal; the load order is what makes them resolve.

### 1.5 Wake lock and PM hooks
<!-- ev:mi_sys-2-interfaces.1.5 -->
- **Wake lock**: on every MI device open, after `connected()`, mi_common runs `call_usermodehelper("/bin/sh", {"/bin/sh",
  "-c", "echo mi_<name> > /sys/power/wake_lock"}, …, UMH_WAIT_PROC)`; on release the same with `wake_unlock`. Open and close
  **wait for the shell to exit**, so a shell is spawned on every MI open and close. The kernel has `CONFIG_PM_WAKELOCKS`
  (`wake_lock_store`, `pm_wake_lock`, `pm_show_wakelocks` in the kernel symbol map), so `/sys/power/wake_lock` exists and each open creates / activates a wakeup source named `mi_<name>` [I,
  strong]; the kernel image has no autosleep strings, so the locks change nothing at run time [I].
- **PM hooks**: the `mi` class gets `core_module_suspend / core_module_resume(dev, state)`: find the registered object whose
  `"mi_<name>"` is a prefix-match of the device name, take its mutex and call its suspend (+0x2c) / resume (+0x28) callback. If
  no object matches, the loop ends on the list head and the code still locks and uses `head[0xc]` as a mutex (a defect,
  unreachable because every class device belongs to a registered object). The camera never suspends [S: 58-mi-sys].

### 1.6 What every MI module carries (`mi_device.c` compiled into each module)
- Module parameters `debug_level` (uint), `debug_file`, `debug_func` (strings) and the procfs files
  `/proc/mi_modules/mi_<name>/{debug_level, debug_file, debug_func, module_version_file}`, made with the ops of §1.4; module
  parameter and proc file set the same variables.
- **Debug filters**: a write to `debug_file` / `debug_func` stores up to 256 bytes and replaces the last stored byte by NUL
  (normally the newline); reading shows the string. Every function's log call is wrapped as `if (level > n || func/file match)
  MI_SYS_LOG_IMPL_PrintLog(…)`: a site prints when the module's `debug_level` exceeds the site's level **or** the site's
  function name is a substring of `debug_func` (`strstr(filter, func)`) **or** the basename of its source file is a substring of
  `debug_file`.
- `mi_sys_RecodeTimeStamp(line, "name")` calls — boot-time profiling markers (§6.1).
- `mi_sys_RegisterDev(info, usrData, procfs_ops, proc_dir)` + `mi_sys_RegistCommand(name, nargs, fn, handle)`: the module
  becomes an mi_sys device (`mi_sys-1-engine.md` §2) and gets `/proc/mi_modules/mi_<name>/mi_<name><dev>`, whose read prints the
  procfs-ops state and whose write runs the registered text commands (§10.3).
- Source paths in the strings: `alkaid/sdk/interface/src/<mod>/` and `alkaid/sdk/impl/<mod>/`.

## 2. mi_sys.ko — load, parameters, init, per-process life, exit

### 2.1 Load and unload [R]
`init_module`: `/proc/mi_modules/mi_sys/{debug_level, debug_file, debug_func, module_version_file}`; `MI_COMMON_CreatePollDev`
(`alloc_chrdev_region(…, 1, "mi_poll")` + cdev; no class device, hence the `mknod` in demo.sh; §4); `MI_SYS_IMPL_Init` (§2.3);
`/proc/mi_modules/mi_sys/reserve_2_free` (§10.1); `MI_DEVICE_Register` with name `sys` → `/dev/mi_sys`, 44 ioctls,
connected / disconnected = `mi_sys_process_init` / `mi_sys_process_exit` (§2.4), resume / suspend = `MI_SYS_IMPL_Resume` /
`MI_SYS_IMPL_Suspend` (§2.5). `cleanup_module` removes the proc files and the device only.

### 2.2 Module parameters [R]
demo.sh loads mi_sys with `cmdQBufSize=3072 logBufSize=256` only
(`dump/…/customer/demo.sh:31`).

| param | default | use |
|---|---|---|
| `cmdQBufSize` | 1024 | KiB of the MMA buffer `CMDMEM` given to `MHAL_CMDQ_InitCmdqMmapInfo` (§7.9) |
| `logBufSize` | 1024 | KiB of the MMA ring buffer `sys-logBuffer` of the MI log (§9) |
| `default_config_path` | `charp` at `.data+0x238`, placeholder `"ABABABABABABABABABAB"` = `/config` | directory of `config_tool`, `mmap.ini` and `miuprotect.txt`; also exported as a symbol (§6.2) |
| `config_ini_path` / `config_pq_path` | `/misc/config.ini` / `/misc/PQConfig.ini` | arguments of `load_config` (§2.3 step 4) |
| `bEnableMmu` (`.data+0x16d`) | 1 | MMU path of the MMA heaps (§7.3) |
| `entry_size` | 0 | MMU page size in KiB; 0 = chosen from the memory size (§7.3) |
| `bDebugMmu` | 0 | MMU guard pages (§7.3) |
| `bEnableMmuPool` | 0 | debug aid of the MMU path (§7.3) |
| `bEnableMiuProtect` (`bool`, `.data+0x140`) | 1 | gates the call of `init_glob_miu_kranges` (§7.7) |
| `bSelfFence`, `bEnableCmdqReset` | 1, 1 | CMDQ software fences (`mi_sys-1-engine.md` §7.1) / CMDQ reset on timeout [I from the name] |
| `debug_level` (`.data+0xd8`), `debug_file`, `debug_func` | 2, '', '' | log filter (§1.6) |

### 2.3 `MI_SYS_IMPL_Init` (module load, every `/dev/mi_sys` open, ioctl 0) [R]
Reference counted; the body runs on the first call only:
1. semaphores, the buffer-handle IDR (§2.4), memory caches, work queues `mi_wq` and `mi_cmdq_wq`; `MDrv_MIU_Init` (failure →
   `0xA009201F`).
2. `MI_SYSCFG_SetupMmapLoader`: creates `common/mmap_info` and `common/memory_info` (§10.2), then runs the usermode helper
   `/config/config_tool` with argv `{"/config/load_mmap", "/config/mmap.ini"}`, env `HOME=/ TERM=linux
   PATH=/sbin:/usr/sbin:/bin:/usr/bin LD_LIBRARY_PATH=/lib:/config/lib`, waiting for it to exit. config_tool picks its action
   from argv[0] and writes the parsed table into `mmap_info` (`config_tool.md` §2.3).
3. `parse_miuprotect_file` (`/config/miuprotect.txt`); its result sets the kernel-range protection switch (§7.7).
4. `MI_SYSCFG_SetupIniLoader`: creates the `config_info`, `pq_*` and `*_table` files, runs config_tool with argv
   `{"/config/load_config", "/misc/config.ini", "/misc/PQConfig.ini"}`.
5. Compares the `E_LX_MEM` length from mmap.ini with the kernel command line (warning only).
6. Heaps: for each boot MMA reservation (`mstar_driver_boot_mma_buffer_num` / `mma_config[]`, filled by the kernel from
   `mma_heap=`) an MMA heap is created (§7.2), linked, removed from the protected kernel ranges (`deleteKRange`, if MIU protect
   is on, §7.7) and — **for the MMU heap** — given an idle MIU protect block in "access from VPA only" mode over its physical
   range (`mi_sys_SetAccessFromVpaOnly(block, start, end, 1)` → `MDrv_MIU_SetAccessFromVpaOnly`; block index kept at MMU info
   +0x7014): hardware may reach those pages only through MMU addresses [I from the driver function's name]. Plus the heap
   `imi_mma_heap` (internal memory, §7.2). A boot reservation named `MMU_MMA` is merged into the following reservation of the
   same MIU only on `I2M` revision 1 with 64 MiB. The per-heap success line (`mi_sys_mma_allocator_create success,
   heap_base_addr=… length=…`) is printed at KERN_NOTICE and is filtered on this camera's console; the one visible line in the
   boot log (`heap_base_addr=20000000 length=43800`) is the `imi_mma_heap` (0x43800 bytes), printed with `CamOsPrintf`.
7. `MI_SYSCFG_InitCmdqMmapInfo`: the CMDQ buffer (§7.9).
8. debug proc tree (§10), MI log init (§9), version printk; mi_sys registers itself as MI device SYS/0 (module 9, dev 0, no
   channels; handle `g_hSysDrv`) with its ten proc commands (§10.4).

`MI_SYS_IMPL_Exit` decrements the count (§2.5).

### 2.4 Per-process life and buffer handles [R]
- open of `/dev/mi_sys` → `mi_sys_process_init` → `MI_SYS_IMPL_Init` (count +1).
- close (including process death) → `mi_sys_process_exit(client)` → `MI_SYS_IMPL_FreeUsrProcessBufById(tgid)` frees everything
  recorded for that process — output-buffer handles (ioctl 0x13), input-buffer handles (0x11), map-to-user tokens, MMA
  allocations (0x1c) and private-heap allocations (0x25) — then `MI_SYS_IMPL_Exit` (count −1).
- the `MI_SYS_Init` / `MI_SYS_Exit` ioctls (0x00 / 0x01) change only the count.

**Handle records** (0x40 bytes, kmem cache `g_miSysBufhandledataCachep`), one per-process list:
| off | field |
|---|---|
| +0x00 | list node (per-process clean-up list) |
| +0x08 | kind: 0 input buffer, 1 output buffer, 2 map-to-user token (`E_MI_SYS_IDR_BUF_TYPE_MMAP_TO_USER_SPACE`), 3 MMA allocation (`MmaAlloc`), 4 private-heap allocation |
| +0x0c | buffer reference |
| +0x10..+0x1c | ChnPort |
| +0x20 | flags |
| +0x24 | owner tgid (−1 = kernel) |
| +0x28 | u64 address |
| +0x30 / +0x34 | debug tag |
| +0x38 | IDR id |
| +0x3c | user-picture worker (ioctl 0x29) |
IDR ranges: input-buffer handles 0x40–0x1FFF (kind 0), output-buffer handles 0x2000–0x3FFF (kind 1); records of kinds 3 and 4
sit in the same list without an id.

### 2.5 `MI_SYS_IMPL_Exit`, suspend and resume [R]
- **`MI_SYS_IMPL_Exit`** (count 1 → teardown; else count − 1): `MI_SYS_IMPL_CheckExitReady` waits up to 100 × 100 ms until
  mi_sys's own SYS device is the only registered device (else `0xA0092012`, nothing torn down); then: log system exit, SYS
  device unregistered, VB-pool allocator collection deinit, CMDQ buffer freed (`MI_SYSCFG_ExitCmdqMmapInfo`), every MMA heap
  given back to the protected ranges (`addKRange`, except imi) and the VPA-only block released (MMU heap), heaps destroyed,
  `g_mmuMmaEnable` = 0, kernel protect removed (`exit_glob_miu_kranges`, §7.7), `mmap_info` / ini loader proc files removed,
  debug proc tree removed, the kmem caches (records, chunks, allocations, ring-pool records …), the 36 per-module mutex pairs,
  the handle IDR, semaphores, both work queues destroyed, the direct-buffer wrapper allocator released. The MMU itself is not
  disabled or reset (no `MDrv_MMU_*` call in Exit). The module is never unloaded and the count never drops to 0 while the SYS
  device exists, so normal operation never reaches the teardown [I, consistent with the code].
- **Suspend** (`MI_SYS_IMPL_Suspend`, via mi_common's class PM hook, §1.5): only with the MMU active — drops the VPA-only
  protection of the MMU heap.
- **Resume** (`MI_SYS_IMPL_Resume`): `MDrv_MIU_Init`, re-applies MIU protection (`mi_sys_ResumeMiuProtect`), and for the MMU
  heap re-programs the MMU (reset, page size, region, callback, enable), re-maps every VA page with a non-zero use count to its
  slice, and re-arms VPA-only protection. Failure → `0xA009201F`.

### 2.6 Configuration getters [R]
The tables config_tool writes into `/proc/mi_modules/common/*` (kernel side §10.2) are read through these exports:
- `MI_SYSCFG_GetMmapInfo(name, &p)` returns 1 and a pointer to `{u8 gid, layer, miuNo, cmaId; u32 addr, size, align,
  memoryType}` for a section of mmap.ini, else 0. The camera's mmap.ini has four sections: `E_LX_MEM`, `E_MMAP_ID_DUMMY1`,
  `E_LX_LOGO_RESERVED_FB`, `E_MMAP_ID_EMI`. Requesters: mi_sys (`E_LX_MEM`, §2.3 step 5), mi_venc (`E_MMAP_ID_EMI`), fbdev (the
  names in `fbdev.ini`: `E_MMAP_ID_FB`, `E_MMAP_ID_HW_CURSOR` — absent from mmap.ini → MMA fallback), mi_gfx
  (`E_MMAP_ID_GE_VQ`, absent), mi_disp (`E_MMAP_ID_XC_MAIN_FB`, `E_MMAP_ID_XC_MLOAD`, absent), mi_ai / mi_ao (`E_MMAP_ID_AI`,
  `E_MMAP_ID_AO`, `E_MMAP_ID_AO_TMP`). No module reads `E_LX_LOGO_RESERVED_FB` or `E_MMAP_ID_DUMMY1`. The full requester list is
  in `config_tool.md` §2.5.
- `MI_SYSCFG_GetMemoryInfo(&p)` returns 1 and a pointer to the parsed `memory_info` table if config_tool wrote one, else 0
  (used by the MMU page mode, §7.3).
- `MI_SYSCFG_GetPanelInfo(intf, timing)` (mi_disp): the panel record whose interface / timing words match (record +0x84 / +0x88,
  i.e. `g_stConfig` +0x8c / +0x90); timing 0x38 = the first record of the interface.
- `MI_SYSCFG_GetPqTbl` (mi_disp): the PQ table. It is the only PQ getter; the NR tables have no exported getter and no reader.
- `MI_SYSCFG_GetDLCTableCount()`: byte 0 of `g_stConfig` (count of DLC tables loaded from config.ini).

## 3. `/dev/mi_sys` ioctls [R]

### 3.1 Conventions
Every handler takes one global mutex and leaves it to mi_common to release after the copy-back (§1.3), so all mi_sys ioctls
run one at a time; none of the reached ones waits for data (0x13 GetBuf has no timeout); the unreached 0x11 waits, up to its
timeout, for a free user-buffer slot. Each returns the IMPL result unchanged (the Ghidra text shows several wrappers as
`void`; the disassembly passes r0 through). Payload offsets are inside
the payload (the wrapper is §1.3). "Handle" = an id of the buffer-handle IDR (§2.4). "Used" = reached by prog_uvc's MI code,
libcus3a or libmi_iqserver (the user-side table is in `mi-user-abi.md` §3.3, §10); the rest is not reached on this camera. The command
words match the user library's, e.g. 0xC1286913 = `_IOWR('i', 0x13, 0x128)`. 

Return codes used: 0, `0xA0092003` ILLEGAL_PARAM, `0xA0092005` UNEXIST, `0xA0092008` NOT_SUPPORT, `0xA009200C` NOMEM,
`0xA009200D` NOBUF, `0xA0092012` BUSY, `0xA009201F` FAILED.

### 3.2 The table
| nr | name | payload | behaviour | used |
|---|---|---|---|---|
| 0x00 | Init | 4, ignored | `MI_SYS_IMPL_Init` (count +1, §2.3) | app |
| 0x01 | Exit | 4 | count −1 | app |
| 0x02 | BindChnPort | 0x30 as 0x03 | bind type FRAME_BASE, param 0 (`mi_sys-1-engine.md` §3.2) | iqserver |
| 0x03 | BindChnPort2 | 0x30: +0 src ChnPort, +0x10 dst ChnPort, +0x20 srcFps, +0x24 dstFps, +0x28 bind type, +0x2c bind param | bind (`mi_sys-1-engine.md` §3.2) | app |
| 0x04 | UnBindChnPort | 0x30 (two ChnPorts used) | unbind | app |
| 0x05 | GetBindbyDest | 0x30: +0 src (out), +0x10 dst, +0x28 u32 bind type (out) | source of a destination input port; +0x28 = the input port's bind type (input port +0xd4) | iqserver |
| 0x06 | GetVersion | 0x80 out | `"Sigmastar Module mi_sys version: project_commit.7251436 sdk_commit.80327e1 build_time.20220721223317"`, zero-padded | app |
| 0x07 | GetCurPts | 8 out | `mono_us − mono_at_base + pts_base` | app |
| 0x08 | InitPtsBase | 8 in | `pts_base = arg`, `mono_at_base = now` (µs, monotonic clock) | app |
| 0x09 | SyncPts | 8 in | same as 0x08 | app |
| 0x0a | Mmap | 0x18: +0 u8 bCache, +4 size, +8 VA (out), +0x10 u64 MIU address | map physical memory into the calling process (§7.4) | app (VENC / JPD stream buffers), libcus3a |
| 0x0b | Munmap | 8: VA, size | `CamOsMemFromUserModeUnmap` | app, libcus3a |
| 0x0c | Va2Pa | 0x10: +0 u32 VA of the caller, +8 u64 MIU address (out) | no `mm` (kernel thread) → `0xA0092005`; `find_vma` + `follow_pfn` on the caller's address space (taken with `down_write`), CPU physical → MIU (§7.1); no VMA or no PFN → `0xA0092003` | — |
| 0x0d | SetReg | 8: +0 u32 register address (CPU physical, inside the RIU window `Chip_Get_RIU_Phys`..+size), +4 u16 value, +6 u16 mask | outside the window → `0xA009201F`; else read-modify-write of the 16-bit register at VA `(addr & ~3) + 0xDE000000` (static RIU mapping 0x1F000000 → 0xFD000000): `reg = reg & ~mask \| value & mask` | — |
| 0x0e | GetReg | 8: +0 address, +4 u16 value (out) | same window check, 16-bit read | — |
| 0x0f | SetChnMMAConf | 0x4c: +0 module id, +4 dev, +8 chn, +0xc char heap[0x40] | the heap name (≤ 0x3f chars, copied) becomes the channel's MMA heap (channel +0x0c, replaces an older one). Device not registered → `0xA009201F`; chn ≥ count → `0xA0092003`; device with registration byte +0x18 set (VENC) → `0xA0092008`; with the MMU on it warns "Mmu enable, not support set mma heap" | — |
| 0x10 | GetChnMMAConf | 0x50: as 0x0f + `+0x4c u32 length` | copies the channel's heap name (≤ min(length, 0x40) − 1 chars, empty if none); errors as 0x0f | — |
| 0x11 | ChnInputPortGetBuf | 0x1a0: +0 ChnPort, +0x10 `MI_SYS_BufConf_t` (0x78), +0x88 BufInfo (0x108, out), +0x190 handle (out), +0x194 s32 timeout ms, +0x198 u32 flags | user space gets an empty buffer to fill and later push into an **input** port. Port or channel disabled → none. At most 4 user buffers per input port (atomics +0xa0 held + +0xa4 queued); at the limit waits uninterruptibly on +0xb4 up to the timeout (negative → 2 jiffies). Allocation with the input port's ext-conf from its user allocator (+0x144, `mi_sys_SetInputPortUsrAllocator`) or the default policy, owner = port name. Flags bit 31 → mapped into the caller (or the kernel when called without `mm`). Handle 0x40–0x1FFF recorded with the caller's tgid. If the pass is not in sync-infer mode and the port is unbound, the pass graph is re-evaluated. No buffer → handle −1, `0xA009200D` | iqserver |
| 0x12 | ChnInputPortPutBuf | 0x118: +0 handle, +8 BufInfo (0x108), +0x110 u8 bDrop | handle must be kind 0 (else `0xA0092003`). bDrop → unmap / flush, free. Else copies from the given BufInfo into the buffer: PTS (+0), end flag (+0x14), and per type — RAW: content size (+0x34 ≤ buffer size); FRAME: +0x08 u64, +0x20 / +0x28 / +0x30, crop words +0x7c / +0x80; MULTIPLANE: per plane two words; then unmap, sequence −1, timestamp, **inject** into the port (`EnqueueBufToInputPort(…, inject = 1)`, `mi_sys-1-engine.md` §4.5) and free the handle | iqserver |
| 0x13 | ChnOutputPortGetBuf | 0x128 (§3.3) | take a buffer from the port's user FIFO | app (YUYV / NV12 branch), iqserver |
| 0x14 | ChnOutputPortPutBuf | 4: handle | unmap if mapped, drop the buffer reference, free the handle; unknown handle → `0xA0092003` | app |
| 0x15 | SetChnOutputPortDepth | 0x18: ChnPort, +0x10 u32 user depth, +0x14 u32 queue depth | stores both in the output port; user > queue → `0xA0092003` (`mi_sys-1-engine.md` §4.7) | app |
| 0x16 | GetChnOutputPortDepth | 0x18: ChnPort, +0x10 user depth (out), +0x14 queue depth (out) | reads output port +0x0c / +0x14; unknown port → `0xA0092003` | — |
| 0x17 | poll: GetFd | 0x14: ChnPort, u32 id out | create / reference the port's poll object, return its id (§4) | app (`MI_SYS_GetFd`) |
| 0x18 | poll: CloseFd | 4: fd | drop the reference held by fd (via `fget`) | app (`MI_SYS_CloseFd`) |
| 0x19 | poll: get fd | 0x14: ChnPort, s32 fd out | fd registered for the port, or `0xA0092005` | app |
| 0x1a | poll: set fd | 0x14: ChnPort, s32 fd | register fd for the port | app |
| 0x1b | ChnPortInjectBuf | 0x14: +0 handle, +4 destination ChnPort | handle must be kind 1 (an output buffer from 0x13): unmap, inject the same buffer into the destination **input** port, free the handle; result of the enqueue (`0xA009200C` if the port refused) | — |
| 0x1c | MmaAlloc | 0x30 (§3.3) | MMA allocation for the caller | libcus3a, iqserver |
| 0x1d | MmaFree | 8: u64 MIU address | find the record by address, free; unknown → `0xA0092003` | libcus3a, iqserver |
| 0x1e | FlushInvCache | 8: VA, len | `CamOsMemFlush(VA, len)` (§7.4); zero arguments → `0xA0092003` | app (VENC GetStream) |
| 0x1f | Read_Uuid | 8: u64 out | `msys_read_uuid(&out)` (kernel built-in, eFuse id); failure → `0xA009201F` | — |
| 0x20 | ConfigPrivateMMAHeap | 0x3c: +0 type, +4 module id, +8 u8 bCreate, +0xc dev, +0x10 chn, +0x14 port, +0x18 size, +0x1c char heap[0x20] | converted to the kernel form (§7.5) and passed to `MI_SYS_IMPL_ConfigPrivateMMAPool` with the tag 0x4D495F53 (`'MI_S'` = from user space). **With MMU heaps enabled the user call is ignored (returns 0)**; the log text says "private pool is disabled by default" | — |
| 0x21 | MemsetPa | 0x10: +0 u64 MIU address, +8 u32 value, +0xc u32 length | `msys_dma_fill({addr, len, value})` (BDMA); address 0 or length 0 → BUG; DMA failure → `0xA009201F` | — |
| 0x22 | MemcpyPa | 0x18: +0 u64 dst, +8 u64 src, +0x10 u32 length | `msys_dma_copy({src, dst, len})`; zero arguments → BUG | — |
| 0x23 | BufFillPa | 0x78: +0 `MI_SYS_FrameData_t` (0x68: +4 pixel format, +0x18 / +0x1a u16 width / height, +0x28 u64 phys[3], +0x40 u32 stride[3]), +0x68 u32 colour, +0x6c rect {u16 x, y, w, h} | fills the rectangle (clipped to the frame; start outside → 0) plane by plane with `msys_dma_fill` (whole lines) or 2-D `msys_dma_blit`. Formats: YUYV and RGB565 / 1555 / 4444 (2 B/px), ARGB8888 (4), I2 / I4 / I8, YUV semi-planar 422 (Y from colour bits 16–23, UV from bits 0–15), NV12 (UV interleaved pattern, half height), RGB888 / BGR888 (3 B/px); others → `0xA0092003` | — |
| 0x24 | BufBlitPa | 0xe0: +0 dst FrameData, +0x68 dst point / rect, +0x70 src FrameData, +0xd8 src rect | copies a rectangle between two frames of the same format, clipped to both; full-width contiguous → `msys_dma_copy`, else `msys_dma_blit`. Formats: as 0x23 plus Bayer 8 / 10 / 12 / 14 / 16 bit (the 16-bit group only with compress mode 4) | — |
| 0x25 | PrivateDevChnHeapAlloc | 0x40: +0 char name[0x20] (`"owner#heap"`: before `#` the owner name, after it the heap name), +0x20 u32 (the kernel overwrites it with the heap-name pointer; user space must leave 0), +0x24 size, +0x28 u64 address (out), +0x30 module id, +0x34 dev, +0x38 chn, +0x3c u8 flag | finds the registered device (module, dev) and allocates from its private heap (§7.5), falling back to the general MMA heap; recorded for the caller's tgid | — |
| 0x26 | PrivateDevChnHeapFree | 0x18: +0 u64 address, +8 module id, +0xc dev, +0x10 chn | frees into the private heap that contains the address, else `MI_SYS_IMPL_MmaFree` | — |
| 0x27 | EnableChnOutputPortLowLatency | 0x18: ChnPort, +0x10 u8 bEnable, +0x14 u32 param | bEnable 0 → output port +0x20e = 0, +0x210 = 0; bEnable ≠ 0 and param ≠ 0 → +0x20e = bEnable, +0x210 = param; (bEnable ≠ 0, param 0) → no change. Effect: each output task carries `param` for that port (+0x44 + 4i) and a per-port bit; when the module reports the task partly done (`CheckOutputTaskStatus` = 4) the buffer goes to the user FIFO at once and the bit is cleared, so the final completion does not queue it again (`mi_sys-1-engine.md` §4.6) | — |
| 0x28 | ChnDupBufHandle | 8: +0 handle in, +4 new handle (out) | handle of kind 0 or 1 only (else `0xA0092003`); copies the handle record, takes a new buffer reference (mapped again if the original was mapped), allocates a new id in the same range, and counts it as held (+0x1c output / +0xa0 input). No memory → `0xA009200C` | — |
| 0x29 | ChnInputPortSetUserPicture (SetUserPicture) | 0x118: +0 handle (kind 0, from 0x11, already filled), +8 BufInfo, +0x110 u32 fps (1–1000) | refuses if the handle already has a user-picture worker (`0xA0092012`) or the port is in user-picture mode; creates a 0x2c-byte worker record {thread, fps, wait queue, atomic tick +0x20, atomic enabled +0x24, HW-timer id +0x28} and a kernel thread (`mi_sys_internal_user_picture_worker_thread`) | — |
| 0x2a | ChnInputPortEnableUserPicture | 4: handle | port bound → `0xA009201F`; registers a mi_sys HW timer (§8) with period 1000/fps ms whose callback sets *tick* and wakes the worker; port byte +0x9c = 1 (binding to this port is now refused, `mi_sys-1-engine.md` §3.2); timer enabled. On every tick the worker re-injects the same buffer into the port (sequence −1, new timestamp) | — |
| 0x2b | ChnInputPortDisableUserPicture | 4: handle | clears +0x9c and *enabled*, stops the thread, disables and unregisters the timer, frees the record, then drops the buffer through PutBuf(…, bDrop = 1) | — |

### 3.3 The two payloads of the reached calls with their own layout
**0x13 ChnOutputPortGetBuf**, payload `+0x00 ChnPort`, `+0x10 MI_SYS_BufInfo_t (0x108, in/out)`, `+0x118 handle (out)`,
`+0x120 u32 flags`. Flags bit 31 = map the buffer into the caller: `MI_SYS_ChnOutputPortGetBuf` writes `0xA0000000`,
`…GetBufPa` writes 0 (libmi_sys.so disassembly at 0x1e90 / 0x1f8e). Action: take the oldest buffer of the port's user FIFO
(empty → `0xA009200D` NOBUF; `mi_sys-1-engine.md` §4.5); allocate a handle in 0x2000–0x3FFF; map it if bit 31 (failure →
`0xA009201F`); record `{handle, buffer, tgid}` (§2.4); copy BufInfo out.

**0x1c MmaAlloc**, payload `+0x00 char heap[0x20]` (empty = default heap; `"a#b"` is split into two names), `+0x20` scratch (the
kernel stores the owner name `"app-mmaAlloc"` there), `+0x24 u32 size`, `+0x28 u64 MIU address (out)`. Allocates from the MMA
allocators (§7.2), records `{address, tgid}` (kind 3); failure → `0xA009200C` NOMEM.

### 3.4 Kernel-mode MI_SYS API (37 exports, no importer)
<!-- ev:mi_sys-2-interfaces.3.4 -->
`MI_SYS_Init, Exit, BindChnPort, BindChnPort2, UnBindChnPort, GetBindbyDest, GetVersion, GetCurPts, InitPtsBase, SyncPts,
SetReg, GetReg, ReadUuid, Va2Pa, Mmap, Munmap, FlushInvCache, SetChnMMAConf, GetChnMMAConf, ChnInputPortGetBuf,
ChnInputPortPutBuf, ChnOutputPortGetBuf, ChnOutputPortPutBuf, ChnPortInjectBuf, SetChnOutputPortDepth, GetChnOutputPortDepth,
MMA_Alloc, MMA_Free, MemsetPa, MemcpyPa, BufFillPa, BufBlitPa, ConfigPrivateMMAPool, ChnInputPortSetUserPicture,
EnableUserPicture, DisableUserPicture, GetFd, CloseFd`.
Same signatures as the user library (`MI_U16 u16SocId` first where the M6 `mi_sys.h` has it). Each builds the ioctl payload on
its stack exactly as libmi_sys does, fetches the handler by command number from the ioctl table (`MI_DEVICE_WrapperFunction(cmd)`
returns `table[cmd & 0xff]` for nr < 0x2c, else NULL), calls it with a zeroed 16-byte context (`from = 0`, kernel) and unlocks
the mutex the handler left in the context; outputs are copied back as in user space (`ChnOutputPortGetBuf` sets flags
0xA0000000 like the library; buffers are then mapped into the *current* task, or the kernel when it has no `mm`).
`MI_SYS_GetBindbyDest(?, &dst, &src, &bindType)` reads the bind type back from +0x28. Exceptions: `MI_SYS_GetFd` returns
`0xA009201F` with fd −1 (poll fds do not exist in kernel mode); `MI_SYS_CloseFd` issues ioctl 0x18.

## 4. `/dev/mi_poll` and poll objects [R]
- The node: `MI_COMMON_CreatePollDev` at module load (`alloc_chrdev_region(…, 1, "mi_poll")` + cdev, no class device; demo.sh
  creates the node with `mknod`).
- File ops: `open` → 0; `ioctl(fd, any cmd, id)` → stores `id` as the file's poll object; `poll` → `poll_wait` on the object's
  wait queue, then its state function: bit 0 → `POLLIN`, bit 1 → `POLLOUT`. No `release`: the object is dropped by mi_sys
  ioctl 0x18 before `close()` (user side in `mi-user-abi.md` §5).
- The id handed to user space is the object's kernel address; the libraries treat the id as opaque.
- Object: `{owner; wait_queue_head; …; state_fn (+0x20); release_fn (+0x24); refcount (+0x28)}`.
- Exports (mi_venc uses them for `MI_VENC_GetFd`): `MI_COMMON_CreatePoll(&slot, state_fn, release_fn)` (create, or take a
  reference), `MI_COMMON_PollRelease(&slot)` (last reference → `release_fn`, free), `MI_COMMON_WakeUpClient(obj)` (wake
  waiters).
- mi_sys output port: readable while its user FIFO is not empty; woken when a buffer is added to the FIFO
  (`mi_sys-1-engine.md` §4.5). Release resets the port's fd to −1.

## 5. libmi_sys.so — the 34 exports outside the shared user-ABI table [R]
<!-- ev:mi_sys-2-interfaces.5 -->
The library's other exports (the 155-row table: wrapper, packing, `MI_SYS_Init`, binds, GetBuf / PutBuf, depth, poll fds …)
are in `mi-user-abi.md` §10; the kernel side of every nr is §3. Importers on the camera of the 34 below: libcus3a, libmi_iqserver,
and the unused libmi_ipu / ive / sed / vdf / MD / OD / VG.

| function (header signature) | nr (payload) | payload built by the library | copy-back / logic |
|---|---|---|---|
| `MI_SYS_BindChnPort(soc, *src, *dst, srcFrmrate, dstFrmrate)` | 2 (0x30) | +0 src, +0x10 dst, +0x20 srcFrmrate, +0x24 dstFrmrate; +0x28 / +0x2c left uninitialised | old bind (no bind type); the kernel uses defaults |
| `MI_SYS_GetBindbyDest(soc, *dst, *src out, u32 *bindType)` | 5 (0x30) | +0x10 dst | ret 0 → `*src = +0` (0x10), `*bindType = +0x28`; else returns the code |
| `MI_SYS_ReadUuid(soc, u64 *uuid)` | 0x1f (8) | wrapper points at the caller's u64 | the kernel writes it |
| `MI_SYS_Mmap(u64 phy, u32 size, void **va, bool cache)` | 0xa (0x18) | +0 u8 bCache, +4 size, +0x10 u64 phy | `*va = +8` always |
| `MI_SYS_Munmap(va, size)` | 0xb (8) | +0 va, +4 size | — |
| `MI_SYS_Va2Pa(va, u64 *pa)` | 0xc (0x10) | +0 va, +8 u64 preset 0xFF | ret 0 → `*pa = +8` |
| `MI_SYS_SetReg(soc, addr, u16 val, u16 mask)` | 0xd (8) | +0 addr, +4 val, +6 mask | — |
| `MI_SYS_GetReg(soc, addr, u16 *val)` | 0xe (8) | +0 addr | `*val = +4` always |
| `MI_SYS_SetChnMMAConf(soc, module, dev, chn, char *heap)` | 0xf (0x4c) | +0 module, +4 dev, +8 chn, +0xc heap (strlen+1 bytes; no length cap — a name ≥ 0x40 overruns the stack payload) | — |
| `MI_SYS_GetChnMMAConf(soc, module, dev, chn, void *buf, u32 len)` | 0x10 (0x50) | +0 module, +4 dev, +8 chn, +0x4c len | `memcpy(buf, +0xc, min(len, 0x40))` always |
| `MI_SYS_ChnInputPortGetBuf(*chnPort, *bufConf (0x78), *bufInfo out (0x108), *hBuf out, s32 ms)` | 0x11 (0x1a0) | +0 chnPort, +0x10 bufConf, +0x194 ms, **+0x198 flags = 0x80000000** (map into the process) | `*bufInfo = +0x88`, `*hBuf = +0x190` always |
| `MI_SYS_ChnInputPortGetBufPa(…)` | 0x11 | same, flags = 0 (no mapping) | same |
| `MI_SYS_ChnInputPortPutBuf(hBuf, *bufInfo, bool drop)` / `…PutBufPa` | 0x12 (0x118) | +0 hBuf, +8 bufInfo (0x108), +0x110 u8 drop | — (both identical) |
| `MI_SYS_DupBuf(hBuf, *newHBuf)` | 0x28 (8) | +0 hBuf, +4 = 0 | `*new = +4` always |
| `MI_SYS_EnableChnOutputPortLowLatency(soc, *chnPort, bool en, u32 param)` | 0x27 (0x18) | +0 chnPort, +0x10 u8 en (+0x11..0x13 zeroed), +0x14 param | — |
| `MI_SYS_GetChnOutputPortDepth(soc, *chnPort, *user, *queue)` | 0x16 (0x18) | +0 chnPort | unless −1: `*user = +0x10`, `*queue = +0x14` (each only if non-NULL) |
| `MI_SYS_ChnPortInjectBuf(hBuf, *dstChnPort)` | 0x1b (0x14) | +0 hBuf, +4 chnPort | — |
| `MI_SYS_MMA_Alloc(soc, char *heap, u32 size, u64 *phy)` | 0x1c (0x30) | +0 heap[0x20] (≤ 31 chars, zero-filled; NULL = default), +0x20 = 0, +0x24 size | ret 0 → `*phy = +0x28` |
| `MI_SYS_MMA_Free(soc, u64 phy)` | 0x1d (8) | +0 u64 phy | — |
| `MI_SYS_FlushInvCache(va, len)` | 0x1e (8) | +0 va, +4 len | — |
| `MI_SYS_ConfigPrivateMMAPool(soc, MI_SYS_GlobalPrivPoolConfig_t *c)` | 0x20 (0x3c) | converts the union to the flat kernel form of §3.2: +0 type (`c->eConfigType`), +4 module, +8 u8 bCreate, +0xc dev, +0x10 chn, +0x14 port, +0x18 size, +0x1c heap[0x20]. Type 0 VPE→VENC ring: size = `u32VencInputRingPoolStaticSize`, heap; type 1 per channel: module, dev (+8 of the union), chn, size, heap; type 2 per device: module, dev, size, heap; type 3 per output port: module, dev, chn, port, heap, size. Heap names copied up to 31 bytes (type 3: 32) | other type → `0xA009201F` without an ioctl |
| `MI_SYS_MemsetPa(soc, u64 pa, u32 val, u32 len)` | 0x21 (0x10) | +0 pa, +8 val, +0xc len | — |
| `MI_SYS_MemcpyPa(soc, u64 dst, u64 src, u32 len)` | 0x22 (0x18) | +0 dst, +8 src, +0x10 len, +0x14 = 0 | — |
| `MI_SYS_BufFillPa(soc, MI_SYS_FrameData_t *f, u32 val, MI_SYS_WindowRect_t *r)` | 0x23 (0x78) | +0 frame (0x68), +0x68 val, +0x6c rect (8), +0x74 = 0 | — |
| `MI_SYS_BufBlitPa(soc, *dstFrame, *dstRect, *srcFrame, *srcRect)` | 0x24 (0xe0) | +0 dst frame, +0x68 dst rect (8: u16 x, y, w, h), +0x70 src frame, +0xd8 src rect | — |
| `MI_SYS_PrivateDevChnHeapAlloc(soc, module, dev, chn, char *name, u32 size, u64 *phy, bool flag)` | 0x25 (0x40) | +0 name[0x20] (≤ 31 chars; NULL → `"app-privAlloc"`), +0x20 = 0, +0x24 size, +0x30 module, +0x34 dev, +0x38 chn, +0x3c u8 flag | ret 0 → `*phy = +0x28` |
| `MI_SYS_PrivateDevChnHeapFree(soc, module, dev, chn, u64 phy)` | 0x26 (0x18) | +0 phy, +8 module, +0xc dev, +0x10 chn | — |
| `MI_SYS_ChnInputPortSetUserPicture(hBuf, *bufInfo, MI_SYS_UserPictureInfo_t *u)` | 0x29 (0x118) | zeroed, +0 hBuf, +8 bufInfo (0x108), +0x110 `u->u32SrcFrc` (fps) | — |
| `MI_SYS_EnableUserPicture(hBuf)` / `DisableUserPicture(hBuf)` | 0x2a / 0x2b (4) | +0 hBuf | — |
| `MI_SYS_CloseFd`, `MI_SYS_GetFd` | 0x18; 0x19 / 0x17 / 0x1a | `mi-user-abi.md` §2 | — |
| `_MI_PRINT_GetDebugLevel(name)` | — | `mi-user-abi.md` §6 (common library mechanics) | — |

Data exports: `g_mi_fd_table`, `g_last_fd_index`, `g_mi_fd_semalock` (`mi-user-abi.md` §2.1), `sstar_mi_so_version_string`.

## 6. Exports to other kernel modules [R; signatures I from call sites unless marked]
<!-- ev:mi_sys-2-interfaces.6 -->

`__ksymtab_strings` lists **154** exports; **87** are imported by some module, 72 of them
by vif, isp, scl, rgn, gfx, venc, disp, hdmi, sensor, jpd, vdec or fbdev and 15 only by mi_vdisp, mi_ai or mi_shadow; the
other **67** have no importer (the 37 of §3.4 and those of §6.2; list). h = the device
handle `mi_sys_RegisterDev` returns. Users below are every importer of the dump.

### 6.1 Imported exports
| group | export (signature) | users | what |
|---|---|---|---|
| registration | `mi_sys_RegisterDev(info *, usrData, procfs_ops *, dir)` | fbdev, ai, ao, disp, gfx, hdmi, ipu, isp, jpd, ldc, mipitx, panel, rgn, scl, sensor, shadow, vdec, vdisp, venc, vif | `mi_sys-1-engine.md` §2.1 |
| | `mi_sys_UnRegisterDev(h)` | same | removes the device |
| | `mi_sys_RegistCommand(cmd, maxArgs, fn, h)` | same | a text command of the device's `mi_<mod><dev>` proc file (§10.3) |
| logging | `MI_COMMON_RealGetSelfDir` | same | the module's proc directory |
| | `MI_COMMON_Print` | ai, ao, disp, gfx, hdmi, ipu, isp, jpd, ldc, mipitx, panel, rgn, scl, sensor, shadow, vdec, vdisp, venc, vif | printk (§9) |
| | `MI_SYS_LOG_IMPL_PrintLog(fmt, …)` | the same without ipu | the MI log (§9) |
| state | `mi_sys_EnableChannel(h, chn)` / `mi_sys_DisableChannel(h, chn)` | ai, disp, ipu, isp, jpd, ldc, mipitx, scl, shadow, vdec, vdisp, venc, vif | gate task scheduling per channel |
| | `mi_sys_EnableOutputPort(h, chn, pass, port)` | ai, disp, isp, jpd, ldc, scl, shadow, vdec, vdisp, vif | gate per output port |
| | `mi_sys_DisableOutputPort(h, chn, pass, port)` | ai, disp, jpd, ldc, shadow, vdec, vdisp, vif | |
| | `mi_sys_EnableInputPort(h, chn, pass, port)` | disp, isp, ldc, mipitx, scl, shadow, vdisp, venc | gate per input port |
| | `mi_sys_DisableInputPort(h, chn, pass, port)` | disp, ldc, mipitx, shadow, vdisp, venc | |
| | `mi_sys_BlockingDisableInputPort` / `…OutputPort(h, chn, pass, port, timeoutMs)` | isp, scl (1000 ms) | wait for the tasks in flight, then disable |
| port config | `mi_sys_SetOutputPortBufExtConf(h, chn, pass, port, &ext)` | isp, ldc, scl, shadow, vdec, vif | buffer geometry (`mi_sys-1-engine.md` §4.2) |
| | `mi_sys_SetInputPortBufExtConf(h, chn, pass, port, &ext)` | disp, isp, ldc, scl, shadow, venc | same |
| | `mi_sys_SetOutputPortBufClearConf(h, chn, pass, …)` | isp, ldc, scl | padding clear (`mi_sys-1-engine.md` §4.2) |
| | `mi_sys_SetChnOutputPortDepth(&ChnPort, user, queue)` | scl | depths (`mi_sys-1-engine.md` §4.7) |
| | `mi_sys_GetChnBufInfo(h, chn, pass, &info)` | disp, jpd, vdec, venc | queue counters |
| sideband | `mi_sys_SetInputPortSidebandMsg(h, chn, pass, port, u64)` | disp, isp, scl, venc | 64-bit message carried with bound buffers (`mi_sys-1-engine.md` §5) |
| | `mi_sys_GetOutputPortSidebandMsg(h, chn, pass, port, &u64)` | scl, vif | same |
| binding | `mi_sys_UnBindChnPort(0, &arg)` | isp, scl, vif | unbind from inside a module |
| scheduling | `mi_sys_NotifyWakeupISREvent(h, pass)` | ai, disp, isp, jpd, ldc, mipitx, scl, shadow, vdisp, venc, vif | wakes the pass's MAIN and output waits; called from the modules' interrupt paths (`mi_sys-1-engine.md` §6) |
| | `mi_sys_NotifyWakeupISREventByChn` | vdec | per channel |
| | `mi_sys_DoDirectBufTask(task, timeoutMs)` | scl (`MI_SCL_StretchBuf`) | runs one task on existing buffers (§7.6) |
| memory | `mi_sys_MMA_Alloc(heap or 0, owner, size, &addr)` | fbdev, disp, gfx, isp, jpd, rgn, scl, vdisp | §7.2 |
| | `mi_sys_MMA_Free(addr)` | the same plus ao | |
| | `mi_sys_PrivateDevChnHeapAlloc(h, chn, pass, owner, size, &addr, flag)` / `…Free` | ai, ao, ipu, isp, jpd, ldc, rgn, scl, vdec, venc, vif | §7.5; the IMPL behind it is `MI_SYS_IMPL_PrivateDevChnHeapAlloc(dev, chn, heapName, owner, size, &addr, flag)` [R] |
| | `mi_sys_Vmap(addr, size, bCache)` / `mi_sys_UnVmap(va)` | fbdev, ai, ao, disp, ipu, isp, jpd, ldc, rgn, scl, vdec, vdisp, venc, vif | kernel mapping (§7.4) |
| | `mi_sys_VFlushInvCache(va, size)` | ipu, isp, jpd, ldc, rgn, scl, vdec, venc, vif | cache (§7.4) |
| | `mi_sys_Mmap` / `mi_sys_Munmap` | isp | user mapping (§7.4) |
| | `mi_sys_Miu2Cpu_BusAddr` | fbdev, isp, jpd, scl | §7.1 |
| | `mi_sys_MemcpyPa` | isp, ldc, scl | BDMA (§7.8) |
| | `mi_sys_MemsetPa` | rgn (clears a newly allocated front buffer: I2 / I4 / I8 → 0, other formats → `0x23232323 \| mask`, `rgn-gop.md` §3.3) | BDMA |
| | `mi_sys_BufFillPa`, `mi_sys_BufBlitPa` | rgn, vdisp (vdisp's 0x98-byte copy records → `BufBlitPa`, `mi_vdisp-mi_shadow.md` §4.5) | BDMA |
| | `mi_sys_SetMiuProtect` | isp, scl | MIU protect window (§7.7) |
| | `mi_sys_setMiuProtectMMa` | ipu, vdec, venc | same |
| | `mi_sys_IsMmuMma(&b)` | venc | `g_mmuMmaEnable` (§7.3) |
| | `mi_sys_GetChnMMAConf` | venc | the channel's heap name (ioctl 0x10 form) |
| | `mi_sys_GetModuleMMaHeapName` | ipu, vdec, venc | |
| | `mi_sys_FreeMMAPReservedArea` | fbdev | `logo2free` (§10.1) |
| ring pools | `mi_sys_RingPool_{Create, Destroy, AppendTailContentTailRecord, EnsureTailSpaceTimeout, ReleaseHeadRecord, GetRingPoolTotalSpace, SetRecordMinAllocSize}`, `mi_sys_Ringpool_GetRecordTotalNum` | jpd, vdec, venc | ring buffer of variable-size records in one MMA block — the bitstream buffers (`mi_sys-1-engine.md` §10) |
| | `mi_sys_RingPool_GetHeadRecordTimeout` | jpd, venc | |
| | `mi_sys_RingPool_{EnsureTailSpaceInISR, CancelUnFinishedTailRecord, DumpRecordInfo}`, `mi_sys_Ringpool_GetHeadRecordTotalNum` | venc | |
| | `mi_sys_Ringpool_GetRecordUnreadNum` | vdec, venc | |
| | `mi_sys_RingPool_GetHeadRecordUnrelease` | vdec | |
| time | `mi_sys_GetCurPts(&u64)` | ai, isp, jpd, rgn, scl, venc, vif | ioctl 0x07 time base |
| | `mi_sys_RecodeTimeStamp(line, name)` | fbdev, disp, isp, scl, sensor, venc, vif | boot-time profiling markers (kernel `recode_timestamp`) |
| | `mi_sys_RegisterHwTimer`, `mi_sys_UnRegisterHwTimer`, `mi_sys_EnableTimer`, `mi_sys_DisableTimer` | ai, vdec | §8 |
| config | `MI_SYSCFG_GetMmapInfo` | fbdev, ai, ao, disp, gfx, venc | §2.6 |
| | `MI_SYSCFG_GetPanelInfo` | disp, panel | §2.6 |
| | `MI_SYSCFG_GetPqTbl` | disp | §2.6 |
| poll | `MI_COMMON_CreatePoll`, `MI_COMMON_PollRelease`, `MI_COMMON_WakeUpClient` | venc | §4 |
| allocators | `generic_allocator_init / on_ref / on_unref`, `generic_allocation_init / on_ref / on_unref` | vdisp | §7.6 |
| | `mi_sys_SetInputPortUsrAllocator(h, chn, pass, port, allocator)` | vdisp | input +0x144 (used by ioctl 0x11 GetBuf); references the new, unreferences the old |
| | `mi_sys_SetPeerOutputPortCusAllocator(h, chn, pass, inPort, allocator)` | vdisp | sets the user allocator (+0x158) of the **output port bound to this input** — the consumer decides where the producer allocates; input not bound → `0xA0092003` |
| | `mi_sys_EnsureOutportBkRefFifoDepth` | vdisp | back-reference queue depth (`mi_sys-1-engine.md` §4.7) |
| | `mi_sys_GetInputPortBufExtConf(h, chn, pass, port, &ext)` | vdisp | merged ext-conf of the bound output port (port value lcm-combined with all its consumers); unbound → zeros |
| output buffers | `mi_sys_ChnOutputPortGetBufPa(&ChnPort, &bufinfo, &handle, flags)` | ai | `MI_SYS_IMPL_ChnOutputPortGetBuf` — the ioctl 0x13 path for kernel callers (handle 0x2000–0x3FFF, recorded for the current tgid; flags bit 31 maps into the current task) |
| | `mi_sys_ChnOutputPortPutBuf` / `…PutBufPa(handle)` | ai | `MI_SYS_IMPL_ChnOutputPortPutBuf` (ioctl 0x14) |
| map to user | `mi_sys_MmapBufToMultiUserVirAddr(h, &bufinfo, &token, a4, bFlush)` | shadow | maps an existing buffer into the current process without a buffer handle: a handle record of kind 2 is returned as an opaque token; RAW / META buffers map the whole buffer, FRAME buffers map plane 0 and derive the plane 1 / 2 VAs from their phys offsets; optional cache flush. No buffer → `0xA009200D`, map failure → `0xA009201F`. a4 and bFlush are 1 at every mi_shadow call site; the meaning of a4 is not known |
| | `mi_sys_UnmapBufToMultiUserVirAddr(h, token, &bufinfo, a4)` | shadow | token must be kind 2 and belong to the same device handle (else BUG); unmaps and returns the bufinfo pointer; a4 = 1 at every call site |

### 6.2 Exports without an importer (besides the kernel-mode API of §3.4)
| export | behaviour |
|---|---|
| `MI_COMMON_Info / Warning / Error(fmt, …)` | `vprintk` of a level prefix + fmt (256-byte buffer) |
| `MI_COMMON_SetupDebugFile(name, set, show)` / `UnloadDebugFile(name)` | slot table of 36 entries × 0x34 {name[0x20], proc entry, ops, two callbacks}; creates / removes `/proc/mi_modules/common/<name>` with the seq-file ops `s_stCommonInfoProc` that call the two callbacks (§10.2). Name ≥ 32 chars or table full → `0xA009201F` |
| `MI_COMMON_SetupDebugRawFile` / `UnloadDebugRawFile` | the same for raw (non-seq) files: 36 × 0x2c slots, `commonraw_proc_read/write` (§10.2). `UnloadDebugFile` releases the wrong semaphore on its not-found path (`up()` of the raw table's) |
| `MI_SYSCFG_GetDLCTableCount()`, `MI_SYSCFG_GetMemoryInfo(&p)` | §2.6 |
| `default_config_path` | the module parameter string (§2.2) |
| `mi_device_mma_heap_file_ops` | file ops of `/proc/mi_modules/mi_sys/reserve_2_free` (§10.1) |
| `mi_sys_Init / mi_sys_Exit` | `MI_SYS_IMPL_Init / Exit` (count ±1, §2.3) |
| `mi_sys_AllocMMALowLevelBufAllocation(h, chn, &bufconf)` | allocation from the channel's MMA heap (channel +0x0c) with owner `"AllocMMALowLevelBuf"` and the channel's debug tag; returns the allocation object (no buffer reference) |
| `mi_sys_Cpu2Miu_BusAddr(u64 cpu)` | CPU → MIU rule of §7.1 (CPU < 0x2000_0000 is logged and BUG) |
| `mi_sys_MiPts2BootTimePts(u64 pts, &u64 out)` | out = pts + (stored u64 at .bss+0xba8 − stored u64 at .bss+0xba0) — the inverse of GetCurPts, i.e. the monotonic time a PTS corresponds to [I: the two words are the ones InitPtsBase writes] |
| `mi_sys_GetCur_UserMapCtx()` | `current->mm` |
| `mi_sys_GetOutputPortBufExtConf(h, chn, pass, port, &ext)` | the merged ext-conf (`mi_sys-1-engine.md` §4.2) |
| `mi_sys_SetInputPortBufClearConf(h, chn, pass, port, u8)` | input +0x158 (`mi_sys-1-engine.md` §4.2) |
| `mi_sys_SetOutputPortBufForceAlloc`, `mi_sys_SetOutputPortUserFIFOBlockMode` | `mi_sys-1-engine.md` §4.7 |
| `mi_sys_SetOutputPortUsrAllocator(h, chn, port, allocator)` | last pass only; refused (`0xA009201F`) while the port has bound inputs; references the new allocator (`ops.OnRef`), unreferences the old (`ops.OnUnref`), stores it at output +0x158 (used by the output-buffer allocation after the private heap, `mi_sys-1-engine.md` §4.3) |
| `mi_sys_cust_allocator_create(name, &custOps, priv)` | custom allocator (§7.6) |
| `mi_sys_ModTimer(h, timer, period)` | HW timer period change (§8) |
| `mi_sys_Read_Uuid` | ioctl 0x1f body |
| `mi_sys_RingPool_GetRingPoolConfig`, `mi_sys_RingPool_ReleaseTailRecord`, `mi_sys_Ringpool_GetHeadRecordInISR` | ring pools (`mi_sys-1-engine.md` §10) |
| `mi_sys_ProductHashGuard` | empty function |

## 7. Memory [R unless marked]

Every allocation carries an owner
name (`"app-mmaAlloc"`, `"CMDMEM"`, `"disp-screenLayer"`, `"gfx_vq"`, `"isp-mhal"`, port buffers `"scl0-out0-0"` …), shown in
`/proc/mi_modules/mi_sys_mma/<heap>` (§10.1).

### 7.1 Address spaces
MIU (bus) address ↔ CPU physical address:
| MIU address | CPU address |
|---|---|
| < 0x6000_0000 | MIU + 0x2000_0000 |
| 0x6000_0000 – 0x7FFF_FFFF | MIU + 0x1_0000_0000 |
| ≥ 0x8000_0000 | MIU + 0x1_8000_0000 |
CPU → MIU (`mi_sys_Cpu2Miu_BusAddr`): high word ≠ 0 → CPU − 0x1_0000_0000; else CPU − 0x2000_0000 (CPU < 0x2000_0000 is an
error). User-visible addresses (ioctls, BufInfo) are MIU addresses. The middle row is the MMU window: on platform "M6" a bus
address ≥ 0x6000_0000 is reached by the CPU at bus address + 0x1_0000_0000 (LPAE, above 4 GiB), so `map_user` / `vmap_kern` of
an MMU allocation map CPU physical 0x1_6xxx_xxxx pages (page aligned, per plane).

### 7.2 MMA heaps and the chunk manager
**Heaps.** One MMA heap per boot reservation (§2.3 step 6) plus `imi_mma_heap` (internal memory, its boot-log line in §2.3
step 6; used by the SRAM path — `mi_sys-1-engine.md` §9). On this camera one boot heap, `mma_heap_name0`
(`mma_heap=mma_heap_name0,miu=0,sz=0x9000000`, 144 MiB), and it is an MMU heap (§7.3).

`mi_sys_mma_allocator_create(name, ?, u64 cpuBase, length)`: cpuBase and length must be page aligned (imi: 256-byte aligned).
A range already covered by a registered heap of the same name and size returns NULL (no duplicate); a range inside another heap
records that heap as parent (+0xe8). A named heap gets a proc entry `/proc/mi_modules/mi_sys_mma/<name>` (dump / exec, §10.1).
| off | field |
|---|---|
| +0x08 | magic `'AMMH'` 0x484D4D41 (`__MI_SYS_MMA_ALLOCATOR_MAGIC_NUM__`) |
| +0x0c | length |
| +0x10 | u64 CPU base |
| +0x18 | u64 base of the heap's address space (MMU VA base for the MMU heap, else 0) |
| +0x20 | chunk manager (below), covering the heap (MMU heap: the whole VA window) |
| +0x78 | generic allocator (§7.6) with ops `g_mmaAllocatorOps` |
| +0x98 | name[0x20] |
| +0xb8 | lock |
| +0xd8 | list of private heaps carved from it (§7.5) |
| +0xe0 | u8 MMU heap |
| +0xe4 | MMU info block (0x7018 bytes, §7.3) |
| +0xe8 | parent heap |
| +0xec | u8 = `bDebugMmu` at creation |

**Chunk manager** (`mi_sys_chunk_mgr_t`, magic `'PAEH'` 0x48454150): +0x04 first chunk, +0x0c total, +0x10 free bytes,
+0x14..+0x20 atomics (peak used, …), +0x28 lock, +0x50 u64 owner tag. **Chunk** (`mi_sys_chunk_t`, 0x50, same magic): +0x04
offset, +0x08 length, +0x0c used, +0x10 thread name (16), +0x20 owner name (16), +0x30 prev, +0x34 next, +0x40 u64 tag.
- `mi_sys_alloc_chunk(mgr, size, &offset, owner, bTopDown)`: **best fit** — the smallest free chunk ≥ size (an exact fit stops
  the scan); bottom-up takes the start of the chosen chunk, top-down (`bTopDown`, used for private heaps and on request of the
  imi heap's callback) scans from the last chunk backwards and takes the end. Size > free → dump + `0xA009200C`; size 0 →
  `0xA0092003`.
- The tag (bit pattern `0x4D4E…` + device pointer / chn, set by the output-buffer allocation, `mi_sys-1-engine.md` §4.3)
  charges the bytes to the channel (+0x1b0c, peak +0x1b10) or device (+0xd58, peak +0xd5c) for the procfs statistics
  (`mem_stat`, §10.1).
- `mi_sys_free_chunk(mgr, offset)` merges with free neighbours (BUG on inconsistent links). Other chunk calls:
  `shrink_chunk`, `exchange_bufinfo_chunk` (behind `MI_SYS_IMPL_ExchangeBufInfo`), `merge_sram_chunk` (imi heap compaction,
  `mi_sys-1-engine.md` §9).

### 7.3 The MMU path — active on this camera
<!-- ev:mi_sys-2-interfaces.7.3 -->
`g_bHwSupportMmu` is a constant 1 in `.data`, `bEnableMmu` (`.data+0x16d`) defaults to 1, and
`mi_sys_mma_allocator_create` turns the MMU on for the **first** heap whose range fits the MMU window and whose name is not
`imi_mma_heap` / `mma_heap_codec` / `mma_heap_ipu`. The camera's boot log shows `_MI_SYS_MMU_Callback … Status=0x2,
PhyAddr=0x63fc0000, ClientId=0x3d, Name=MOPROT0_C_R` (and `…0x63c00000 … MOPROT0_Y_R`) on every boot, normal boots included
(it is part of the start-up noise listed in together with `[CMDQ]cmdq(4)
ERR: WAIT_TRIG_TIMEOUT … DISP0_INT`); that callback is registered only in this branch (and re-registered on resume) — so
`mma_heap_name0` is an MMU heap here [I, strong]. The CMDQ buffer at MIU 0x6018b000 (same log) and the DISP fault addresses
0x63xxxxxx are MMU virtual addresses. The faults are MOP reads of 0x63c00000 / 0x63fc0000 before those pages are mapped; they
show that the hardware does go through the MMU and have no other visible effect [I]. Buffers handed to the hardware are
therefore MMU-virtual and physically scattered in pages of the MMU page size [I: 256 KiB, mode 1 below].

Set-up (once, in `mi_sys_mma_allocator_create`):
- page mode `u16PageMode` (`mi_sys_mma_ConfigPageMode`): from `entry_size` (module parameter, KiB) if set, else from the total
  memory (`memory_info` of config_tool if present, else `lx_mem*_size`): ≤ 128 MiB → mode 0, ≤ 256 MiB → mode 1, else 2.
  Page = 128 KiB << mode; 1024 entries; VA window = 128 MiB << mode. This camera: `lx_mem_size` 0xffe0000 (procfs) → mode 1
  [I from the rule]: 256 KiB pages, 256 MiB window. The mode actually chosen is a camera reading, not known statically.
- window index = MIU address of the heap >> (27 + mode) — must be below the index of MIU 0x6000_0000 and the heap must end in
  that window; the VA window starts at MIU 0x6000_0000 (index 0x60000000 >> (27 + mode)). The heap start is rounded up and its
  length down to whole pages.
- `MDrv_MMU_Reset`, `MDrv_MMU_SetPageSize(mode)`, `MDrv_MMU_SetRegion(vaIndex, paIndex)` (retried with (pa, pa); failure is a
  BUG), `MDrv_MMU_CallbackFunc(_MI_SYS_MMU_Callback)` (logs the fault with `MDrv_MIU_ClientIdToName` and, if MMU logging is on,
  dumps the chunk manager), `MDrv_MMU_Enable`; `g_mmuMmaEnable` = 1.
- MMU info block (0x7018 bytes, `pstMmaMmuInfo`): +0x00 u16 first PA slice index, +0x04 number of PA slices = heap length /
  page, +0x08 free-slice count, +0x0c free-slice list, +0x14 + 0x14·i `mmu_pa_slices[1024]` {list node, u32 PA (MIU), s32
  `current_mmu_index` (−1 = free), u8 needs-unmap}, +0x5014 + 4·v atomic use count per VA page, +0x6014 + 4·v VA → slice map,
  +0x7014 the VPA-only protect block (§2.3 step 6).
- The heap's chunk manager covers the whole VA window; the heap's +0x18 = VA base.

**Allocation** in the MMU heap (`mi_sys_mma_allocator_alloc`): size from the buffer geometry (`mi_sys-1-engine.md` §4.2),
rounded to 4 KiB (with `bDebugMmu`: to whole pages plus one guard page; with the `debug_mmu addsize_before/after` command:
extra guard bytes around the buffer); chunk in VA space; for every VA page the range touches whose use count is 0, a free PA
slice is taken, a stale mapping is removed (`MDrv_MMU_UnMap`) and `MDrv_MMU_Map(vaPage, paSlice)` is programmed; then all use
counts of the range +1. Not enough free slices → log "free %d < need %d", `0xA009200C`-style failure (NULL). The returned bus
address = VA base + offset (+ front guard).
**Release** (`mi_sys_mma_allocation_on_release`): user and kernel mappings dropped, chunk freed, page use counts −1; a page
reaching 0 returns its slice to the free list and is unmapped at once — or, with the global delay-unmap flag set, marked
"needs unmap" and unmapped when the slice is reused.

Other MMU pieces: `MI_SYS_IMPL_IsMmuMma(&b)` → `g_mmuMmaEnable` (export `mi_sys_IsMmuMma`, used by venc); ioctl 0x20 is
ignored and 0x0f warns when the MMU is on (§3.2); `bEnableMmuPool`, `bDebugMmu` / `debug_mmu` and
`_MI_SYS_Check_MMU_List_Coherence` (verifies the free-list length and the VA ↔ slice links) are debug aids;
`MI_SYS_IMPL_Resume` re-registers the callback (§2.5). On platform `"I2M"` a device with registration byte +0x18 set gets the
heap name `"MMU_MMA"` for each channel at creation [R: 7-character literal; the only such string is "MMU_MMA"]; not effective
on M6 (`mi_sys-1-engine.md` §2.2). The free-slice count under load and the content of
`/proc/mi_modules/mi_sys_mma/mma_heap_name0` (VA window and slice use) are not known (camera readings).

### 7.4 Mapping and cache
- **Into a process** (ioctl 0x0a, and GetBuf 0x11 / 0x13 with flags bit 31): the address must be 4 KiB aligned, the size is
  rounded up to 4 KiB; the range must lie inside a boot MMA reservation or outside kernel RAM (`pfn_valid` false); mapping by
  `CamOsMemFromUserModeMap` (cached or uncached per `bCache`). Munmap → `CamOsMemFromUserModeUnmap`.
- **Kernel mapping** for modules: `mi_sys_Vmap(addr, size, bCache)` / `mi_sys_UnVmap(va)` / `mi_sys_VFlushInvCache(va, size)`;
  `mi_sys_Mmap` / `mi_sys_Munmap` are the user-mapping calls for modules (isp).
- **Cache**: `FlushInvCache` → `CamOsMemFlush(va, len)` (clean + invalidate [I]).

### 7.5 Private heaps (`MI_SYS_IMPL_ConfigPrivateMMAPool`, `_MI_SYS_IMPL_ConfigDevChnPrivateMMAHeap`, `_MI_SYS_IMPL_ConfigChnPortBufPrivateMMAHeap`)
A private heap is a block carved from an MMA heap and managed by its own chunk manager, reserved for one (module, dev, chn),
one (module, dev) or one output port. Kernel config form (from ioctl 0x20, or from `GetOutputPortBuf`, the output-buffer
allocation, when the port heap must be re-created): +0x00 type, +0x04 u8 bCreate, then per type —
| type (M6 `MI_SYS_InsidePrivatePoolType_e`) | fields | action |
|---|---|---|
| 0 VPE→VENC ring pool | +0x08 size, +0x0c heap[0x20] | per-dev heap of module 9 (SYS), dev 0, chn 0 |
| 1 per channel | +0x08 module, +0x0c dev, +0x10 chn, +0x14 heap[0x20], +0x34 size | `ConfigDevChnPrivateMMAHeap(mod, dev, chn, heap, size)` |
| 2 per device | same, chn = −1 | same |
| 3 per channel output port | +0x08 module, +0x0c dev, +0x10 chn, +0x14 port, +0x18 heap[0x20], +0x38 size | `ConfigChnPortBufPrivateMMAHeap` (the result hangs at output port +0x15c) |
bCreate with size 0 → `0xA0092003`; other types → `0xA0092003`. Destroy (bCreate 0): refused with `0xA0092012` while any chunk
is in use.
Object (0xa0): +0x00 dev (−1 for a named heap), +0x04 chn (−1 = per device), +0x08 owner tgid, +0x10 chunk manager, +0x58 node
in the parent MMA heap's list, +0x68 u64 bus base, +0x70 granularity (0x1000; 0x2000 for named heaps), +0x74 node in the
per-module list, +0x7c name. A heap name `"<pool>#<name>"` makes a **named** heap (looked up by name, allocated with 8 KiB
slack). Owner names: `"<mod><dev>_priMma"` / `"<mod><dev>_chn<n>_priMma"`.
- `MI_SYS_IMPL_PrivateDevChnHeapAlloc(dev, chn, heapName, owner, size, &addr, flag)` (behind the export and ioctl 0x25): named
  heap → by name; else the (dev, chn) heap, else the per-device one; chunk of size rounded to 4 KiB (top-down if `flag`);
  private heap absent or full → `MI_SYS_IMPL_MmaAlloc_ByChn` (general MMA). When the device has channels the allocation is
  recorded (handle kind 4) with the channel owner's tgid. Free looks the address up in the module's private heaps, then the
  SYS list, else frees to MMA.
- `MI_SYS_IMPL_MmaAlloc_ByChn(heap, owner, size, &addr, tagLo, tagHi)`: `"a#b"` in either name is split (empty owner →
  `"app-privAlloc"`); RAW allocation from the MMA allocators; recorded (kind 3) with the tgid of the caller / the channel
  encoded in the tag; tag < 2 → not recorded. `_MI_SYS_IMPL_MmaAlloc_ByDev` = the same from `imi_mma_heap`.
- On this camera nothing configures a private heap (ioctl 0x20 is not reached, and is ignored under the MMU), so every
  "private" allocation of the modules is a plain MMA allocation [I].

VB pools: not present in this build (no code, no strings); the 2018 source's `mi_sys_vb_pool_impl.c` was removed. Only the
VB-pool allocator collection's deinit remains in Exit (§2.5).

### 7.6 Generic, custom, meta and direct-buffer allocators
- **Generic allocator** (`generic_allocator_init(obj, ops)`): +0x00 atomic refcount, +0x04 magic `'MALC'` 0x4D414C43, +0x08
  list, +0x10 ops = {OnRef, OnUnref, OnRelease, alloc (+0x0c), suit_bufconfig (+0x10)} (all required). `on_ref` / `on_unref`
  count; the last unref calls OnRelease.
- **Allocation** (`generic_allocation_init(obj, ops)`): +0x00 magic `'ALLC'`, +0x04 atomic `ref_cnt`, +0x08 ops = {OnRef,
  OnUnref, OnRelease (+0x10), map_user, unmap_user, vmap_kern (+0x1c), vunmap_kern (+0x20), hook (+0x24)} (first three
  required); +0x28 the `MI_SYS_BufInfo_t`; +0x130 / +0x134 used by mi_sys's quota wrapper (`mi_sys-1-engine.md` §4.3).
- **MMA allocation** = generic allocation + up to 4 plane chunks (multi-plane buffers allocate one chunk per plane), plane VAs at
  +0x158 (user) / +0x148 (kernel), size +0x180, kept in a hash by bus address (`mi_sys_mma_find_allocation_by_pa`).
- **Custom allocator** (`mi_sys_cust_allocator_create(name, &custOps, priv)`): 0x180-byte object, magic `'CUST'`, embedding a
  generic allocator with `g_miCustAllocatorOps`; `custOps` (0x34 bytes) must have `alloc_cust_dma` and `free_cust_dma`; two
  optional function pairs are zeroed unless both members are present; with a name, a proc entry under the allocator proc dir
  (dump / exec callbacks) is created.
- **Meta allocator** (`'META'`, created on first use by `mi_sys_get_global_meta_allocator`): one 32 KiB MMA block `"metaData"`,
  kernel-mapped cached, cut into 128 records of 256 bytes; serves buf type META with size ≤ 0x100 (else NULL); used for META
  outputs (`mi_sys-1-engine.md` §4.3).
- **Direct-buffer wrapper heap** (`mi_sys_directbufwrapper_heap_allocator_create`, a custom allocator named
  `"mi_sys_directbufwrapper_heap"`, magic `'HWAR'`, on the SYS device `g_hSysDrv`): wraps buffers that already exist (buf-conf
  `bDirectBuf`, the `mi_sys_DoDirectBufTask` path of SCL StretchBuf) so they can travel as mi_sys buffers; it keeps one 4 KiB
  `"DirectHeapAlloc"` placeholder allocation and a use count.

### 7.7 MIU protect
<!-- ev:mi_sys-2-interfaces.7.7 -->
mi_sys protects the kernel's `lx_mem*` ranges against writes by media clients and gives the modules write windows; the MIU
protect hardware and the client → group table are `kernel-platform.md` §12.6, §12.2. The MIU-protect file parser is at 0x11db4 (no symbol); the kernel ranges come from the source file
`impl/sys/mi_sys_mma_miu_protect_impl.c` [R string].

**The whitelist file and the switch.** `parse_miuprotect_file` (unnamed `parse_miuprotect_file`, named by its debug-match string;
Ghidra shows it `void` but it returns a value in r0 [R disassembly 0x1ecc / 0x1f02]):
- path = `default_config_path` unless it still holds the placeholder `"ABABABABABABABABABAB"`, in which case `"/config/"`; then
  `"miuprotect.txt"` is appended with no separator. With demo.sh's parameters the path is `/config/miuprotect.txt`.
- 100-byte buffer (`CamOsMemAlloc`; failure → log, return `0xA009201F`); `filp_open(path, O_RDONLY, 0644)`; failure → log
  "read miuprotect.txt failed! The miu whitelist will use default values." (only at debug level > 4) and return
  `0xA009201F`.
- `vfs_read(file, buf, 100, &file->f_pos)` under `set_fs(KERNEL_DS)`, `filp_close`. A return of 0 → log "read miuprotect.txt
  failed!" (debug level ≠ 0), return `0xA009201F`. Any non-zero return (a byte count, or a negative error — the test is `!= 0`
  [R 0x1e94]) → parse: `strsep(&p, ",")`, `CamOsStrtoul(tok, &end, 0)`, the low 16 bits stored to a static `u16[16]` (`.bss`
  `.LANCHOR1` of that unit), up to 16 tokens; return **0**.
- The buffer pointer handed to `CamOsMemRelease` on success is the `strsep` cursor, not the allocation: NULL when the text ran
  out before 16 tokens, a pointer inside the buffer when the 16th token was taken with text left [R 0x1ec6 / 0x1ef8]. So the
  100-byte buffer is never freed on success, and in the 16-token case an interior pointer is released [I: `CamOsMemRelease`
  is the CamOs `kfree` wrapper].

`MI_SYS_IMPL_Init`, right after `MI_SYSCFG_SetupMmapLoader` [R disassembly 0x7fb2–0x8244]:
| `parse_miuprotect_file` | `g_kernelProtectClientId` | `g_kprotectEnabled` (`.data+0x184`, initial 1) |
|---|---|---|
| 0 (file read) | the parsed `u16[16]` list | **0** |
| ≠ 0 (no file, empty file, no memory) | `MDrv_MIU_GetDefaultClientID_KernelProtect()` | 1 |
`g_kprotectEnabled` is written only here (the two stores at 0x7fd4 / 0x8236 are its only writes). Then, only if
`bEnableMiuProtect` is non-zero, `init_glob_miu_kranges()` is called; the INFO "end" log follows at debug level > 4. So
`bEnableMiuProtect` gates the call, and **the presence of `/config/miuprotect.txt` switches the kernel-range protection off**.

**`init_glob_miu_kranges`** (line 0x331 of its source file): `g_kprotectEnabled == 0` → `MI_SYS_LOG_IMPL_PrintLog(WRN
"Miuprotect LX memory disable!!! Please remove miuprotect.txt in your partition!!!")` when the debug level is > 1 or the debug
function / file filter matches (with the default `debug_level` 2 the line is printed), and return **without initialising
anything**: the per-MIU range lists, their counters and the three mutexes (`.bss+0x1ca4`, stride 0xb8) stay zero and no
`lx_mem*` range is protected. Otherwise it builds the lists and protects each `lx_mem`, `lx_mem2`, `lx_mem3` range per MIU with a
protect block (`MDrv_MIU_Protect(block, whitelist, start, end, enable)`, whitelist = `g_kernelProtectClientId`). Its pair
`exit_glob_miu_kranges` (called unconditionally from `MI_SYS_IMPL_Exit`) prints the shorter WRN "Miuprotect LX memory disable !"
(line 0x3a6, same debug condition) and returns when `g_kprotectEnabled == 0`; else it empties and destroys the three range lists.

**Range maintenance.** The MMA heaps are cut out of the protected ranges (`deleteKRange` when a heap is created, `addKRange`
when it is freed); `_miu_kernel_protect`, `addKRange`, `deleteKRange` and `mi_sys_ResumeMiuProtect` (re-applies after resume)
all test `g_kprotectEnabled` first and do nothing when it is 0 — so the client ids written in the file are stored but never
reach `MDrv_MIU_Protect`; the parsed list is dead data. The message text matches the code: the file is a switch that disables
LX protection [I: the vendor's intent was a custom whitelist and the `== 0` branch looks inverted; the binary does what is
described].

**Module windows.** `mi_sys_SetMiuProtect` (isp, scl; own comma-separated id list) and `mi_sys_setMiuProtectMMa` (ipu, vdec,
venc) add write windows for modules. `mi_sys_SetMiuProtect` does not test `g_kprotectEnabled`; with the file present it locks
a mutex `init_glob_miu_kranges` never initialised (`.bss+0x1ca4 + 0xb8·miu`) [R; I: works only because a zeroed CamOs mutex
happens to be usable, not checked].

**On this camera.** The `/config` dump has no `miuprotect.txt` [R: find], so the
parse fails, protection is on with the driver's default list, and the message is not printed [I: the running `/config` is the
dumped one]. The list shown by `mi_global_info` as "KernelProtect IP white list" is `MDrv_MIU_GetClientID_KernelProtect(miu)` —
the MIU driver's own list, not the file's: CPU_W 112, MCU51_RW 15, USB20_H_RW 18, USB20_RW 17, USB30_RW 16, EMAC_RW 23,
EMAC1_RW 24, SD30_RW 19, SDIO30_RW 20, AESDMA_RW 4, URDMA_RW 7, BDMA_RW 2, MOVDMA0_RW 3, SATA_RW 22 (procfs); the "MmuProtect IP
white list" is empty. The MMU heap's physical range is additionally locked to VPA-only access (§2.3 step 6).

### 7.8 BDMA operations
`MemsetPa`, `MemcpyPa`, `BufFillPa`, `BufBlitPa` (ioctls 0x21–0x24, §3.2, and the exports of §6.1) use the kernel built-ins
`msys_dma_fill`, `msys_dma_copy`, `msys_dma_blit`.

### 7.9 CMDQ buffer
`MI_SYSCFG_InitCmdqMmapInfo` (§2.3 step 7): MMA allocation `CMDMEM` of `cmdQBufSize` KiB (3 MiB with demo.sh; at MIU
0x6018b000 in the boot log, an MMU address), kernel map, zero, `MHAL_CMDQ_InitCmdqMmapInfo`; freed by
`MI_SYSCFG_ExitCmdqMmapInfo` (§2.5). The CMDQ objects mi_sys takes per device pass (`MHAL_CMDQ_GetSysCmdqService`, the service
table, fences, reset) are `mi_sys-1-engine.md` §7.

## 8. HW timer (`mi_sys_RegisterHwTimer` …) [R]
Users: mi_vdec (player), mi_ai, user pictures (ioctl 0x2a).  It is the only interrupt mi_sys
requests.
- **Context** (0x1a0, allocated on the first registration): +0x00 u8 IRQ installed, +0x08 u64 register base, +0x10 IRQ
  number, +0x14 u8 timer running, +0x1c + 0x18·i **16 slots** {+0 device handle, +4 callback, +8 u8 used, +9 u8 enabled, +0xc
  tick count, +0x10 period (ticks), +0x14 callback argument}. One spinlock (`g_miSysspinlock_hw_timer`) and one semaphore.
- **Init** (`_MI_SYS_IMPL_InitHwTimer`): DT node `compatible = "sstar,timer"` → platform device → first valid memory resource
  (register base) and its interrupt (`CamIrqOfParseAndMap`) → `request_threaded_irq(irq, _MI_SYS_HwTimerIrqHandler, NULL,
  0x80, "sys-hwtimer-handler", ctx)`. Failures return the raw code 0x1F.
- `mi_sys_RegisterHwTimer(h, cb, arg, periodTicks, &slot)`: the same (h, cb, arg) twice → raw 0x12; no free slot → raw 0x1F;
  context allocation failure → raw 0xC (these three are plain numbers, not `0xA009xxxx`). Fills a slot (used = 1, disabled).
- `mi_sys_EnableTimer(h, slot)`: slot enabled; if the timer is not running: registers (VA = phys − 0x2200_0000, i.e. the
  static RIU map) +0x0C ← 0, +0x08 ← 12000, +0x18 rewritten, +0x00 |= 0x101 (enable + interrupt) → **1 ms tick** from a 12 MHz
  clock [I: 12000 counts per interrupt]; running = 1. So `periodTicks` is in milliseconds (user pictures pass 1000/fps).
- `mi_sys_DisableTimer(h, slot)`: slot disabled; when no slot is enabled any more: +0x00 |= 2 (stop), running = 0.
- `mi_sys_ModTimer(h, slot, period)`: new period. `mi_sys_UnRegisterHwTimer(h, slot)`: clears the slot; when no slot is used:
  `free_irq`, frees the context (the next registration re-initialises). A slot belonging to another handle or not in use →
  BUG.
- **IRQ** (`_MI_SYS_HwTimerIrqHandler`): for each used and enabled slot, count + 1; when count reaches a non-zero period, count
  = 0 and `cb(arg)` is called inside the spinlock (interrupt context). Period 0 never fires.

## 9. MI log and log-to-disk [R]

- **Module table** (`_MI_SYS_LOG_IMPL_ConfigInfoInit`): MMA block `"sys-logConfig"` of 0x5a0 bytes, kernel-mapped, 36 entries ×
  0x28 {name[0x1c], u32 level (+0x1c, default 2), u32 (+0x20)}; entry *i* is named after module id *i*'s prefix in lower case
  ("modulemax" for unused), its level read from `/sys/module/<name>/parameters/debug_level` (first character − '0'; missing →
  2). `MI_SYS_LOG_IMPL_LogAddModule(name, id)` / `LogRemoveModule` rename an entry. The byte after the table (`[0x24]` of the
  mapping) mirrors the output mode.
- **Ring buffer** (`_MI_SYS_LOG_IMPL_LogBufferInit`, only if `logBufSize` > 0): MMA `"sys-logBuffer"` of `logBufSize` KiB
  rounded to 4 KiB, at most 1 MiB (demo.sh: 256 KiB), uncached kernel mapping; ring {VA, size, write, read, …, u64 total
  written}; a writer that would overrun pushes the read index forward (the oldest text is lost), readers are woken
  (`wqNoData`).
- **`MI_SYS_LOG_IMPL_PrintLog(fmt, …)`** (every module's log macro ends here, behind the filter of §1.6): formats into 256
  bytes (overflow → ends with `"...\n"`); if the log system is not initialised or the output mode is 0 → `MI_COMMON_Print`
  (printk); mode 1 → appended to the ring buffer.
- **Output mode / log path**: initially mode 0 and an empty log path, so everything goes to printk. Writing `log=<dir>` to
  `/proc/mi_modules/mi_log_info` sets the path and mode 1 (`log=/dev/null` → mode 0); `storepath=<dir>` sets the directory for
  debug dumps (default `"/mnt"`); `<module>=<level>` or `all=<level>` sets levels (written back to the module's `debug_level`
  sysfs parameter); `help` prints usage (still mentioning HiSilicon's `/proc/msp/log`). Reading the file shows path, store path
  and every module's level with its name (`g_debugLevelName`).
- **Log-to-disk thread** `"mi_log"` (`LogUdiskWriteThread`, priority 1, stack 0x800, started at init when the buffer exists):
  in mode 0 it sleeps 10 ms and re-checks — **a kernel thread polls every 10 ms forever** on this camera; in mode 1 it waits
  (10-jiffy slices) for text, reads up to 96 KiB from the ring and appends it to `<log path>/mi.log`
  (`O_WRONLY|O_CREAT|O_APPEND|O_LARGEFILE`, 0644).

## 10. procfs [R]

### 10.1 What exists and where
`_MI_SYS_IMPL_Drv_Proc_Init` creates the debug views `mi_global_info`, `mi_bufqueue_status`, `mi_log_info`,
`mi_graph_contrl`, `mi_infer_graph.py`, `mi_dump_buffer_delay_worker` directly under `/proc/mi_modules/` (the camera's procfs dumps show
`/proc/mi_modules/mi_bufqueue_status`, `…/mi_graph_contrl`, `…/mi_global_info`), and `miu_protect` / `mem_stat` under
`/proc/mi_modules/mi_sys_mma/`.
| path | mode | read | write |
|---|---|---|---|
| `/proc/Sigmastar/` | dir | — (created; nothing in mi_sys puts files in it) | — |
| `/proc/mi_modules/common/` | dir | §10.2 | §10.2 |
| `/proc/mi_modules/mi_global_info` | 0444 | `miu_and_lx_info` (MIU bus bases, `lx_mem*` address / size), KernelProtect and MmuProtect whitelists (16 entries each, `clientId name`, §7.7), PAGE_OFFSET / TASK_SIZE / VMALLOC range, version string | — |
| `/proc/mi_modules/mi_log_info` | 0666 | §9 | §9 (`log=`, `storepath=`, `<module>=<level>`, `all=`, `help`) |
| `/proc/mi_modules/mi_infer_graph.py` | 0444 | a Python script describing the pass graph (devices, passes, binds with colours / labels by bind type; `_MI_SYS_IMPL_GeneralGraphPythonScript`; `mi_sys-1-engine.md` §8) | — |
| `/proc/mi_modules/mi_graph_contrl` | 0666 | `dump bind on` / `dump bind off` | exactly `"dump bind on\n"` / `"dump bind off\n"` toggles the bind-dump flag used by the graph code; anything else → −EINVAL |
| `/proc/mi_modules/mi_bufqueue_status` | 0666 | per enabled input port: module, dev, chn, pass, port, `UsrInjectQ_cnt`, `BindInQ_cnt`, pending bytes; per enabled output port: `DrvBkRefFifoQ` count / size, `UsrGetFifoQ` count | accepted, ignored [I: the handler is 68 bytes] |
| `/proc/mi_modules/mi_dump_buffer_delay_worker` | 0444 (root may write) | state of running `dump_buffer` jobs | `force_stop_dump <id>` (decimal or 0x-hex) — the id is the **kernel address of the job record**, and the handler writes 1 to byte id + 0x3c without validating it: a root-only arbitrary one-byte kernel write |
| `/proc/mi_modules/mi_sys_mma/miu_protect` | 0444 (root may write) | `kprotect_status`: per MIU the protect blocks and ranges | `miu_block_id start end` style command (usage text printed on error) → `MDrv_MIU_Protect` |
| `/proc/mi_modules/mi_sys_mma/mem_stat` | 0444 | `stat_memory_usage`: per heap and per module / device / channel current and peak usage from the chunk tags (§7.2) | — |
| `/proc/mi_modules/mi_sys_mma/<heap>` (one per MMA heap, plus the meta / custom / direct-wrapper allocators) | | allocator dump (chunk list: offset, length, used, thread, owner) | `help`, or `<dir> <offset> <length>` → dumps that range of the heap into `<dir>/mma__<heap>__<offset>__<length>.bin` |
| `/proc/mi_modules/mi_sys/{debug_level, debug_file, debug_func, module_version_file}` | | mi_common standard files (§1.4, §1.6) | |
| `/proc/mi_modules/mi_sys/reserve_2_free` | 0640 | a value | `mmaheap2free` — only when **no MI device is registered**: every boot MMA reservation is returned to the kernel page allocator (`free_reserved_area`), pages freed reported; else lists the devices and fails (`0xA009201E`); `logo2free <name>` — frees the `mmap_reserved_config` entry `<name>` (e.g. the boot-logo frame buffer); fbdev calls the same through `mi_sys_FreeMMAPReservedArea` |
| `/proc/mi_modules/mi_sys/mi_sys0` | | §10.4 | §10.3, §10.4 |
| `/proc/mi_modules/mi_<mod>/mi_<mod><dev>` (every registered device) | | §10.3 | §10.3 |

### 10.2 `/proc/mi_modules/common/*` — kernel side of the config_tool contract
Created by `MI_COMMON_SetupDebugFile` (seq files with a token parser: the write is split on white space into ≤ 32 tokens and
handed to the set callback as `(print, ctx, argv, argc)`) or `MI_COMMON_SetupDebugRawFile` (raw: the set / get callbacks
receive the user buffer and the file position). config_tool writes `mmap_info` by seeking (`pwrite` at the item index);
`config_info` and the PQ files are written with plain `write` at position 0, and the raw handlers never advance the position.
The meaning of the records' fields is `config_tool.md` §2.2, §5.2, §6.3.
| file | kind | write | read |
|---|---|---|---|
| `memory_info` | tokens | exactly 7 numbers (`kstrtouint`, base 0): `u32TotalSize u32Miu0Size u32Miu1Size u32MiuBoundary u32MmapItemsNum bIs4kAlign bMiu1Enable`; else −EINVAL | the same 7 values, labelled; −EINVAL-like (−126) while `u32MmapItemsNum` = 0 |
| `mmap_info` | raw, record = file position | at position *i* < `u32MmapItemsNum`: text `"%s %u %u %u %u %u %u %u %u"` = name, gid, addr, size, layer, align, memoryType, miuNo, cmaId → item *i* (0x18 bytes: name pointer (kstrdup), u8 gid, u8 layer, u8 miuNo, u8 cmaId, u32 addr, size, align, memoryType; at most 24 items); position ≥ count → error log | item *i* as `"<name>:GID=%d,Addr=0x%x,Size=0x%x,Layer=%d,Align=0x%x,MemoryType=0x%x,MiuNo=%d,CMAID=%d\n"` |
| `config_info` | raw, binary | bytes copied into `g_stConfig` at the file position; after each write every panel record (0x170 bytes, records start at `g_stConfig` + 8, count at `g_stConfig` + 4) has its three embedded **user-space pointers** (record +0xf4 with u32 count at +0xf8, +0xfc with count at +0x100 — arrays of u32 —, +0x140 with byte length at +0x144) deep-copied from the writer's memory (`CamOsCopyFromUpperLayer`) and replaced by kernel copies; byte 0 of `g_stConfig` = DLC table count | bytes from `g_stConfig` at the position |
| `pq_info` | raw | at least 0x24 bytes (shorter → −EINVAL; of a longer write the first 0x24 are used): the PQ header (table dimensions and sizes) | the same 0x24 bytes |
| `noise_table`, `motion_table`, `motion_hdmi_dtv_table`, `motion_comp_pc_table`, `misc_table`, `misc_luma_table`, `misc_param_table`, `pq_table` | raw | the whole table in one write whose length must equal the size given by `pq_info` (e.g. noise: two dimension bytes multiplied; `pq_table`: the u32 at `pq_info` + 0x20); kmemdup'ed | the stored table if the requested length matches, else −EINVAL |
Readers in the kernel: the getters of §2.6.

### 10.3 The per-device file `mi_<mod><dev>`
<!-- ev:mi_sys-2-interfaces.10.3 -->
Every module that registers with mi_sys gets `/proc/mi_modules/mi_<mod>/mi_<mod><dev>` through `mi_sys_RegisterDev` +
`mi_sys_RegistCommand`. Read: the module's procfs ops (`OnDumpDevAttr`, `OnDumpChannelAttr`, `OnDumpInputPortAttr`,
`OnDumpOutPortAttr`, `OnHelp`) after mi_sys's own per-device part. Write: `echo <cmd> > …`, the common commands of every device
— `help`, `dump_buffer <chn> iport|oport <port> UsrInject|BindInput|UsrGetFifo <dir> bufnum=N|time=ms|start/end`,
`reset_counter [cmdq] [passid]` — plus the module's registered commands (each module document lists its own; the `help`
output of `mi_scl0` (e.g. `irqmode [source IrqMode]`) and `mi_disp0` is in `mi-capture.md` §6.11 and `mi_disp-mi_hdmi.md` §4.13; `echo help` to `mi_isp0` / `mi_vif0`
printed nothing to dmesg [R]).

### 10.4 `/proc/mi_modules/mi_sys/mi_sys0`
<!-- ev:mi_sys-2-interfaces.10.4 -->
The file `mi_sys0` belongs to mi_sys's own device SYS/0 (§2.3 step 8; also the device used by user pictures and the
direct-buffer heap).
- read: `MI_SYS_IMPL_DumpAllModuleInfo` — mi_sys common info, CMDQ kick-off counters and buffer usage per CMDQ, then for every
  registered device a counter table per channel (`pre` = OnPreProcess results ok / busy / drop, `Enq` = EnqueueInputTask
  results run / done / retry / drop, `bar`, `checkin` / `checkout` = Check{Input,Output}TaskStatus results) — the format seen
  on the camera [R camera].
- write: the common commands of §10.3 and **ten mi_sys-only commands** (table at `.rodata+0x4fb4`, {name, max args,
  handler}). `help` lists only `dump_buffer` and `reset_counter` (as read on the camera); the ten are not
  listed but exist:
  | command | args | effect |
  |---|---|---|
  | `debug_ring` | 1/0 | CMDQ debug ring mode |
  | `debug_frc` | `on <mod> <dev> <chn> <pass> <port>` / off | FRC debug for one port (the "Total Used Buffer >= Quota", "Prev OutputPort framectrl" logs, `mi_sys-1-engine.md` §4.4) |
  | `show_sram` | — | SRAM state per device (`mi_sys-1-engine.md` §9) |
  | `show_threads` | — | worker threads |
  | `set_ouputport_depth` | `<mod> <dev> <chn> <pass> <port> <user> <queue>` | `mi_sys-1-engine.md` §4.7 |
  | `set_thread_priority` | `<mod> <dev> <prio>` | worker thread priority |
  | `debug_mmu` | `addsize_before\|addsize_after <size>`, `delay_unmap`, `debug_log` … | MMU guard sizes, delayed unmap, MMU logging (§7.3) |
  | `debug_mem_peak` | `<…>` | recount memory peaks (`MI_SYS_ReCountMemPeak`) |
  | `enable_backtrace` | 1/0 | stack dumps in error paths |
  | `set_inputport_pattern` | `<mod> <dev> <chn> <pass> <port> <bEnable>` | input-port pattern mode (debug) |
- Outside the debug command, a depth is set only through `MI_SYS_SetChnOutputPortDepth` (ioctl 0x15) from the application;
  a call from a second process is not documented [I]; made from a second process on the camera (SCL port 0, queue 5 → 4 → 3)
  it takes effect at once [R camera].

## 11. Dependencies [R]
- mhal: only `MHAL_CMDQ_{InitCmdqMmapInfo, DeinitCmdqMmapInfo, GetCmdqHWNumber, GetFreeId, GetSpecificId, ReleaseId,
  GetSysCmdqService, ReleaseSysCmdqService}`.
- kernel built-ins: `MDrv_MIU_*` (init, protect, client ids, VPA access), `MDrv_MMU_*` (the MMU path, §7.3),
  `msys_dma_{copy, fill, blit}`, `msys_read_uuid`, `Chip_*` (platform name, cache flush, RIU base), the boot memory layout
  (`mma_config`, `mstar_driver_boot_mma_buffer_num`, `mmap_reserved_config`, `lx_mem*`), CamOs (threads, timers, IDR, memory,
  user mapping), `recode_timestamp`, `CamIrqOfParseAndMap` + `request_threaded_irq` (HW timer), `call_usermodehelper`,
  `filp_open` / `vfs_read`.
- mi_common: `MI_DEVICE_*` and the proc ops (§1.4).
