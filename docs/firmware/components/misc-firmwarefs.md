# misc-firmwarefs — the MISC partition's firmwarefs and the `/bin/fwfs` FUSE daemon
<!-- ev:misc-firmwarefs -->

MISC (mtd5, flash 0xE90000, 0xA0000 B = 10 × 64 KiB erase blocks) holds a SigmaStar "firmwarefs" ("fwfs") filesystem:
littlefs v2.2.1 renamed (`lfs` → `fwfs`, magic `"littlefs"` → `"firmwarefs"`) plus a SigmaStar layer called **TFTL**
("tiny flash translation layer") that splits each 64 KiB erase block into 32 KiB **subblocks** for file data, so that
small files do not each take a whole erase block [R source + image]. Metadata (directories, the superblock) stays in whole
64 KiB blocks in littlefs's two-block pairs. Linux serves it as `/misc` through the FUSE daemon `/bin/fwfs`; the stock
U-Boot mounts it with the same core to read the boot logo and its `config.ini`. This document owns the on-flash format,
the image as shipped, `/bin/fwfs` and U-Boot's firmwarefs glue. Elsewhere: the partition map and the file inventory
(what each MISC file is) — `firmware-image.md` §1.1, §1.4; U-Boot's `bootlogo` and the `ini_*` commands that use the
glue — `uboot.md` §5.2, §9.2; the `PQ.bin` format — `uboot.md` §6.2; `config.ini` as config_tool reads it —
`config_tool.md` §3, §5; `ws.txt` — `prog_uvc.md` §5.2; the boot script `rcS` that starts the daemon — `rootfs-customer.md` §2.3.

**Tags.** [R] read from a binary, its disassembly or decompile, the NOR dump, a boot log or a source file; [S] stated by a
source (a reference source text, a header comment); [I] inference. A heading tagged [R] covers every statement under it
unless marked otherwise.

## 1. Where the code comes from

### 1.1 Base and versions [R]

- The closest littlefs release to fwfs's `fwfs.c` is **v2.2.1**: after the `lfs` → `fwfs` rename it differs by 227
  removed and 1021 added non-blank lines; every other v2.x tag differs by more (v2.2.0 238/1032, v2.1.4 481/1414,
  v2.3.0 899/1323).
- Versions in the source: software `FWFS_VERSION 0x00030002` (fwfs v3.2), on-disk `FWFS_DISK_VERSION 0x00020000`
  (= littlefs disk v2.0), `FWFS_TFTL_DISK_VERSION 0x00040000` (TFTL v4.0). `/bin/fwfs --version` prints
  `firmwarefs version: v%d.%d` / `firmwarefs disk version: v%d.%d` [R strings].
- License headers are mixed: the littlefs BSD-3 notice is kept, a GPL-2 header and a "SigmaStar trade secret / reverse
  engineering … prohibited" header are added on top.

### 1.2 Three copies of the core [R]

The core is `firmwarefs/fwfs.c` and `firmwarefs/fwfs_tftl.c`, with the CRC routine `fwfs_crc`.

- **`/bin/fwfs` = oi.** Every named `fwfs_*` / `fwfs_tftl_*` core function of `/bin/fwfs` exists in oi; the oi
  functions missing from the binary are inlined `static inline` helpers, plus the whole `FWFS_MIGRATE` part
  (`fwfs_migrate`, `fwfs1_*`), which is **compiled out** (`fwfs_fuse_migrate` is a 6-byte stub). Every log format and
  every assert expression of oi is in the binary and vice versa, except the three migrate-only logs
  (`fwfs_block_check(fwfs, block)` exists as the tail of a longer merged string). All 73 `printf("%s:%d:…")` /
  `__assert_fail` call sites match their oi line through `__LINE__`: the offset oi − binary is a constant **15** (the
  15-line GPL header the oi copy carries in addition) up to `fwfs.c` line 1336 and **13** from line 1994 on — the
  binary's source had **2 more lines** somewhere between the end of `fwfs_dir_fetchmatchraw` and the "Expanding
  superblock" debug in `fwfs_dir_compact`; in `fwfs_tftl.c` the offset is 15 up to line 777 and 14 from line 813 on —
  1 line fewer, which fits the commented-out `//header->tag = 0x7f7f7f7f;` in `fwfs_tftl_header_fill`. The 2 extra
  `fwfs.c` lines are not identified; [I] they carry no code (a `FWFS_TRACE`, which compiles to nothing, or blank
  lines), because the function sizes and call sets in that range match oi.
- **Stock U-Boot = oi exactly.** All 86 `__LINE__` anchors that could be parsed in `fwfs.c` and `fwfs_tftl.c` of
  the stock U-Boot binary (of 89 calls) sit at oi line − 15, i.e. the U-Boot core is the oi text including the
  `fwfs_tftl.c` comment line `/bin/fwfs` lacks. Its glue is described in §5.
- **ssd** has identical headers and differs from the oi core only in how the TFTL header is zeroed (`header = {0,}`
  replaced by an explicit `header->reserved = 0x0;` in `fwfs_tftl_header_fill`, one line more); the on-flash result is
  the same. It is not the `/bin/fwfs` revision either (that one has one line *fewer* there).

### 1.3 Build of `/bin/fwfs` [R]

