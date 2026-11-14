# config_tool — `/config/config_tool` and the mmap.ini / config.ini / PQConfig.ini contract
<!-- ev:config_tool -->

`config_tool` is a SigmaStar user-space helper that `mi_sys.ko` runs twice during its insmod. It turns text files into the
kernel tables of `/proc/mi_modules/common/` and exits: `load_mmap` parses `/config/mmap.ini` (a C header of `#define`s) into
`memory_info` (one text line, 7 numbers) and `mmap_info` (one text line per memory item, addressed by file position = item
index); `load_config` (0x1381a) parses `/misc/config.ini` (iniparser grammar) into one 0x8fc8-byte binary struct written to
`config_info` (a list of display timings / panels, 0x170 bytes each), and `/misc/PQConfig.ini` into `pq_info` + `pq_table`
(a header and one opaque PQ blob) [R]. On this camera the kernel keeps 4 mmap items, of which only `E_MMAP_ID_EMI` has a
consumer (mi_venc, one path) and `E_LX_MEM` a sanity check; 60 timing / panel records, read by mi_disp (the HDMI / VGA
timing table handed to mhal DISP; the BT656 / BT1120 set-up through mhal PNL) and mi_panel; nothing for PQ (PQConfig.ini is absent, all PQ files stay empty);
the six noise / motion / misc tables are never written by this config_tool and have no kernel consumer [R].

This document owns the mmap.ini grammar and items, the config.ini / PQConfig.ini grammar and keys (U-Boot's `[LOGO]` keys
included), the meaning of the `/proc/mi_modules/common/*` records (the mmap tables, the 0x170-byte panel record, the PQ
header) and the config.ini timing ids. Elsewhere: the creation of the procfs files, their token / raw kinds and the
`MI_SYSCFG_*` getters — `mi_sys-2-interfaces.md` §2.6, §10.2; mi_sys's init sequence around the two helper calls and its
module parameters — `mi_sys-2-interfaces.md` §2.2, §2.3; the MMU page mode that reads `memory_info` —
`mi_sys-2-interfaces.md` §7.3; the physical memory map — `kernel-platform.md` §11; where `config.ini` and `PQ.bin` sit in the
MISC firmwarefs — `misc-firmwarefs.md` §3; U-Boot's `bootlogo` reading of `config.ini` — `uboot.md` §5.6; the `PQ.bin` format
— `uboot.md` §6.2; mi_disp's mhal-timing → SYSCFG-id switch and its 0x74-byte panel configs — `mi_disp-mi_hdmi.md` §4.3; mhal's
use of them — `mhal-disp.md` §7.1; mi_panel, mhal PNL, `board.ini` and `model/Customer.ini` — `panel-mipitx.md` §2, §3, §4, §9; `fbdev.ini` —
`rgn-gop.md` §5.2; mi_venc's EMI buffer — `mi_venc.md` §7.4; the vendor file inventory — `firmware-image.md` §3.

**Tags.** [R] read from a binary, its disassembly or decompile, a file of the dump, the NOR image or a live capture; [S]
stated by a source (a reference source text, a header); [I] inference. A heading tagged [R] covers every statement under it
unless marked otherwise.

## 1. The binary and how it is run