- `MAX_CACHE_POOL_SIZE` is **4** (assert string `cache_pool_size <= 4`; the oi header says 8).
- `FWFS_ASSERT` is libc `assert()` → `__assert_fail` → **abort** (U-Boot's build only prints, §5).
- Logs go to `printf` (stdout); `FWFS_TRACE` is compiled out; DEBUG / WARN / INFO / ERROR are compiled in.
- Toolchain: SigmaStar gcc 9.1.0 2019.11 (Linaro glibc 2.30), Thumb-2 `-O2`.
- The binary has symbols (214 named functions); its debug info covers only crt / libgcc objects, the fwfs code has no
  DWARF [R `readelf`].

### 1.4 Defects of the core source (in both builds) [R source]

None of them affects reading an image.
- `fwfs_tftl_deinit` frees its two arrays only when the pointers are NULL (inverted test — never frees).
- The "Found in use or abandoned subblock … after free subblock" check in `fwfs_tftl_alloc` reads subblock `j` instead
  of `m` (never fires).
- `fwfs_tftl_alloc` ignores the return of `cfg->erase`.
- `fwfs_superblock_fromle32` does not convert the two TFTL words (harmless on little-endian ARM).
- `fwfs_tftl_seed` is declared, never defined.
- `fwfs_ctz_extend` and `fwfs_file_relocate` ignore the result of programming the TFTL header (a failed header write
  goes unnoticed until a debug traversal).

## 2. On-flash format [R source, image]

All multi-byte fields are little-endian except the metadata **tags** and the **TFTL header word**, which are big-endian.
Where the text says "littlefs" it means littlefs v2.2.1 / disk format v2.0 behaviour that fwfs keeps unchanged.

### 2.1 Geometry and addressing [R source `fwfs_tftl.c`, image]

- `block_size` = the device's erase block = 64 KiB on MISC; `subblock_size` = 32 KiB; 2 subblocks per block.
- TFTL is on iff `subblock_size > 0 && subblock_size != block_size` (`fwfs_tftl_isenabled`). With TFTL off the format is
  plain littlefs v2.0 with block indexes; everything below describes TFTL on, as on MISC.
- Every block reference stored on flash or used in RAM is a **LEB** word:
  `magic<<28 | subblock_id<<22 | block_id` (4 / 6 / 22 bits). Magics: **0xA** metadata (a whole erase block,
  subblock_id 0), **0xB** CTZ data (one subblock), 0xC "null" (RAM only: `0xC0000000` replaces littlefs's
  `0xFFFFFFFF` null), 0xD "inline" (RAM only: `0xD0000000`). The superblock pair is `{0xA0000000, 0xA0000001}`
  (blocks 0 and 1). Flash offset = `block_id*block_size` (+ `subblock_id*subblock_size` for 0xB).
- On flash only 0xA and 0xB values occur (directory pairs, tails, CTZ heads and skip pointers) [R image]. littlefs's
  null pair `{-1,-1}` is never written: a soft tail is written only when not null.
- Erase: metadata LEBs erase their whole block; erasing a 0xB LEB is a no-op — data subblocks are erased only when TFTL
  hands out a fresh whole block for data (§2.7).

### 2.2 Metadata blocks and pairs [R source; littlefs v2 semantics unchanged]

A directory is a chain of metadata **pairs** (two whole blocks). A block of a pair holds `rev` (le32 revision) followed
by a log of **commits**. The active block is the one with the newer revision (sequence compare: `(int32)(a−b) > 0`)
that has at least one valid commit; the other one is the previous state.
- **Tag** (32 bit, stored big-endian, XOR-ed with the previous tag; the first tag of a block is XOR-ed with
  `0xFFFFFFFF`): `bit31` invalid flag (0 = valid) · `bits30..20` type3 (11 bits; `type1` = bits 30..28) · `bits19..10`
  id (0x3FF = not tied to an entry) · `bits9..0` size of the data that follows (0x3FF = "deleted", no data). Data
  length = `size`, except `size==0x3FF` → 0. Tag and data are packed with no alignment.
- **Types** (type3): `0x001` REG name, `0x002` DIR name, `0x0FF` superblock name, `0x200` DIRSTRUCT (8 B: pair),
  `0x201` INLINESTRUCT (file content in the tag, 0–1022 B), `0x202` CTZSTRUCT (12 B, §2.4), `0x300+n` user attribute
  n, `0x401` CREATE (splice +1 at id), `0x4FF` DELETE (splice −1 at id), `0x500+x` CRC, `0x600` SOFTTAIL, `0x601`
  HARDTAIL (8 B: next pair), `0x7FF` MOVESTATE (12 B gstate delta). The internal "from" types (`0x101` FROM_MOVE,
  `0x102` FROM_USERATTRS) never reach flash.
- **Commit**: tags, then one or more CRC tags. The CRC is littlefs's CRC-32 (reflected, poly 0xEDB88320, init
  0xFFFFFFFF, **no final XOR**; 4-bit table in `fwfs_crc`), run over `rev` (first commit of the block only) and every tag
  and data byte of the commit up to and including the CRC tag word; the le32 value follows the tag. The CRC tag's `size`
  covers itself + padding: commits are padded to the next `prog_size` (2048) boundary; since a tag size is at most
  0x3FE, longer padding is written as several CRC tags, each with its own CRC (reset to 0xFFFFFFFF), the last one ending
  exactly on the boundary. Padding bytes are covered by no CRC and are left as 0xFF. The CRC type's low bit
  (`chunk&1`) tells the reader to flip bit31 of the expected next tag: the writer sets it when the first word of the
  next program unit is *not* erased, so that word reads as an invalid tag and ends the log.
- A reader walks tags until an invalid tag, a tag running past the block, or a CRC mismatch; only commits closed by a
  good CRC count. `dir->erased` (append allowed) needs, besides littlefs's "last tag was CRC and the offset is
  prog-aligned", the raw next word's bit31 = 1 (i.e. erased flash) — a SigmaStar addition.
- **Entries**: `id` indexes the directory's entries in **name order** (byte compare, then length); id 0 of the root
  pair at blocks 0/1 is the superblock. A NAME tag with an id ≥ the current count creates the entry (compaction writes
  no CREATE tags); CREATE / DELETE shift later ids. Later tags override earlier ones of the same type and id.
- **Tails**: a SOFTTAIL links to the next pair of the global metadata list (another directory); a HARDTAIL continues
  the *same* directory in another pair (split directory). `tail` pair words are LEBs.
- **gstate**: the MOVESTATE deltas of all pairs XOR together into a 12-byte global state `{tag, pair[2]}` (pending
  move: `type1` set, id = moved entry, in `pair`; orphan count in `size`). Fully zero = consistent.
- **Compaction** (when a pair's active block is not appendable or is full): `rev+1`, erase the *other* block, write all
  live tags, tail, gstate delta, CRC; the pair is then swapped. A pair that fails or reaches
  `rev % ((block_cycles+1)|1) == 0` (501 for `block_cycles=500`) is **relocated** to a newly allocated block, except the
  superblock pair, which is instead "expanded" (root moved into a new pair, blocks 0/1 keep only the superblock) when
  the filesystem uses less than half the blocks; a superblock pair that cannot be written freezes the filesystem
  (`Superblock … has become unwritable`, -ENOSPC).
- **Directory split**: a compaction that would need more than `min(block_size−36, block_size/2)` = 32 KiB of tags, or
  ≥ 0xFF entries, splits into a new pair joined by a HARDTAIL.

### 2.3 Superblock [R source, image]

Entry id 0 of the pair at blocks 0/1: name tag `0x0FF` with the 10 bytes `firmwarefs` (littlefs: 8 bytes `littlefs`)
and an INLINESTRUCT of 32 bytes (8 × le32):

| offset | field | value written / checked |
|---|---|---|
| +0x00 | `version` | 0x00020000 |
| +0x04 | `block_size` | |
| +0x08 | `block_count` | |
| +0x0C | `name_max` | 255 |
| +0x10 | `file_max` | 0x7FFFFFFF |
| +0x14 | `attr_max` | 1022 |
| +0x18 | `tftl_version` | 0x00040000 |
| +0x1C | `subblock_size` | |

Mount follows the tail list (soft and hard tails) from `{b0,b1}`, looking for the name tag in each pair and XOR-ing up
the gstate. It checks: disk major == 2 and minor ≤ 0; `name_max` / `file_max` / `attr_max` ≤ the configured ones;
TFTL major == 4 and minor ≤ 0; `subblock_size` **equal** to the configured one (else `Unsupported subblock_size`). It
does **not** check `block_size` or `block_count` (littlefs 2.2.1 did not either) — the configuration given to mount is
what the allocator uses [R source; the binary's `fwfs_mount` has no further check: same anchors, no other error
strings].

### 2.4 Files: inline or CTZ [R source]

- **Inline**: the content sits in an INLINESTRUCT tag of the directory (a 0-byte file too). A file stays inline while
  its size ≤ `min(0x3FE, cache_size, subblock_size/8)` = **1022 B** with the MISC geometry (in both the Linux and the
  U-Boot configuration, §4.2, §5); growing past that moves it out (`fwfs_file_outline`).
- **CTZSTRUCT** (12 B): `head` (LEB of the *last* data subblock), `size` (bytes), `fid` (file id, §2.6). littlefs's is
  8 bytes without `fid`.
- Data lives in a littlefs **CTZ skip list** in which each "block" is one 32 KiB subblock (`B = subblock_size − 8`
  usable bytes after the TFTL header). The subblock with index *n* (counting from 0 at the file start) holds, after its
  8-byte header, `ctz(n)+1` le32 back pointers (none for n = 0): pointer *k* → the subblock of index `n − 2^k`; then
  data. So subblock 0 holds `B` = 32760 data bytes, subblock *n* holds `B − 4·(ctz(n)+1)`. Position → (index, offset):
  littlefs `lfs_ctz_index` with `b = B − 8` = 32752: `i = pos / b; if i: i = (pos − 4·(popc(i−1)+2)) / b,
  off = pos − b·i − 4·popc(i)`; then the physical offset in the subblock = `8 + off` (the source adds `header_size` to
  the result).
- The bytes after the last data byte of the head subblock are whatever was there (0xFF on a file written by the
  camera, 0x00 in the factory image, §3.3).

### 2.5 TFTL data-subblock header [R source `fwfs_tftl_header_fill`, image]

8 bytes at the start of every 0xB subblock: word 0 **big-endian** `(fid & 0x7FFF0000) | (index & 0xFFFF)` with bit31
forced 0; word 1 reserved, written 0. The header is programmed (with read-back verify) right after the subblock is
allocated, before the skip pointers and data. Readers use it only for checks: with `debug ≥ 1` the traversal verifies
fid / index (`Recorded fid … doesn't match`); the allocator uses its bit31 to tell a partly written subblock (0 → used,
"abandoned") from an erased one (1 → free).

### 2.6 fid [R source `fwfs_get_fid`, `fwfs_dir_fetchmatchraw`]

`fid` = the filesystem `seed` at the moment the file was created (or turned from inline into CTZ); the seed is the XOR
of the CRCs of every commit fetched since mount (each fetch counts again). Only bits 30..16 are recorded in subblock
headers. It serves only to tell which file a subblock belongs to; nothing looks files up by fid.

### 2.7 TFTL allocation and the subblock state map (RAM only) [R source `fwfs_tftl.c`]

Nothing of TFTL's state is stored on flash besides the headers; it is rebuilt by traversing the filesystem.
- 4 bits per subblock: FREE 0, CTZ 1, METADATA 2, ABANDON 3, UNKNOWN 4 (INVALID 0xF = error result).
- After each traversal ("snapshot") the states advance: in use → CTZ / METADATA; previously used but no longer
  referenced → ABANDON (a subblock cannot be rewritten without erasing its block); on the first pass every FREE
  subblock of a block that holds CTZ data becomes UNKNOWN (it could be a half-written subblock from before a power cut).
- Metadata allocation (`ctz=false`): a whole free block from the littlefs lookahead allocator, where "free" means all
  its subblocks are FREE / ABANDON / UNKNOWN; the lookahead starts at `seed % block_count`.
- Data allocation (`ctz=true`): first a free subblock inside a block that already holds CTZ data (UNKNOWN ones are
  probed: header bit31 = 1 → FREE, else ABANDON); only if there is none, a whole free block is **erased** and its
  subblock 0 used. So a block's subblocks are filled in order, and a block is erased only when every subblock in it is
  dead.
- `fs_size` (for the superblock expansion rule, §2.2, and `statfs`, §4.3) counts blocks with any non-FREE subblock.

### 2.8 Differences from littlefs v2.2.1, complete list [R]

On flash: LEB block numbers (§2.1); the 8-byte header in data subblocks (§2.5) and therefore data blocks of
`subblock_size`, not `block_size`; CTZSTRUCT 12 bytes with `fid` (§2.4); superblock name 10 bytes `firmwarefs`;
superblock struct 32 bytes with `tftl_version`, `subblock_size` (§2.3); the "erased" test also looks at raw bit31
(§2.2). Tags, CRC, commits, ids, tails and gstate are the littlefs v2.0 disk format unchanged. These differences are why
stock `littlefs-python` mounts the MISC image once the magic is renamed but cannot read the directory
(LFS_ERR_CORRUPT).

Behaviour only: a read-cache pool of up to 4 (8 in the oi header) LRU caches; separate file read / program caches of
`file_cache_size`; read-bypass alignment for the 8-byte header; `fwfs_alloc_reset` fixed to `seed % block_count`
(littlefs 2.2.1 had `% block_size`); truncate looks up `size−1`; `debug` levels (1: cache-overlap and subblock-header
checks, 2: also a blank check before every program and of every allocated subblock); `dump_on_full`; the
`folder_compress_policy` file, the `compact` / `compact_on_mount` magics (§2.9); the `bbt` callback for NAND bad blocks
(§4.4).

### 2.9 Features in the code that MISC does not use [R]

- **`folder_compress_policy`** [R source]: if a directory contains a file of that name whose content starts with
  `Aggressive:This policy will reduce nand lifetime!!!!`, every commit to that directory is done as a full compaction
  (`force_compact`). MISC has no such file → unused.
- **`compact = 0x434F4D50` ('COMP')**: the same forcing for every directory — documented "should be used by mkfwfs
  program only"; `/bin/fwfs` never sets it (§4.2) → unused.
- **`compact_on_mount = 0x434D4E54` ('CMNT')**: at mount, compact every pair whose log could shrink by at least one
  program unit; not settable from `/bin/fwfs` → unused.
- User attributes (`fwfs_getattr` / `setattr` / `removeattr`): in the core; the FUSE front end has no xattr operations
  (§4.3) → unreachable on the camera; MISC holds none.
- Directories: supported; MISC has none.
- Migration from firmwarefs v1: compiled out of `/bin/fwfs` (`--migrate` is accepted but fails, §4.2); present in oi
  under `FWFS_MIGRATE`.
- `debug` ≥ 1 checks and `dump_on_full`: off on the camera (rcS passes no `--debug`).

## 3. The MISC image of 2026-10-16 [R: tag dump]
<!-- ev:misc-firmwarefs.3 -->

### 3.1 Map (partition offset 0xE90000, 10 × 64 KiB)

| block | subblock 0 (0x…0000) | subblock 1 (0x…8000) |
|---|---|---|
| 0 | root / superblock pair, **active**: rev 11, one commit 0x004–0x800, rest 0xFF | (part of the metadata block) all 0xFF |
| 1 | root / superblock pair, previous: rev 10, one commit 0x004–0x800, rest of the block **0x00** | (metadata) 0x00 |
| 2 | `config.ini` index 2 (head), hdr `00000002` | `PQ.bin` index 0 (head), hdr `00000000` |
| 3 | `sigmastar1024_600.jpg` index 0, hdr `27950000` | `sigmastar1024_600.jpg` index 1 (head), hdr `27950001` |
| 4 | `upgrade.jpg` index 0 (head), hdr `27950000` | unused, all **0x00** |
| 5 | `config.ini` index 0, hdr `00000000` | `config.ini` index 1, hdr `00000001` |
| 6–9 | erased (0xFF) | erased |

### 3.2 Superblock and root directory
<!-- ev:misc-firmwarefs.3.2 -->

- Superblock: version 0x00020000, block_size 65536, **block_count 6**, name_max 255, file_max 2147483647, attr_max 1022,
  tftl_version 0x00040000, subblock_size 32768. No tail, gstate zero (no pending move, no orphans).
- Root entries — ids are in name order:

| id | name | kind | struct | data subblocks (index → LEB → flash) |
|---|---|---|---|---|
| 0 | `firmwarefs` | superblock | inline 32 B | — |
| 1 | `PQ.bin` | REG | CTZ head `0xB0400002` size 25892 fid 0 | 0 → b2.s1 → data at 0xEB8008 |
| 2 | `config.ini` | REG | CTZ head `0xB0000002` size 76005 fid 0 | 0 → b5.s0 0xEE0008; 1 → b5.s1 0xEE800C; 2 → b2.s0 0xEB0010 |
| 3 | `sigmastar1024_600.jpg` | REG | CTZ head `0xB0400003` size 49873 fid `0xA79530E5` | 0 → b3.s0 0xEC0008; 1 → b3.s1 0xEC800C |
| 4 | `upgrade.jpg` | REG | CTZ head `0xB0000004` size 17833 fid `0xA79530E5` | 0 → b4.s0 0xED0008 |
| 5 | `ws.txt` | REG | inline, 0 B | — |

- The flash offsets of `config.ini`'s data (8 / 12 / 16 bytes into its three subblocks) are the 8-byte TFTL header
  followed by 0, 1 and 2 CTZ skip pointers (indexes 0, 1, 2).
- The extracted files: `config.ini` is byte-identical to
  `/config/config.ini` (= `/config/config.ini`); `sigmastar1024_600.jpg` and `upgrade.jpg` are
  byte-identical to `/customer/misc/*`; `PQ.bin` has no other copy in the dump (MISC only).
- Every commit's CRC verifies. Block 1 (rev 10) holds the four factory files; block 0 (rev 11) holds the same plus
  `ws.txt`. prog_uvc creates `ws.txt` (`fopen("/misc/ws.txt","ab+")`); block 1 not being appendable (zeros after its
  commit), the core compacted the root into block 0 with rev 11. So block 0 was rewritten on the camera, and what the
  factory block 0 held is gone ([I] an older revision, ≤ 9, of the same root).
- Tag order in the compacted log: superblock, then ids 2, 1, 3, 4, 5 (littlefs's traversal order; nothing in the format
  depends on it).

### 3.3 Factory image traits and what the camera's TFTL makes of them
<!-- ev:misc-firmwarefs.3.3 -->

- The factory image was not produced by littlefs on flash: unused space is **0x00**, not 0xFF (the rest of block 1,
  b1.s1, the tails of all head subblocks, the whole of b4.s1). Both `fid 0` (seed 0, a freshly formatted host image)
  and `block_count 6` (just enough for the content) point to SigmaStar's host tool `mkfwfs` making a 6-block image,
  padded with zeros to the 10-block partition [I; the source documents the `compact` field as for mkfwfs, §2.9]. No
  `mkfwfs` is in the dump or in oi [R].
- At the first allocation after mount TFTL sees: blocks 0, 1 METADATA; b2.s0/s1, b3.s0/s1, b4.s0, b5.s0/s1 CTZ;
  **b4.s1 UNKNOWN → header word 0x00000000 has bit31 = 0 → ABANDON** (not used until all of block 4 is freed and
  erased); blocks 6–9 FREE. Free capacity: 4 whole blocks [I from the §2.7 rules applied to the image].
- Block 1's zero tail means the next root commit after a fresh flash compacts instead of appending (as happened with
  `ws.txt`) [R §2.2 rules + image].