### 1.1 File [R]
ELF32 ARM EABI5 (Thumb-2, unoptimised `-O0`-style code with frame pointer), dynamically linked against glibc only
(`libc.so.6`, `libgcc_s.so.1`), stripped; file 48.5 KB, `.text` 0x640c (25.6 KB), `.rodata` 0x156f, `.data` 0x2b28 (one
table, §5.3), `.bss` 0x2d5fc (≈182 KB of static work buffers). Source file names left in the strings: `src/mmapparser.c`,
`src/mmapinfo.c` (MStar "Supernova" MMAP parser: "Please increase MAX_ITEM_COUNT size in
Supernova/core/MMAPInfo/inc/MMAPInfoDataType.h"), an embedded iniparser (strings `iniparser: …`), and the SigmaStar config
loader (function names in log strings: `LoadIniFile`, `ParserStrArry` (0x12344), `LoadPanelInfo` (0x12654), `SetPanelInfo` (0x13644), `LoadPQConfig` (0x1387a),
`DumpPQConfig`). 25 functions; the decompile is complete (no unresolved switches). `/config/dump_config` and
`/config/dump_mmap` are symlinks to `config_tool`. No user-space program reads any of the files config_tool writes: no hit
for `mmap.ini`, `E_MMAP_ID`, `config_info`, `PQConfig` in `/customer`, the rootfs or `/config/lib`.

### 1.2 Dispatch on `argv[0]` [R `main` (0x144c0) = main]
<!-- ev:config_tool.1.2 -->
main first sets `RLIMIT_CORE` to unlimited, then tests `argv[0]` with `strstr` — **substring, and all four tests run in
sequence** (a name containing two keywords runs both). `load_mmap` / `load_config` are names in argv, not files.
| argv[0] contains | action | arguments used |
|---|---|---|
| `dump_mmap` | read `memory_info` (one `read` of ≤ 0x400 bytes, printed with `printf(buf)` — the kernel text is used as a format string), then `pread(fd, buf, 0x400, i)` on `mmap_info` for i = 0, 1, … printing each, until a read returns ≤ 0; returns −1 ("read mmap_info failed!") unless `errno` is 126 (`ENOKEY`, the kernel's end marker) | none |
| `load_mmap` | §2 | `argv[1]` = mmap.ini path |
| `dump_config` | §6.4 | none |
| `load_config` | §5 then §6 | `argv[1]` = config.ini path, `argv[2]` = PQConfig.ini path |
main returns 0 except the error returns of the mmap paths (−1). Everything is printed to stdout; when mi_sys runs the tool
as a usermode helper stdout is not connected to the console, so none of its messages appear in the boot log
(has the kernel's lines around the calls, no config_tool line).

### 1.3 How mi_sys runs it [R `MI_SYSCFG_SetupMmapLoader`, `MI_SYSCFG_SetupIniLoader`, strings, boot log]
Both calls are `call_usermodehelper(path, argv, envp, UMH_WAIT_PROC)` from `MI_SYS_IMPL_Init` (`mi_sys-2-interfaces.md`
§2.3, steps 2 and 4), with `envp = {HOME=/, TERM=linux, PATH=/sbin:/usr/sbin:/bin:/usr/bin, LD_LIBRARY_PATH=/lib:/config/lib}`.
Module parameters: `default_config_path` (default placeholder `ABABABABABABABABABAB` → `/config`), `config_ini_path`
(default `/misc/config.ini`), `config_pq_path` (default `/misc/PQConfig.ini`).
| call | path | argv | before the call the kernel… |
|---|---|---|---|
| 1 | `<dir>/config_tool` | `<dir>/load_mmap`, `<dir>/mmap.ini` | creates `mmap_info` (raw) and `memory_info` (token file), zeroes the item table (24 × 0x18) and the memory-info record, prints `MI_SYSCFG_SetupMmapLoader default_config_path:%s, argv1:%s,argv2:%s` |
| 2 | `<dir>/config_tool` | `<dir>/load_config`, `config_ini_path`, `config_pq_path` (each replaced by `/config/model/config.ini` / `/config/model/PQConfig.ini` if the parameter equals the placeholder) | creates the ten raw files `config_info`, `pq_info`, `noise_table`, `motion_table`, `motion_hdmi_dtv_table`, `motion_comp_pc_table`, `misc_table`, `misc_luma_table`, `misc_param_table`, `pq_table`; prints `config...... cmdpath:%s, argv%d:%s` for argv0–argv3 (argv3 is the NULL terminator → `(null)` in the log) |
`<dir>` = `default_config_path`; the boot log prints `default_config_path:/config/config_tool`, which is the composed
helper path, not the parameter. So on the camera the calls are `{/config/load_mmap, /config/mmap.ini}` and
`{/config/load_config, /misc/config.ini, /misc/PQConfig.ini}`: the mmap.ini location follows `default_config_path`, while
the config.ini / PQConfig.ini locations are separate parameters. The load_config paths are cut at 0x31 bytes; the
load_mmap paths are built with unbounded `memcpy` into ~0x32-byte stack buffers (a long `default_config_path` overruns
them).

Because the helper runs at mi_sys insmod time, `/config` and `/misc` (the MISC firmwarefs served by `/bin/fwfs`) are
mounted before `mi_sys.ko` is loaded; the rcS order guarantees it [R boot log, parameter list]. mi_sys ignores the
helper's result (the return value is unused): with the helper missing or failing the tables stay empty. After
call 2 mi_sys compares the `E_LX_MEM` size from `mmap_info` with a value from the kernel command line and only prints a
warning on mismatch, `Get LX_MEM fail in mmap ....` when the item is missing. The tables are read by mi_disp at its init
(first open of `/dev/mi_disp`) and by mi_venc on its EMI path (§2.5).

### 1.4 The procfs files it writes [R `commonraw_proc_read/write`, `MI_COMMON_SetupDebug*File`]
The files (`/proc/mi_modules/common/memory_info`, `mmap_info`, `config_info`, `pq_info`, the tables) and their creation
are `mi_sys-2-interfaces.md` §10.2. Two kinds:
- **token file** (`memory_info` only): a write is split on white space into at most 32 tokens and passed to the set
  callback as `(argv, argc)`; a read prints the show callback through `seq_printf` (normal text file).
- **raw file** (all others): `write(fd, buf, n)` copies the whole user buffer into a kmalloc'ed (vmalloc if > 4 KiB and
  kmalloc fails) kernel buffer and calls `set(kbuf, pos64, n)`; `read(fd, buf, n)` allocates n bytes, calls
  `get(kbuf, pos64, n)` and copies **as many bytes as the callback returned** to the user. Properties:
  1. **The file position is never advanced** by either handler. Every `read` / `write` sees position 0 unless the caller
     uses `pread` / `pwrite` / `lseek`. config_tool therefore uses `pwrite(…, i)` for mmap item *i*, while `config_info`
     and the PQ files are written with plain `write` at position 0; `cat config_info` repeats the first 4 KiB forever
     (the live capture shows exactly that, §5.6).
  2. A write delivers the whole record in one call: the callback's return value goes back to the writer unchanged
     (logged only with mi debug on when it differs from n); a short or failed write followed by another `write` lands at
     position 0 again, not after the first part. Empty writes or a NULL buffer → −EINVAL with a `dump_stack`; a failed
     `copy_from_user` → −EFAULT.
- The read path copies the callback's return value worth of bytes from a buffer that is **not zeroed**: callbacks that
  return the requested length but fill less (`pq_info`) leak kernel heap to the reader (the 2026-10-16 capture of `pq_info`
  shows network packet bytes after the 0x24-byte header). `config_info` reads have no bound against the struct size
  (pos + n beyond 0x8fc8 reads adjacent kernel `.bss`) [R `MI_SYSCFG_ConfigGet`].

## 2. `load_mmap`: mmap.ini → `memory_info` + `mmap_info`

### 2.1 mmap.ini grammar [R `MMAPFileParser` (0x11024), `mmap_parse_line` (0x114d4) line parser]
mmap.ini is a C header produced by SigmaStar's "SN SCA" tool (`SCA_TOOL_VERSION "SN SCA V3.0.2 "`); config_tool reads only
`#define` lines:
1. `fopen` fails → the tool prints two warnings and `load_mmap` returns −1 **without writing anything** (the kernel tables
   stay empty; every `GetMmapInfo` then fails).
2. Pass 1 reads the file in `fgets` chunks of at most 127 bytes and looks for the substring `#define`. If no chunk has one
   the file is taken as encrypted ("Need Call mapi_secure::DoMmapFileDecryption") and the tool **aborts** (`assert(0)`).
3. Pass 2: every 127-byte chunk that contains `#define` **anywhere** (also inside a `//` comment) is parsed:
   - after `#define`: skip blanks / tabs; NAME = up to the first of space, tab, `(`, `)`; skip blanks → VALUE. An empty NAME
     or VALUE skips the line.
   - VALUE without parentheses: the token up to the first space, tab or `/`. If the rest of the line contains `Alignment`,
     the first token of the rest split on the character set `/Alignmet ` is kept as ALIGNTEXT (so `//Alignment 0x01000` →
     `0x01000`).
   - VALUE with `(`: the text between the first `(` and the first `)` (`(MIU0 | TYPE_NONE | UNCACHE | TYPE_NONE)` →
     `MIU0 | TYPE_NONE | UNCACHE | TYPE_NONE`). A `(` without `)` (or `)` alone) dereferences NULL → crash.
4. Each parsed NAME is stored into a symbol table {char name[100]; u32 value} of at most 810 entries (overflow →
   "MAX_ITEM_COUNT size = 810. It is not enough!!" and abort). How the value is computed depends on NAME:
| NAME | value stored |
|---|---|
| `MIU_DRAM_LEN`, `MIU_DRAM_LEN0/1/2`, `ENABLE_MIU_1/2`, `MIU_INTERVAL`, `MIU_INTERVAL2`, `MIU0/1/2`, `MIU0/1/2_END_ADR`, `MMAP_COUNT`, `TEST_4K_ALIGN` (exact match) | `strtoul(VALUE, 0)` (C base prefix rules: `0x…` hex, leading `0` octal; 32-bit, larger values saturate) |
| `TYPE_NONE`, `UNCACHE`, `CACHE`, `REMAPPING_TO_USER_SPACE`, `CMA` (exact) | "shift expression": `sscanf("%x %s %x")` → `a >> b` if the operator is `>>`, `a << b` if `<<`, else 0 with "Error, not define case" (`%x`: both numbers are read as hex) |
| contains `GROUP_SELMIU` | VALUE split on `:`; entry *k* is stored as NAME + one decimal digit of *k*, value `strtoul(part, 16)` |
| ends in `_LAYER`, `_CMA_HID` | `strtoul(VALUE, 0)` |
| ends in `_ADR` | two entries: NAME with `_ADR` replaced by `_ALIGN` = `strtoul(ALIGNTEXT)`, and NAME = `strtoul(VALUE)`. **A `_ADR` line without an `//Alignment` comment calls `strtoul(NULL)` and crashes** |
| ends in `_LEN` | NAME = `strtoul(VALUE)`; and the module name (NAME minus `_LEN`, if shorter than 100) is appended to the module list (≤ 100 names; the check uses the symbol-table bound, so more than 100 `_LEN` lines overflow a stack array) |
| ends in `_MEMORY_TYPE` | VALUE split on spaces and `\|`, each word looked up in the symbol table and OR-ed; an unknown word prints "Error, not search in table" and contributes 0 |
| anything else (`*_AVAILABLE`, `*_GAP_CHK`, `CPU_ALIGN`, `MIU*_GROUP_PRIORITY`, `NONCACHE_BUFFERABLE`, `SCA_TOOL_VERSION`, …) | not stored |
Lookups are exact (length + bytes). Lines without `#define` (comments, `/* CHK_VALUE = … */`) are ignored. Since
`NONCACHE_BUFFERABLE` is never stored, a `_MEMORY_TYPE` that names it gets 0 for that word.

### 2.2 Building the tables [R `ParserMmap` (0x15d56), `mmap_build_tables` (0x16ff6)]
A second walk over the symbol table, in file order:
- globals: `MIU_DRAM_LEN` → TotalSize, `MIU_DRAM_LEN0` → Miu0Size, `MIU_DRAM_LEN1` → Miu1Size, `MIU_INTERVAL` → MiuBoundary
  (and the address offset of MIU1 items), `MIU_INTERVAL2` (offset of MIU2 items), `TEST_4K_ALIGN` → bIs4kAlign (u8),
  `MIU1` | `MIU2` values → the MIU mask. `MIU_DRAM_LEN2`, `ENABLE_MIU_1/2`, `MIU0`, `MIUx_END_ADR`, `MMAP_COUNT`, `CMA` and
  the GROUP_SELMIU bit table (`table[miu][group][bit]`, miu ≤ 2, group digit ≤ 6, ≤ 16 bits — violations abort) are stored
  but **never output** (unused).
- per-module fields: the module part of the name (name minus suffix) must be in the module list (i.e. the module has a
  `_LEN` line); otherwise the field is dropped. The item is created on the module's first field; **item id = index of the
  module in the module list** (order of `_LEN` lines); at most 133 items. Fields: `_LAYER` → layer (u8), `_ALIGN` → align,
  `_ADR` → addr, `_LEN` → size, `_CMA_HID` → cmaId, `_MEMORY_TYPE` → memoryType and **miuNo = memoryType & (MIU1|MIU2)**; if
  miuNo is 1 the address gets `+ MIU_INTERVAL`, if 2 `+ MIU_INTERVAL2`. This offset is applied when `_MEMORY_TYPE` is met,
  so it relies on `_ADR` coming before `_MEMORY_TYPE` (the SCA layout).
Memory-type bits (from the defines, evaluated as in §2.1): bits 0–1 MIU number (`MIU0`=0, `MIU1`=1, `MIU2`=2), `UNCACHE` 0x4,
`REMAPPING_TO_USER_SPACE` 0x8, `CACHE` 0x10, (`NONCACHE_BUFFERABLE` 0x20 — not resolvable), `CMA` 0x40. No consumer on this
camera reads memoryType (the consumers of §2.5 take addr, size, miuNo only).

### 2.3 Writes [R main]
1. `open(memory_info, O_WRONLY)`, one `write` of `"%u %u %u %u %u %u %u\n"` = TotalSize, Miu0Size, Miu1Size, MiuBoundary,
   item count, bIs4kAlign, **(Miu1Size != 0)** as bMiu1Enable (`ENABLE_MIU_1` is not used for it).
2. `open(mmap_info, O_WRONLY)`; for i = 0 … count−1: `pwrite(fd, line, len, i)` with line =
   `"%s %u %u %u %u %u %u %u %u\n"` = module name *i*, item *i*'s id, addr, size, layer, align, memoryType, miuNo, cmaId.
   Name and item are both taken by index *i*; they belong together only when items were created in `_LEN` order, which
   the SCA layout guarantees [I]; a module whose first field precedes an earlier module's `_LEN` would be mislabelled [I].
   Return values of the writes are not checked.

### 2.4 Kernel side [R `MI_SYSCFG_SetMemory/ShowMemory/SetMmap/ShowMmap/GetMmapInfo/GetMemoryInfo`]
- `memory_info` write: exactly 7 tokens, else −EINVAL; each parsed with `kstrtouint(base 0)` whose errors are ignored (a
  bad token keeps the previous token's value); the last two are truncated to u8. Read: the seven values labelled
  (`u32TotalSize:0x%x` … `bMiu1Enable:%d`), −126 while the item count is 0.
- `mmap_info` write at position *i*: accepted only if *i* < `u32MmapItemsNum` (so `memory_info` must come first) and the
  high 32 bits of the position are 0; parsed with `sscanf("%s%u%u%u%u%u%u%u%u")` (return value unchecked) into item *i* =
  {char *name (kstrdup; an older name is leaked), u8 gid, u8 layer, u8 miuNo, u8 cmaId, u32 addr, size, align,
  memoryType}. Limits not checked: the table has 24 items (`memset 0x240`) but only `i < count` is tested (count > 24
  overruns it); the name goes into a 32-byte stack buffer (a name of 32+ characters smashes the kernel stack). Read at
  position *i* < count: `"%s:GID=%d,Addr=0x%x,Size=0x%x,Layer=%d,Align=0x%x,MemoryType=0x%x,MiuNo=%d,CMAID=%d\n"`; beyond →
  −126 (`ENOKEY`, what `dump_mmap` waits for).
- `MI_SYSCFG_GetMmapInfo(name, &p)`: linear `strcmp` → p = pointer to `{u8 gid, layer, miuNo, cmaId; u32 addr, size, align,
  memoryType}` (= `MI_SYSCFG_MmapInfo_t` of the 2018 header [S]), returns 1; else 0. `MI_SYSCFG_GetMemoryInfo(&p)`: 1 +
  pointer to `{TotalSize, Miu0Size, Miu1Size, MiuBoundary, ItemsNum, u8 bIs4kAlign, u8 bMiu1Enable}` once the count is
  non-zero.

### 2.5 Every mmap.ini item and its consumer [R]
Requesters found by scanning every module for `E_LX_*` / `E_MMAP_ID_*` strings and their call sites:
| item | requester | use | in the camera's mmap.ini |
|---|---|---|---|
| `E_LX_MEM` | mi_sys init (§1.3) | size compared with the kernel command line; warning only | yes: 0 / 0xFFE0000, layer 0 |
| `E_MMAP_ID_EMI` | mi_venc (one input path, below) | base address of a work buffer; **absent → `BUG()`**, `BUG_ON` when too small | yes: 0xFFE0000 / 0x20000 (the top 128 KiB of DRAM, above `E_LX_MEM`) |
| `E_MMAP_ID_DUMMY1` | none | — (the 0–0xF700000 range below the logo FB) | yes, unused |
| `E_LX_LOGO_RESERVED_FB` | none in the kernel | the boot-logo frame buffer 0xF700000 / 8 MiB; the kernel actually reserves it from `bootargs mmap_reserved=fb,…` (`uboot.md` §5.6) | yes, unused |
| `E_MMAP_ID_XC_MAIN_FB`, `E_MMAP_ID_XC_MLOAD` | mi_disp init | passed to `MHAL_DISP_InitMmapConfig(0/1, …)` when present — an empty function returning 1 in this mhal, so the areas would be ignored | no → skipped |
| `E_MMAP_ID_GE_VQ` | mi_gfx open | GE command queue, else MMA | no |
| `E_MMAP_ID_AI`, `E_MMAP_ID_AO`, `E_MMAP_ID_AO_TMP` | mi_ai / mi_ao | audio buffers | no (modules unused) |
| `fbdev.ini` `FB_MMAP_NAME` (`E_MMAP_ID_FB`, `E_MMAP_ID_HW_CURSOR`) | fbdev (`rgn-gop.md` §5.2) | fixed FB address + miuNo, else `FB_BUFFER_LEN` KiB from MMA | no → MMA |
The `E_MMAP_ID_EMI` path is one input path of mi_venc (in the code: input mode 0 with codec values 0xb / 0xc and a size
test): the EMI area is taken for ring input in HW auto-sync mode, and HW_AUTOSYNC is offered only from SCL on chips > 4
[R, `mi_venc.md` §7.4, §11]. `memory_info` is read by the MMU set-up (`mi_sys-2-interfaces.md` §7.3): TotalSize 256 MiB → page
mode 1, the same mode the `lx_mem` fallback (`lx_mem_size` 0xffe0000) gives on this camera [R live value + rule]; the mode
actually chosen has not been read on the running camera.

### 2.6 The camera's mmap.ini → kernel state
config_tool output for `/config/mmap.ini` [R by the rules above]; the live capture of
2026-10-16 shows the same `memory_info` and item 0 (`cat` re-reads position 0, so only item 0 is in the capture) [R live]:
```
memory_info: 268435456 268435456 0 2147483648 4 1 0     (live: 0x10000000 0x10000000 0x0 0x80000000 4 1 0)
mmap_info 0: E_LX_MEM 0 0 268304384 0 4096 4 0 0        (live: E_LX_MEM:GID=0,Addr=0x0,Size=0xffe0000,Layer=0,Align=0x1000,MemoryType=0x4,MiuNo=0,CMAID=0)
mmap_info 1: E_MMAP_ID_DUMMY1 1 0 259022848 1 4096 4 0 0
mmap_info 2: E_LX_LOGO_RESERVED_FB 2 259022848 8388608 1 4096 4 0 0
mmap_info 3: E_MMAP_ID_EMI 3 268304384 131072 0 4096 4 0 0
```
mmap.ini fields with no effect on this camera [R: never output]: the whole `MIU SETTING` block (`MIU0/1/2_GROUP_SELMIU`,
`*_GROUP_PRIORITY` — the MIU arbiter grouping is not configured from here; it is `kernel-platform.md` §12's), `ENABLE_MIU_*`,
`MIU_DRAM_LEN2`, `MIU_INTERVAL2`, `CPU_ALIGN`, `MMAP_COUNT`, `MIU0_END_ADR`, `*_AVAILABLE`, `*_GAP_CHK`, `*_CMA_HID` (all 0),
and the `CHK_VALUE` comment (SCA's checksum, verified by nobody).

## 3. The ini grammar of config.ini / PQConfig.ini [R `iniparser_load` (0x15424), `iniparser_line` (0x1518a), dictionary 0x148f6…0x14a78, getters 0x1502e…0x15a76]
The embedded parser is stock iniparser 4.x (N. Devillard) with one addition (key normalisation around `:`):
1. Lines are read with `fgets(1024)`. A physical line longer than 1023 bytes that does not end the file → "input line too
   long" and the **whole load fails** (dictionary freed, `load_config` writes nothing).
2. A line whose length is exactly 1 (a bare `\n`, or a single character at EOF without newline) is skipped before any other
   processing.
3. Trailing white space (space, `\t`, `\n`, `\r`, `\f`, `\v`) is stripped. If the line then ends in `\`, the backslash is
   removed and the next physical line is appended **with its leading white space** (unlimited logical length — this is how
   the multi-line `{…}` lists are written).
4. The logical line is trimmed on both sides and classified:
   - empty → ignored; first character `#` or `;` → comment;
   - `[` … `]` (first and last character) → section name = text up to the first `]`, trimmed, **lower-cased**; the section
     becomes current and is itself stored as a key with no value (a repeated section, like the two `[LOGO]` lines, just
     re-selects it);
   - contains `=` → key = text before the first `=`, trimmed, lower-cased; value by the first matching form:
     `key = "text"` (inside double quotes, may hold `;` and `#`), `key = 'text'`, `key = text` where text stops at the first
     `;` or `#` (inline comment) and is trimmed; a value that is exactly `""` or `''` becomes empty; `key = ;…` / `key = #…` /
     `key =` → empty value;
   - anything else → "syntax error" (counted, see 6).
5. Each key is stored as `section:key` (before the first section the section is the empty string, so `:key`). A key that
   exists already gets its value replaced. Values keep their case.
6. Error accounting (upstream behaviour): the error counter is **overwritten** by the result of every stored section / key,
   so a syntax error is forgotten once a later line is stored; only errors after the last stored line, a too-long line, or
   an allocation failure make the load fail.
7. Lookups (`section:key`) are normalised the same way — trimmed, white space around the first `:` removed, lower-cased —
   so names are case-insensitive. Hash: Jenkins one-at-a-time (`(h + c) * 1025; h ^= h >> 6`, final `h *= 9; h ^= h >> 11;
   h *= 32769`) plus a full `strcmp`.
Typed getters used by config_tool:
| getter | rule | "not found" |
|---|---|---|
| string (`iniparser_getstring` (0x1597a)) | the stored text | NULL |
| int (`iniparser_getint`) | `strtol(text, 0)` — **C prefixes: `0x` hex, leading `0` octal** (`08` → 0); empty text → 0 | returns 0 and found = 0; a stored value of **−1 is indistinguishable from "missing"** (→ 0, found = 0) |
| bool (`iniparser_getboolean` (0x159e0)) | first character `y Y 1 t T` → 1, `n N 0 f F` → 0 | anything else (e.g. `2`, `on`) counts as missing → 0, found = 0 |
| string array (`ParserStrArry`, `ParserStrArry`) | text between the first `{` and the last `}`; elements split on `,`, each trimmed of white space; empty elements (e.g. after a trailing comma) are dropped; the element count is the number of tokens split on space-or-comma, so **an element containing a space makes the count larger than the number of elements and the trim loop then dereferences an unfilled (NULL) slot → crash** | NULL / no `{…}` → nothing |
| number array (`parse_number_array` (0x125ea)) | each element `strtoul(…, 0)`, stored into an array of element size 1 or 4 — but only the **low byte** is written (`strb`), so in the 4-byte MCU arrays `0xFFFFFE` becomes `0x000000FE` [R disassembly 0x12638] | — |

## 4. config.ini: sections, readers, copies

### 4.1 Sections of the live file [R]
The camera loads `/misc/config.ini` (MISC firmwarefs). It is **byte-identical to `/config/config.ini`** (76005 bytes):
every byte of `/config/config.ini` is found in the MISC region of `nor.bin` in three in-order
runs — file 0x0000–0x7ff7 at flash 0xEE0008, 0x7ff8–0xffeb at 0xEE800C, 0xffec–0x128e4 at 0xEB0010 —
i.e. three 32 KiB firmwarefs subblocks (indexes 0, 1, 2 at b5.s0, b5.s1, b2.s0) whose data starts after an 8-byte TFTL
subblock header followed by 0, 1 and 2 CTZ skip pointers (`misc-firmwarefs.md` §3). Sections as config_tool's parser sees
them:
| section | keys | reader | status |
|---|---|---|---|
| `[ROOT]` | `m_pnlList` (7 names), `m_hdmiList` (34), `m_btList` (19) | config_tool §5.1 | used; `m_bootlogoList` (U-Boot's multi-logo list, `uboot.md` §5.6) absent |
| `[LOGO]` (written twice; the second line re-selects it) | `m_eDeviceType`=2, `m_sParaTarget`=DACOUT_1080P_60, `m_wDevId`=0, `m_wDispWidth/Height/Fps`=1920/1080/60, `m_sLogoFile0`=sigmastar1024_600.jpg, `m_sLogoFile1`=upgrade.jpg | U-Boot `bootlogo` (`uboot.md` §5.6); `m_sParaTarget` is read by nobody (no reference in U-Boot or config_tool) | used by U-Boot only |
| `[LOGO1]` | the same keys for a second display (`m_sParaTarget`=SAT070AT50H18BH, `m_wDevId`=1, 1920×1080@30) + `m_sPQFile0`=PQ.bin | U-Boot only if `ROOT:m_bootlogoList` named it | unused |
| `[PQ_PATH]` | `default = "/misc/PQ.bin"` | none: config_tool reads `[PQ_PATH]` from **PQConfig.ini**, not from config.ini (§6); U-Boot takes the PQ file name from `m_sPQFile0` | unused (a hint of what PQConfig.ini holds, §6.2) |
| 60 panel / timing sections named in the lists | §5.2 keys | config_tool → mi_disp / mi_panel | used (written to the kernel) |
| `[ST7710S]`, `[FPGA_800x480]`, `[ILI9881C_31Pin]` | panel keys | none (not in any list) | unused |
Every key present in a listed section is one config_tool reads for that section's interface type; there are no unknown
keys and no keys made dead by the interface condition. Key sets by type:
- `DACOUT_*` / `VGAOUT_*` (intf 7 = DAC interlaced, 8 = DAC progressive; 34 sections): name, intf, the four
  `m_bPanelInv*`, H/V sync width + back porch, H/V start, width / height, Max / typ / Min H total, V total and DCLK
  (**Hz**), spread-spectrum freq / ratio (0).
- `BT1120OUT_*` (intf 14) / `BT656OUT_*` (intf 12): as DAC without Max / Min, plus `m_bPanelDither`, `m_ePanelRgbDataType`,
  `m_ePanelRgbDataSwap`, the four `m_ucPanelSwap*`, `m_wPadmux`; `BT1120OUTPUT` / `BT656OUTPUT` in the panel list are the same
  with `m_wPanelDispPath`.
- MIPI-DSI panels (intf 11: RM68200, ILI9881C_40Pin, SAT070BO30I21Y0; unlisted ST7710S, ILI9881C_31Pin): + output bit mode,
  the DSI D-PHY timing set, porch set, lanes / format / ctrl mode, polarity / lane-swap bytes, `m_pCmdBuff` (init sequence,
  486–842 bytes).
- TTL panel SAT070AT50H18BH (intf 0), MCU panel ILI9163C_128x128 (intf 15, with init / auto command arrays).
The external LCD panels are not on this camera's hardware; their records are written to the kernel like the rest (§5).

DCLK unit: in mhal the DCLK field only computes the frame rate, fps = round(DCLK / (VTotal · HTotal)), which takes Hz; the
HDMI pixel clock comes from mhal's per-timing tables, not from DCLK [R, `mhal-disp.md` §7.1]. U-Boot's `bootlogo` table (the
ssd_lcd API) takes MHz — a different consumer (`uboot.md` §5.4).

### 4.2 The copies [R]
| copy | size | what it is | read by |
|---|---|---|---|
| `/misc/config.ini` (MISC) | 76005 | the live file | U-Boot `bootlogo`, config_tool (via `config_ini_path`) |
| `/config/config.ini` | 76005 | identical to MISC (§4.1) | config_tool, as its first fallback path (§5.1) |
| `/customer/misc/config.ini` | 27641 | an older SDK version of the same file: `m_pnlList` = ST7710S, SAT070AT50H18BH; `m_hdmiList` = 26 timings (no 29.97 / 59.94, no 1440p, no BT list); **DCLK values in MHz** (`m_wPanelDCLK = 148` where the live file has 148000000), every `m_wPanelVStart` = 0 in the DAC sections, a different 2K2K@30 timing with spread spectrum 25/192, SAT070AT50H18BH with `m_wPanelFixedDCLK` = 51 and other polarity / bit mode, an extra unlisted `[SAT070CP50]`; no `[LOGO1]`, `[PQ_PATH]` | none — build-time source of a MISC image [I; no reader in any binary, R] |
| `/customer/misc/config_cardv.ini` | 32061 | a car-DVR LCD build: `m_pnlList` = RM68200, LX50FWB4001V3; empty `m_hdmiList`; `[LOGO]` for the LCD (`m_eDeviceType` 1, 480×854@60, `bootlogo.jpg`); panels `ST7710S`, `T30P117`, `T30P133`, `RM68200`, `LX50FWB4001V3`, three of them intf 17 (`TTL_SPI_IF`) with SPI-init keys `m_pSpiCmdTable`, `m_wGpioBlEn`, `m_wGpioSpiClk/Rst/Sce/Sda`, `m_wSpiBitsMode` that **this config_tool does not read** (a newer tool version [I]) | none |
Loaded by this config_tool, the older copy would hand mhal DCLK values a million times smaller, i.e. frame rates a million
times too small [I from the DCLK use above].

### 4.3 The other ini files of the dump [R string search]
- `boot.ini` (`[MUSIC_CFG]` boot music on / num / names, `[MUSIC_VOL_CFG]`, `[LOGO_CFG]` logo on / num / names `boot0.jpg`,
  `CAP1_NAME` / `CAP2_NAME`): no reader in U-Boot, the kernel, `/customer`, the rootfs or `/config`; unused (a cardv-SDK
  leftover [I]).
- `/config/board.ini` (`[PanelRelativeSetting]` PWM port, LVDS connect / swap / polarity, `#@CRC=0x725D` = the 16-bit byte
  sum of the preceding 460 bytes) and `model/Customer.ini`: read by no binary (rootfs, `/config`, `/customer`, kernel, U-Boot, NOR
  searched); specified in `panel-mipitx.md` §9.
- `/config/model/LCM.ini` and `/customer/LCM/*.ini` (five panels) are configurations of external LCD panels, hardware not
  on this camera.
- `fbdev.ini`: read by fbdev.ko (`rgn-gop.md` §5.2).

## 5. `load_config`: config.ini → `config_info`

### 5.1 Flow [R `load_config_main` (0x15b0c), `load_config`, `SetPanelInfo`]
1. Load `argv[1]` with the iniparser (§3). If that fails, try `/config/config.ini`, then `/config/model/config.ini`; if all
   fail, return **without writing `config_info`** (the PQ step still runs).
2. For each list `m_pnlList`, `m_hdmiList`, `m_btList` of `[ROOT]` in this order: read it as a string array (§3); a missing
   or empty list is skipped ("ParserStrArry failed"); for each name build one 0x170-byte record (§5.2) — **a name without a
   matching section still yields a record, all zero** (intf 0, timing 0) — and append the records to a static 0x8fc8-byte
   buffer `MI_SYSCFG_Config_t` (name from the tool's own error string): records at +8, count at +4 incremented per record.
   There is **no check against the capacity** (100 records, below): a 101st record overruns the buffer.
3. `open(config_info, O_WRONLY)`, `write` the whole 0x8fc8 (36808) bytes. The loop "until all written" never needs a second
   round because the kernel takes the whole buffer; if `write` returned −1 the loop would go on with a shrinking offset.
Byte 0 of the struct (`m_u8DLCTableCount` in the 2018 header [S], returned by the exported `MI_SYSCFG_GetDLCTableCount`,
which no module imports) and everything after the last record stay 0 — this config_tool has no DLC / gamma / audio sections
at all. Struct: `{u8 dlc_count; u8 pad[3]; u32 count; record[100]}` = 8 + 100 × 0x170 = 0x8fc8 — the kernel symbol
`g_stConfig` has exactly this size [R `nm`; 100 = `PANEL_MAX_NUM` of the 2018 header, S]. Offsets relative to `g_stConfig`
are the record offsets + 8 (interface word +0x8c, timing word +0x90, array pointers +0xfc / +0x104 / +0x148; the name of
record 0 at +0x0a).

### 5.2 The 0x170-byte record [R `LoadPanelInfo`; consumer column from mi_disp]
Each key is looked up as `<list name>:<key>` (the section is the name from the list, not `m_pPanelName`). "int" = §3 int
getter (missing → 0), "bool" = bool getter. Flags are u8 0/1 that tell the consumer which groups are valid. The right column
is the member of mhal's `MhalPnlUnifiedParamConfig_t` (OpenIPC u-boot `drivers/mstar/panel/drv/mipnl/pub/mhal_pnl_datatype.h`
[S]) that mi_disp fills from the field before `MhalPnlSetUnifiedParamConfig` (BT656 / BT1120 interface set-up); mi_panel writes the same layout. The member order is a positional match [I, strong: same
order, sizes and flag semantics]; this mhal build's struct has a 4-byte gap at +0x30 and a 2-byte gap at +0x44 relative to
the public header (polarity at +0x34, SSC at +0x46, total 0xd4) [R, `panel-mipitx.md` §4.8].
| off | type | key / source | condition, notes | Unified member |
|---|---|---|---|---|
| 0x000 | u16 | `m_wPanelDispPath` int | read by mi_panel Init as the `u16 u16Id` argument of `MhalPnlCreateInstanceEx`, i.e. the mhal PNL context id (0 → timing generator bank 1151, 1 → 1161; > 1 fails) [R, `panel-mipitx.md` §3.3]; no reader in mi_disp / mi_sys | — |
| 0x002 | char[128] | `m_pPanelName` string, ≤ 127 chars + NUL | empty if missing; it, not the section name, is matched against the timing table (§5.3) | `pPanelName` (pointer to this field) |
| 0x082 | u8 | `m_bPanelDither` bool | | (mi_disp's HDMI / VGA table of §5.5: bit 0 of its flags byte) |
| 0x084 | u32 | `m_ePanelIntfType` int = `MhalPnlLinkType_e` | 0 TTL, 1 LVDS, 2 RSDS, 3 MiniLVDS, 4/5 analog / digital MiniLVDS, 6 MFC, **7 DAC_I, 8 DAC_P**, 9 PDPLVDS, 10 EXT, **11 MIPI_DSI, 12 BT656**, 13 BT601, **14 BT1120, 15 MCU**, 16 sRGB, 17 TTL_SPI_IF; key of `GetPanelInfo` | `eLinkType` |
| 0x088 | u32 | timing id, derived | §5.3; second key of `GetPanelInfo` | — |
| 0x08c | u8 | TgnTimingFlag = 1 | always | `u8TgnTimingFlag` |
| 0x08d | u8 | TgnPolarityFlag = 1 | always | `u8TgnPolarityFlag` |
| 0x08e | u8 | TgnRgbSwapFlag = 1 | always | `u8TgnRgbSwapFlag` |
| 0x08f | u8 | TgnSscFlag = 1 | always | `u8TgnSscFlag` |
| 0x090 | u8 | TgnOutputBitMdFlag | = `m_eOutputFormatBitMode` found | `u8TgnOutputBitMdFlag` |
| 0x091 | u8 | TgnFixedDclkFlag | = `m_wPanelFixedDCLK` found | `u8TgnFixedDclkFlag` |
| 0x092 | u8 | TgnPadMuxFlag | = `m_wPadmux` found | `u8TgnPadMuxFlag` |
| 0x093 | u8 | MpdFlag (MIPI-DSI) | = 1 if intf 11 | `u8MpdFlag` |
| 0x094 | u8 | RgbDataFlag | = `m_ePanelRgbDataType` or `m_ePanelRgbDataSwap` found | `u8RgbDataFlag` |
| 0x095 | u8 | RgbDeltaMdFlag | = both delta modes found (even is read only if odd was found) | `u8RgbDeltaMdFlag` |
| 0x096 | u8 | McuPhaseFlag | intf 15: `m_wMcuPhase` found | `u8McuPhaseFlag` |
| 0x097 | u8 | McuPolarityFlag | intf 15: `m_wMcuPolarity` found | `u8McuPolarityFlag` |
| 0x098 | u8 | McuRsPolarityFlag | intf 15: `m_wMcuRsPolarity` found | `u8McuRsPolarityFlag` |
| 0x099 | u8 | McuConfigFlag | = 1 if intf 15 | `u8McuConfigFlag` |
| 0x09c–0x0aa | u16 ×8 | `m_wPanelHSyncWidth`, `m_wPanelHSyncBackPorch`, `m_wPanelVSyncWidth`, `m_wPanelVBackPorch`, `m_wPanelHStart`, `m_wPanelVStart`, `m_wPanelWidth`, `m_wPanelHeight` | always read | `stTgnTimingInfo`: HSyncWidth, HSyncBackPorch, VSyncWidth, VSyncBackPorch, HStart, VStart, HActive, VActive |
| 0x0ac–0x0b6 | u16 ×6 | `m_wPanelMaxHTotal`, `m_wPanelHTotal`, `m_wPanelMinHTotal`, `m_wPanelMaxVTotal`, `m_wPanelVTotal`, `m_wPanelMinVTotal` | always | HTotal, VTotal (Unified); all six go to mi_disp's HDMI / VGA table (§5.5) |
| 0x0b8–0x0c0 | u32 ×3 | `m_wPanelMaxDCLK`, `m_wPanelDCLK`, `m_wPanelMinDCLK` (Hz in the live file, §4.1) | always ("m_w" is a misnomer: 32-bit) | `u32Dclk` = DCLK; all three to the HDMI / VGA table |
| 0x0c4–0x0c7 | u8 ×4 | `m_bPanelInvDCLK`, `m_bPanelInvDE`, `m_bPanelInvHSync`, `m_bPanelInvVSync` bool | always | `stTgnPolarityInfo` |
| 0x0c8–0x0cb | u8 ×4 | `m_ucPanelSwapChnR/G/B`, `m_ucPanelSwapRgbML` int (low byte) | always; mi_disp maps R ∉ {1,2} → 3, G ∉ {1,3} → 2, B ∉ {1,2,3} → 1 (so 0 = default R3 G2 B1) | `stTgnRgbSwapInfo` |
| 0x0cc, 0x0ce | u16 ×2 | `m_wSpreadSpectrumFreq`, `m_wSpreadSpectrumRatio` | always | `stTgnSscInfo` step, span |
| 0x0d0 | u32 | `m_eOutputFormatBitMode` | 0 10-bit, 1 6-bit, 2 8-bit, 3 565 (comment in config_cardv.ini [S]); mi_disp clamps > 3 → 2 (> 2 → 2 in the HDMI table) | `eOutputFormatBitMode` |
| 0x0d4 | u32 | `m_wPanelFixedDCLK` | | `u32FDclk` |
| 0x0d8 | u16 | `m_wPadmux` | | `u16PadMux` |
| 0x0da–0x0dc | u8 ×3 | `m_wMcuPhase`, `m_wMcuPolarity`, `m_wMcuRsPolarity` | intf 15 only | `u8McuPhase`, `u8McuPolarity`, `u8McuRsPolarity` |
| 0x0e0, 0x0e4 | u32 ×2 | `m_wMcuHActive`, `m_wMcuVActive` | intf 15 | `stMcuInfo.u32HActive/VActive` |
| 0x0e8–0x0ea | u8 ×3 | `m_wMcuWRCycleCnt`, `m_wMcuCSLeadWRCycleCnt`, `m_wMcuRSLeadCSCycleCnt` | intf 15 | cycle counts |
| 0x0ec | u32 | `m_eMcuType` | intf 15; mi_disp: not 0/1 → 0 | `enMcuType` |
| 0x0f0 | u32 | `m_eMcuDataBusCfg` | intf 15; mi_disp: > 0x12 → 0 | `enMcuDataBusCfg` |
| 0x0f4, 0x0f8 | ptr, u32 | `m_pMcuInitCmdBuff` number array (u32 elements, **only the low byte stored**, §3), element count | intf 15; **`m_pMcuAutoCmdBuff` is parsed into the same two fields** and overwrites the init list when both exist [R code; live: ILI9163C has 176 init and 14 auto elements, the kernel record holds count 14] | `stMcuInitCmd` |
| 0x0fc, 0x100 | ptr, u32 | auto-command array | never set by this config_tool (the kernel still deep-copies it if non-zero) | `stMcuAutoCmd` |
| 0x104–0x132 | u16 ×24 | `m_wHsTrail`, `m_wHsPrpr`, `m_wHsZero`, `m_wClkHsPrpr`, `m_wClkHsExit`, `m_wClkTrail`, `m_wClkZero`, `m_wClkHsPost`, `m_wDaHsExit`, `m_wContDet`, `m_wLpx`, `m_wTaGet`, `m_wTaSure`, `m_wTaGo`, `m_wHactive`, `m_wHpw`, `m_wHbp`, `m_wHfp`, `m_wVactive`, `m_wVpw`, `m_wVbp`, `m_wVfp`, `m_wBllp`, `m_wFps` | intf 11; the first 14 go to u8 members (low byte) | `stMpdInfo` D-PHY timings, porches, BLLP, fps |
| 0x134–0x13c | u32 ×3 | `m_eLaneNum`, `m_eFormat`, `m_eCtrlMode` | intf 11; mi_disp: lanes outside 1–4 → 1, format > 3 → 3, ctrl > 3 → 1 | `enLaneNum`, `enFormat`, `enCtrl` |
| 0x140, 0x144 | ptr, u32 | `m_pCmdBuff` number array (u8 elements), byte count | intf 11; DSI init sequence | `pu8CmdBuf`, `u32CmdBufSize` |
| 0x148–0x150 | u16 ×5 | `m_wSyncCalibrate`, `m_wVirHsyncSt`, `m_wVirHsyncEnd`, `m_wVsyncRef`, `m_wDataClkSkew` | intf 11 (SyncCalibrate to a u8 member) | same names |
| 0x152–0x156 | u8 ×5 | `m_ucPolCh0`…`m_ucPolCh4` | intf 11; 0 HW default, 1 positive, 2 negative (header comment [S]) | `u8PolCh0…4` |
| 0x157–0x15b | u8 ×5 | `m_ucClkLane`, `m_ucDataLane0`…`3` | intf 11 | `u8SwapCh0…4` (lane mapping) [I positional] |
| 0x15c | u32 | `m_ePacketType` | intf 11; not 0/1 → 0 | `enPacketType` |
| 0x160 | u32 | `m_ePanelRgbDataType` | any intf | `stRgbDataInfo.eRgbDtype` |
| 0x164 | u8 | `m_ePanelRgbDataSwap` | any intf | `stRgbDataInfo.u8RgbDswap` |
| 0x168, 0x16c | u32 ×2 | `m_ePanelRgbDeltaOddMode`, `m_ePanelRgbDeltaEvenMode` | any intf | `stRgbDeltaInfo` |
Gaps 0x083, 0x09a–0x09b, 0x0dd–0x0df, 0x0eb, 0x165–0x167 are padding (0).

### 5.3 Timing id [R `.data` 0x290dc, loop in `LoadPanelInfo`]
<!-- ev:config_tool.5.3 -->
For intf 7, 8, 12 and 14 (DAC_I, DAC_P, BT656, BT1120) the **`m_pPanelName`** value is compared (`strcmp`, exact case) with
an 81-entry built-in table `{u32 flag; u32 timing; char name[128]}` (dump). Found → that
timing id; not found → 0x39 for intf 7/8, 0x38 for 12/14. Any other intf → 0x38. The table's first word (1 for DAC / BT
names, 0 for VGA names) is never read — in the 2018 struct the matching field is `bHdmiTx` [I]. Ids (the newer SigmaStar
`DISPLAYTIMING` enum; the 2018 header's enum is an older, shorter one [S]):
| id | names | id | names |
|---|---|---|---|
| 2 / 3 / 4 / 5 | 480I, 480P, 576I, 576P | 21 / 22 / 23 / 24 | 1470P_24/30/60, 2205P_24 |
| 6 / 7 / 8 / 9 | 720P_2997, _50, _5994, _60 | 25–28 | 2K2KP_24/25/30/60 |
| 10 / 11 / 12 / 13 | 1080P_24, _25, _30, _2997 | 29 / 30–32 | 4K540P_240 / 4K1KP_30/60/120 |
| 14 / 15 / 16 / 17 / 18 | 1080I_50, 1080P_50, 1080P_5994, 1080I_60, 1080P_60 | 33–38 | 4K2KP_24/25/2997/30/50/60 |
| 19 / 20 | 1440P_50/60 | 39–43 | 4096P_24/25/30/50/60 |
| 46 / 47 / 48 / 49 | VGA 1024×768, 1366×768, 1280×800, 1280×1024 | 50 / 51 / 52 | VGA 1440×900, 1600×1200, 1680×1050 |
| 0x38 (56) | "any timing of this interface" — `GetPanelInfo` with timing 0x38 returns the first record of the interface | 0x39 (57) | not resolved (no record can be asked for it) |
The DAC names are `DACOUT_<x>`, BT1120 `BT1120OUT_<x>` and BT656 `BT656OUT_<x>` with the same ids (480I / 480P / 576I / 576P /
720P / 1080P / 1080I subsets). Ids 0, 1, 44, 45, 53–55 have no name in this tool; mi_disp's mhal-timing → SYSCFG-id switch
(`mi_disp-mi_hdmi.md` §4.3) never requests any of them, and no binary on hand names them — their names are not known.

### 5.4 Kernel side [R `MI_SYSCFG_ConfigSet/ConfigGet/GetPanelInfo`]
- `config_info` write (raw, position *p*, length *n*): first, for every record already counted, the three pointers at record
  +0xf4, +0xfc, +0x140 are freed if non-zero; then `memcpy(g_stConfig + p, buf, n)` **with no bound** on p + n; then for each
  of the (new) count records every non-zero pointer is treated as a **user-space address of the writing process** and
  replaced by a kernel copy: +0xf4 (`count@+0xf8` × 4 bytes), +0xfc (`count@+0x100` × 4), +0x140 (`count@+0x144` bytes)
  (`CamOsMemAlloc` + `CamOsCopyFromUpperLayer`; an allocation failure leaves the user pointer in place). Returns n. So the
  struct is taken in one `write` from the process that owns the arrays; a second write frees the kernel copies and then
  copies from those kernel addresses as if they were user addresses (corrupted arrays or a fault).
- Read: `memcpy(buf, g_stConfig + p, n)`, returns n, no bound (§1.4).
- `MI_SYSCFG_GetPanelInfo(intf, timing)` (exported; users mi_disp, mi_panel): with timing 0x38 the first record whose +0x84
  equals intf; otherwise the first with +0x84 == intf and +0x88 == timing; returns a pointer to the record (it points into
  `g_stConfig`, so consumers see the kernel copies of the arrays); not found → NULL and the log line
  `[MI_SYSCFG_GetPanelInfo 67] eIntfType = %d eTiming = %d not found!!!`. The loop index is a u8, so with more than 255
  records it would wrap (the struct holds 100).

### 5.5 Consumers of the records [R mi_disp, mhal]
- **mi_disp init** (first open): for every mhal timing index *i* = 0…0x26 (the mhal timing enum: 0 PAL, 1 NTSC, 9 1080I50,
  0xa 1080I60, …) a SYSCFG timing id is chosen by a switch in `MI_DISP_IMPL_Init` and `GetPanelInfo(intf, id)` is called
  with **intf 7 (DAC_I) for *i* ∈ {0, 1, 9, 0xa} and 8 (DAC_P) otherwise** [R disassembly 0x97a0–0x97cc:
  `intf = (n > 1 && n−9 >u 1) ? 8 : 7` for the next index n] — consistent with the intf values of the DAC records. Each hit
  fills one 0x74-byte mhal panel config `MHAL_DISP_PanelConfig_t` (name pointer, link type in bits 1–4 and dither in bit 0 of a flags byte, the four polarity bits, all 14 timing u16 including
  Max / Min totals, the three DCLKs, SSC, bit mode clamped to ≤ 2, the index, a valid flag), and the 0x27 configs go to
  `MHAL_DISP_InitPanelConfig` (called even when nothing was found); an index without a record leaves its slot invalid. The
  switch and the copy rules are `mi_disp-mi_hdmi.md` §4.3's. Of the 0x74-byte config mhal reads only bValid, eTiming,
  HSyncWidth, HSyncBackPorch, HStart, Width, HTotal, VSyncWidth, VBackPorch, VStart, Height, VTotal, DCLK (only to compute
  the frame rate) and the SSC step / span (stored, not programmed); Max / Min totals, Max / Min DCLK, polarities, dither,
  bit mode and link type are ignored (`mhal-disp.md` §7.1). So the HDMI / VGA porch timings mhal uses come from these records.
- **mi_disp interface set-up** (SetPubAttr): BT1120 and all panel interfaces take their sync info from SYSCFG, while the
  user SyncInfo is copied for CVBS / YPbPr / VGA / HDMI / BT656_H / BT656_L. The panel set-up — `GetPanelInfo(intf, id)`,
  the record copied into `MhalPnlUnifiedParamConfig_t` (column of §5.2) → `MhalPnlCreateInstanceEx` /
  `MhalPnlSetUnifiedParamConfig` — runs only for BT656 (link 0xc) and BT1120 (0xe); `GetPanelInfo(0, id)` is only the
  fallback for unsupported link types (`mi_disp-mi_hdmi.md` §4.6.3).
- **mi_panel**: imports `GetPanelInfo`; its Init reads record +0x000 as the PNL context id (§5.2, `panel-mipitx.md` §3.3).
This camera drives HDMI only, so the 34 DAC / VGA records matter and the 26 panel / BT records are loaded and unused [I from
the hardware].

### 5.6 Check against the live kernel state [R]
The 2026-10-16 capture contains `cat /proc/mi_modules/common/config_info`. Because the raw read never advances the position
(§1.4), `cat` received the first 4 KiB of `g_stConfig` over and over; the capture also went through a terminal (LF → CRLF,
bytes ≥ 0x80 stored as UTF-8, TAB expanded, a lone 0xFF swallowed). Undoing the first two, building the 0x8fc8-byte struct from config.ini in memory, applying the TAB model to it and
walking both streams side by side gives:
Result for the 4 KiB = `dlc_count`, the count and records 0–10 (RM68200 MIPI, SAT070AT50H18BH TTL, ILI9881C_40Pin MIPI,
BT1120OUTPUT, BT656OUTPUT, ILI9163C MCU, SAT070BO30I21Y0 MIPI, DACOUT_576I / 480I / 1080P_24 / 25): count = 60 (0x3c);
every byte equal except (a) the array pointers (kernel addresses 0xc5… / 0xc4… in the live data, as §5.4 says) —
their counts match: 842, 486, 14 (MCU: the auto list), 617 —, (b) one TAB expansion that the simple column model gets wrong
by one column, (c) two dropped 0xFF bytes. Records 11–59 are not in the capture; they are produced by the same code paths
(DAC and BT branches already covered).

## 6. PQ: PQConfig.ini → `pq_info` + `pq_table` (and the six NR tables)

### 6.1 What config_tool does [R `LoadPQConfig`]
1. `argv[2]` NULL → nothing. Load it with the iniparser; on failure try `/config/PQConfig.ini`, then
   `/config/model/PQConfig.ini`; if all fail, **return without touching any PQ file**. None of the three exists on the camera
   (`/misc` holds config.ini, PQ.bin, two jpgs, ws.txt; `/config/model` holds LCM.ini, Customer.ini), so the PQ files keep
   the kernel's zeroes.
2. Header = 0x24 zero bytes. String `[PQ_PATH] default`; if set and the file opens: size by `fseek/ftell`, read whole; on a
   full read header +0x20 = size, else "read pq bin failed" (size stays 0, the blob is not sent).
3. `write(pq_info, header, 0x24)`; then, if size ≠ 0, `write(pq_table, blob, size)`. On a failed write it retries once and
   then calls `close()` on the write's return value instead of the fd (harmless at exit).
config_tool never fills the 17 DBC bytes or any table dimension: **the noise / motion / misc tables cannot receive data from
this tool** (their size is rows × cols from the header = 0, and a 0-byte write is refused by the raw layer). The only PQ
payload it can deliver is one opaque file.

### 6.2 What PQConfig.ini would carry
Exactly one key is read: `[PQ_PATH] default = "<path of a PQ binary>"` [R]. The live config.ini has a `[PQ_PATH]` section
with `default = "/misc/PQ.bin"`, and MISC holds `PQ.bin` (25892 bytes) [R] — so the intended file is
```
[PQ_PATH]
default = "/misc/PQ.bin"
```
saved as `/misc/PQConfig.ini` (or `/config/PQConfig.ini`, `/config/model/PQConfig.ini`) [I: vendor intent from the section
name and the file pair; nothing ships the file]. Were it present, `pq_info.size` = 25892, `pq_table` = PQ.bin, and mi_disp
init would pass it to mhal (§6.3). That is the same PQ.bin U-Boot would load through `m_sPQFile0`, which only `[LOGO1]`
names, so U-Boot's PQ path (`MDrv_PQ_Init`, `uboot.md` §6) gets no file either: no code path loads `PQ.bin` on this
camera [R]. The `PQ.bin` format is `uboot.md` §6.2; what `MHAL_DISP_DeviceSetPqConfig` does with it, `mhal-disp.md` §13.1.

### 6.3 Kernel side [R `MI_SYSCFG_PQConfigSet/Get`, `MI_SYSCFG_PQ*TableSet/Get`, `MI_SYSCFG_GetPqTbl`]
`pq_info` holds `MI_SYSCFG_PQConfig_t` (names from the 2018 header [S], which matches every offset the kernel and
`dump_config` use):
| off | field |
|---|---|
| 0x00–0x10 | 17 DBC bytes: `MAX_VIDEO_DBC`, `MID_VIDEO_DBC`, `MIN_VIDEO_DBC`, `MAX_PWM`, `BACKLIGHT_THRES`, `MIN_PWM`, `Y_GAIN_M`, `Y_GAIN_L`, `C_GAIN_M`, `C_GAIN_L`, `ALPHA_BLENDING_CURRENT`, `AVG_DELTA`, `FAST_ALPHABLENDING`, `LOOP_DLY_H/MH/ML/L` (dynamic backlight control of TV panels) |
| 0x11/0x12 | rows / cols of `motion_table` (DynamicNR motion) |
| 0x13/0x14 | `motion_hdmi_dtv_table` |
| 0x15/0x16 | `motion_comp_pc_table` |
| 0x17/0x18 | `misc_param_table` |
| 0x19/0x1a | `misc_luma_table` |
| 0x1b/0x1c | `noise_table` |
| 0x1d/0x1e | `misc_table` (MISC_Noise) |
| 0x1f | padding |
| 0x20 | u32 `pq_table` size (new in this SDK; the 2018 struct ends at 0x1f) |
- `pq_info` write: n < 0x24 → −EINVAL; otherwise the first 0x24 bytes are copied, returns n. Read: n < 0x24 → −EINVAL, else
  copies 0x24 bytes and returns n (rest of the user buffer = kernel heap garbage, §1.4).
- Table writes: n must equal rows × cols from the header (`pq_table`: the size word), else −EINVAL; the old copy is freed and
  the data `kmemdup`ed. Reads: n must equal the same size. So `pq_info` comes first, the tables after.
- Users: `MI_SYSCFG_GetPqTbl(&size)` returns the `pq_table` pointer (size ≠ 0) → mi_disp init passes `{1, size, ptr}` to
  `MHAL_DISP_DeviceSetPqConfig(0, …)` (the device argument is the integer 0). The six NR tables and the DBC bytes have **no
  reader** (no getter is exported; `MI_SYSCFG_GetDLCTableCount` is exported but imported by no module) — TV-SDK leftovers,
  unused [I: origin].
Live state 2026-10-16: `pq_info` = 0x24 zero bytes; every table read fails with EINVAL (`cat` asks 4096 ≠ 0) [R capture] —
the state an absent PQConfig.ini gives.

### 6.4 `dump_config` [R `dump_config_main` (0x15bc8), `dump_panel_info` (0x13c50)]
Reads `config_info` (0x8fc8 bytes) and prints `panel cnt:%d` and per record one `PanelName:%s Intftype:%d timing:%d` line
(name at struct +0x0a = record +0x02, intf, timing); then reads `pq_info`, prints `pq table size:%d` and only the first 9 DBC
bytes as `DBC Value=` 3 × 3, then reads each NR table with rows × cols from the header and prints it as `{0x..,…}` rows
(`start dump [<name>](rows, cols)` … `end dump`); `pq_table` itself is not dumped.

## 7. Defects, by section [R unless marked]
config_tool:
| # | defect | effect | § |
|---|---|---|---|
| 1 | `m_pMcuAutoCmdBuff` stored into the init-command fields | MCU panel loses its init sequence (live: 14 of 176) | 5.2 |
| 2 | number arrays store only the low byte (`strb`) | MCU u32 commands such as `0xFFFFFE` become `0xFE` | 3 |
| 3 | `_ADR` line without `//Alignment` → `strtoul(NULL)`; `(` without `)` → NULL dereference; no `#define` at all → `assert(0)` | config_tool crashes, nothing written | 2.1 |
| 4 | no bound on records (> 100) or `_LEN` modules (> 100) | buffer / stack overrun | 2.1, 5.1 |
| 5 | array element containing a space | miscount → NULL dereference | 3 |
| 6 | int value −1 reads as "missing"; leading `0` = octal | silent 0 / wrong values | 3 |
| 7 | mmap line *i* pairs module name *i* with item *i* | mislabelled items if a module's first field precedes an earlier module's `_LEN` [I] | 2.3 |
| 8 | `write` loops on `config_info` and PQ error paths (`close(ret)`) | only on kernel errors | 5.1, 6.1 |
| 9 | the NR tables and DBC bytes are never sent | `noise/motion/misc_*` always empty | 6.1 |
| 10 | `dump_mmap` prints kernel text with `printf(buf)` | format-string use | 1.2 |
Kernel side (mi_common / mi_sys):
| # | defect | effect | § |
|---|---|---|---|
| 11 | raw files never advance the file position | `cat` loops on the first 4 KiB; multi-call writes overwrite position 0 | 1.4 |
| 12 | `config_info` get / set have no bound; `pq_info` read returns n bytes with 0x24 filled | read past `g_stConfig`; kernel heap leak to the reader | 1.4, 6.3 |
| 13 | a second `config_info` write frees the kernel copies, then copies from those kernel addresses as user pointers | corrupted arrays / fault | 5.4 |
| 14 | `mmap_info`: 24-item table but only `i < count` checked; name parsed into a 32-byte stack buffer; old names leaked | overrun / stack smash with long names | 2.4 |
| 15 | `memory_info`: `kstrtouint` errors ignored | a bad token silently repeats the previous value | 2.4 |
| 16 | `GetPanelInfo` u8 index | wraps beyond 255 records (struct holds 100) | 5.4 |