- The daemon is given `--block_count=10` (§4.2), and mount does not compare it with the superblock (§2.3), so the
  allocator uses blocks 6–9 although the superblock records 6. A reader configured from the superblock (6 blocks) reads
  everything only as long as nothing was allocated in blocks 6–9 [I from §2.3, §2.7].
- Live state [R]: `/misc` is mounted `fuse.fwfs rw,nosuid,nodev,
  relatime,user_id=0,group_id=0`; `df` shows **640.0K total, 384.0K used, 256.0K free (60 %)** = 10 blocks, 6 counted
  as used (blocks 0–5, b4 included because of its CTZ and abandoned subblocks), 4 free — the state of §3.1 and above.
  `/proc/mtd`: `mtd5: 000a0000 00010000 "MISC"`.

## 4. `/bin/fwfs` — the FUSE daemon [R: disassembly]
<!-- ev:misc-firmwarefs.4 -->

### 4.1 Binary and process

- 68.8 KB ELF (25.6 KB `.text`), ARM Thumb-2 hard-float, needs `libfuse.so.2` (libfuse 2.9.9 on the camera; FUSE 2.6+
  API, `fuse_main_real`, `fuse_opt_parse`) and glibc. Three glue files — `fwfs_fuse.c`, `fwfs_fuse_bd.c`,
  `fwfs_fuse_mtd.c` — around the core (§1.2).
- Global state at 0x29278: `+0` device path (strdup), `+4` the `fwfs_t` (0xE0 B), `+0xE4` the `struct fwfs_config`
  (options are parsed straight into it), `+0x154` format flag, `+0x155` migrate flag. One filesystem per process.
- rcS (`/etc/init.d/rcS` l.24) runs it in the foreground of the boot script:
  `fwfs --block_size=65536 --subblock_size=32768 --block_cycles=500 --read_size=2048 --prog_size=2048
  --cache_size=32768 --file_cache_size=32768 --cache_pool_size=2 --block_count=10 --lookahead_size=8 /dev/mtd5 /misc`.
  `fuse_main` daemonizes (no `-f`) after mounting, so rcS continues with the filesystem mounted, and the daemon outlives
  the boot (it serves `/misc`). `main` always adds **`-s`** (single-threaded FUSE loop), so all operations are
  serialized; the core is not thread-safe and needs no locks this way.
- `/misc` must be mounted before `mi_sys.ko` is loaded: mi_sys runs its usermode helper `/config/config_tool` (module
  parameter `default_config_path`, default `/config/config_tool`) at insmod, once as `load_config /misc/config.ini
  /misc/PQConfig.ini` (MISC holds no `PQConfig.ini`), and the helper reads `/misc/config.ini` into
  `/proc/mi_modules/common/*` (config_tool.md §1.3). The rcS order guarantees it [R boot log, parameter list].

### 4.2 Command line

`fwfs [options] device mountpoint` (help text in the binary). fuse_opt table (template → field of `fwfs_config`):

| option | field (cfg offset) | default if 0 after parsing (`fwfs_fuse_defaults`, then the MTD probe) | camera |
|---|---|---|---|
| `-b=` / `--block_size=` | block_size (+0x28) | MTD `erasesize`; block device: `BLKSSZGET` | 65536 |
| `--block_count=` | block_count (+0x2C) | MTD `size/block_size`; block device: `BLKGETSIZE` | 10 |
| `--subblock_size=` | subblock_size (+0x1C) | = block_size (→ TFTL off) | 32768 |
| `--block_cycles=` | block_cycles (+0x30) | 512 | 500 |
| `--read_size=` | read_size (+0x20) | MTD `writesize`, else block_size | 2048 |
| `--prog_size=` | prog_size (+0x24) | MTD `writesize`, else block_size | 2048 |
| `--cache_size=` | cache_size (+0x34) | block_size | 32768 |
| `--cache_pool_size=` | cache_pool_size (+0x3C) | 4 | 2 |
| `--file_cache_size=` | file_cache_size (+0x38) | **bug**: when 0, the code sets `cache_size = subblock_size` and leaves file_cache_size 0 (the help says "block_size") → `fwfs_init`'s `subblock_size % file_cache_size` divides by 0 | 32768 |
| `--lookahead_size=` | lookahead_size (+0x40) | 8192 (bytes of bitmap) | 8 |
| `--debug=` | debug (+0x18) | 0 | — |
| `--name_max=`, `--file_max=`, `--attr_max=` | +0x64, +0x68, +0x6C | core defaults 255 / 2^31−1 / 1022 | — |
| `--format` | key 2 → format flag | — | — |
| `--migrate` | key 3 → migrate flag | — | — |
| `-V`/`--version`, `-h`/`--help` | keys 1 / 0 | print versions / help (`fuse_main` with `--version` / `-ho`), exit | — |

- No device → `missing device parameter`, exit 1.
- `--format`: probe the device, `fwfs_format` with the config, exit (an error is printed as
  `fwfs_fuse.c:575:error: <strerror>`).
- `--migrate` (without `--format`): always fails with EOPNOTSUPP (95) — migration is compiled out and
  `fwfs_fuse_migrate` (returns −95) is never even called.
- Otherwise mount (`fwfs_fuse_mount` = probe + defaults + `fwfs_mount`); on error print strerror and exit with that
  errno; on success `fuse_main_real(args + "-s", ops, 0xB4)`, unmount on a non-zero return.
- Not settable from the command line: `compact`, `compact_on_mount`, `dump_on_full`, static buffers [R opt table].
- MISC mounts only with exactly the rcS geometry: with `--subblock_size` omitted or equal to the block size the daemon
  expects the plain-littlefs layout and rejects MISC (`Unsupported subblock_size`, §2.3), and without an explicit
  `--file_cache_size` it divides by zero (above).

### 4.3 FUSE operations (`fwfs_fuse_ops` @0x290B4, FUSE 2.9 `struct fuse_operations`)

All handlers return the core's result directly (core errors are already `−errno`). File handles are `malloc`ed
`fwfs_file_t` (0x64 B) / `fwfs_dir_t` (0x38 B) stored in `fi->fh`.

| op | does |
|---|---|
| `init` | sets FUSE_CAP_ATOMIC_O_TRUNC and FUSE_CAP_BIG_WRITES (0x28) in `conn->want` |
| `destroy` | `fwfs_unmount`, close the device (MTD: also free the bad-block map) |
| `getattr` | `fwfs_stat` → stat: mode `0777` + S_IFREG/S_IFDIR, size; uid/gid/nlink/times all 0 (files show date 1970) |
| `fgetattr` | regular file, size = `fwfs_file_size` (returns 0 even if that fails) |
| `access` | `fwfs_stat` only (existence; the mask is ignored) |
| `statfs` | bsize = frsize = block_size; blocks = block_count; bfree = bavail = block_count − `fwfs_fs_size` (TFTL: blocks with any non-free subblock); namemax = configured name_max (0 on the camera); files/ffree 0 |
| `open` | flags: O_RDONLY→FWFS_O_RDONLY(1), O_WRONLY→2, O_RDWR→3, (3→0); O_CREAT→0x100, O_EXCL→0x200, O_TRUNC→0x400, O_APPEND→0x800; `fwfs_file_open` |
| `create` | `open` (mode ignored) then `fwfs_file_sync` — the new entry is committed at once |
| `read` / `write` | if `fwfs_file_tell` ≠ offset → `fwfs_file_seek(SEEK_SET)`; then `fwfs_file_read/write` (bytes or −errno). Writes stay in the file cache until flush/sync/close or the cache fills |
| `flush`, `fsync` | `fwfs_file_sync` (`flush` is called on every `close()` of a descriptor) |
| `release` | `fwfs_file_close` (syncs), free |
| `truncate` | `fwfs_file_open(path, WRONLY)`, `fwfs_file_truncate`, close — on a truncate error the stack-allocated file is **not closed** and stays linked in the core's open-file list (dangling pointer, [I] corrupts later operations) |
| `ftruncate` | `fwfs_file_truncate` on the open handle |
| `opendir` / `readdir` / `releasedir` | `fwfs_dir_open` / loop `fwfs_dir_read` → filler (the core yields `.` and `..` first), offset 0 (whole listing at once) / `fwfs_dir_close` |
| `mkdir` | `fwfs_mkdir` (mode ignored) |
| `unlink`, `rmdir` | both `fwfs_remove` (a directory must be empty) |
| `rename` | `fwfs_rename` (replaces a destination of the same type; directories must be empty) |
| `chmod`, `utimens` | no-op, return 0 |
| `chown`, `link`, `symlink`, `mknod` | return −1 (EPERM) |
| not provided | `readlink`, xattr ops (so the core's user attributes are unreachable), `fsyncdir`, `lock`, `flock`, `fallocate`, `ioctl`, `poll`, `bmap` |

So a write to `/misc` reaches the flash at `close()` / `fsync()`, or earlier when the file cache fills.

### 4.4 Device back ends
<!-- ev:misc-firmwarefs.4.4 -->

`fwfs_is_block(dev)`: a block device whose major is not 31 (mtdblock) → "bd" back end; anything else (the camera's
`/dev/mtd5` char device, an mtdblock node, a regular file) → MTD back end, which needs `MEMGETINFO` — so a plain image
file cannot be mounted by this binary.
- **MTD** (`fwfs_fuse_mtd.c`): `open(dev, O_RDWR)`; `MEMGETINFO` (`0x80204D01`) fills unset `read_size` / `prog_size`
  (= writesize; 1 on NOR, hence the explicit 2048 in rcS), `block_size` (= erasesize 64 KiB), `block_count`
  (= size / block_size). read / prog = `lseek64(block*block_size + off)` + `read` / `write` on the char device (the
  kernel MTD layer programs NOR synchronously); erase = `MEMERASE` (`0x40084D02`, `{block*block_size, block_size}`);
  sync = nothing. Block numbers past `block_count` → assert / abort.
- **NAND bad-block map** (unused on this NOR camera) [R `fwfs_fuse_mtd_create` @0x11120, `fwfs_fuse_mtd_bbt`
  @0x11024]: only when `MEMGETINFO.type` is 4 (`MTD_NANDFLASH`) or 8 (`MTD_MLCNANDFLASH`). A probe call of the
  SigmaStar ioctl `0x80084D33` for block 0 must return `>= 0`; then every block is queried, blocks with flag 0 are
  printed (`"%s: block 0x%x is a factory bad block (0x%x)\n"`) and skipped in a logical → physical map applied to every
  read / prog / erase, the tail is filled with `0xFFFFFFFF` (→ `-EIO` on access); the map is freed again if no block was
  bad. fwfs never sends the companion `MEMSETBBTBLKINFO`. The camera's flash is type 3 (NOR), so the map is never built.
- **bd** (`fwfs_fuse_bd.c`): `open(O_RDWR)`, `BLKSSZGET` (0x1268) → block_size, `BLKGETSIZE` (0x1260) → block_count,
  lseek + read / write, erase = no-op, sync = `fsync`. Unused on the camera.

**The ioctl `0x80084D33`** = `_IOR('M', 51 (0x33), struct sstar_bbt_info_user)` = **`MEMGETBBTBLKINFO`** ("Get an
block info in bbt: good block, factory bad block or running time bad block") [S: i6c kernel tree; struct l.49]. Argument, 8 bytes:

| offset | field | direction | meaning |
|---|---|---|---|
| +0 | `__u32 u32_offset` | in | byte offset of the erase block inside the MTD partition (fwfs passes `block × block_size`) |
| +4 | `__u8 u8_type` | out | `0x0F` good block, `0x00` factory bad block |
| +5 | `__u8 padding[3]` | — | untouched by the kernel; fwfs ignores it (reads `word1 & 0xFF`) |

In the i6c (5.10.61) tree [S: `drivers/mtd/mtdchar.c` l.898–921] the kernel copies the struct in, calls
`mtd_block_isbad(mtd, u32_offset)`, sets `u8_type = 0x00` (`FACTORY_BAD_BLOCK`) for a result `> 0` and `0x0F`
(`GOOD_BLOCK`) for a result `<= 0` — an error from `mtd_block_isbad` (offset out of range → `-EINVAL`) is reported as a
good block, not as an ioctl error — copies the struct back and returns 0; the command is among the "safe" ones
(allowed on a read-only open) [S: l.657]. The third state of the header comment ("running time bad block") has no
value and no code there; `MEMSETBBTBLKINFO` = `_IOWR('M', 52, …)` = `0xC0084D34` is defined [S: l.217] but has no
`case` in `mtdchar_ioctl` (→ `-ENOTTY`). Both are SigmaStar additions, absent from vanilla 4.9 and from the i6e
(4.9.84) tree [S].
**The camera's kernel does not implement it** [R]: in its `mtdchar_ioctl` (kernel `Image`, `ksyms` 0xc01b661c) the
`cmd` compare tree, run from its first compare (0xc01b6684), sends `0x80084D33` and `0xC0084D34` to 0xc01b66f8 =
`mvn r4, #24` → **`-ENOTTY`**, while `MEMGETINFO` (0x80204D01), `MEMERASE` (0x40084D02) and `MEMGETBADBLOCK`
(0x40084D0B) leave the tree into their case bodies; the function's literal pool holds no `0x…4D33` / `0x…4D34` word,
and the bytes of `0x80084D33` occur nowhere in `Image` or the stock `uboot.bin`. The 4.9.227 kernel follows the
i6e / vanilla command set here. In the whole dump (rootfs, miservice, customer) the constant's bytes occur only in
`/bin/fwfs` [R byte search; a value built by `movw/movt` would not be found this way — fwfs loads it from a
literal; the other files were not searched for that form [I]]. So on this kernel a NAND build of the same rootfs would get `-ENOTTY` at the probe and run without a bad-block map; the
NAND path of fwfs works only with an i6c-style kernel [I].

### 4.5 Write path and wear on MISC [R core rules §2; the numbers [I] from them]

- A file write ends, at sync / close, as: data → newly allocated subblocks (copy-on-write; for files > 1022 B), then
  one metadata commit to the root pair (new CTZ struct or inline data). Each commit takes at least one 2 KiB program
  unit, so the root's active 64 KiB block takes about 31 commits after a compaction before the next compaction erases
  the other block of the pair.
- The root / superblock pair (blocks 0/1) takes all metadata wear: every 501st compaction it would be relocated, but
  the superblock pair cannot move; it can only "expand" (move the root out) while fewer than half the blocks are in
  use — MISC has 6 of 10 in use, so it keeps compacting in place [I]. Data subblocks rotate through free blocks
  starting from `seed % block_count`.
- Writers of MISC on this camera: prog_uvc creates `ws.txt` once and reads its crop mode from the first character
  (prog_uvc.md §5.2; no writer of its content is recorded there); opening an existing file with `O_CREAT` writes nothing.
  kjd_lvgl replaces `/misc/sigmastar1024_600.jpg` from the SD card (kjd_lvgl.md §3.1) — one CTZ rewrite + one commit.
  Readers: U-Boot `bootlogo` (§5), config_tool through mi_sys (`/misc/config.ini`, §4.1).
- Power loss: littlefs's guarantees hold (a commit is valid only with its CRC; CTZ data is written before the commit
  that references it); TFTL recovers half-written subblocks as ABANDON through the header bit31 test (§2.7).

## 5. U-Boot's firmwarefs [R: oi `fs/firmwarefs/firmwarefs.c`]
<!-- ev:misc-firmwarefs.5 -->

- Core: the oi text exactly (§1.2); asserts only print `%s:%d:assert fail:%s` and continue.
- Mount (`firmwarefs_mount`, 0x23e69144): SPI-NOR probe (`spi_flash_probe(0, 0, 1000000, 3)` (0x23e198a4), [I]
  `spi_flash_probe(bus, cs, speed, mode)`), the partition range by name from MXPT (`mxpt_find_partition` (0x23e1dcbc) fills start block,
  block count, page size), then a configuration of read / prog / cache / file_cache 2048, subblock 32768, block_size
  65536, cache_pool 4, block_cycles 500, lookahead 8, and block_count and block_offset from the partition (MISC → 10
  blocks at block 0xE9). The block-device callbacks read with `spi_flash_read`, program with `spi_flash_write`, erase
  64 KiB with `spi_flash_erase`; sync is a no-op.
- Glue revision: the stock configuration struct has one field fewer than the oi header — no `bbt` member, so
  `subblock_size` is at +0x18 and `block_count` at +0x28 — a slightly older glue revision; oi's `firmwarefs.c` adds NAND
  bad-block mapping and an `open` flag translation.
- API (`include/firmwarefs.h`): `firmwarefs_mount(partition, mnt_path)`, `_unmount`, `_open(name, flags, mode)`
  (POSIX O_* → FWFS_O_*; `O_RDONLY` is 0, so "read-only" is the fallback when no flag maps), `_close`, `_read`,
  `_write`, `_lseek` (oi source bug: `SEEK_CUR` falls through to `SEEK_END` and returns the size). One global mount.
- Users in the stock U-Boot: `bootlogo` (mounts `MISC`, reads `config.ini`, the logo JPEG and optionally the PQ file —
  uboot.md §5.2) and the commands `ini_get_filesize`, `ini_create_file`, `ini_put_buf`, `ini_write_file`,
  `ini_get_str`, `ini_get_int`, `ini_get_str_array`, `ini_get_buf` (each mounts, works, unmounts; syntax and effects in
  uboot.md §9.2). `ini_create_file <part> <file> <text>` opens `O_RDWR|O_CREAT` and writes, and `ini_put_buf` /
  `ini_write_file` write into existing files, so U-Boot can also **write** MISC; `bootcmd` runs only `bootlogo`, so the
  writers are unused on this camera.
- Both sides use the same core and the same on-flash geometry, so a file written by Linux is readable by U-Boot and
  vice versa. The cache sizes differ (Linux 32 KiB and pool 2, U-Boot 2 KiB and pool 4) without affecting the format
  or the inline limit (§2.4) [R source]. The stock U-Boot reads MISC only through firmwarefs [I from the users
  above].

## 6. Not known

- The 2 extra lines of `/bin/fwfs`'s `fwfs.c` against the oi / stock U-Boot text between `fwfs_dir_fetchmatchraw`
  and `fwfs_dir_compact` (§1.2; [I] without code): not identified; a build of the source with the SigmaStar gcc
  9.1.0 2019.11 ARM toolchain would allow an instruction-stream comparison, and that toolchain is not on hand.
- What the factory block 0 held before the camera rewrote it (§3.2): overwritten in the only dump; another unit's
  factory dump would show it.
- The host tool (`mkfwfs`, [I]) that produced the zero-padded 6-block image (§3.3): in no source tree on hand; the
  vendor SDK would be needed.
