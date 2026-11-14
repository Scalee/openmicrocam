# ipu-firmware-2-engines — `ipu_firmware.bin` part 2: interpreter and op resolver, IPU200M HAL and compute engines, operator kernels, the Anjoy build
<!-- ev:ipu-firmware-2-engines -->

`ipu_firmware.bin` is the RV32IMC program of the RISC-V core inside the NPU ("IPU", "DLA") of the SigmaStar M6 SoC on
this camera. Part 1, `ipu-firmware-1-runtime-model.md`, holds the image, the runtime, the CCIF server, the host
commands, the SGSI model file, the option tables and the offline command block with its wire format (framing, control
words, delta form, engine instruction headers, the DMA / conv / output-stage / SR field layouts, the decoder registers).
This document is part 2:

- **the interpreter and the op resolver** — registration records, the 96 registered operators, how a subgraph is built
  into op nodes and domains and how an invoke runs them (§2);
- **the IPU200M HAL** — register-block descriptors, EMI ↔ L2 DMA, CMDQ, BDMA, decoder replay of the offline block (§3);
- **the compute engines** — the L2 memory and its VR / MR buffers (§4), the SR register file (§5), the convolution /
  pooling engine with the tile plan it executes (§6), the vector unit (§7), the LUT unit (§8);
- **the operator kernels** — the shared numeric routines (§9) and every registered kernel: checks, option fields, CPU
  arithmetic, tile planning and the engine-instruction programme it emits (§10);
- **the Anjoy build** — how the other build on hand differs (§11);
- what the firmware does not show: the engines' own arithmetic (§12).

Tags: **[R]** read from the binary (`ipu_firmware.bin`, its decompile, disassembly, strings) or from a file named, or
from an emulation run of the binary's own code; **[S]** stated by a source (the public TensorFlow Lite schema and
reference semantics, the R-FCN definition, the Caffe RoiPool rule — each named where used); **[I]** inference. A heading
tagged [R] means every statement under it is [R] unless marked otherwise. Function names taken from the `__func__`
strings of the asserts and log calls inside a function are [R]; field names in plain text that are not in code font are
chosen by position or use and are [I]; other functions get a descriptive name of ours [I], given with the address. Addresses are RISC-V addresses of
the firmware (file offset + 0x2000_0000). "Body word *k*" or "w*k*" is word *k* of an engine instruction, word 0 being
the header (part 1 §8.5.1: bits 6:0 class, 12:8 register A, 17:13 register B or the format-B length, 22:18 register C,
26:23 the format-A length *L*). "Op 1" / "op 3" / "op 4" are the decoder control words of part 1 §8.3 (op 1 = write
bytes into the decoder's address space, op 3 = barrier on a resource id, op 4 = write the fence register). "Anjoy" is
the Anjoy MC-H12S firmware and its model (part 1, preamble); "cmd+0x…" is a byte offset into that model's command
vector.

Binary [R]: `/config/dla/ipu_firmware.bin` (602 632 B, build `Q_0.1.0_verified_21110517`; on the camera
`/config/dla/ipu_firmware.bin`).

## 1. How the work is divided

- **Offline and online.** An op whose RunMode (part 1 §6.6) is 1 ("invoke by cmdq") has its slice of the converter's
  offline command block replayed by a decoder (§3.5); the RISC-V does not compute it. Every other op runs **online**:
  its registered function (§2.3) runs on the RISC-V and either computes on the CPU or builds engine instructions and
  pushes them to decoder 0 (part 1 §8.1, §8.7). With the invoke's force-online flag set, every op runs online (§2.5) [I].
- **What the RISC-V computes** [I, from the structure of every kernel read]: (a) the parameters it puts into engine
  instructions (multipliers, shifts, clamps, offsets, coefficients, addresses, tile counts) and (b) the kernels that run
  "by CPU" (scalar loops over tensors in DDR or L2: RANGE, SHAPE, the CPU resize paths, the NMS pre-filter, TOPK's
  read-back). What a vector-unit, LUT or conv instruction does with its parameters is the hardware's definition; this
  document states the firmware's side exactly and gives the engine's side as parameter roles where only those are known
  (§12).
- **Where the plan comes from.** The firmware does not choose convolution tile sizes or L2 slot assignments; it executes
  the converter's partition records and slot groups (§6.7). It does plan tiles itself for resize (§10.11), element-wise
  domains (§10.2), reductions, softmax and the custom ops.

## 2. Interpreter and op resolver

### 2.1 Op resolver [R: `_sgs_OpResolverMgAddRef` 0x2001_C798; made by]
<!-- ev:ipu-firmware-2-engines.2.1 -->
One resolver per firmware boot (0x200A_8050, 0xC8C B; its pointer is the global 0x2009_32D0, part 1 §2.3): up to 128
builtin entries of 16 B {version, op type, registration, name} (`__SGS_TF_OpResolveAddBuiltinOps` 0x2001_C6BA — one
entry per version 1…*n*) and up to 64 custom entries {1, 0x20 (CUSTOM), registration, name} from +0x804
(`__SGS_TF_OpResolveAddCustomOps` 0x2001_C608). Lookup: `sgs_tf_OpResolveBuiltinFindOpByType` /
`sgs_tf_OpResolveCustomFindOpByName` ("Can not find OpCode: %d(%s) Version %d." / "Can not find OpCodeName: %s Version
%d."). The firmware op type equals the TFLite builtin code (the converter 0x2000_D922 is an identity map over the codes
it knows). Creating the resolver also initialises the L2 buffers (`l2_buffers_init`, §4.4).

### 2.2 Registration record [R words; I: the field names, from the assert strings `ST_TF_Domain0Ops.stDomain0HeaderOps.pFnHeaderExeLayer`, `ST_TF_Domain1Ops.stDomain1ElementwiseOps.pFnExeTile` / `pFnCalcOutputTile`, `E_SGS_OP_RESOLVE_TYPE_ELEMENTWISE`]
0x28 B in `.data`:

| offset | content |
|---|---|
| +0x00 … +0x08 | 0 |
| +0x0C | `eType`: 1 = domain-0 op (convolution class); 2 = domain-1 header op (a whole layer run by one function); 3 = ELEMENTWISE (run tile by tile inside a fused domain) |
| +0x10 | flag: 1 = needs the IPU compute engines, 0 = DMA / CPU only [I] |
| +0x14 | the layer function (types 1 / 2) or, for type 3, the common header `SGS_FW_DefaultElementwiseHeader` 0x2002_87E0 (§10.2) |
| +0x18 | type 3: 1 = the tile kernel uses a LUT or the vector unit with requantisation [I] |
| +0x1C | type 3: tile kernel `pFnExeTile` |
| +0x20 | type 3: `pFnCalcOutputTile` (0x2002_86FA for all) |

### 2.3 Registered operators [R]
<!-- ev:ipu-firmware-2-engines.2.3 -->
96 entries; kind B = builtin (TFLite code), C = custom (by name); v = versions registered; type / flag = +0x0C / +0x10;
fn = the layer or tile function the registration points at.

| kind | op | v | type/flag | fn |
|---|---|---|---|---|
| B 58 | LESS | 1 | 3/0 | 0x20061224 |
| B 61 | GREATER | 1 | 3/0 | 0x20061810 |
| B 71 | EQUAL | 1 | 3/0 | 0x20061d4a |
| B 72 | NOT_EQUAL | 1 | 3/0 | 0x200622f8 |
| B 62 | GREATER_EQUAL | 1 | 3/0 | 0x200628a6 |
| B 86 | LOGICAL_AND | 1 | 3/0 | 0x20062e54 |
| B 87 | LOGICAL_NOT | 1 | 3/0 | 0x20063572 |
| B 55 | MAXIMUM | 1 | 3/0 | 0x2005ffce |
| B 57 | MINIMUM | 1 | 3/0 | 0x20060924 |
| B 47 | EXP | 1 | 3/1 | 0x200653d8 |
| B 41 | SUB | 1 | 3/1 | 0x2005f6d4 |
| B 65 | SLICE | 1 | 2/0 | 0x2001d830 |
| B 22 | RESHAPE | 1 | 2/0 | 0x20053fb2 |
| B 2 | CONCATENATION | 1 | 2/0 | 0x20036aaa |
| B 45 | STRIDED_SLICE | 1 | 2/0 | 0x2001dd96 |
| B 88 | UNPACK | 1 | 2/0 | 0x2001e84c |
| B 69 | TILE | 1 | 2/0 | 0x2001defa |
| B 39 | TRANSPOSE | 1 | 2/0 | 0x2001e80a |
| B 49 | SPLIT | 3 | 2/0 | 0x2001db42 |
| B 94 | FILL | 1 | 2/0 | 0x20047824 |
| C | DummyOp | 1 | 2/0 | 0x200476b8 |
| C | Reshape_Alias | 1 | 2/0 | 0x20053ea8 |
| C | Tile_Alias | 1 | 2/0 | 0x2001ddd2 |
| C | Pad_Alias | 1 | 2/0 | 0x2004998e |
| C | Split_Alias | 1 | 2/0 | 0x2001d86c |
| C | Unpack_Alias | 1 | 2/0 | 0x2001e848 |
| C | Slice_Alias | 1 | 2/0 | 0x2001d688 |
| C | Concatenation_Alias | 1 | 2/0 | 0x20036aa6 |
| C | Concatenation_Skip | 1 | 2/0 | 0x20036aa6 |
| C | ResizeBilinear_Alias | 1 | 2/0 | 0x20053fec |
| C | StridedSlice_Alias | 1 | 2/0 | 0x2001db7e |
| C | Transpose_Alias | 1 | 2/0 | 0x2001e830 |
| C | TFLite_Detection_NMS | 1 | 2/1 | 0x2003c9d6 |
| C | TFLite_YoloV3_Unpack | 1 | 2/0 | 0x2004b004 |
| C | PostProcess_Unpack | 1 | 2/0 | 0x2004b004 |
| C | PostProcess_Max | 1 | 2/0 | 0x20049c52 |
| C | Align | 1 | 2/0 | 0x20033fa4 |
| C | Align2 | 1 | 2/0 | 0x20033e96 |
| C | Align2_Alias | 1 | 2/0 | 0x20033e96 |
| C | RoiPooling | 1 | 2/0 | 0x200433d0 |
| C | PSRoiPooling | 1 | 2/0 | 0x2004123c |
| C | Fix2Float | 1 | 2/0 | 0x2003d15a |
| C | Customized_Concatenation | 1 | 2/0 | 0x20036aaa |
| B 83 | PACK | 1 | 2/0 | 0x20049798 |
| B 0 | ADD | 1 | 3/1 | 0x2005f918 |
| B 36 | GATHER | 1 | 2/1 | 0x200479b8 |
| B 3 | CONV_2D | 1 | 1/1 | 0x2002c60c |
| B 4 | DEPTHWISE_CONV_2D | 2 | 1/1 | 0x2002c624 |
| B 17 | MAX_POOL_2D | 1 | 2/1 | 0x2002c63c |
| B 1 | AVERAGE_POOL_2D | 1 | 2/1 | 0x2002c654 |
| B 34 | PAD | 1 | 2/1 | 0x20049b48 |
| B 53 | CAST | 1 | 3/1 | 0x2003654c |
| B 25 | SOFTMAX | 1 | 2/1 | 0x200656cc |
| B 14 | LOGISTIC | 1 | 3/1 | 0x20065162 |
| B 18 | MUL | 1 | 3/1 | 0x2005fd2e |
| B 19 | RELU | 1 | 3/1 | 0x200534a2 |
| B 21 | RELU6 | 1 | 3/1 | 0x200534a2 |
| B 20 | RELU_N1_TO_1 | 1 | 3/1 | 0x200534a2 |
| B 98 | LEAKY_RELU | 1 | 3/1 | 0x20053616 |
| B 54 | PRELU | 1 | 3/1 | 0x2005361a |
| B 40 | MEAN | 1 | 2/1 | 0x2005c556 |
| B 42 | DIV | 1 | 3/1 | 0x2005fb66 |
| B 93 | ZEROS_LIKE | 1 | 2/0 | 0x2001ea22 |
| B 48 | TOPK_V2 | 1 | 2/0 | 0x2001e6d2 |
| B 77 | SHAPE | 1 | 2/0 | 0x2001d62c |
| B 96 | RANGE | 1 | 2/0 | 0x2004dcdc |
| B 116 | ROUND | 1 | 3/0 | 0x2001d33c |
| B 75 | SQRT | 1 | 3/1 | 0x20065b58 |
| B 9 | FULLY_CONNECTED | 1 | 1/1 | **none** |
| B 101 | ABS | 1 | 3/0 | 0x200638bc |
| B 59 | NEG | 1 | 3/0 | 0x20063a38 |
| B 76 | RSQRT | 1 | 3/1 | 0x20065dc8 |
| B 28 | TANH | 1 | 3/1 | 0x2006618a |
| B 23 | RESIZE_BILINEAR | 1 | 2/0 | 0x2005416a |
| B 56 | ARG_MAX | 1 | 2/1 | 0x20036422 |
| B 97 | RESIZE_NEAREST_NEIGHBOR | 1 | 2/0 | 0x200541a6 |
| B 102 | SPLIT_V | 3 | 2/0 | 0x2001db42 |
| C | Dilation | 1 | 2/0 | 0x20047686 |
| C | Split_Alias | 1 | 2/0 | 0x2001d86c |
| C | Reciprocal | 1 | 3/1 | 0x2006624c |
| C | DmaCoefMatrix | 1 | 2/0 | 0x2003ca6a |
| C | DmaCoefMatrix_Alias | 1 | 2/0 | 0x2003ca6e |
| B 74 | SUM | 1 | 2/1 | 0x20053112 |
| C | SGS_LSTM | 1 | 2/0 | 0x200406ec |
| C | PhaseGen | 1 | 2/1 | 0x20036e96 |
| C | Atan2 | 1 | 2/1 | 0x200381e4 |
| C | PhaseModify | 1 | 2/1 | 0x2003fe40 |
| C | WiggleErr | 1 | 2/1 | 0x200675d2 |
| C | CondLess | 1 | 2/1 | 0x2003f458 |
| C | CondGreat | 1 | 2/1 | 0x20039a42 |
| C | CustomNotEqual | 1 | 2/1 | 0x20040b4a |
| C | Clip | 1 | 2/1 | 0x200392f8 |
| C | CustomFloorMod | 1 | 2/1 | 0x2003d20e |
| C | FormatConvert | 1 | 2/1 | 0x2003e224 |
| C | FourBytesFlip | 1 | 2/1 | 0x2003ec00 |
| B 82 | REDUCE_MAX | 1 | 2/1 | 0x200494ae |

Notes [R]:
- `Split_Alias` is registered twice (same registration). Functions shared by several entries: `Concatenation_Alias` /
  `Concatenation_Skip`, `Align2` / `Align2_Alias`, `TFLite_YoloV3_Unpack` / `PostProcess_Unpack`, `RELU` / `RELU6` /
  `RELU_N1_TO_1`, `SPLIT` / `SPLIT_V`; `CONCATENATION` and `Customized_Concatenation` share the registration 0x2009_22D4.
- **FULLY_CONNECTED is registered with no function**: a model with it fails at invoke ("Alert: empty registration for op
  code"), so a converter for this firmware has to express FC as a 1×1 convolution [I].
- Several entries point at stubs or jump chains, not at a body: the alias entries are `return 0` / `return −1` / a jump
  (§10.13); LEAKY_RELU's 0x2005_3616 is `j` → 0x2005_8732 → `j` → 0x2005_C876, the real tile function (§10.6);
  `DmaCoefMatrix` 0x2003_CA6A is `j` → 0x2005_CA34 (§10.19); TOPK_V2's entry 0x2001_E6D2 calls `OPS_IPU_TopK` 0x2001_E4B2
  (§10.15). The tile functions of ADD 0x2005_F918, DIV 0x2005_FB66 and MUL 0x2005_FD2E are separate functions next to
  SUB's 0x2005_F6D4 (the first decompile, had merged them into `SGS_Sub_IPUExeTile`, and
  0x2005_C876 into `MeanRunAsAvePool_Domain1HeaderExecuteLayerFunc`; has them apart).
- **Not supported in this build** [R: absence from the resolver]: option tables that exist in the schema but have no
  registered op — CustomMulAdd, CustomAlphaBlending, Gelu, LayerNorm, Pool3D, ImageConcatenation, Conv3D,
  MedianFilter — and every TFLite builtin not in the table (the LSTM / RNN family, L2_NORMALIZATION, SPACE_TO_DEPTH,
  BATCH_TO_SPACE, …; ARG_MIN, LESS_EQUAL and PADV2 too).

### 2.4 Interpreter: create [R unless marked]
`SGS_TF_InterpreterCreateByModel(&interp, resolver, modelRecord)` (0x2000_AFF2; called by host command CreateChannel,
part 1 §5.2) builds per subgraph:
- its tensor records (`SGS_FW_ConvertTfLiteTensorToIpuTensorRecord`, `convert_tflite_tensor_to_ipu_tensor`
  0x2002_7D4E — the IPU tensor record and the conv tensor struct, §9.2);
- its op nodes, the registration found per OperatorCode ("Fail to get registration for builtinOpCode(%s) %d." /
  "Fail to get registration for customOpCode(%s) %d.");
- the grouping of the op nodes into **domains** (records of 0x94 B), the domain type from the model (Operator field 8,
  part 1 §6.6): DOMAIN0_HEADER 1, DOMAIN0_MEMBER 2, DOMAIN1_HEADER 3, DOMAIN1_MEMBER 4, ELEMENT_WISE 5, FAKE_HEADER 6
  [I: the numbering from the name order in `sgs_tf_ConvertDomainFromTf` and the dispatch of §2.5].

Limits: "Interpreter tensors/inputs/outputs max: %d, already: %d, add tensors %d over load !!", "Domain: max: %d Domain
Array overflow !!!", "Domain0[%d]: max op list: %d …". The host's scratch ("variable") buffer holds every non-constant
tensor at the offset the model gives (Tensor `stAlloc`, part 1 §6.4); constant tensors point into the model's Buffer
data.

### 2.5 Interpreter: invoke and run modes [R unless marked]
- `SGS_TF_InterpreterFlipInOutBuffer` (0x2000_B68A) points the subgraph's input and output tensors at the host's buffers
  for this invoke (part 1 §5.4). SGS_LSTM uses the same call to swap a sub-network's buffers per time step (§10.19).
- `_sgs_doWorkThread(interp, sg, forceOnline, 0)` (0x2000_B8AE):
  1. a subgraph with op nodes but no domains: every op without a function gives "Alert: do not implement op code: %s
     (%d)!!!" and the invoke fails;
  2. subgraph record byte +0x5C = 0: data are moved with BDMA (`SGS_ShiftDataByBdma`, after `dma_relocate_consts` (0x2002eb32)) — the model's
     constant tensors relocated into the scratch buffer [I: what is moved, from `Drv_Bdma_MoveData`'s users, §3.4];
  3. the run contexts of type-0x11 tensors are cleared;
  4. byte +0x5C = 1: the subgraph's input tensors are flagged (+0x120) [I: the fast-run case];
  5. the statistics record is reset (part 1 §5.6; its flag = `forceOnline`); two `mcycle` stamps are taken; the DMA
     counters are reset and started;
  6. for each domain in order: its online flag (+0x91) is set to `forceOnline`; domain types 1–2 →
     `do_process_domain0`, 3 / 4 / 6 → `do_process_domain1`, other → "Unknown domain type: %d."; a failing domain prints
     "Run Domain opNode error !!!." and the invoke returns 0 (failure);
  7. at the end: CMDQ flush (0x2002_76CE), the end stamps, DMA counters 0 / 1 into the statistics record.
- **Run modes per op** (`eRunMode`, part 1 §6.6; printed "invoke by cmdq" / "invoke by cpu" / "skip" / "Invalid"): an op
  either replays its slice of the offline command block through a decoder (`_HAL_IPU_CommitOfflineCmdblock`, §3.5;
  fields `pCmdEntry`, `s32Size`, `u32Fence`, `s32DecodeId`, `s32Offset`), or is computed online by its registration's
  function, or is skipped (ops that only relabel memory) [R strings; I: the mapping from the strings]. With `forceOnline`
  = 1 (the host's sysfs `force_online_mode`, `ipu-host.md` §4.10) every op runs online instead of from the command block
  [I: from the flag's use in the domain records]. In the Anjoy model one RunMode-1 slice covers the whole network
  (part 1 §8.6).
- The domain record's fields beyond those named here (+0x91, and the resume position of §10.2) and the layout of the run
  contexts are not known.

## 3. IPU200M HAL

The accelerator is reached at RISC-V address **0x0100_0000** + offset; the host (mhal, `ipu-host.md` §2) sees the same
registers at CPU physical **0x1800_0000** + offset, so the global block the host calls "IPU+0x42xxxx" is RISC-V
0x0142_xxxx, as the CCIF block at +0x41_0000 is (part 1 §4.2) [R]. The RISC-V reaches the engines partly through fixed
absolute addresses (DMA, BDMA, CMDQ, `hal_ipu_ctrl`) and partly through driver objects that hold a base pointer, so the
base constants live in register-block descriptors, not inline [R]. Nothing in the firmware programs the IPU PLL or power;
that is the host's (`ipu-host.md` §2.3–§2.4) [R].

### 3.1 Register-block descriptors [R: data at 0x2009_1A30, 0x2009_1A40, 0x2009_1E20, 0x2009_1E30, 0x2009_2EC0, 0x2009_2EE8; RISC-V base addresses read from them]

| descriptor | name string | base(s) and words |
|---|---|---|
| `ipu_ctrl` | 0x2009_1650 | 0x0100_0000 — the decoder control registers (part 1 §8.7; the control object 0x2009_1A10 holds the base at +0x28) |
| `ipu_sr` | 0x2008_A51C | descriptor 0x2009_1E30: +0x00 name, +0x04 base 0x0101_0000, **+0x0C bank 0 = 0x0101_0000, +0x10 bank 1 = 0x0101_0200** (the SR register file, §5.1); from +0x14 the firmware's SR value cache (§5.4); the pointer cell 0x2009_2138 holds the descriptor's address |
| `ipu_lut` | 0x2009_1674 | LUT descriptor 0x2009_2EC0: words {0x8000, 0, 0x8000, 0x8800, 0x9000, 0x9800, 0xA000, 0xB000}, of which +0x0C … +0x20 (0x8000, 0x8800, 0x9000, 0x9800, 0xA000, 0xB000) are the **decoder op-1 target addresses** of the two LUT units' tables and configuration, not offsets from a register base; a nested "ipu_lut_ipu_ctrl" pointer. LUT CPU-access object 0x2009_2EE8: +0x0C … +0x20 = the CPU addresses of the same six images, 0x0102_0000, 0x0102_0800, 0x0102_1000, 0x0102_1800, 0x0102_2000, 0x0102_3000 (= decoder address + 0x0101_8000), and at +8 a CPU-access lock byte (§8.2) |
| `ipu_l2mem_ctrl` | 0x2007_E908 | 0x0120_0000 |
| `ipu_global_ctrl` | 0x2008_A50C | 0x0142_0000 — the DLA global registers: IRQ mask +0x38, IRQ status +0x40, **+0x48** written 1 / 0 around every CPU write to SR bank 0 (§5.2), the status / clear register pairs +0x58 … +0xC0 that mhal's `MHAL_IPU_clear_dla_irq` also drives (`ipu-host.md` §2.6), +0xB0 trigger, heap bounds +0xC4 / +0xC8 |

`ipu_global_ctrl` and `ipu_l2mem_ctrl` are the blocks mhal and the host smoke test see from the host side
(`ipu-host.md` §2, §6).

### 3.2 EMI ↔ L2 DMA (`hal_dma.c`, `drv_dma_ops_ipu200.c`) [R]
`hal_dma_load` (0x2000_8FB4) and `hal_dma_store` (0x2000_9916) encode one **engine instruction** of class 0x04 (load,
EMI → L2) or 0x44 (store, L2 → EMI) into a command buffer `{words, count}` — the encoder's last argument — of 5, 7 or 9
words (the header's length field 4 / 6 / 8; header base 0x8000_0004 for a load, 0x8000_0044 for a store); the word
layout — `emi_addr`, the tensor SR index *t* (< `TENSOR_SR_SIZE` = 0x40), `qid`, the windows and strides — units and
range asserts are part 1 §8.5.2. The instruction travels in the decoder command stream like every other engine
instruction.

- **Units.** L2 windows are given to the encoder in bytes and stored as **L2 slots** (>> 4, `L2_SLOT_WIDTH_INBYTES` =
  16): width and stride must be 16-aligned. DDR mode (encoder argument 8): 0 linear, 2 windowed (width / height even,
  channel = 1), 3 4-byte elements (EMI address 4-aligned). The "extract box" path (`emi_extbox_close == false`) is the
  image-input crop.
- **Queues.** `hal_dma_getqid` (0x2000_9FBE) maps a DDR type to one of 8 hardware queues (`qid < 8`): type 1 (U8
  feature) → q0; 2 / 5 / 8 → q1; 3 → q2; 4 / 9 → q5; 6 / 10 / 11 → q6; 7 / 12 → q4 (the types are the
  `E_TF_TENSOR_DDR_TYPE_*` of part 1 §6.5 plus the kernel / weight ones).
- **The SGS_DMA layer** (`SGS_DMA_MoveData` 0x2003_09E0, `_SGS_DMA_LoadDataToL2` 0x2003_0242, `_SGS_DMA_SaveDataToEmi`
  0x2003_04F4, `SGS_DMA_FillMemory` 0x2003_139A, `SGS_DMA_AlignTensor` 0x2003_1250) splits a tensor into windows that fit
  the L2 limits (`SGS_DMA_AdjustEmiWidth` / `AdjustEmiHeight` / `AdjustL2Width` 0x2002_FC22 … 0x2002_FE00) and emits one
  instruction per window. EMI addresses become bus addresses by adding 0xE000_0000 after the DMA address-map step of
  part 1 §5.5. `SGS_DMA_OpsStart` prints `slot_Num = 0x20, L2_BUFFER_DEPTH = 0x3fff, L2_Buffer_Size = 0x3fff0` (§4.1).
  `SGS_DMA_FillMemory`'s fill is a vector-unit instruction (class 0xC3, §7.8). The DMA-layout ops' use of this layer:
  §10.13.

### 3.3 CMDQ (`drv_cmdq.c`) [R]
CMDQ feeds the hardware a queue of register writes; only **channel 3** is used (every entry asserts
`E_CMDQ_CHANNEL_3`). A command is {u32 mask, u32 value, u32 reg-index}; two modes:
- **direct RIU mode** (flag at 0x200A_A488 = 1): each command is applied at once as a masked write to RISC-V address
  **0x1080_0000 + (reg-index & 0xFFFFFF)·4** (`*reg = (*reg & ~mask) | (value & mask)`) — the CMDQ register window;
- **ring mode** (default): commands are copied into a ring buffer (base 0x200A_A498, 0x40 cache lines, wrapping), the
  range is cache-cleaned (`cache_set_range` + `cache_op(1,1,0)`) and `Drv_Cmdq_Trig` starts the hardware;
  `Drv_Cmdq_GetIdleBufferSize` / `Drv_Cmdq_WaitForBuffer` gate on free space; the buffer start must be 64-byte aligned
  (`(u32CmdStart & (CACHE_LINE_ALIGNMENT-1)) == 0`); up to `COMMAND_COUNET` = 0x40 commands per batch.

Functions: `Drv_Cmdq_AddCommand` 0x2002_F728, `Drv_Cmdq_Trig` 0x2002_FA6E, `Drv_Cmdq_GetIdleBufferSize` 0x2002_F644,
`Drv_Cmdq_WaitForBuffer` 0x2002_F6E6, `Drv_Cmdq_GetReadPointer` 0x2002_F5D6, `Drv_Cmdq_SetDebugLevel` 0x2002_FB2E. The
invoke ends with a CMDQ flush (0x2002_76CE, §2.5).

### 3.4 BDMA (`drv_bdma.c`) [R]
A block-copy engine, again **channel 3** only (`E_BDMA_CH3`). `Drv_Bdma_MoveDataByHw` 0x2002_F032 assembles a CMDQ
command block from a source and a destination descriptor ({u32 width, offset, size, addr}) through helpers
(0x2002_EBBA … 0x2002_EFC0), appends it with `Drv_Cmdq_AddCommand` and triggers it; `Drv_Bdma_OpsStart` 0x2002_F42E /
`Drv_Bdma_OpsDone` 0x2002_F31C bracket a batch. 64-bit per-channel byte counters at 0x200A_A2D8. `Drv_Bdma_MoveData`
0x2002_F22A is the entry used by `SGS_ShiftDataByBdma` at the start of an invoke (§2.5). The copy goes to bus addresses
(the descriptor address already carries the 0xE000_0000 top), so BDMA moves EMI → EMI, not through L2.

### 3.5 Decoder replay of the offline command block (`hal_cmdq.c`) [R]
Two decoders, ids 0 and 1 (registers: part 1 §8.7). `_HAL_IPU_DecoderDMACMDQCreate` (0x2003_1D94) sets up a per-decoder
command context at boot: run mode 1 → decoder 1 (context 0x2009_219C), run mode 2 → decoder 0 (context 0x2009_213C); it
asserts `s32Size == 0` at create and keeps a reference count. `_HAL_IPU_CommitOfflineCmdblock` (0x2003_1C72) replays one
op's slice {`pCmdEntry`, `s32Size`, `u32Fence`, `s32DecodeId`} (Operator field 12, part 1 §6.6): it calls the
**line-mode** trigger `_HAL_IPU_TriggerDecoderDMALineMode` (0x2003_1BF8), which itself spins on the decoder's done bit,
and returns — it does not wait on the fence. The **ring-mode** entries `_HAL_IPU_TriggerDecoderDMARingMode`
(0x2003_1BC0) and `_HAL_IPU_WaitDecoderDMARingMode` (0x2003_1C3E) are **stubs that assert(0)**
(`__assert_func(..., "0")`); `u32Fence` reaches only that ring-mode wait. So only line mode is implemented: the firmware
hands the decoder one command block and waits for it; the fence value ends the block as the operand of op 4 (part 1
§8.3.5).

Line-mode triggers [R, register addresses only]: `hal_ipu_ctrl_dec0_line_mode_trigger` 0x2000_A2AA and the decoder-1
trigger 0x2000_A48A both assert that the EMI address and the size are 16-aligned. Decoder 0: pulse the control word
(`*(ctrl+0x28)` word = 1 then 0, control byte +0x28 = 0x10), the block's bus address (argument + 0xE000_0000) into the
EMI-address register pair **0x01AA_6018 / 0x01AA_601C**, the length (>> 4) into the control object, the global run
`0x0100_012C |= 1` and **0x0142_00B0 = 1** (the trigger mhal also uses), then spin on the control object's status
(+0x6C bit 0) while draining the DLA IRQ status at **0x0142_0040** into its status / clear pairs 0x0142_0058 …
0x0142_00C0, and finally set +0x74 bit 0. The IRQ pairs the host clears in `MHAL_IPU_clear_dla_irq` are the ones the
firmware polls here; while a block runs the firmware drives the decoder entirely and the host does not see a DLA
interrupt, which is why `mi_ipu`'s handler treats a DLA IRQ with no CCIF bit as spurious (`ipu-host.md` §2.6, §3.4).

## 4. L2 memory: VR and MR buffers (`ipu_vrbuf.c`, 0x2000_C010–0x2000_D1BE) [R unless marked]

### 4.1 Geometry
The on-chip L2 is handed out as **slots**: a slot is one 16-byte-wide column (`L2_SLOT_WIDTH_INBYTES` = 16) of **512
lines** (`ONE_SLOT_DEPTH`: depths are stored ×512, `vrbuf_depth % ONE_SLOT_DEPTH == 0`; addresses split as line = bits
8:0, slot = bits ≥ 9). The DMA layer prints `slot_Num = 0x20, L2_BUFFER_DEPTH = 0x3fff, L2_Buffer_Size = 0x3fff0`
(`SGS_DMA_OpsStart`) and the reduce kernels assert `s32InSlotNum < VRBUF_SLOT_TOTAL_NUM` with 0x20, so the L2 is **32
slots × 512 lines × 16 B = 256 KiB** [I: the product of the constants]. Conv buffers may use up to `CONV_SLOT_TOTAL_NUM`
= **24** slots (assert in `pooling_exec_layer_callback_helper`; `vrbuf_reinstall_vectors` (0x2000c960) re-installs the vector buffers when the
four conv buffers together take more than 0x18 slots); the default vector buffers live in slots 24–31 (§4.4).

### 4.2 Buffer ids and the record table
17 **VR buffer ids** 0 … 0x10 (`bufid < VRBUF_ID_TOTAL`, asserted ≤ 0x10) and 8 **MR buffer ids** 0 … 7 (`bufid <
MRBUF_ID_TOTAL`); the reduce kernels install MR tables as their accumulator tables (§10.14).
`vrbuf_get_bufid(type)` (0x2000_C51E) maps a buffer **type** 1 … 12 to a VR id; type 13 (0xD) is "customer" (set
directly by id); other types assert `isvalid_vrbuftype`:

| type | VR id | use (who asks for it) |
|---|---|---|
| 1 | 0xD | conv **IB** (input) — `load_ib_fromemi_16_2_9_helper`, `Conv_*` |
| 2 | 8 | conv **KB** (kernel / weights) — `load_conv_kb_fromemi` |
| 3 | 0x10 | conv **AB** (accumulator) |
| 4 | 0xF | conv **OB** (output; `get_conv_ob_bufid()` = 0xF) — `store_conv_ob_toemi` |
| 5 / 9 | 1 | vector buffer (element-wise ping-pong records at 0x200A_910C / 0x200A_913C, `l2_buffers_init`) |
| 6 / 10 | 0 | vector buffer |
| 7 / 11 | 3 | vector buffer |
| 8 / 12 | 2 | vector buffer |
| 13 | any (customer) | `set_customer_vr_tbl2`, `vrbuf_init_bufid` |

The four conv ids are returned by four 2-instruction functions 0x2002_B14A (0xD), 0x2002_B14E (0xF), 0x2002_B152 (8),
0x2002_B156 (0x10); `isconv_vrbufid(id)` is "id ∈ {8, 0xD, 0x10, 0xF}". Types 9–12 name the same four vector ids as
5–8; the element-wise code keeps them as a second set of ping-pong records [R: `l2_buffers_init`; I: a second operand set].

**Record** — 17 × 0xA0 bytes at **0x200A_66E4** (cleared as a whole, 0xAA0 bytes, by `conv_engine_teardown` / `vrbuf_reset_all` /
`vrbuf_reset_all_b` (0x2000d178)):

| offset | field | written by |
|---|---|---|
| +0x00 | VR id | every init |
| +0x04 | type (1 … 13; `_getVrbufInfo` 0x2000_C37E returns the record only for 1 … 13, else NULL) | init |
| +0x08 | `vrbuf_width_inslots` (≤ `VRBUF_WIDTH_INSLOTS_MAX` = 6) | init |
| +0x0C | `vrbuf_depth` = depth in slots × 512 (≤ 32 slots, `MAPPING_TABLE_CONV_NUM`); get: `vrbuf_get_depth` (0x2000c50e) | init |
| +0x10 | `vrbuf_depth_used` — allocation pointer in lines; set: `vrbuf_set_depth_used(id, v)` (0x2000c4a0) (reset / restore), get: `vrbuf_get_depth_used` (0x2000c4fe) | `vrbuf_alloc`, conv loops |
| +0x14 | zeroed by every init, no reader found | init |
| +0x18 | **conv mapping table**, u16 × depth in slots (≤ 32) | 0x2000c010 |
| +0x58 | **DMA / vector mapping table**, u16 × width·depth (≤ 32; for OB: the first 16 entries) | `_set_dma_vec_mapping_table` |
| +0x78 | OB only: second table — the vector-processor view (entries 16 … of the pushed table) | `_set_dma_vec_mapping_table` |
| +0x98 | `sub_width` in slots | init |
| +0x9C | ping-pong selector (0 / 1); `vrbuf_pingpong_flip` 0x2000_C412 toggles it and returns the new value, `vrbuf_set_pingpong_type` 0x2000_C460 sets it, `vrbuf_get_pingpong_type` (0x2000c4b8) reads it | conv loops |

`vrbuf_alloc(id, lines, &offset)` (0x2000_C2D4) is a **bump allocator** inside one buffer: if `depth_used + lines ≤
vrbuf_depth` it returns the old `depth_used` as the offset and advances it, else it logs `bufid %u
vrbuf_depth_used=%d, require depth_size=%d, now vrbuf_depth=%d` and returns 0 (callers assert). There is no free: the
conv loops rewind it with `vrbuf_set_depth_used(id, saved)` [R: `Conv_FFDIDOHW` saves `vrbuf_get_depth_used` before a tile group and
restores it after]. The offset is a line number inside the buffer, i.e. the low bits of a conv-space address (§4.5) [I].
`_vrbuf_slots(id)` (0x2000_C3CA) = width × depth / 512 = the slots the buffer occupies; `vrbuf_reinstall_vectors` = the slots of
IB + KB + OB + AB.

### 4.3 Slot tables and how they reach the hardware
A buffer is described to the engines by **u16 slot-table entries** pushed into the decoder's address space with op 1
(part 1 §8.3.1; header `0x1000_0000 | (bytes − 1) << 16 | address`), each preceded by an op-3 barrier on the buffer
(part 1 §8.3.2). Entry format — high byte = one-hot **slot group** (slot >> 3, bit 8 + group), low byte = mask of the
slots inside the group (slot & 7) [R: the two builders below]:

- **Conv mapping table** (`vrbuf_build_conv_map`, record +0x18): one entry per depth row *k* of the buffer, from its first-slot
  list *s_k*: `(1 << (8 + (s_k >> 3))) | (((1 << width) − 1) << (s_k & 7))` — the row covers `width` adjacent slots.
  The bit field is not guarded against a row crossing a group boundary (s_k & 7 + width > 8 spills into the group byte)
  [R; I: the planner never does it].
- **DMA / vector table** (`_set_dma_vec_mapping_table` 0x2000_C068, record +0x58): asserts `width % sub_width == 0`;
  for *j* = 0 … width/sub_width − 1, for each row *k*, for *m* = 0 … sub_width − 1: base = s_k + j·sub_width, entry =
  `(1 << (8 + (base >> 3))) | (1 << ((base + m) & 7))` — one entry per slot, ordered sub-column-major, so width·depth
  entries.

Decoder addresses and barrier ids [R: the op-1 / op-3 words built in `vrbuf_init_bufid`, `_vrbuf_init_bufid`,
`_bufid_init_vector_helper`, `set_customer_vr_tbl2`, `set_customer_mr_tbl`, `conv_barriers` (0x2000d118)]:

| target | decoder address (op 1) | barrier (op-3 id) before the write |
|---|---|---|
| VR *id* conv mapping table | `id · 0x100` (0x000 … 0x1000) | 0x20 + *id*; none for AB (id 0x10 → 0x1000) |
| VR *id* DMA / vector table | `id · 0x100 + 0x80` | 0x20 + *id* (customer tables: 0 = all) |
| MR *m* table (≤ 16 entries) | `(m + 0x11) · 0x100` (0x1100 … 0x1800) | 0x40 + *m* |

The prologue of every layer of the Anjoy block (part 1 §8.6) is this pattern, and its MR tables 0 / 1 are the same
encodings [R]. The values differ: the converter assigns its own slots (Anjoy VR 1 table 0x0804 = slot 26) where the
firmware's defaults put VR 1 in slots 28–29 [R]. `conv_barriers` issues the four conv barriers (0x2D, 0x28, 0x30, 0x2F:
IB, KB, AB, OB) without writing anything — a "wait until the conv buffers are free" [R; I: purpose].

### 4.4 Initialisation paths

| function | when | does |
|---|---|---|
| 0x20029406 | once, from `_sgs_OpResolverMgAddRef` (boot, op-resolver creation, §2.1) | fills the element-wise ping-pong records (types 5–8 at 0x200A_910C, 9–12 at 0x200A_913C, 12-byte records with the VR id in word 1); `hal_ipu_ctrl_open` with an all-zero configuration (line mode); `vrbuf_reset_all`; **MR 0 = {0x0001, 0x0002}, MR 1 = {0x0004, 0x0008}** (data 0x2009_1D08 / 0x2009_1D0C) |
| 0x2000cef6 / `vrbuf_reset_all_b` | boot; start of 20+ custom / pooling / resize ops (and their end, to restore the defaults) | op 3 id 0; clear all 17 records; `_bufid_init_vector_helper` 5, 6, 8, 7 |
| `_bufid_init_vector_helper(t)` 0x2000_C9F4 | from the above | VR id of *t*: width 1, sub-width 1, depth 2 slots, type *t*; slots {0x1C, 0x1D} (type 5 → VR 1), {0x18, 0x19} (6 → VR 0), {0x1A, 0x1B} (7 → VR 3), {0x1E, 0x1F} (8 → VR 2); op 3 (0x20 + id); op 1 of the 2-entry vector table (4 bytes) to id·0x100 + 0x80; no conv mapping table. Asserts if the record is already in use |
| `conv_engine_setup(ib, kb, ab, ob)` (0x2000c99e) | conv-engine setup, slot 0 of the ops table 0x2009_1D10 (`conv_ops_setup_slot` (0x20027790)) | `_vrbuf_init_bufid` for types 1, 2, 4, 3; if the four conv buffers take > 24 slots: op 3 id 0 (they overlap the vector slots) |
| 0x2000cbae | conv-engine teardown, slot 1 of the same table (`conv_ops_teardown_slot` (0x20027780)); also the pooling headers | if the conv slots > 24: op 3 id 0, clear, re-install the vector buffers; then zero type / width / depth of the four conv records |
| `_vrbuf_init_bufid(type, shape)` 0x2000_C5E4 | from `conv_engine_setup` | width / sub-width from `get_buffer_widths` (0x2002c1ea) (§4.6), depth = shape +0x08 slots, first-slot list = shape +0x0C; the record must be free (`depth == 0`, else assert `ret` in `_vrbuf_conv_setup_helper`); conv mapping table to id·0x100; for IB / KB / OB also the DMA / vector table to id·0x100 + 0x80. OB: `width ≤ 2`, `width·depth ≤ 16` (`VP_ENTRY_OFFSET_IN_OBVRBUF`); its pushed table is 16 + width·depth entries: entries 0–15 the DMA view (sub-width 1), entries 16 … the vector-processor view (sub-width = width) |
| `vrbuf_init_bufid(id, width, depth, slots, sub)` 0x2000_C102 | `_SGS_ConvBilinear` (resize on the conv engine, §10.11) | the same for an explicit id, type 13, barrier id 0 |
| `set_customer_vr_tbl2(id, table, depth ≤ 32)` 0x2000_CF40 | 27 custom / pooling / resize / DMA-coef ops | caller-made vector table; type 13, width 1, sub-width 1 |
| `set_customer_mr_tbl(m, table, n ≤ 16)` 0x2000_D048 | 20 custom ops | MR table; no record |

Customer tables used by the kernels (§10) are mostly **one-hot slot entries**: entry *i* = (1 << (*i* mod 8)) \|
(0x100 << (*i* div 8)), i.e. slot *i* (as `_SGS_Softmax_InitL2Buffer`, REDUCE_MAX, Clip, RoiPooling build them).

### 4.5 Address spaces and conversions
A **conv-space address** of a conv buffer is `slot_row << 9 | line` (line 0 … 511 inside a 512-line slot, row = index
into the buffer's conv mapping table) [R: the `>> 9` / `& 0x1FF` splits below]. `vrbuf_get_convaddr(id, a)` only asserts
`isconv_vrbufid` and returns *a*. Conversions (W = width in slots +0x08, S = sub-width +0x98, D = depth in slots, *p* =
ping-pong index passed by the caller; r = a >> 9, l = a & 0x1FF):

| function | used by | result |
|---|---|---|
| `vrbuf_convert_convaddr2dmavpaddr(id, a, p)` 0x2000_CCD0 — IB, KB, AB (asserts not OB) | `load_ib_fromemi_16_2_9_helper`, `load_conv_kb_fromemi` | S = 2: r' = r + (W ≥ 3 ? D·p : 0); else r' = S·r + (W > S ? D·S·p : 0); returns r' << 9 \| l |
| `vrbuf_conv_ob_convert_convaddr2dmaaddr(0xF, a, p)` 0x2000_CD7E | `store_conv_ob_toemi` | r' = r + (W > 1 ? D·p : 0) |
| `vrbuf_conv_ob_convert_convaddr2vpaddr(0xF, a, p)` 0x2000_CDF4 | `convolution_IPU` (output stage word 1, part 1 §8.5.4) | S = 2: b = a + 0x1000, r' = (b >> 9) + (W ≥ 3 ? D·p : 0); else b = a + 0x2000, r' = (b >> 9)·S + (W > S ? D·S·p : 0); returns r' << 9 \| (b & 0x1FF) |
| `vrbuf_get_dmavecaddr_vector(type, a)` 0x2000_CE92 | DMA / vector kernels | asserts `a < depth` and `width == 1`; returns *a* |

So the DMA / vector tables index slots as rows of the conv table times the sub-width, and **ping-pong is a second copy
of the buffer in the next D (or D·S) table entries** [I: from the arithmetic]; the OB vector view starts 16 entries
(0x2000 = 16 × 512) — 8 entries for S = 2 — into the OB table, which is where `_vrbuf_init_bufid` puts the vector half
(§4.4) [R]. The Anjoy output-stage address 0x2000 (part 1 §8.5.4) is that offset with r = 0 [R value; I: the link].
Inside a vector-unit instruction, L2 addresses are 14-bit line numbers of this vector-space view (§7.1).

### 4.6 Buffer widths (`tile_partition_util_hw.c`) [R]
Enums from the asserts and compares: tensor **usage** (conv tensor struct +0x1C, §9.2) 1 = `EN_TENSOR_FEATURE_MAP_IB`, 2 =
`EN_TENSOR_FEATURE_MAP_OB`, 3 = `EN_TENSOR_WEIGHT`, 4 = accumulator (AB) [R: `tensor->usage == …` asserts with the
constants 1, 2, 3; `get_ab_hwtile_size_invb` passes 4]. The IPU dtype (struct +0x18) is §9.2; `hal_set_simd` and the
conv modes are §6.2.

Width in slots (`get_width_inslots` 0x2002_AA5C (conv_mode, usage, dtype); OB: `get_ob_width_inslots` 0x2002_ABF2
(lane mode, dtype)) and sub-width (`get_subwidth_inslots_hw` 0x2002_ACDA) — what `get_buffer_widths` puts into the record:

| buffer | width in slots | sub-width |
|---|---|---|
| IB | mode 0, 4 → 1; 1, 5 → 4; 2, 3 → 6 | mode 0, 1, 4, 5 → 1; 2, 3 → 2 |
| KB | dtype 8 (4-bit) → 1, else 2 | same as width |
| AB | mode 0: INT32 → 4, INT48 → 6; mode 1, 5: INT32 → 1, INT48 → 2; mode 2, 4 → 3; mode 3 → 1 (other dtypes assert) | 0 |
| OB | dtype 4 → 1; lane mode 0: 16-bit (3, 7) → 1, INT32 → 2, INT48 → 3, 8 / 4-bit assert (`width_inbits % 16 == 0`); lane mode 1: 8-bit → 1, 16-bit → 2, INT32 → 4, INT48 → 6, 4-bit assert | = width (asserted ≤ 2) |

These are lanes × element bits / 128 (one slot line = 128 bits) for 16 output lanes (mode 0 / lane mode 1), 4 output
lanes (modes 1 / 5: INT48 rounds up), 16 × 24-bit (modes 2 / 4), 8 lanes (OB lane mode 0) [I: the arithmetic fits every
row]. The lane mode is `get_ob_lane_mode` (0x2002ac98)'s value (also the conv instruction's `ObDataFormat`, §6.3).
`pixelsize_oneslot_channel(dtype)` — elements per slot line: 1, 2, 4 → 16; 3, 7 → 8; 5 → 4; 8, 9 → 32; 6, 10 assert
(dtype 4 → 16 does not fit a 32-bit float; the conv path never takes FLOAT32 [I]). `alignup_tile_di_hw(di, step)` rounds
the remainder `di % step` (≤ 64) up to the next power of two; `get_alignmentsize_inpixels_hw` gives the di alignment used
by the IB tile size (§6.8).

## 5. SR — the scalar / short-vector register file [R unless marked]

### 5.1 The register file
**32 registers**, id = `bank << 4 | index` (operand structs {u32 index, u32 bank}; every encoder packs `(bank & 1) << 4 |
index & 0xF`). The CPU sees them memory-mapped through the `ipu_sr` descriptor (§3.1: bank 0 = 0x0101_0000, bank 1 =
0x0101_0200) — host CPU view 0x1801_0000 / 0x1801_0200 by the 0x0100_0000 → 0x1800_0000 mapping of §3, a +0x1700_0000 shift [I]. Inside a bank, register *i*
= id & 0xF has one address per **value type** (`hal_ipu_sr_set_value` 0x2003_3506 / `hal_ipu_sr_get_value` 0x2003_3754,
type codes 4–9):

| type | width | bank offset | constraint |
|---|---|---|---|
| 4 | 8 bit | +0x00 + *i* | |
| 5 | 16 bit | +0x40 + 2*i* | |
| 6 | 32 bit | +0x80 + 4*i* | |
| 7 | 128 bit | +0x100 + 16*i* (4 words) | |
| 8 | 256 bit | +0x100 + 16*i* (8 words = registers *i*, *i*+1) | *i* even (`(index & 0x1) == 0`) |
| 9 | 512 bit | +0x100 + 16*i* (16 words = *i* … *i*+3) | *i* % 4 = 0 (`(index & 0x3) == 0`) |

Bank size 0x200 = 0x100 + 16 × 16 [R: arithmetic; matches the bank-1 base]. Whether the 8 / 16 / 32-bit views alias the
low bytes of the 128-bit register or are separate storage is not decidable from the firmware; its value cache treats
each width as a separate entry with its own valid bit (§5.4), which suggests separate views [I]. In the decoder's
address space the SR file is region 0x4000–0x7FFF (part 1 §8.3.1).

### 5.2 CPU access and jump conditions
`sr_write_value(type, id, value*)` (0x2003181a) writes through `hal_ipu_sr_set_value`; for **bank 0** (id < 16) it first writes
**0x0142_0048 = 1** and afterwards 0 (`ipu_global_ctrl` +0x48, `global_ctrl_48_set` (0x200334dc) / `global_ctrl_48_clear` (0x200334e6)) — a CPU write enable or
arbitration for bank 0 [R writes; I: meaning; the register's name is not known] — and then invalidates the cached value
(`sr_cache_set_valid(id, type, 1)` (0x200086da)). `sr_read_value(type, id, value*)` (0x200318aa) reads through `hal_ipu_sr_get_value`, with no bracket.
Users: `drv_jump_CheckJumpCondition` (below), the online NMS kernels (`sgs_NMS_Yolov3_Normal_Ipu`, `nms_online_kernel_first` …
`nms_online_t5` (0x20046b96)), `topk_kernel` (0x2001e412) (TOPK), `dma_relocate_consts`. A CPU read of an SR register is always preceded by draining the
decoder queue (`dec0_drain(0)` (0x20008566), part 1 §8.3.5) in the kernels that read results back (§10.15, §10.16).

`drv_jump_CheckJumpCondition(a, b, cond)` (0x2005_CF6A, `Utility/IPU200M/drv_jump.c`) reads 32-bit SR *a* (bank 0) and
SR *b* + 16 (bank 1) and evaluates, unsigned: 0 *a* > *b*, 1 *a* < *b*, 2 *a* = *b*, 3 *a* ≠ *b*, 4 always, 5 *a* ≥ *b*,
6 *a* ≤ *b*; others log `Unkown condition type %d` and assert. Called by the NMS loop functions 0x2004_3822,
0x2004_3A7A, 0x2004_3E2A, 0x2004_4122: loop control of the online kernels runs on the RISC-V, which waits for the engine
and reads the counters the SR operations computed [I: from the call pattern].

### 5.3 Micro-op encodings (class 0x7F) — the complete set
<!-- ev:ipu-firmware-2-engines.5.3 -->
Every store of the class byte 0x7F / 0xFF into an instruction buffer in the decompile is in the ten encoders
0x2000_88FC–0x2000_8F90 (`drv/drv_sr.c`, `hal_ipu_sr.h`) [R: scan of the decompile for byte stores of 0x7F / 0xFF and for
header constants with bits 6:0 = 0x7F]; there is no other SR emitter. An SR
instruction is a 32-bit header followed by 0–16 immediate words (part 1 §8.5.6). Fields: bits 6:0 = 0x7F; bit 7 = form
bit; **22:18 = rd**; **12:8 = rs**; **17:13 = rb, a 5-bit immediate or the immediate-word count**, by opcode; **30:23 =
opcode** (bit 31 = engine instruction); the 16-bit load uses only 31:28 as opcode. Read as the top bits of the header,
the ten operations are 0xA4, 0xA6 (`smove128`), 0xB, 0x83, 0x848, 0xF28, 0xC08 (add), 0xC48 (and), 0xCA (left shift),
0xCC (right shift). Names from the callers' assert / function names where they exist:

| op (30:23) | header | words | encoder ← callers | operation |
|---|---|---|---|---|
| 0x48 | 0xA400_207F \| rd<<18 (17:13 = 1) | 2: + imm32 | `sr_load_imm32` ← `nms_load_imm_step` (0x2006a19a) (NMS), `SGS_Softmax_RecipOfAcc_OneLane`, `softmax_recip_step` | rd(32) ← imm32 [I: name-less; cache type 6] |
| 0x4C | 0xA600_007F \| rd<<18 \| (4n)<<13 | 1 + 4n | `drv_ipu_sr_smove128_immed` 0x2000_8990 ← Clip, ReduceSum / Reduce* kernels, `reduce_sum_kernel` (0x2005204e) | `smove128`: rd ← 128·n-bit immediate, n = 1, 2, 4 (types 7, 8, 9) [S: name] |
| 0x6x–0x7x (31:28 = 0xB) | 0xB000_007F \| rd<<18 \| imm[10:0]<<7 \| imm[15:11]<<23 | 1 | `sr_load_imm16` (0x20008a7e) ← `cmp_load_imm16` (0x2005ff7e) (compare / Argmax / Reduce helpers) | rd(16) ← imm16 [I: cache type 5] |
| 0x06 | 0x8300_007F \| rd<<18 \| rs<<8 | 1 | `sr_move` (0x20008b24) ← `nms_move_step` (0x2006a1fa) (NMS) | rd ← rs [I]; invalidates rd's 16-bit cache entry |
| 0x06, bit 7 | 0x8300_00FF \| rd<<18 \| rs<<8 | 1 | `sr_move_b` (0x20008ba8) ← `_SGS_ReduceMaxOnInnermostDim`, `softmax_sub_max` (softmax) | second form of the same op [I] |
| 0x09 | 0x8480_007F \| rd<<18 \| rs<<8 | 1 | `sr_unary_op` (0x20008c2a) ← `nms_unary_step` (0x2006a26e) (NMS) | unary register op, writes rd (16-bit entry invalidated) [I]; operation not known |
| 0x65, bit 7 | 0xF280_00FF \| rd<<18 \| ra<<8 \| rb<<13 | 2: + 0x0000_203E | `sr_three_reg_op` (0x20008cae) ← `SGS_Softmax_RecipOfAcc_OneLane`, `softmax_recip_step` | three-register op; word 1 = 0x3E \| 1<<13 with bits 14–18 clear [R]; used as rd = 2^30 / rb with ra = 2^30, i.e. a **division** [I: from "RecipOfAcc" and the operands, §10.8]; its rounding is not known |
| 0x81 | 0xC080_007F \| rd<<18 \| rs<<8 | 2: + imm32 | `sr_add_imm` (0x20008d80) ← **`NMS_IPU_sAlu_add`** | rd ← rs + imm [S: name] |
| 0x89 | 0xC480_007F \| rd<<18 \| rs<<8 | 2: + imm32 | `sr_and_imm` (0x20008df8) ← **`NMS_IPU_sAlu_and`** | rd ← rs & imm [S: name] |
| 0x94 | 0xCA00_007F \| rd<<18 \| rs<<8 \| imm<<13 | 1 | `hal_ipu_sr_leftShift` 0x2000_8E70 ← **`NMS_IPU_sAlu_leftShift`** | rd ← rs << imm, `sr_imm8 < 16` asserted [S] |
| 0x98 | 0xCC00_007F \| rd<<18 \| rs<<8 \| imm<<13 | 1 | `hal_ipu_sr_rightShift` 0x2000_8F12 ← **`NMS_IPU_sAlu_rightShift`** | rd ← rs >> imm [S: name]; signed or unsigned is not known |

The `NMS_IPU_sAlu_*` wrappers (0x2006_A4CA, 0x2006_A56A, 0x2006_A60A, 0x2006_A6AA, `op_custom_ipu_implement.c`) assert
`eISAStatus == EN_IPU_ISA_STATUS_SUCCESS` on the encoder's return. All wrappers build the instruction in a local buffer
and push it with `dec_push_insn(0, buf, 0)` (0x2000844c) (decoder 0, part 1 §8.7) unless the byte 0x200E_696C is non-zero; no writer of
that byte was found in the decompile, so it reads as an always-0 "suppress push" switch [R: no store; I].

### 5.4 The firmware's SR value cache
Lives in the `ipu_sr` descriptor itself (0x2009_1E30): valid masks **+0x14** (8-bit), **+0x18** (16-bit), **+0x1C**
(32-bit), **+0x20** (128-bit; a 256-bit value sets 2 bits, a 512-bit 4); values **+0x24 + id** (8-bit), **+0x44 + 2·id**,
**+0x84 + 4·id**, **+0x104 + 16·id** (to +0x303); **+0x304** byte = cache enable [R: `sr_cache_lookup` (0x2000859a) lookup, `sr_cache_store` (0x200087fc)
store, `sr_cache_set_valid` set / clear valid bits, `sr_cache_clear_all` (0x200088de) clear all]. An immediate load whose value equals the valid
cached value sets the instruction buffer's skip byte (+8) — only if +0x304 ≠ 0 — and nothing is pushed. Moves and the
CPU write path invalidate the destination's entry; shifts, add, and and the three-register op do not touch the cache (so
a later immediate load to a register they wrote could be wrongly skipped if the cache is enabled [I]). An op-1 write
into the SR region of the decoder space drops the whole cache (part 1 §8.3.1).

## 6. Convolution / pooling engine [R unless marked]

The bit layout of the conv instruction 0x0C / 0x2C / 0x8C and of its output stage 0x38 / 0xB8 (field names from
`hal_Conv_CompareParameter` 0x2003_2196, an assert-based self-check of the packed words against their source parameters)
is part 1 §8.5.3–§8.5.4. This section gives the opcode forms, where every field comes from (`hal_conv` 0x2003_2684 and
its caller `convolution_IPU` 0x2006_AA06), the parameter records behind the output stage, the resize form, and the tile
plan the conv kernels execute. The asserts of `hal_Conv_CompareParameter` name the hardware fields: `IbBufAddr`,
`KbBufAddr`, `AbBufAddr`, `ObBufAddr`, `conv_mode`, `is_int8`, `HwTileHeight` / `HwTileWidth`, `di_hw` / `do_hw`,
`dx_num` / `dy_num`, strides sx / sy, dilations fsx / fsy, paddings tp / bp, `kb_iscompressed`, the bias / shift /
rcshift flags. Source files: `hal_conv.c`, `conv_pool_ipu.c`, `conv_priv_common.c`, `sdk_conv.c`,
`sdk_conv_hw.c`, `sdk_pool.c`, `tile_partition_util.c`, `tile_partition_util_hw.c`.

### 6.1 Opcode byte and buffer ids
The engine's opcode is the whole low byte: 0x0C | out<<5 | ab<<7 by `hal_conv`'s output target (argument 36, from
`convolution_IPU` argument 16): target 0 → **0x0C** = accumulate only (the result stays in AB), 1 → **0x2C** = output
to a vector-processor buffer (followed by the 0x38 stage), 2 → **0x8C** = output to a conv buffer; target 3+ asserts. **0xAC** is
a fourth form, the resize instruction of §6.6 (its own encoder). The three 5-bit buffer fields of the header (IB 12:8, KB
17:13, OB 22:18) are `combine_bufid_type(id, pp)` = **VR id & 0xF | ping-pong index << 4**, the index being the buffer's
current ping-pong selector (record +0x9C, §4.2) read by `vrbuf_get_pingpong_type`. Conv addresses (word 1 IbBufAddr / KbBufAddr,
word 2 AbBufAddr, word 9 ObBufAddr) are the §4.5 conv-space addresses; for target 1 the OB address is converted with
`vrbuf_conv_ob_convert_convaddr2vpaddr` (ping-pong 0).

### 6.2 Conv modes and derived fields (`hal_conv`)
<!-- ev:ipu-firmware-2-engines.6.2 -->
**conv_mode** (Conv2D option +0x48 `s32PartitionConvIpuMode`, part 1 §7.3): 2 = `EN_IPU_CONV_MODE_DEPTHWISE`, 4 =
`EN_IPU_CONV_MODE_PIXELMODE` [S: assert strings]. `hal_set_simd` (0x2003_20E6) gives each mode its lanes {input channels
per pass for 8-bit / 16-bit input, output channels per pass, taps}: mode 0 and 4 {16 / 8, 16, 1}; 1 and 5 {64 / 32, 4,
1}; 2 {16 / 8, 1, 9}; 3 {16 / 8, 2, 9}; others assert — the shape depends on the conv mode and on 8- vs 16-bit input.
The accumulator dtype (`get_abDtype_fromib`): IB dtype 4 → 4; modes 2 and 4 → 10 (INT24); IB dtype 1 or 9 → 5 (INT32);
IB dtype 3 → 6 (INT48). Names of the modes other than 2 and 4 are not in the binary [R: strings]; from the lane shapes:
0 = 16 in × 16 out channels per pass (the general mode), 1 / 5 = 64 in × 4 out, 3 = 3×3 window as 9 taps × 2 out
channels, 2 = 9 taps, one output per input channel (depthwise); 5 differs from 1 only in the no-padding, no-dilation
kernel arithmetic below [I].

Inputs (as `convolution_IPU` passes them): conv_mode *m*; IsInt8 *b* (IB dtype 1 or 9); IB tile width *W*, height *H*,
channels *di*; kernel width *kw*, height *kh*; output channels *do*; strides *sx*, *sy*; dilations *fsx*, *fsy*; paddings
*lp*, *rp*, *tp*, *bp* (from the IB hardware-tile record words 0, 2, 3, 0xD–0x15, §6.8). With (*dl*, *ol*, *taps*) =
`hal_set_simd(m, b)` (*dl* = 16 / 64 lanes for 8-bit, 8 / 32 for 16-bit), U = 32 if *b* else 16, and c(x) = ⌈x / U⌉ − 1:

- *di'* = `alignup_tile_di_hw(di, dl)` (remainder rounded up to a power of two, ≤ 64 asserted); *r* = *di'* mod *dl*;
  *fdi* = min(*dl*, *di'*); *fdo* = min(*ol*, *do*); *frag* = *do* mod *ol*.
- word 5 tile counts: `Full_di_full_do` = c(*fdi*·*fdo*·*taps*), `Frag_di_full_do` = c(*fdo*·*r*·*taps*); mode 2:
  `Frag_di_frag_do` = c(*r*·*taps*), `Full_di_frag_do` = c(*fdi*·*taps*); other modes: c(*frag*·*r*·*taps*),
  c(*fdi*·*frag*·*taps*) (all 4-bit, so −1 → 0xF = "none").
- *K* = *kw*·*kh*; modes 2 and 3 (the 9-tap modes): *K* = ⌈*kw*/3⌉·⌈*kh*/3⌉, and the paddings must be ≤ 1 on every side
  (`padding_left <= 1 && …` asserts).
- word 4: `full_di_reqnum` = ⌊*di'*/*dl*⌋·*K* if *dl* ≤ *di'*, else *K*; `CNum` = ⌈*di*/*dl*⌉ − 1.
- word 7: `kbbuf_reqnum` = ⌈*di*/*dl*⌉·*K*; `HwTileWidth` = *W*; `CompressKb` = (KB tensor +0x30 == 2).
- word 5 `WH`: *W*·*H*; modes 2 / 3: ⌈*W*·0.5⌉·⌈(*tp*+*H*+*bp*)/3⌉·2.
- word 6: `FxNum` = (*kw* − 1)·*fsx*, `FyNum` = (*kh* − 1)·*fsy*; modes 2 / 3 with ⌈*kw*/3⌉, ⌈*kh*/3⌉; mode 5: *kw* − 1,
  *kh* − 1.
- word 3: `Dxnum` = (*lp* + *W* + *rp*) − (*FxNum* + 1), `Dynum` = (*tp* + *H* + *bp*) − (*FyNum* + 1); modes 2 / 3:
  − (3·*FxNum* + 1); mode 5: *W* − *kw*, *H* − *kh* (no padding) — the span of kernel origins.
- word 8: `di_hw` = *di* − 1, `do_hw` = *do* − 1; word 1 `nBufSel01` = 0 and word 2 `AbBufId` = 0 from every caller.

⌈ ⌉ / ⌊ ⌋ are `ceil` 0x2007_05BC / `floor` 0x2007_09DC on doubles (`__floatsidf` 0x2007_562C, `__divdf3` 0x2007_3D90,
`__muldf3` 0x2007_46F0, `__fixdfsi` 0x2007_5530; constants 0.5 at 0x2008_6170, 3.0 at 0x2007_DF58) (§9.1).

**Check against the Anjoy block** [R: over the 960 expanded conv instructions of
the block, with one delta base shared by the conv classes (part 1 §8.4)]: the tile counts, `CNum`, `full_di_reqnum`,
`kbbuf_reqnum` and `WH` recomputed from the primary fields match in all 908 conv-form instructions; the other 52 are the
0xAC resize form (§6.6). `Dynum` matches in all 908 too; right padding is not encoded, and `Dxnum` implies *rp* = 0 (851)
or 1 (57). The Anjoy layers use mode 4 (layer 0, 3 input channels), **mode 3 (layer 1)** and mode 0 (all others)
[R]; mode 3's ⌈3/3⌉ − 1 = 0 is why layer 1's 3×3 kernels show `FxNum` = `FyNum`
= 0.

### 6.3 Per-channel parameters
**Flag bytes** (`convolution_IPU` argument 5, filled by `conv_flag_bytes` (0x2006b2c2) from the bias tensor's first word *n* = number of
per-channel parameter vectors, and per tile by `Conv_FFDIDOHW` / `Conv_FFDIHWDO` / `DepthWise_Conv_FFDIHWDO`):

| byte | set when | goes to |
|---|---|---|
| 0 | *n* ≥ 2 (a scale vector) | word 9 bit 30 `NeedScale`; output stage 0x38 (with scale) vs 0xB8 (without) |
| 1 | *n* ≥ 3 (`has_obshift`) | word 4 bit 30 `ReadObShift` |
| 2 | *n* ≥ 4 (`bias_para->has_rcshift_vec`, asserted together with byte 1) | word 4 bit 31 `ReadRcShift` |
| 3 | *n* ≥ 5 (a 32-bit vector) | enables byte 4 |
| 4 | per tile: byte 3 set and this pass does not read AB back (`ReadAb` = 0) | header **bit 30** and the 10-word form when the target is AB (0x0C with *L* = 9); `ReadAb` = 1 with byte 4 set asserts |

So header bit 30 marks "apply the fifth (32-bit) per-channel vector on this pass" and forces word 9 (its `BiasAddr`
etc.) into an accumulate-only instruction [R: flag flow; I: the reading of the fifth vector's role; what the engine does
with that vector is not known]. `load_bias_paras_group` (0x2006_B2F2) loads the per-channel parameter group from EMI
into L2 after an op-3 barrier on the target buffer: u16 vectors (bias, plus scale when byte 0) as dtype 3 / DDR type 0xC
to L2 offset 0; when byte 1: two s8 vectors (ob-shift, rc-shift) as dtype 2 / DDR type 9 from EMI + ⌈n_ch/8⌉·32 to L2
offset 0x100 into the KB buffer; when byte 3: one s32 vector as dtype 5 / DDR type 0xE from EMI + (⌈n_ch/8⌉ +
⌈n_ch/16⌉)·8 to L2 offset 0x180 [R: arguments of `load_conv_para_group_fromemi`]. Word 9 `BiasAddr` = L2 bias offset / 8
(offset asserted `% 8 == 0`); `NeedBias` = a bias tensor is present; `ObDataFormat` = (`get_ob_lane_mode` lane mode == 1).

### 6.4 Post-quantisation record and clamp bounds
`convolution_IPU`'s last argument, built in `conv_exec_layer_callback_helper` 0x2005_B83A; the output stage packs it
(part 1 §8.5.4):

| offset | content | source (Conv2D / DepthwiseConv2D option field, part 1 §7.3) | used as |
|---|---|---|---|
| +0x00 | s32 accumulator shift | field 35 +0x31C / field 36 +0x320 | \|v\| → conv word 9 `CfgTruncate` (6 bits), v < 0 → word 9 `ShiftDirection` |
| +0x04 | u32 | field 36 +0x320 / field 37 +0x324 | low u16 → output stage word 3 (0xB8 form only) — a per-layer scale when there is no scale vector [I] |
| +0x08 | u32 | field 37 +0x324 / field 38 +0x328 | low 5 bits → output stage word 1 bits 21:17 (output shift) |
| +0x0C | u16 zero point | `**(tensor + 0x4C)` = zero_point[0] of the layer's **input** tensor (`pstInput`, `tensor_zero_point(…, 0)` (0x2002c3d6)) | the padding value (§6.5) |
| +0x0E / +0x10 | s16 upper / lower clamp | `sgs_Get_ConvPostQuant_Boundary` from the **output** tensor (`pstOutput`, `tensor_post_quant_bounds(…, 0)` (0x2002c3b8)) and the fused activation (below) | output stage word 2 (upper \| lower << 16) |

Conv2D field 38 (+0x328, i16) is not part of the record: it is the last argument of `Conv_FFDIDOHW` / `Conv_FFDIHWDO`
(compared with 4 when deciding `ReadAb`, §6.10), and `== 2` selects a loop variant in the callback; its meaning (a
partition / accumulation mode) is not named [I].

**`sgs_Get_ConvPostQuant_Boundary`** (0x2002_7A06): called three times from `conv_exec_layer_callback_helper` (call
sites 0x2005_BB8A, 0x2005_BD56, 0x2005_BDF2: loop orders 2 and 1 of Conv2D, and DepthwiseConv2D) with the output tensor
and the option's fused activation (Conv2D option +0x1C, DepthwiseConv2D +0x20; TFLite numbering 0 NONE, 1 RELU,
2 RELU_N1_TO_1, 3 RELU6 [S: TFLite schema; R: the four branches]). It writes the lower bound to its third argument and
the upper bound to its fourth (record +0x10 / +0x0E) and returns 1, or 0 after the log line `Unsupport eTensorTypeInEmi
%d …` for another activation (the caller then asserts). Notation: *s* = output scale (f32), *z* = output zero point (low
16 bits, signed), `q(c)` = `round((double)(float)(c / s))` — the quotient is a **single-precision** division
(`__divsf3`), widened to double, rounded half away from zero (§9.1); `clampD(v, lo, hi)` = lo if v < lo, hi if v > hi,
else `(int)v` (comparisons on the double).

| output type | NONE (0) | RELU (1) | RELU_N1_TO_1 (2) | RELU6 (3) |
|---|---|---|---|---|
| INT16 (7) | [−32767, 32767] | [0, 32767] | [clampD(q(−1), −32767, 32767), clampD(q(1), −32767, 32767)] | [0, clampD(q(6), 0, 32767)] |
| UINT8 (3), *M* = 255 | [0, *M*] | [*z*, *M*] | [clampD(q(−1) + *z*, 0, *M*), clampD(q(1) + *z*, 0, *M*)] | [*z*, clampD(q(6) + *z*, 0, *M*)] |
| UINT4 (11), *M* = 15 | as UINT8 with *M* = 15 | | | |
| any other | assert `sgs_firmware.c` line 0xCC | | | |

INT16 is symmetric — no zero point enters, and the lower bound is −32767, not −32768. For RELU6 on INT16 the lower clamp
of `q(6)` is 0 (a negative scale would give 0). For UINT8 / UINT4 the RELU and RELU6 lower bound is the zero point taken
as is (not clamped). The constants are f32 6.0 (0x2008_84C0), f32 −1.0 (0x2008_84C4), f32 1.0 (0x2007_D454), f64 ±32767.0
(0x2008_84C8 / 0x2008_84D0). How the engine applies the record (order of the accumulator shift, scale, output shift and
clamps, and its rounding) is not visible to the firmware (§12).

### 6.5 Pad value (decoder address 0x1900) and decoder address 0x1908
When the zero point is non-zero and any of the four paddings is non-zero, `convolution_IPU` emits **op 1 to address
0x1900, 2 bytes** = the zero point (for a 4-bit input, dtype 9: `s16ZeroPoint <= 15` asserted and the nibble doubled,
zp | zp << 4) and sets word 6 bit 31 `padding_not_0`. So decoder address 0x1900 is the conv engine's pad-value register
[I: from the use]. The 4-byte write to **0x1908** has two firmware writers: `dma_op_end` (0x2002fffe) (end of a DMA layout op —
`SGS_OPS_Transpose`, `dma_layout_op` (0x200307e4)) re-installs the default vector buffers (§4.4) and writes **0x1908 = 0**, exactly
the Anjoy epilogue's op 1 (part 1 §8.6); `dma_op_start(id, slots, n, flag)` (0x2002ff56) (start of a DMA op — `SGS_DMA_OpsStart`,
`SGS_OPS_Transpose`, both giving VR 0 all 32 slots 0 … 31 as a customer vector table) would write **0x1908 = id |
0x8000_0000** when `flag` = 1, but both callers pass 0, so that path is unused. So 0x1908 holds "VR *id* selected" in its
low bits with an enable in bit 31, cleared when the DMA op returns L2 to the default layout [I: meaning from the two
writers; the register's name is not known].

### 6.6 Resize form 0xAC (`resize_conv_form` (0x20033072) ← `_SGS_ConvBilinear`, `DMA/IPU200M/drv_resizebilinear_ipu200.c`)
9 words (*L* = 8); header 0xAC | source VR (5-bit, id | pp<<4) << 8 | destination VR << 18 | 8 << 23 | bit 31:

| word | content |
|---|---|
| 1 / 2 | quantised x / y start fraction (qfX / qfY) |
| 3 | output tile width (15:0) / height (31:16) |
| 4 | input tile width / height |
| 5 | input L2 offset (15:0) / output L2 offset (31:16), advanced per channel line by the input / output window areas in_w·in_h / out_w·out_h |
| 6 / 7 | quantised x / y scale (qsW / qsH) |
| 8 | 5:0 x shift, 11:6 y shift (the fixed-point exponents), 12 pixel mode, **13 nearest** (`_SGS_ConvBilinear` argument 5: 1 from `SGS_OPS_ResizeNearest`, 0 from `SGS_OPS_ResizeBilinear`), 14 channels ≠ 1, 15 source hardware buffer (1 bit), 17:16 destination hardware buffer (2 bits) |

The scale, fraction and shift values are computed by `fixq31` (0x2005d2b0) as §10.11 gives. Hardware buffer numbers
(`_hal_convBilinear_get_hardware_buffer_id` 0x2003_3024): VR 8 or 0xA → 0, 0xB or 0xC → 1, 0xE → 2, 0xF → 3; other VR
ids assert. Anjoy cmd+0x11930: 0x8438_08AC 0 0 0x001E_0011 0x000F_000A 0 0x4000_0000 0x4000_0000 0x0002_67DF → VR 8 →
VR 0xE, output 17 × 30 from input 10 × 15, scales 0x4000_0000 with shifts 31 = 0.5 per axis (a ×2 upsampling, the
model's RESIZE_NEAREST_NEIGHBOR), bits 13 and 14 set [R values; I: the link to the model op]; this is the value
`fixq31` gives for sW = sH = 0.5, fX = fY = 0 (shift 31, qs = 0x4000_0000, qf = 0) [R]. The 52 Anjoy instructions of
class 0x2C with bit 7 are this form, its two RESIZE_NEAREST_NEIGHBOR.

### 6.7 Tile planning: where the plan comes from
Functions: `conv_exec_layer_callback_helper` 0x2005_B83A, `Conv_FFDIDOHW` 0x2006_B6D2, `Conv_FFDIHWDO` 0x2006_CB22,
`DepthWise_Conv_FFDIHWDO` 0x2006_C220, the `tile_partition_util*.c` functions 0x2002_A9D0–0x2002_C2CC,
`get_weight_hwtile_info_invb_online` 0x2002_BBFA [R].

The firmware does **not** choose tile sizes: every size comes from three **partition records** the converter stores in
the op's options (the SGS-only Conv2D / DepthwiseConv2D fields, part 1 §7.3), read by
`get_IB_tensor_and_partition_from_interpreter` (= `get_elementsize_inbits_280e6` 0x2002_80E6), `get_kb_tensor` (0x2002826a) (KB),
`get_ob_tensor` (0x200282fa) / `get_OB_tensor_and_partition_from_interpreter` (OB), plus the slot lists (`stPartition{KB,IB,OB,AB,VB}
GroupsSlot`, slot-group vectors such as Conv2D field 31; the first-slot lists of §4.3) and the per-tile KB table `kb_tile_info` (Conv2D field 40: count +0x338,
pointer +0x33C; DepthwiseConv2D +0x33C / +0x340; §6.9). "Online" planning means only this: when the model carries **no**
`kb_tile_info` (count or pointer 0), the callback allocates 0x16_0000 bytes and builds the table itself with
`get_weight_hwtile_info_invb_online` (≤ 0x2_0000 tiles, `u32TileNum <= MAX_HW_TILE_NUM`), freeing it afterwards. The
converter must produce a partition that fits (§6.10, asserts); the firmware never re-splits.

**KB partition** (`kbpartition_info`, words; the `kb_dim_count` and `ff % ff_simd` asserts name the parts) [R offsets;
I: the names in brackets]:

| off | field | | off | field |
|---|---|---|---|---|
| +0x00 | [di_simd] — di alignment of a KB hardware tile | | +0x18 | [di_sw] — di per software tile |
| +0x04 | [do_simd] | | +0x1C | [do_sw] |
| +0x08 | `ff_simd` | | +0x20 | [di] — total input channels |
| +0x0C | `ff` (taps; `ff % ff_simd == 0`) | | +0x24 | [do] — total output channels |
| +0x10 | `di_hw` — di per hardware tile | | +0x28 / +0x2C | kernel width / height |
| +0x14 | `do_hw` | | | |

`get_kb_dim_counts` (0x2002_B348): counts {⌈di / di_sw⌉, ⌈di_sw / di_hw⌉, ⌈do / do_sw⌉, ⌈do_sw / do_hw⌉}, each asserted
> 0 (`kb_dim_count->di_sw_dim_count > 0` …).

**IB partition** (`ibpartition_info`, words, as `get_ib_hwtile_fromob` reads it): [0] di, [2] di per tile, [4] input
width, [6] input height, [8] / [9] stride x / y, [10] / [11] padding left / top, [14] / [15] dilation x / y [R positions;
I: names]. **OB partition** (`obpartition_info`): [0] do, [1] do per software tile, [2] do per hardware tile, [4] width,
[5] tile width, [6] height, [7] tile height [R positions].

**Loop order and mode checks** (`conv_exec_layer_callback_helper`): Conv2D field 15 (+0x4C) is the **loop order**: 1 →
`Conv_FFDIDOHW`, 2 → `Conv_FFDIHWDO`, else `invalid parameter looporder %d` and assert; a DepthwiseConv2D asserts op =
DEPTHWISE_CONV_2D and mode 2 and calls `DepthWise_Conv_FFDIHWDO`. When the input tensor's DDR type (model tensor +0x18)
is an image type (1–5 or 15–17, part 1 §6.5) the layer must be PIXELMODE (mode 4) with KB and IB partitions of `di_hw ==
3` (asserted) — the first-layer path (Anjoy layer 0, mode 4). A first conv layer whose option +0x20 is set also programs
the DMA colour matrix (`dma_color_matrix` (0x2006b260), §10.19). `get_conv_configinfo_from_interpreter` reads the loop
configuration (`stConfigInfo`, whose `split_mode.kb_split_mode` an assert compares with `EN_TILE_KB_MODE_FFdido`):
Conv2D {+0x60 field 20 = **IB reuse mode**, +0x50 field 16 = **KB reuse mode**, +0x48 conv mode, i16 +0x328 field 38};
DepthwiseConv2D {+0x64, +0x54, +0x4C} [R; I: the "reuse" reading, from §6.10]. `conv_buffer_shapes` (0x20027ed4) builds the four buffer
shape records {conv mode, usage, depth in slots, first-slot list} — IB {usage 1, +0x18C, list +0x10C}, KB {3, +0x108,
+0x88}, AB {4, +0x294, +0x214}, OB {2, +0x210, +0x190} (Conv2D offsets; DepthwiseConv2D +4) — that slot 0 of the ops
table 0x2009_1D10 passes to `conv_engine_setup` (§4.4) before the loops, and slot 1 tears down after. So the L2 slot lists
are the converter's slot groups, unchanged, and the slot-group counts at +0x108 / +0x18C / +0x210 / +0x294 are the KB /
IB / OB / AB buffer depths in slots.

### 6.8 Tile shapes and buffer depths
**OB hardware tile** (`get_ob_hwtile_shape_byindex` 0x2002_BA30; the software tile `get_ob_swtile_shape_byindex`
0x2002_B920 is the same with do per software tile): for origin (x0, y0, do0): +0x04 do = min(do_hw, do − do0), +0x08 w =
min(tile w, W − x0), +0x0C h = min(tile h, H − y0), +0x10 x0, +0x14 y0, +0x1C do0, +0x20 = tensor +0x20 (asserts OB
usage 2). `get_ob_tile_size_invb`: +0x2C = OB width in slots (§4.6), +0x30 **depth** = ⌈do / *P*⌉ · w · h lines, *P* =
elements per slot line × width in slots.

**IB hardware tile from an OB tile** (`get_ib_hwtile_fromob` 0x2002_B456, kernel *kw* × *kh* from KB partition +0x28 /
+0x2C): first input column xs = sx·x0 − lp, last xe = (x0 + w − 1)·sx − lp + (*kw* − 1)·dil_x (asserted ≥ 0,
`w1_dag_br >= 0`), the same in y; the range is clipped to [0, input width − 1] / [0, height − 1] and **what is clipped
becomes the tile's padding** (left = −xs if xs < 0, right = xe − (W_in − 1) if beyond; top / bottom alike). Record
words: [0] di = min(di per tile, di − di0), [2] / [3] width / height, [4] / [5] / [6] x / y / di origin, [8] tensor
+0x20, [0xD] / [0xE] sx / sy, [0xF] lp, [0x10] tp, [0x11] rp, [0x12] bp, [0x13] / [0x14] dilations — exactly the inputs
`convolution_IPU` hands `hal_conv` (§6.2). `get_ib_hwtile_size_invb` 0x2002_B6A2 then sets +0x2C width in slots and
**depth** (+0x30) = size / *P*_IB (asserted divisible: `total_size_inpixels % pixelsize_oneline == 0`), size = di' · W ·
H with di' = di rounded up to `get_alignmentsize_inpixels_hw` (§4.6), modes 2 / 3: W rounded up to even and H' =
⌈(tp + H + bp)/3⌉·3; on the **first-layer** path (image input, flag `first_layer`): di must be below the alignment and
size = ⌈W·H·4 / *P*⌉·*P* (four bytes per pixel).

**AB** (`get_ab_hwtile_size_invb` 0x2002_BB40): depth = W·H lines, or ⌈do / *P*⌉·W·H when di is split (di ≠ di_sw or ≠
di_hw). **Fixed depths**: the IB buffer is allocated with the depth of the partition's nominal IB tile (`ib_tile_depth` (0x2002c240))
except at the last tile (`pstIbHwTileShape->vrbuf_depth <= s32IbFixedHwTileVrBufDepth`); the KB buffer with the largest
tile depth of the KB table (`get_kb_fixedhwtile_size_invb` 0x2002_C2CC).

### 6.9 The KB tile table (`kb_tile_info`, 11-byte records)
The records are written outside the option struct at a cursor the parser advances; the struct holds count + pointer
(part 1 §7.2).

| off | content |
|---|---|
| +0x00 | u32 byte offset of the tile in the layer's weight data |
| +0x04 | u32 byte size (`tile_data_lenth`) |
| +0x08 | u16 KB buffer depth in lines |
| +0x0A | flags: bit 0 = **zero tile** (skipped), bit 1 = **last** non-zero tile of its output group (the pass that writes OB), bit 2 = **not first** (accumulate onto AB: `ReadAb`) |

Index of the tile (`conv_tile_index` (0x2002b3d4)) = ((*i*_do_sw · *n*_di_sw + *i*_di_sw) · *n*_do_hw + *i*_do_hw) · *n*_di_hw +
*i*_di_hw with the counts of `get_kb_dim_counts`. The online builder (`get_weight_hwtile_info_invb_online`) fills it in
that nesting order: per tile the weight shape (`get_weight_hwtile_shape_byindex`: di and do of the tile, element count
*kw*·*kh*·di·do), then with *P* = pixels per KB line, a = ⌊di/di_simd⌋, r' = `alignup_tile_di_hw`(di mod di_simd), b =
⌊do/do_simd⌋, s = do mod do_simd, f = ff_simd:

pixels = (ff / f) · ( ⌈s·r'·f / *P*⌉ + b·⌈do_simd·r'·f / *P*⌉ + a·⌈di_simd·s·f / *P*⌉ + a·b·⌈do_simd·di_simd·f / *P*⌉ ) · *P*

— the four full / fragment blocks of the conv engine's tile counts (§6.2), each padded to whole lines; depth = pixels /
*P*, size = pixels · element bits / 8, offset = running sum of sizes. An empty tile (element count 0) gets size 0, depth
0, flags 3. A tile is marked zero either by a **sparse bitmap** (the model's per-tile bit, when present; a zero tile then
occupies no bytes) or, without one, by scanning its weights for all zeros (`_check_HWKBTile_allZero`; the bytes stay in
the data). A second pass over each (do_sw, do_hw) group sets bit 2 on every non-zero tile after the first and bit 1 on
the last non-zero one; when the whole group is zero, its last tile gets flags = 2 exactly (zero bit cleared), so one
pass still runs and writes the output (bias only).

### 6.10 The loop nests [R: loop bounds, steps and calls; I: the summary words "reuse", "resident"]
Notation: OB partition {do, do_sw, W, w, H, h}, KB partition {di, di_sw, di_hw, do_sw, do_hw}; IB = VR 0xD, KB = 8, AB =
0x10, OB = 0xF (§4.2); "reset" = `vrbuf_set_depth_used(id, 0)` + ping-pong 0; "save / restore" = the buffer's allocation pointer
and ping-pong index. Every conv layer starts with an op-7 marker naming the IB buffer (part 1 §8.3.4).

**`Conv_FFDIDOHW` (loop order 1)**

```
for y0 in 0..H step h:                                   # OB partition [6]/[7]
  for x0 in 0..W step w:                                 # [4]/[5]
    if KB mode == 2: reset KB                            # weights stay resident across all later (x0,y0)
    for do0 in 0..do step do_sw:                         # OB partition [0]/[1]
      if IB mode == 2: reset IB
      for di0 in 0..di step di_sw:                       # KB +0x20 / +0x18; AB pointer saved at di0 = 0, restored after
        if KB mode == 1: reset KB
        for o in do0 .. do0 + min(do_sw, do − do0) step do_hw:
          OB hw tile (x0, y0, o); AB tile size; AB alloc (AB reset first if di = di_sw = di_hw)
          IB: reset (IB mode 1) / restore to the position saved at the first o (mode 2)
          for i in di0 .. di0 + min(di_sw, di − di0) step di_hw:
            IB hw tile from the OB tile (§6.8); IB alloc (fixed depth unless the last di tile)
            load IB from EMI (load_ib_fromemi_16_2_9_helper) only if (IB mode != 2 or do0 == 0) and o == do0
            t = kb_tile_info[index(do0/do_sw, di0/di_sw, (o−do0)/do_hw, (i−di0)/di_hw)]; KB alloc (fixed depth)
            if not t.zero or field 38 == 4:
              load KB tile (offset, size) only if (KB mode != 2 or x0 = y0 = 0)
              load the bias / scale / shift group when the do range moved (≤ 0x400 channels per load)
              ReadAb = t.not_first or (field 38 == 4 and i is not the first)
              if t.last: OB alloc (ObBufAddr)
              convolution_IPU(...)                       # one conv + output stage (§6.1-§6.4)
            flip IB ping-pong if more than one IB tile per do range; flip KB ping-pong when double-buffered
    OB sw tile size; store OB to EMI (store_conv_ob_toemi), or keep it in L2 for the next fused op
      (_Conv_GetObTilePartitionInfo when the following op consumes L2 directly); flip OB ping-pong;
      OB pointer = 0 or half its depth (ping-pong halves); AB reset
    (fused next op present: re-plan its buffers, `conv_replan_fused` (0x200276ca) when the conv buffers take ≤ 24 slots)
```

The do-block loop and the store sit at the same level: an OB software tile (x0, y0, do-block) is finished and stored
before the next do-block. KB data are loaded once per layer in KB mode 2 (all KB tiles resident) and per di step in mode
1; IB data once per (x0, y0) in IB mode 2 and once per do-block in mode 1.

**`Conv_FFDIHWDO` (loop order 2)** — the same body with the **do-block loop outermost**: `for do0 in 0..do step do_sw:
for y0: for x0: for di0: for o: for i: …`, store per (do-block, y0, x0). The KB tiles of one do-block are reused over the
whole image; IB tiles are reloaded per do-block [R: loop markers at 0x2006_CB22 +0x1F0 … +0x8A0].

**`DepthWise_Conv_FFDIHWDO`** — `for c0 in 0..di step di_sw (KB +0x20/+0x18): for y0: for x0: for i in c0 … step IB di
per tile: IB tile, KB tile (load_conv_kb_fromemi), bias group, convolution_IPU (mode 2, one output channel per input
channel); flip IB / KB; then store the OB tile, flip OB`. Depthwise has no di accumulation, so no AB read-back.

**Asserts that bound the plan**: `pstIbHwTileShape->vrbuf_depth <= s32IbFixedHwTileVrBufDepth`,
`pstKbHwTileShape->vrbuf_depth <= s32KbFixedHwTileVrBufDepth`, `CONV_PARA_SIZE_IN_PIXELS >= s32KbDohw` (≤ 0x400 output
channels per bias group), `vrbuf_depth(u32ObBufId) >= 2*stObSwTileShapeInfo.vrbuf_depth` (OB ping-pong needs two tiles),
`pstObShapeInfo->tile_w * pstObShapeInfo->tile_h <= ONE_SLOT_DEPTH` (an OB tile's pixels fit 512 lines),
`_Conv_IsObDoHwValid(pstObTensor, pstObPartitionInfo->do_hw)`, `s32L2BiasOffset % 8 == 0`, and the `vrbuf_alloc`
failure asserts.

## 7. Vector unit (`vp1_hal.h`, `datatype_convert_util.c`, the kernels' own encoders)
<!-- ev:ipu-firmware-2-engines.7 -->

Framing (header, length, delta form) is part 1 §8.2–§8.5.1. The bodies below are the union of what the firmware's
encoders write [R: every encoder outside the DMA, conv, output-stage, LUT and SR
classes — 78 functions — passed through a scan that groups them per class byte and reduces them to written bit fields,
with their callers; encoders that the scan reads through an array base (`auStack_74[0]`, class 0xBA) read by hand].
Names exist only where a string or a firmware use gives them; otherwise the operation is inferred from the kernel that
emits the class [I], and where neither helps the operation is not known (§12). The generic per-lane helpers the custom
ops call are the `vp1_*_hal` functions (`vp1_vPoolingAvg_hal`, `vp1_vRoiPooling_hal`, `vp1_vCopyNByx_hal`, …).

### 7.1 Common header conventions and the two operand forms [R]
Register fields hold **VR buffer ids** — `id & 0xF` at 11:8 (A) / 16:13 (B) / 21:18 (C) with the buffer's ping-pong
index at bit 12 / 17 / 22 (the same `combine_bufid_type` packing as the conv engine, §6.1); L2 addresses inside a buffer
are 14-bit line numbers (the vector-space address, §4.5). Header bits **29:27** are a 3-bit field and **bit 30** a form
flag; bit 31 = 1. Two operand forms occur:

- bit 30 = 0: A / B / C are VR buffer ids with their ping-pong bits (the element-wise, reduce and copy kernels);
- bit 30 = 1 ("register form"): A / B / C are written as `x & 0xF` (often `x & 7`) without ping-pong bits and 29:27
  carries another 3-bit caller value (the NMS helpers 0x2006_8xxx–0x2006_Axxx write their 4th argument there; in 0xBA
  it repeats C). The body's addresses are then usually 0, i.e. the operands are whole registers [I: register form].

Besides the VR buffers the instructions name **vector registers** by small numbers in body fields (register 6 = the lane
reduction target, register 8 = a loaded constant vector, §7.8) and **SR registers** (§5) [R: the values; I: that these
are register files of the vector unit].

**Second-operand source** (w4 13:0 in the compare / select / expand classes): **5** = the SR scalar register 5, **8** =
vector register 8, **6** = register 6 [R: values per path in §10.4, §10.14; I: the name "operand source"]. The bit-7
("scalar") variant of a compare class takes the SR operand, the plain class a VR or vector register 8 (§7.4).

**Element format** (w1 22:21 in the short form; printed `eOpFormat` by the L2 copy's debug line): 0 u8, 1 s8, 2 s16
[R: `_SGS_OPS_GetHwDataFormat`: INT8 → 1, UINT8 → 0, INT16 → 2, other → log `invalid tensor type` and 1]; the value 3
written by class 0x66 is a fourth format (32-bit [I]).

### 7.2 The short form (V): 4 or 5 words, format A [R positions]
*L* = 3 → 4 words, 4 → 5 words.

| word | bits | content |
|---|---|---|
| 1 | 15:0 | 16-bit operand: in `SGS_OPS_CopyDataInL2` 15:8 = `u8VecLen` (lines per vector, ≤ 0xFF) and 7:0 = 1; elsewhere a length, an immediate, a fill value or a lane mask from the caller |
| 1 | 20:16 | sub-operation (written as a 7-bit value at 22:16 with 22:21 = 2: 0x41, 0x43, 0x47 = sub-ops 1, 3, 7 in 0x3E / 0x3F / 0x53) |
| 1 | 22:21 | **element format** `eOpFormat` (§7.1): 2 in the copy (its debug print `u8VecLen=%d. u8DstBufId=%d, bufType=%d, eOpFormat=%d, waddr=%d` prints 2 for the 0x40_0000 it writes); 3 (0x60_0000) in 0x66; from the caller (`(x & 3) << 21`) in the 0x43 `SGS_DMA_FillMemory` path and 0x6D |
| 1 | 23 | flag (0x80_0000) |
| 1 | 24 | flag (0x100_0000): in the predicated writes, selects which lanes are written (§7.4) |
| 1 | 31 | flag (0x8000_0000 — e.g. 0x80C0_0000 in max / argmax 0x02) |
| 2 | 12:0 | a 13-bit L2 line (second read operand where used) |
| 2 | 26:13 | a 14-bit L2 line — **`waddr`** in the copy (the destination vector address) — or a vector-register number |
| 2 | 27, 28, 29 | flags (bit 29 = 0x2000_0000: "write a predicate" in the compare classes, §7.4; 28 = a caller bit; 27 in 0xA1 / 0xA3 / 0xBA) |
| 3 | 13:0 | a 14-bit L2 line — the **source** in the copy (`u32SrcL2Addr`) |
| 3 | 29:14 | 16-bit **repeat count**: number of vectors (copy: 255-line vectors; most others ⌈*n*/8⌉ or 1 = 0x4000) |
| 4 | 13:0 | the second-operand source (§7.1) or another small code (1, 4, 5, 6, 8 by class) |
| 4 | 31:16 | 16-bit second immediate |

`SGS_OPS_CopyDataInL2` (0x2002_E240, class **0xF3**, 4 words) is the reference for the names: header A = source VR, C =
destination VR, w1 = 0xFF01 (or *r* << 8 \| 1 for the remainder *r*), w1 22:21 = 2, w2 26:13 = destination line
(`vrbuf_get_dmavecaddr_vector`), w3 = source line \| count << 14; the copy size (16-byte lines) is split into 255-line
vectors plus one remainder vector. It moves s16 lines (format 2).

### 7.3 The requantising form (R) [R]
Classes 0x10, 0x20 / 0xA0, 0x28 / 0xA8, 0x30 / 0xB0, 0x60 / 0xE0 share one body (format B with the length in header
17:13, except 0x10 and 0x20, which are format A); LEAKY_RELU's 0xD8 (§10.6) writes its fields in the same places [I: by
its fields]:

| word | bits | content |
|---|---|---|
| 1 | 15:0 | length in lines (≤ 0x200 = `ONE_SLOT_DEPTH`) |
| 1 | 18:16 | **operation**: 7 = multiply / requantise, 3 = 16 → 8, 4 = 8 → 16 |
| 1 | 23:19 | shift |
| 1 | 25 | 1 (the 0x0207 << 16 template sets bits 16, 17, 18 and 25) |
| 1 | 30:26 | second shift (16 → 8, broadcast ADD / SUB, LUT pre-scale, LEAKY_RELU) |
| 1 | 31 | a flag (the 14-word 0x28 form) |
| 2 | 13:0 | source line (ADD / SUB / MUL: in0 line \| in1 line << 14) |
| 2 | 29:28 | flags |
| 3 | 13:0 | destination line |
| 3 | 29:27, 30 | flags (bit 30 = 0x4000_0000 in PReLU — "write the complementary lanes" [I], §10.6; bit 27 / 28 = signedness in the conversions, §7.6) |
| 4 | 15:0 / 31:16 | clamp: 0x7FFF / 0x8001 (±32767), or upper / lower bounds (RELU family, Clip, LUT) |
| 5 | 15:0 / 31:16 | two u16 coefficients (multiplier / offset or second multiplier) |
| 6 … | | a 4- or 8-word parameter block in the 10- and 14-word forms |

The lengths seen are 4 (0xA0), 6, 10 (0x60), 14 (0x28) words. Reading [I: from the field roles, §10.3]: operation 7
computes `(a · m1 [+ b · m2] [+ offset]) >> shift` with saturation to the w4 bounds; operations 3 / 4 are the
narrowing / widening conversions of §7.6. The exact order of multiply, add, the two shifts, the right-shift rounding and
the saturation are not known (§12).

### 7.4 Predicates and the two write modes [R: instruction order and values in nine kernels (§10.4); I: the semantics]
A compare writes a lane predicate (w2 bit 29 = 1); the following instructions write either with **w1 bit 24 set** or
without it. Both writes must be predicated (MINIMUM issues its bit-24 write last, which would overwrite everything if
bit 24 meant "unconditional") and they are complementary. Two assignments fit all nine kernels: **(A)** bit 24 = the
lanes where the predicate is **true**, clear = where it is false; **(A′)** the reverse, with every predicate below
complemented (0x12 ≤, 0x16 <, 0x1A ≠, 0xD2 / 0xDA "= 0", and bit 24 on 0xDA = OR). Which one the hardware implements is
not known (§12). Under (A):

| class (vector / scalar operand) | predicate | users |
|---|---|---|
| 0x12 / 0x32 / 0xB2 | in0 **>** in1 | GREATER, MAXIMUM, MINIMUM, CondGreat |
| 0x16 / 0x36 / 0xB6 | in0 **≥** in1 | LESS (written as ¬≥), GREATER_EQUAL, CondLess, ARG_MAX / REDUCE_MAX steps |
| 0x1A / 0x3A / 0xBA | in0 **==** in1 | EQUAL, NOT_EQUAL (¬==) |
| 0xD2 | in ≠ 0 (or > 0; booleans cannot tell) | LOGICAL_NOT, PReLU step 1, FloorMod, Atan2 |
| 0xDA | in ≠ 0; with bit 24 the result is ANDed into the existing predicate | LOGICAL_AND, CAST to BOOL |

0x12 vs 0x16 is the strict / non-strict pair (GREATER vs GREATER_EQUAL). The plain class takes in1 from a VR (w4 15:0 =
in1 line, 31:16 = *n*), the 0x3x form from vector register 8, the 0xBx form from SR register 5. **0x13** with bit 24
(A, B, C) writes B where the predicate holds and A elsewhere — a select; **0xC3** is a **fill** of an immediate (A = out,
w1 15:0 = value, 22:21 = format, w2 26:13 = out line, w3 29:14 = lines) and, with bit 24, a predicated fill.

### 7.5 Pooling 0x5B (max) / 0xDB (average) — `vp1_vPoolingAvg_hal` 0x2006_E022 [R]
One function holds three inlined HAL calls (the assert strings name them): pooling type 4 → `vp1_vPoolingMax_hal`, class
**0x5B**; type 5 → `vp1_vPoolingAvg_hal`, **0xDB** (class 0x5B with bit 7); type 10 → `vp1_vPoolingAvgMean_hal`, 0xDB with
w3 bit 30; other types log `Can not support this pooling type %d` and return 0. So the 15 class-0x5B instructions of the
Anjoy block are **max** pooling — its three MAX_POOL_2D [R: class; I: link]. 9 words (*L* = 8):

| word | bits | content |
|---|---|---|
| 0 | 11:8, 12 / 21:18, 22 | source VR id, ping-pong / destination VR id, ping-pong (one ping-pong argument for both) |
| 1 | 22:21 | argument 4 (2 bits; 0 from every pooling caller; the element format in the RoiPooling form) |
| 2 | 26:13 | source L2 line address (14 bits) |
| 3 | 13:0 / 29:14 / 30 | destination L2 line address / depth in 16-lane slots (tile depth >> 4) / AvgMean |
| 4 | 13:0 / 31:16 | `image_width · image_height` (14 bits; asserted `< 2 << 14` as a 15-bit product) / `image_height − f_h + pad_b` |
| 5 | 15:0 / 31:16 | `image_width` / `image_height` |
| 6 | 15:0 / 23:16 / 31:24 | `image_width − f_w + pad_a` / `f_w` / `f_h` |
| 7 | bytes 0–3 | output tile width, output tile height, stride x, stride y |
| 8 | bytes 0–3 | four padding values (pad_a … pad_d) |

Names: `image_width` / `image_height` from the assert `(poolingAttr->image_width * poolingAttr->image_height) < (2 <<
14)` on arguments 8 / 7; `f_w` / `f_h` from `f_w * f_h < AVE_POOL_U8_W_H_MAX` on the pooling record words 2 / 3 the
caller passes as arguments 14 / 15. The names "output tile width / height", "stride" and "padding" are from position (w7
bytes 0 / 1 = arguments 10 / 9, two per-tile values the caller copies into its output-tile record afterwards; w7 bytes 2
/ 3 = pooling record words 0 / 1; w8 = arguments 16, 17, 19, 18, of which 16 (pad_a) enters w6 and 19 (pad_b) enters
w4) [I]; which of the four paddings is left / right / top / bottom is not known. Average pooling over more than 0xFFFF
taps, and average-mean over a tile of more than 0xFFFF pixels, are refused by the caller. The RoiPooling form uses header
**0x8404_005B** (`vp1_vRoiPooling_hal`, 9 words, §10.17) and PSRoiPooling the fixed header **0x8404_00DB** (A 0, C 1,
§10.17). The averaging and its rounding are the engine's (§12).

### 7.6 Type conversion with requantisation 0x28 / 0x30 (bit 7: 0xA8 / 0xB0) — `datatype_convert_util.c` [R]
Encoders `SGS_IPU_HAL_Convert16BitTo8Bit` 0x2002_C904 (**0xA8**), `SGS_IPU_HAL_Convert8BitTo16Bit` 0x2002_CB70 (**0xB0**),
`SGS_IPU_HAL_Convert16BitTo8BitForMathOp` 0x2002_CDDC and `SGS_IPU_HAL_Convert8BitTo16BitForMathOp` 0x2002_CFE2 (both
0xA8). Header: format B, *L* = 5 (6 words); source VR (operand 1 record +4) at 11:8 with its ping-pong
(`sgs_getPingPongType`) at 12, destination VR (operand 2) at 21:18 / 22. Operand records: +4 VR id, +8 L2 line address.
The conversion record is the 0x14-byte slot of a DataConvertParameters entry (part 1 §6.6), offsets from its type word:
+0x04 *bias*, +0x08 *scaler*, +0x0C *shift*, +0x10 *rcshift* (§10.2).

| word | bits | 16 → 8 (0xA8) | 8 → 16 (0xB0); ForMathOp widening (0xA8) |
|---|---|---|---|
| 1 | 15:0 | op length in lines (`sgs_CalcElementwiseOpLength`, asserted ≤ 0x200 = `ONE_SLOT_DEPTH`; output tile channels ≤ 16) | same |
| 1 | 18:16 | operation 3 | operation 4 |
| 1 | 23:19 | *rcshift* (+0x10, 5 bits); ForMathOp: 0 | *shift* (+0x0C, 5 bits); ForMathOp: 0 |
| 1 | 30:26 | *shift* (+0x0C, 5 bits); ForMathOp: 0 | — (bit 25 = 1 from the 0x0204 / 0x0207 template) |
| 2 | 13:0 | source L2 line | same |
| 3 | 13:0 | destination L2 line | same |
| 3 | 28 / 27 | signedness: 1 when the caller's mode argument is 1, 0 when 2 (else assert) — bit 28 for 16 → 8, bit 27 for 8 → 16 | |
| 4 | 15:0 / 31:16 | 0 | 0x7FFF / 0x8001 (0xB0 only: clamp to ±32767) |
| 5 | 15:0 / 31:16 | *scaler* / *bias*; ForMathOp: 1 / the output zero point *z* | *scaler* / *bias*; ForMathOp: 1 / −*z* (the input zero point negated) |

In the ForMathOp forms w1 31:16 is written whole: 3 for the narrowing (operation 3, no shifts) and **0x0204** for the
widening (operation 4, bit 25) — a 16-bit store at byte 6 (`addi a1, zero, 0x204; sh a1, 6(a3)` at 0x2002_D152, which
Ghidra prints as the CSR symbol `&vsie`). The operation field (w1 18:16) is the only difference between the narrowing
and the widening form inside class 0x28. The firmware's debug print `input%d[u8_s16]: {.scaler, .bias, .shift,
.rcshift}` (and `[s16_u8]`, `[merge]`) names the record's fields (in `requant_record_fill` (0x2001f9fa)'s caller). Reading [I: from the field
roles]: widening ≈ `clamp((x · scaler + bias) >> shift, −32767, 32767)` and, for math ops, `x · 1 − z`; narrowing ≈
`(x · scaler + bias) >> shift` with the second (*rcshift*) shift and saturation to u8 inside the engine, and for math ops
`x · 1 + z`. Which of *shift* / *rcshift* applies before the multiply and how the engine rounds a right shift are not
known (§12).

### 7.7 Class 0x60 (bit 7: 0xE0) [R]
The R form of §7.3. `round_kernel` (ROUND, §10.5) emits two **0xE0** instructions per tile: w1 = op length | operation 7
at 18:16 | 5-bit shift at 23:19 | 0x02 at bits 31:24; w2 = source L2 address (30 bits kept); w3 = destination L2 address
with bits 29:27 kept from the template; w5 = one u16 coefficient | 0. The 6-word form "0xE0-6" used by MUL, DIV, CAST,
SQRT, RSQRT, PReLU (B = 5, w1 = *n* \| operation 7 \| shift << 19 \| 0x02 << 24, w5 = multiplier (u16)) is the same
layout; the 10-word 0x60 (`prelu_kernel` (0x2006dc00), PReLU) carries a 16-byte parameter block in w6–w9. So 0x28, 0x30 and 0x60
with bit 7 share one body format whose w1 bits 18:16 select the operation [R for the encoders read; I: that the field
is an operation selector].

### 7.8 Class by class [R: fields and constants from that scan, users from its caller list and the kernels of §10; I: the operation column unless a string or use names it]
"V" = the §7.2 layout, "R form" = §7.3; only what differs from or fills them is listed. "bit30" = the register form
(§7.1). The helper encoders at 0x2006_8xxx–0x2006_Axxx and 0x2004_3xxx–0x2004_9xxx are called only from the NMS kernels
(`sgs_NMS_Yolov3_Normal_Ipu`, the TFLite_Detection_NMS variants and their sub-functions, `op_custom_ipu_implement.c`);
0x2001_DFA6 / E08A / E12E / E1F2 only from `topk_kernel` ← `OPS_IPU_TopK`. Classes with one fixed header in one kernel
(0x01, 0x09, 0x5E, 0xC5, 0xD6) are its private steps; all of PhaseGen's and FloorMod's sequences are fixed instruction
lists with only lengths, immediates and the requantisation record varying. An empty operation cell: no name and no
distinguishing use, not known.

| byte | words | users | fields / constants | operation |
|---|---|---|---|---|
| 0x00 | 4 | softmax `softmax_kernel` (0x200703fa) | header 0x8180_0000 \| A src \| B = the reciprocal's SR register \| C dst; w1 = *n* \| 0x0207 << 16; w2 = src line \| 0x2002_4000 or \| 0x2002_8000 when w1 bit 24 is set; w3 = dst line | normalising multiply by an SR register [R: use in softmax] |
| 0x01 | 5 | `SGS_PhaGen_IPUExeHeader`; Atan2 | header 0x8208_4301 (A 3, B 2, C 2); w1 = 0xC0_0000; w2 = 0; w3 13:0 = length; w4 31:16 = caller u16 | PhaseGen arithmetic step |
| 0x02, 0x82 | 4 | max / argmax: `_SGS_ReduceMax{On,OnNot}InnermostDim`, `_SGS_Argmax{On,OnNot}InnermostDim`, `softmax_max_step` (0x20063bb6), `softmax_sub_max`, `nms_max_step` (0x2006970a), NMS `nms_max_step_b` (0x20068c38), TopK `topk_max_step`, PostProcess_Max | 0x82 = bit30 form (0xC180_0082); w1 = **0x80C0_0000** or 0x8080_0000 \| imm (bit 31, format 2, bit 23); w2 26:13 = constant 6 / 9 / 0xA (a register) or a line; w3 = line \| count | **lane maximum with its index** into an SR register (6 or 8; read back by the CPU: index = low 16 bits, value = high 16 bits, §10.15); in REDUCE_MAX the running max of two vectors; the tie order is not known |
| 0x05 | 5 | NMS `nms_select_step` (0x200690d8); PhaGen (0xCA0C_2305) | bit30 (29:27 = caller); w1 = 0xC0_0000 or 0x1C0_0000; w3 29:14 = ⌈*n*/8⌉; w4 31:16 = ⌈*n*/8⌉ or caller u16 | NMS / PhaseGen vector op (box geometry in NMS [I]) |
| 0x09 | 5 | PhaGen (0xC208_4309), Atan2 | w1 = 0xCF_0000 (22:16 = 0x4F); w4 31:16 = caller u16 | PhaseGen step |
| 0x10 | 6 | reduce-sum: `reduce_sum_kernel`, `_SGS_ReduceOnInnermostDim_ResolveMultiAxis`, `_SGS_ReduceOnInnermostDim_ResolveAllAxis`; Clip | header 0x8284_0110 (+B, B ping-pong at 17); R form: w1 = length \| 0x0207 << 16 \| shift << 19; w3 bit 31 and 29:27; w4 = 0x7FFF \| 0x8001 << 16 (Clip: upper \| lower << 16); w5 = 1 or a coefficient | requantising accumulate / final requantisation of a sum; clamp (Clip) |
| 0x12 | 5 | GREATER `greater_kernel`, MAXIMUM `maximum_kernel`, MINIMUM `minimum_kernel`; Atan2 | header 0x8240_0012 / 0xC240_…; w1 = 5 or 0x40_0000 / 0x140_0000; w2 = line (12:0) \| line << 13 \| bit 29; w3 = line \| count; w4 15:0 = in1 line (14-bit value), 31:16 = u16 (*n*) | compare in0 > in1 (§7.4) |
| 0x13 | 5 | MAXIMUM, MINIMUM, Argmax, FloorMod, FourBytesFlip | 0xC200_0013 bit30 forms; argmax: w1 = 0x140_0000, w2 = 0x2200 or index << 13, w3 = 0x4000 / 0x4001, **w4 = 1 \| 1 << 16** | with w1 bit 24: **select** B where the predicate holds, A elsewhere; index select |
| 0x16, 0x1A | 5 | 0x16: LESS `less_kernel`, GREATER_EQUAL `greater_equal_kernel`, NMS `nms_cmp_step` (0x20069e8e); 0x1A: EQUAL `equal_kernel`, NOT_EQUAL `not_equal_kernel`, NMS `nms_eq_step` (0x200695ea), YOLOv3 score test | bit30 in the NMS forms (header 0xC240_00xx \| A, B \| (C & 7) << 18 \| (x & 7) << 27); w1 = 0x40_0000 (0x1A: + caller bit 24); w2 bit 29 (0x1A: = caller ≠ 1); w3 29:14 = caller u16; w4 31:16 = same u16 | compare: 0x16 in0 ≥ in1, 0x1A in0 == in1 (§7.4) |
| 0x1E | 4 | TopK `topk_mask_step` (0x2001e08a), `topk_mask_step_b` (0x2001e12e); reduce `reduce_mask_step` (0x2004de1e), `reduce_mask_step_b` (0x2004dcfe), `_SGS_ReduceMaxOnInnermostDim`, `softmax_sub_max`; NMS `nms_mask_step` (0x20068f12); PostProcess_Max (0x1E 0xC00); Atan2, FormatConvert, FourBytesFlip | header 0x81C0_001E / 0x8180_001E \| (C & 7) << 18; w1 15:0 = **lane mask** `1 << (n % 8)` or `(−1 << (n % 8)) & 0xFF`, or a length; w2 26:13 = ⌊*n*/8⌋ or *n* − 1; w3 29:14 = ⌈*n*/8⌉ or 1 | masked lane write (partial-vector select by an 8-lane mask: excludes the pad lanes of a row, or one chosen lane) |
| 0x20 | 6 | ADD / SUB `addsub_kernel` (0x2006d6a4); WiggleErr `wiggleerr_kernel` (0x200673ca); FloorMod (0x8288_2020); NMS geometry | R form; ADD: A = in0, B = in1, C = out; WiggleErr: operands by register (A 2, B 3 … 5, C 2), addresses 0, header `0x8288_0220 \| n<<13`; w5 = two u16 (ADD: *scale* \| *scale2* << 16; WiggleErr: from the option record, +0xF4C table, 0x18-byte entries) | requantising add / multiply |
| 0xA0 | 4 | MUL `mul_kernel` (0x2006d942); NMS `nms_mul_step` (0x200692a2); PReLU `_sgs_PRelu_IPUExeTile` | R-form w1 (length \| 0x0207 << 16 \| shift); w2 = line(s) (in0 line \| in1 line << 14); w3 = line, bit 31 in NMS | short requantising op (MUL: (in0 · in1) >> rcshift [I]) |
| 0x21, 0xA1 | 5 | NMS `nms_sum_step` (0x200699a2), `nms_sum_step_b` (0x20069af4); reduce-sum ResolveAllAxis (0xC204_01A1, w4 = 6); 0x21: reduce-sum SplitLane / not-innermost variants (the pair 0x8204_0121 / 0x8204_2121 on the accumulator) | bit30; w1 bit 23; w2 26:13 = line, bit 27 = 1, bit 28 = caller bit; w3 = line \| ⌈*n*/8⌉ << 14; w4 13:0 = 6 | add register 6 to the accumulator (reduce-sum, later chunks) [I]; 0x21: accumulator step [I]; NMS step |
| 0x23, 0xA3 | 5 | `SGS_IPU_HAL_ExpandTileDataIdenticalWithTargetTileInfo`, `SGS_Softmax_RecipOfAcc_OneLane`, argmax / reduce-max, `_SGS_PostProcessUnpack_LoadTensorDataToL2` (0x2004aa4c), reduce-sum (0xCA04_00A3), NMS `nms_broadcast_step` (0x20068d2a), `nms_broadcast_step_b` (0x20068e1e), LOGICAL_AND / MAXIMUM / MINIMUM broadcast, CondLess / CondGreat | header 0x8200_00xx or bit30 0xC200_…; w1 = 0x40_0000 or the element format from the caller (`x << 21`); w2 26:13 = line, 12:0 or 1:0 = small value, bit 27; w3 = line \| count; **w4 13:0 = 4 / 5 / 6 / 8** (the source: 5 SR, 6 register 6, 8 vector register 8) | broadcast / expand / copy (one lane or register to all; with bit 24 a predicated write) |
| 0x25, 0xA5 | 5 | `softmax_max_step` (0x25), `softmax_sub_max` (0xA5) — softmax helpers of `softmax_innermost` / `softmax_not_innermost` | w1 = 0xC0_0000; w2 26:13 = line; w3 = line \| count; w4 13:0 = 8 (0x25) / 5 (0xA5) | subtract the maximum (x − max, softmax) |
| 0x26 | 5 | NMS `nms_26_step` (0x2006a33c) | w1 = 0x40_0000; w2 26:13 = 6; w3 = 1 << 14; w4 13:0 = 5 | NMS step |
| 0x28 | 14 | broadcast ADD / SUB `bcast_addsub_kernel` (0x2006d7fe) (← `SGS_Sub_IPUExeTile`), EXP, `SGS_Logistics_IPUExeTile`, FloorMod | format B, *L* = 13; R form w1 with bit 31; w2 / w3 = source / destination line; w5 = caller word; **w6–w13 = a 32-byte parameter block** copied from the caller (ADD: eight s32 constants; EXP / LOGISTIC: the offset ×8) | requantising op with an 8-word parameter table |
| 0xA8, 0xB0 | 6 | the conversions of §7.6 | §7.6 | narrowing / widening conversion |
| 0xB0 (second encoder) | 6 | `relu_kernel` (RELU / RELU6 / RELU_N1_TO_1) | R form with operation 7; **w4 = upper \| lower clamp << 16** from the option; w5 = *scale* | clamp (ReLU family) |
| 0x2A, 0x2E, 0xAA, 0xAE | 5 | NMS `nms_iou_step_aa` (0x2006a826) (0xAA), `nms_iou_step_ae` (0x2006a916) (0xAE); IoU steps of the NMS stage 3 | bit30; w1 = 0xC0_0000; w3 29:14 = ⌈*n*/8⌉; w4 13:0 = 5 | NMS step |
| 0x32, 0xB2 | 5 | GREATER `greater_kernel` broadcast, MAXIMUM / MINIMUM broadcast, CondGreat, PhaseModify | header 0x8240_00xx \| A << 8; w1 = 0x40_0000 \| …; w2 = line << 13 \| bit 29; w3 = line \| count; w4 = 8 (0x32) / 5 (0xB2) | compare > against vector register 8 / SR 5 |
| 0x36, 0xB6 | 5 | argmax / reduce-max (0x36); LESS / GREATER_EQUAL broadcast; CondLess, PhaseModify (0xB6) | w2 = value \| flags 31, 28; w3 13:0 = line (index >> 1), count 1; w4 13:0 = 8 | compare ≥ against vector register 8 / SR 5 |
| 0x3A, 0xBA | 5 | EQUAL `equal_kernel`, NOT_EQUAL `not_equal_kernel` broadcast; NMS `nms_ba_step` (0x20069378), `nms_ba_step_b` (0x200694d2) (0xBA, read by hand) | 0xBA: bit30, C = 29:27 = caller & 7, bit 22; w2 = line & 0x1FFF \| line << 13 \| bit 27 \| caller bit 28 \| bit 29; w3 = line \| ⌈*n*/8⌉ << 14; w4 13:0 = 6 | compare == (in NMS: suppression by the IoU threshold [I]) |
| 0x3E, 0xBE | 5 | FloorMod / PhaGen (0x8200_203E: A 0, B 1, C 0), NMS `nms_be_step` (0x2006a74a) (0xBE) | w1 22:16 = **0x41** (sub-op 1) + caller bit 24; w3 29:14 = caller u16; w4 = 5 | |
| 0x3F | 5 | compare kernels; MAXIMUM / MINIMUM broadcast (writes in0, w4 = 6); NMS `nms_subop_step` (0x20069f7e) / `nms_subop_step_b` (0x2006a08c); CustomNotEqual | w1 22:16 = **0x43** / **0x47** (sub-ops 3 / 7) or 0x140_0000; w4 13:0 = 6 | compare-and-select against a scalar (CustomNotEqual) [I] |
| 0x46, 0xC6 | 4 | reduce helper `softmax_sum_step` (0x20063ff2) (0x46, softmax sum); reduce-sum `reduce_sum_kernel`, `softmax_sub_max`, `_SGS_ReduceOnInnermostDim_ResolveMultiAxis` (0x8180_00C6) | w1 = 0xC0_0000; w2 26:13 = 6 or 9 (target register), 12:0 = line; w3 = line(s) | lane reduction (sum into register 6) |
| 0x47 | | FormatConvert, FourBytesFlip | | lane gather / scatter between layouts [I] |
| 0x41, 0xC1 | 4 | WiggleErr (0x8188_02C1; 0x41 in its per-block sequence), FloorMod (0xC184_01C1), PhaGen, Argmax, `wiggleerr_imm_step` | w1 15:0 = immediate (negated in one site); w1 = … \| **0x1C0_0000** (bits 22–24); argmax: w1 byte 2 = 0xC0; w3 = line or count | add an immediate (paired with 0xC5; ARG_MAX increments its index register with it) |
| 0xC3 | 4 | **`SGS_DMA_FillMemory`** (0x20030180: w1 15:0 = fill value, w1 22:21 = caller format), compare kernels, CAST `SGS_IPU_IPUExeTile`, PSRoI, PhaGen (0xC18C_00C3, w1 = 0x1000), reduce-sum (0xC180_00C3, 0x8984_00C3), Clip, PostProcess_Max | w1 = 0x140_0000 \| value in most; w3 = line \| count << 14 | **fill / broadcast an immediate** (named by FillMemory); with bit 24 a predicated fill |
| 0x45, 0xC5 | 4 | FloorMod (0xC184_01C5) | w1 = immediate \| 0xC0_0000 | immediate op of FloorMod |
| 0x4A, 0x4E, 0xCA, 0xCE | 4 | NMS `nms_ca_step` (0x200697ec) (0xCA), `nms_ce_step` (0x200691c6) (0xCE); 0x4A / 0x4E: NMS box geometry (stage 2, IoU), YOLOv3 `yolov3_geometry` (0x20044960), `yolov3_geometry_b` (0x20044ce4) | bit30; w1 15:0 = caller, = 0xC0_0000; w3 29:14 = ⌈*n*/8⌉ | NMS step; box geometry / IoU steps [I] |
| 0x52, 0xD2 | 4 | LOGICAL_NOT `logical_not_kernel`, WiggleErr (0x52), FloorMod (0x8180_01D2 / 0x8180_00D2), PReLU (0x81C0_00D2), WiggleErr `wiggleerr_imm_step` (0x89C0_…: 29:27 = 1), Atan2 | w1 = 0x40_0000; **w2 bit 29 = (value < 1)** in FloorMod, 1 elsewhere; w3 = line \| *n* << 14 | sign / non-zero test → predicate (§7.4) |
| 0x53, 0xD3 | 4 | `reduce_d3_helper` (0x20063de4), `nms_d3_step` (0x20068ad6) (NMS), `_SGS_ArgmaxOnInnermostDim`, softmax, reduce-sum variants, PostProcess_Unpack (0x81A0_08D3: VR 8 in place), PostProcess_Max, PhaseModify, FourBytesFlip, FormatConvert | **w1 22:16 = 0x41** (sub-op 1); w2 26:13 = line; w3 = line \| count; argmax: w1 = imm \| imm << 16 | the reduce class of the Anjoy block (part 1 §8.5.1); brackets the softmax row groups |
| 0x56, 0xD6 | 4 | FloorMod / PhaGen (0x8184_01D6, 0x8180_01D6), Atan2 | w1 = 0x40_0000; w2 = bit 29 | select |
| 0xD8 | 6 | LEAKY_RELU `leaky_relu_kernel` | B = 5; w1 = *n* \| 0x0207 << 16 \| *s1* << 19 \| *s2* << 26; w4 = 0x7FFF; w5 = *m1* \| *m2* << 16 (§10.6) | two-slope requantising multiply [I]; operation not known |
| 0x5A, 0xDA | 4 | CAST `SGS_IPU_IPUExeTile`, LOGICAL_AND `logical_and_kernel`, FloorMod (0x8184_02DA), PhaGen (0x8180_02DA), Atan2 | w1 15:0 = immediate, 0x40_0000 (\| 0x140_0000); w2 26:13 = line \| bit 29 | non-zero test → predicate, with bit 24 ANDed (§7.4) |
| 0x5B, 0xDB | 9 | pooling, RoiPooling, PSRoiPooling, MEAN | §7.5 | max / average pooling |
| 0x5E | 4 | PhaGen (0x8180_105E: A 0 with ping-pong 1) | w1 = 0x40_0000; w3 = length | PhaseGen step |
| 0x60 (10 words) | 10 | MUL / DIV broadcast `prelu_kernel`, PReLU | format B *L* = 9; R form w1; w2 / w3 lines; **w6–w9 = a 16-byte parameter block** (eight u16 constants / alphas) | requantising op with 4-word parameters |
| 0xE0 | 6 | MUL, DIV `div_kernel` (0x2006dd1a), CAST, ROUND, SQRT, RSQRT, PReLU, FloorMod, `requant_mul_u16_kernel` (0x2006728a) | §7.7 | requantising multiply by a u16 |
| 0x66 | 4 | JointMicroPhaseModify (`joint_micro_phase_modify` (0x2003fdaa), 0x8180_0180 \| …: A 1, bit 7), ExpandTileData, softmax `RecipOfAcc` (loads a line into SR registers), compare kernels (`cmp_load_const_vector` (0x2005fefa): loads a constant vector into vector register 8; A = in1's VR, w2 26:13 = 8, w3 29:14 = ⌈len / 8⌉) | **w1 22:21 = 3**; w2 26:13 = constant 4 / 5 / 8 or line, 12:0 line, 29:27 flags; w3 = line \| 1 << 14 | load a vector into a register [I: from the two uses] |
| 0x6D, 0xED | 4 | NEG `neg_kernel` (0x6D), ABS `abs_kernel` (0xED); WiggleErr `wiggleerr_neg_step`, `wiggleerr_imm_step`; PhaGen (0x8188_01ED); Atan2 | A = in, C = out; w1 22:21 = element format \| bit 23; w2 26:13 = out line; w3 = in line \| *n* << 14 | negate (0x6D) / absolute value (0xED) [R: the ops that emit them alone] |
| 0x7E | 5 | TopK `topk_select_step` (0x2001e1f2); NMS `nms_select_step_b` (0x20068fcc); PhaGen | A / B / C = caller & 7 (TopK: ping-pong bits 12 and 17 set); w1 = 0x40_0000; w2 26:13 = line; w3 = line \| count; w4 13:0 = line, 31:16 = u16 | knock the chosen lane out of the next round (TopK, NMS) [I] |
| 0xE3 | | FormatConvert | | |
| 0x73, 0xF3 | 4 | `SGS_OPS_CopyDataInL2` (§7.2, 0xF3), the resize repeat path | §7.2 | **L2 → L2 copy** |
| 0x78, 0xF8 | 21 / 5 | LUT | §8 | LUT lookup / s16 → f32 |

Rows name a class byte with and without bit 7 where both occur. For the plain forms 0x21, 0x2A, 0x2E, 0x41, 0x4A, 0x4E,
0x52 and the classes 0x47, 0xE3 the fields above are those of the class's other form or empty: their written fields
per encoder and their order and immediates per kernel are left out of this table (§10 gives the programmes).

## 8. LUT unit (`Lut/lut_ipu.c`, `Lut/lut_ops_comon.c`, `Lut/IPU200M/drv_lut_ops_ipu200.c`) [R unless marked]

Two LUT units (`table_id` 0 / 1). A LUT-driven op carries in its option one or more 0x2F0-byte LUT blocks
(`LutInfomations`: `table_first`, `table_second`, `OpLutConfig`, `OpLutParamters`; layout part 1 §7.2); block *k* sits at
option + 0x2F0·*k*. The firmware loads a block's tables and configuration into the unit (§8.2–§8.3) and runs one LUT
instruction per tile (§8.1). How the unit interpolates from `table_first` / `table_second`, the `intp*_ko` / `ku` slopes
and their shifts, the priorities and `enable_lutSymm` is the unit's own definition, not visible in the firmware (§12):
the firmware transports the model's numbers.

### 8.1 The LUT instruction 0x78 — `SGS_Lut_Impl` 0x2006_80AA (`Lut/lut_ops_comon.c`)
Users: logistic, tanh, sqrt, rsqrt, exp, reciprocal, atan2, wiggle, the softmax exponent (§10.7, §10.8). Class **0x78**,
format B with ***L* = 20 (21 words)**. Header: source VR (argument 2) 11:8, ping-pong (argument 4) 12, destination VR
(argument 3) 21:18, ping-pong (argument 5) 22. The last argument (11) selects the LUT block inside the option (asserted 0
for most ops, ≤ 1 for RSQRT: `lut_info_idx < OPTS_RSQRT_LUT_MAX`). Body, with the `OpLutParamters` offsets and names of
part 1 §7.2 (`OpLutParamters` at +0x2A4 of the block):

| word | content |
|---|---|
| 1 | 15:0 op length (argument 8); 31:16 = 0x0207 |
| 2 | 13:0 source L2 line (argument 6) |
| 3 | 13:0 destination L2 line (argument 7) |
| 4 | 15:0 argument 10 (upper output bound), 31:16 argument 9 (lower output bound) — the "clamp" of §10.7 |
| 5, 6 | 0 |
| 7 | 15:0 `lut_scale` (+0x2AC); 24:16 `lut_exp_start` (+0x2B0, 9 bits; asserted −256 … 255 when `lut_x_function` (+0x29B) = 1); 25 `table_id` (+0x2B4) bit 0; 30:26 `lut_x_shift` (+0x2B8; asserted 0 … 30 when `lut_x_function` = 0) |
| 8 | 0 |
| 9–14 | `lut_x_max`, `lut_x_min`, `lut_y_max`, `lut_y_min`, `lut_x_bias`, `lut_y_bias` (+0x2C4 … +0x2D8, s32 each) |
| 15 | 4:0 `intp_x_ko_shift`, 9:5 `intp_x_ku_shift`, 20:16 `intp_y_ko_shift`, 25:21 `intp_y_ku_shift` (each asserted to fit 5 bits) |
| 16 | 0 |
| 17 | `ocvt_x_offset` \| `ocvt_y_offset` << 16 (+0x2E4, the two i16 read as one word) |
| 18 | `ocvt_x_scale` \| `ocvt_y_scale` << 16 (+0x2E8) |
| 19 | 4:0 `ocvt_x_shift`, 9:5 `ocvt_y_shift`, 14:10 `lut_y_shift` (+0x2BC) |
| 20 | 0 |

Not carried in the instruction: the four `lut_data_bias*` / `bias2 … 7` i16 (not referenced by `SGS_Lut_Impl`), the `OpLutConfig`
interpolation fields and the tables — those go to the unit by §8.2.

### 8.2 Loading a LUT block — `SGS_Lut_SetTableConfig` (0x2006_8070, name from the callers' asserts)
`SGS_Lut_SetTableConfig(pstOptLut, bSetLutTableByCpu)`; every one of its 14 call sites passes `bSetLutTableByCpu` = 0.

1. **Cache check** `lut_cache_check` (0x20067c16): reads unit *u* = block +0x2B4 (`table_id`) **back through the CPU window** — table
   0, table 1 and the 20-byte configuration — and compares them with the block (`memcmp` 0x2007_7840 of 0x82 / 0x202
   bytes, then the configuration field by field; configuration bits 6:5 must read 0). Equal → nothing is loaded. The
   CPU-access lock byte (object +8, §3.1) is set around the read; the "busy" query `lut_busy` (0x200704d4) always returns 0.
2. **Load through the decoder** `lut_load_by_decoder` (0x20067d88) (otherwise): two op-3 barriers (`0x3000_0182`, `0x3000_0162` — ids
   0x182 and 0x162, with bit 8 set, part 1 §8.3.2), then three op-1 writes built by `lut_build_write_a` (0x2006fda4) / `lut_build_write_b` (0x2006fe34)
   (header `addr & 0xFFFF | (len − 1) << 16 | 0x1000_0000`):

| write | decoder address (unit 0 / unit 1) | CPU address (unit 0 / unit 1) | bytes | content |
|---|---|---|---|---|
| table 0 | 0x8000 / 0x9000 | 0x0102_0000 / 0x0102_1000 | 0x84 | `table_first` (65 × i16, block +0x000) **plus a copy of its last entry** (block +0x080) — 66 entries |
| table 1 | 0x8800 / 0x9800 | 0x0102_0800 / 0x0102_1800 | 0x204 | `table_second` (257 × i16, block +0x088) plus a copy of its last entry (+0x288) — 258 entries |
| configuration | 0xA000 / 0xB000 | 0x0102_2000 / 0x0102_3000 | 0x14 | §8.3 |

Decoder addresses from the LUT descriptor 0x2009_2EC0 (+0x0C … +0x20), CPU addresses from the CPU-access object
0x2009_2EE8 (+0x0C … +0x20); CPU = decoder + 0x0101_8000 for all six [R: values; I: that both name the same memory]. The
unused CPU path `lut_load_by_cpu` (0x20067f22) (taken for `bSetLutTableByCpu` ≠ 0) writes the same three images with word stores (0x21
/ 0x81 words, 0x14 bytes) and runs the same comparison afterwards.

### 8.3 Configuration image (20 bytes; `OpLutConfig` fields of part 1 §7.2)

| off | content |
|---|---|
| +0x00 | `intpx_ku` (i16, block +0x294) |
| +0x02 | `intpx_ko` (+0x290) |
| +0x04 | `intpy_ku` (+0x296) |
| +0x06 | `intpy_ko` (+0x292) |
| +0x08 | flags (u32): bit 0 `table_oflow_priority` (+0x299), bit 1 `table_uflow_priority` (+0x29A), bit 2 `table_hybrid_priority` (+0x298), bit 3 `lut_x_function` (+0x29B), bit 4 `enable_lutSymm` (+0x29C) — bit 0 of each byte; bits 6:5 = 0 |
| +0x0C | `table_first[0]` |
| +0x0E | `table_first[64]` |
| +0x10 | `table_second[0]` |
| +0x12 | `table_second[256]` |

So the unit is given the end values of both tables separately (the out-of-range outputs, by their use with the over /
underflow priorities [I]).

### 8.4 The 0xF8 form — `fix2float_insn` (0x2003ca86)
6 call sites, all in `_SGS_Fixed2Float_CModelFixedImpl` (Fix2Float, §10.19): header **0x8004_80F8** — class 0x78 with
bit 7, *L* = 4 (5 words), register C = 1 (bit 18), A = 0, i.e. VR 0 → VR 1; w1 = length (argument 2; 0x200 or a
remainder) | 0x0207_0000; w2 = 0; w3 = bit 31 only; **w4 = the input tensor's f32 scale** (its bit pattern, from the
caller's record). It converts s16 to f32 multiplied by the f32 in w4 [R: data flow; I: that the engine multiplies]; the
engine's float rounding is not visible (§12).

## 9. Numeric routines and types the kernels share

### 9.1 Soft-float and libm routines [R: emulation]
<!-- ev:ipu-firmware-2-engines.9.1 -->
The routines the kernels call lie in 0x2007_05BC–0x2007_7870, the low part of the newlib / libgcc region of part 1
§2.5 (0x2007_05BC–0x2007_9F9F), which carries no names; `ceil` at 0x2007_05BC is its first routine, and the functions
just below it (up to 0x2007_05B6) are the firmware's own LUT-ops helpers (`SGS_Softmax_RecipOfAcc_EightLane` at
0x2007_031E, LUT table copies) [R]. The routines were named by running each under unicorn with probe arguments; the rest of the region stays unnamed:

| address | name | evidence |
|---|---|---|
| 0x2007_05BC | `ceil` | 2.5 → 3, −2.5 → −2, 0.5 → 1 |
| 0x2007_0B90 | `round` | half away from zero: 2.5 → 3, −2.5 → −3, 0.5 → 1, −0.5 → −1 |
| 0x2007_09DC | `floor` | 2.5 → 2, −2.5 → −3 |
| 0x2007_29B4 | `rint` | half to even: 2.5 → 2, 3.5 → 4, 0.5 → 0 |
| 0x2007_0764 / 0x2007_084C | `ceilf` / `floorf` | float forms |
| 0x2007_0934 | `fmaxf` | named by its use in the PSRoiPooling bin arithmetic (§10.17); not in the emulation set |
| 0x2007_0C90 (→ 0x2007_12AC) | `log` (→ `__ieee754_log`) | log 1.5 = 0.405465 |
| 0x2007_0E34 (→ 0x2007_186C) | `pow` (→ `__ieee754_pow`) | 1.5^−2.25 = 0.401601 |
| 0x2007_3588 / 0x2007_4CC8 / 0x2007_46F0 / 0x2007_3D90 | `__adddf3` / `__subdf3` / `__muldf3` / `__divdf3` | 1.5 ∘ −2.25 |
| 0x2007_2D20 | `__negdf2` | |
| 0x2007_5798 / 0x2007_644C / 0x2007_612C / 0x2007_5BEC | `__addsf3` / `__subsf3` / `__mulsf3` / `__divsf3` | float forms |
| 0x2007_446C / 0x2007_44F8 / 0x2007_45F4 | `__eqdf2` / `__gedf2` (= `__gtdf2`, unordered → −2) / `__ledf2` (= `__ltdf2`, unordered → 2) | |
| 0x2007_5F28 / 0x2007_5F94 / 0x2007_6060 | `__eqsf2` / `__gesf2` / `__lesf2` | |
| 0x2007_5530 / 0x2007_55B0 / 0x2007_68C4 | `__fixdfsi` / `__fixunsdfsi` / `__fixsfsi` | truncation toward zero; unsigned: negatives → 0 |
| 0x2007_562C / 0x2007_56F4 / 0x2007_6934 / 0x2007_6A58 | `__floatsidf` / `__floatunsidf` / `__floatsisf` / `__floatunsisf` | |
| 0x2007_6B50 / 0x2007_6C5C | `__extendsfdf2` / `__truncdfsf2` | |

Other library routines the kernels call: `memcmp` 0x2007_7840, `memcpy` 0x2007_7870 (part 1 §2.5), `qsort` 0x2007_8408.
So every "round" in the formulas of this document is C `round` (half away from zero) on a double unless written
otherwise, and a float → int conversion without a rounding call truncates toward zero [R].

### 9.2 Tensor element types
Two numberings exist side by side:

- **The tensor type** (`E_TF_TENSOR_TYPE_*`; the model's tensor record of part 1 §6.4, +0x04, and `eTensorTypeInEmi`
  +0x10) keeps TFLite's numbering for 0–9 and adds two: 0 FLOAT32, 1 FLOAT16, 2 INT32, 3 UINT8, 4 INT64, 5 STRING, 6 BOOL, 7 INT16, 8
  COMPLEX64, 9 INT8, **10 INT4, 11 UINT4** (TFLite has FLOAT64 / COMPLEX128 there [S: TFLite schema]); anything else →
  0xC with a log line (part 1 §6.4) [R: name table `tensor_type_name` (0x2001c4d6); checked against the asserts `==
  E_TF_TENSOR_TYPE_INT16` (compare with 7), `INT32` (2), `FLOAT32 || INT32 || INT16 || INT8` (`(t & ~2) == 0 || ((t −
  7) & ~2) == 0`, i.e. 0, 2, 7, 9)]. Quantisation fields the kernels use (part 1 §6.4): `scale` = f32 at `*(tensor +
  0x44)`, `zero_point` = i64 at `*(tensor + 0x4C)` of which the kernels read the low 16 or 32 bits.
- **The IPU dtype** (+0x18 of the **conv tensor struct** that `convert_tflite_tensor_to_ipu_tensor` 0x2002_7D4E builds
  for the conv / tile-partition code; element bits in brackets) [R]: FLOAT32 → 4 (32), INT32 → 5 (32), UINT8 → 1 (8),
  BOOL (6) → 7 (16), INT16 → 3 (16), INT8 → 2 (8), type 10 → 8 (4), type 11 → 9 (4); 6 = 48 bits and 10 = (no size) are accumulator types: `dtype != EN_INT24 && dtype !=
  EN_INT48` is compiled as dtype ∉ {6, 10}, so **6 = EN_INT48** (48 bits) and **10 = EN_INT24** [R: compare; I: the name
  pairing by size]. 1 = `EN_UINT8` [I: TFLite UINT8 → 1].

The conv tensor struct also holds the usage (+0x1C, §4.6), a word copied into the tile records (+0x20, §6.8) and the
KB compression mode (+0x30 = 2: compressed, §6.2). The **IPU tensor record** of the DMA / resize / transpose code is a
different, 0x94-byte record (`SGS_FW_ConvertTfLiteTensorToIpuTensorRecord`): word 0 element size in bytes, 1 dimension
count, 4 … 13 the shape extended to 10 dimensions with NHWC in dims 6–9 (+0x28 N, +0x2C H, +0x30 W, +0x34 C), 0xE total
bytes, +0x7C / +0x80 row / pixel byte strides, +0x88 = word 0x22 data address, 0x23 the TFLite tensor [R: uses; I: the
names of the stride words].

## 10. Operator kernels
<!-- ev:ipu-firmware-2-engines.10 -->

Each registered op (§2.3) is given by its function, source file, checks, option fields, CPU arithmetic, tile planning
and the ordered engine-instruction programme with its parameter sources. For the long fixed programmes the specification stops at the programme's steps; the complete call / instruction order
with every register number and immediate is not specified.
Option layouts are part 1 §7.3 (tables) and §7.4 (FlexBuffer keys); `OpParameterConvert` = {+0 type, +4 scale, +8 scale2,
+0xC shift, +0x10 offset, +0x14 rcshift} (part 1 §7.2); *E*, *E0*, *E1* below are such entries.

### 10.1 Execution shapes [R]
The registration `eType` (§2.2) sorts the kernels into three shapes:

- **(a) Domain-0 ops (`eType` 1, 3 ops) — the convolution class** (`CONV_POOL/*`): `CONV_2D` (0x2002_C60C),
  `DEPTHWISE_CONV_2D` (0x2002_C624) — `conv_pool_ipu.c` / `sdk_conv.c` / `sdk_conv_hw.c`; for ops in run mode 1 the
  converter's command block carries the layer, otherwise the conv kernels compute online through the loop nests of
  §6.10. `FULLY_CONNECTED` has no function (§2.3).
- **(b) Domain-1 header ops (`eType` 2, 65 ops) — one function runs a whole layer**: the pooling ops and MEAN (§10.10;
  they use the conv / pool engine), the data-movement ops and the alias entries (`DMA/*`, §10.11–§10.13), the
  reductions (`Custom/*`, §10.14–§10.15), SOFTMAX (§10.8) and the detection / image / radar custom ops (§10.16–§10.20).
- **(c) Element-wise tile ops (`eType` 3, 28 ops) — run tile by tile inside a fused domain** under the common header
  `SGS_FW_DefaultElementwiseHeader` (0x2002_87E0, §10.2), tile functions in `Math/elementwise_ipu100.c`,
  `Activation/op_relu.c`, `Math/op_cast.c`, `Lut/lut_ipu.c`: ADD / SUB / MUL / DIV, MAXIMUM / MINIMUM, LESS / GREATER /
  EQUAL / NOT_EQUAL / GREATER_EQUAL, LOGICAL_AND / LOGICAL_NOT, ABS / NEG, RELU / RELU6 / RELU_N1_TO_1 (sharing
  0x2005_34A2), LEAKY_RELU, PRELU, LOGISTIC / TANH / SQRT / RSQRT / EXP / Reciprocal (LUT-backed), ROUND, CAST. The flag
  word of the registration (+0x10, 1 vs 0) marks the ops that need the LUT / vector unit with requantisation;
  `pFnCalcOutputTile` is 0x2002_86FA for all.

### 10.2 Element-wise domains: conversions and tile planning [R unless marked]

**Where the requantisation numbers come from.** The firmware computes **no** requantisation numbers for the
element-wise path: every multiplier, offset and shift is a model value copied into an instruction. Two record sources:
- **Operator-level `DataConvertParameters`** (Operator fields 9 / 10, part 1 §6.6): per input an entry of 0x50 bytes at
  node +0x40 + 0x50·*k* (count +0x220), the output entry at node +0x224 (count +0x274); each entry holds three 0x14-byte
  slots {type, f1, f2, f3, f4} at +0x14 (type 1), +0x28 (type 2), +0x3C (type 3). The debug print of
  `parseOperatorVector` labels them f1 `.bias`, f2 `.scaler`, f3 `.shift`, f4 `.rcshift`, slot type 2 "u8_s16", type 1
  "s16_u8", type 3 "merge". The label is the direction the slot is used for: an input is widened with its **type-2
  (u8_s16)** slot (`memcpy(…, node + 0x68 + 0x50·k, 0x14)` in `_sgs_FuseOperation` 0x2002_98DC), an output is narrowed
  with the output entry's **type-1 (s16_u8)** slot (`get_datatype_convert_param` 0x2002_79BA with index 1 → node
  +0x238), and an input produced by another op of the same fused domain is re-widened with that producer's output
  **type-2** slot (index 2 → node +0x24C). The type-3 "merge" slot is printed but has no reader [R: the only slot
  accesses in the decompile are `node + 0x68 + 0x50·k` and `get_datatype_convert_param(node, 1 | 2, …)`; no index 3, no
  other `0x50·k` offset]; its meaning is not known. Missing slots assert (`pstOpNode->stDataConvIn.s32Size > …`,
  `pstOpNode->stDataConvOut.s32Size > 0`).
- **Option-level `OpParameterConvert`** entries used by the math, LUT and activation kernels (§10.3–§10.7).

**Which conversion `_sgs_FuseOperation` emits** — for each u8 (type 3) input of a fused element-wise domain and for a
u8 output [R: the four wrappers 0x2002_84EC / 0x2002_84FE / 0x2002_8510 / 0x2002_8522, all called only from
`_sgs_FuseOperation`]; fields as §7.6:

| case | encoder | parameters |
|---|---|---|
| input u8 → s16, consuming op ADD (op type 0), MUL (0x12), SUB (0x29) or DIV (0x2A) | `SGS_IPU_HAL_Convert8BitTo16BitForMathOp` 0x2002_CFE2 (0xA8) | the input tensor's **zero point** *z*: w1 31:16 = 0x0204 (operation 4), w5 = 1 \| (−*z*) << 16 |
| input u8 → s16, any other consumer | `SGS_IPU_HAL_Convert8BitTo16Bit` 0x2002_CB70 (0xB0) | u8_s16 slot: w1 23:19 = *shift* (5 bits), operation 4, bit 25; w4 = 0x7FFF \| 0x8001 << 16; w5 = *scaler* (u16) \| *bias* (u16) << 16 |
| output s16 → u8, producing op ADD / MUL / SUB / DIV | `SGS_IPU_HAL_Convert16BitTo8BitForMathOp` 0x2002_CDDC (0xA8) | the output zero point *z*: w1 31:16 = 3 (operation 3, no shifts), w5 = 1 \| *z* << 16 |
| output s16 → u8, any other producer | `SGS_IPU_HAL_Convert16BitTo8Bit` 0x2002_C904 (0xA8) | s16_u8 slot: operation 3; w1 23:19 = *rcshift*, 30:26 = *shift*; w4 = 0; w5 = *scaler* \| *bias* << 16 |

The signedness bit (w3 bit 27 / 28) is the caller's mode argument (1 → set, 2 → clear). So all element-wise arithmetic
runs on s16 data: u8 flows are widened on entry and narrowed on exit.

**Tile planning — `SGS_FW_DefaultElementwiseHeader` 0x2002_87E0.** The header chooses one tile shape for the domain and
then iterates over the tensor in a **resumable 10-level loop** (position saved in the domain record from +100, so a domain
can continue where it stopped):
- shape right-aligned to {N, H, W, C} (rank ≤ 4); lanes per line *k* = 8 (INT16) or 16 (UINT8), other types assert;
- *L* = depth (lines) of the domain's output vector buffer (`vrbuf_get_depth(*0x200a9164)`, asserted > 0 and even,
  `s32OuputVecLen > 0 && …`), halved when the domain ping-pongs (`domain_halve_len`);
- tile W *tw* = min(W, 0x80); tile H *th* = min(H, *L* / *tw*); tile C = C, unless (no input of the domain is a u8 flow,
  `_isFakeHeaderDomain1InputListHasU8Flow`, and every input either is a full tensor or a broadcast vector) and *L* /
  (H·W) ≥ 2: then *tc* = min(⌊C / *k*⌋·*k*, 0x100, *k*·⌊*L* / (H·W)⌋) and the line multiplier `iStack_58` = *tc* / *k*;
  *th* ≤ 0x3FFF;
- loop order: dims 0–6 outermost, then (H, W, C) or, when the input's flag +0x121 is set, the permuted order (C, H, W)
  (indices 7 / 8 / 9 vs 9 / 8 / 7);
- per tile the element-wise tile functions (§10.3–§10.7) run with *n* = `sgs_CalcElementwiseOpLength` lines.

The ping-pong helpers of the fused path are `_sgs_ElementwiseOp_getOutputBufLen` and
`_sgs_ElementwiseOp_getVecAddrByPingPongType` (asserts `eHeaderVrType/eInput1VrType/eOutputVrType==EN_IPU_ISA_PING_BUFFER`,
`s32OutputVecLen …`, `u32Vecaddr==…`); the ping-pong records are those of §4.4 (types 5–8 and 9–12).

**Operands of a tile function.** The exec context's three {VR id, L2 line} records — input 0, input 1, output; *n* =
`sgs_CalcElementwiseOpLength` (lines). "Broadcast" = `sgs_utility_IsBroadCastOp` 0x2002_8608: input 1 is a constant
scalar or vector tensor (`sgs_utility_IsTensorConstAndScalarOrVector`) with fewer elements than input 0 (a constant with
one element is always broadcast); a constant vector whose length differs from input 0's innermost dimension logs
`Tensor %s innermost shape %d is not …` and asserts. Constant lanes (8 per instruction): one element → that value in
all 8 lanes; 2–7 elements → the elements, rest 0; ≥ 8 elements → must equal input 0's innermost dimension, and the 8
values from channel offset *d* = exec-context word 0xB (+8 when context words 0x12 = 1 and 4 = 3), asserting
`di_offset < pstInput1->s32BufElement…`, fewer when the vector ends.

### 10.3 ADD / SUB / MUL / DIV (`Math/elementwise_ipu100.c`) [R unless marked]
Tile functions **`SGS_Sub_IPUExeTile` 0x2005_F6D4, `SGS_Add_IPUExeTile` 0x2005_F918, `SGS_Div_IPUExeTile` 0x2005_FB66,
`SGS_Mul_IPUExeTile` 0x2005_FD2E** (names from their assert strings). All four run on s16 data (§10.2; ADD asserts the
output's `eTensorTypeInEmi` is INT16 or UINT8: `(t & ~4) == 3`). *E0* / *E1* = the option's first / second
`OpParameterConvert` entry (option +0x04 / +0x1C). The fused activation (option +0x00) and the entries' `type` and
`offset` are **not read** by any of the four.

| op | not broadcast | broadcast (input 1 constant) |
|---|---|---|
| ADD | one **0x20** (6 words, `addsub_kernel`): A = in0, B = in1, C = out; w1 = *n* \| operation 7 \| *E0.shift* << 19; w2 = in0 line \| in1 line << 14; w3 = out line; w5 = *E0.scale* (u16) \| *E0.scale2* (u16) << 16 | one **0x28** (14 words, `bcast_addsub_kernel`): A = in0, B = 0xD, C = out; w1 = *n* \| operation 7 \| *E0.shift* << 19 \| (*E0.scale2* & 0x1F) << 26 \| bit 31; w5 = *E0.scale* (u16); **w6–w13 = eight s32 constants**; a single constant must be INT32 (assert) |
| SUB | identical to ADD (same encoder, same fields) | identical to ADD; the constant must be INT32 in both branches |
| MUL | **0xA0** (4 words, `mul_kernel`): A = in0, B = in1, C = out, w1 = *n* \| operation 7 \| *E0.rcshift* << 19, w2 = in0 line \| in1 line << 14, w3 = out line; then **0xE0** (6 words) in place on out: A = C = out, B = 5, w1 = *n* \| operation 7 \| *E1.shift* << 19 \| 0x02 << 24, w5 = *E1.scale* (u16) | one constant: **0xE0** (6 words, `div_kernel`): A = in0, B = 5, C = out, w1 = *n* \| operation 7 \| *E0.rcshift* << 19, w5 = the constant as **u16**; more: **0x60** (10 words, `prelu_kernel`): same w1, **w6–w9 = eight u16 constants** |
| DIV | — (input 1 must be a constant scalar or vector: assert line 0x122) | as MUL broadcast (`div_kernel` / `prelu_kernel`, *E0.rcshift*, u16 constants) |

Reading [I: from the fields; no string names the engine operation]: ADD = `(in0 · scale + in1 · scale2) >> shift` in one
instruction, SUB the same with the converter giving a negative *scale2*; broadcast ADD adds a per-lane s32 constant
(already in the accumulator's scale) with *scale2* reused as a second 5-bit shift; MUL = `(in0 · in1) >> rcshift`, then
a requantising multiply by *E1.scale* `>> E1.shift`; DIV = multiplication by a converter-supplied u16 constant (the
reciprocal) `>> rcshift`. The saturation of the 0x20 / 0xA0 results is not given in the instruction (w4 = 0) — it is the
engine's (§12).

### 10.4 Compares, logical ops, MAXIMUM / MINIMUM, ABS / NEG (`Math/elementwise_ipu100.c`) [R unless marked]
Input 1 is "same shape" when its dimension count and every dimension equal input 0's; otherwise it is a constant
broadcast: a one-element constant (`const_broadcast_load` (0x2005f6c8): shape [1]) is loaded as a 16-bit immediate into SR register 5
(`cmp_load_imm16` → `sr_load_imm16`, the SR 16-bit immediate load, §5.3); a longer constant vector is first loaded into
**vector register 8** by a class **0x66** instruction (`cmp_load_const_vector`, §7.8). The compare / select instructions then
take their second operand from w4 13:0 = 5 (SR) or 8 (vector register 8) with the bit-7 or the 0x3x class (§7.4).
Predicates and the write modes are §7.4; the instructions below are written under reading (A).

| op (tile fn) | instructions |
|---|---|
| LESS `less_kernel` | compare 0x16 (or 0x36 / 0xB6); fill **0xC3** with bit 24, value 0; fill 0xC3 without, value 1 |
| GREATER `greater_kernel` | 0x12 (0x32 / 0xB2); fill 1 (bit 24); fill 0 |
| GREATER_EQUAL `greater_equal_kernel` | 0x16 (0x36 / 0xB6); fill 1 (bit 24); fill 0 |
| EQUAL `equal_kernel` | 0x1A (0x3A / 0xBA); fill 1 (bit 24); fill 0 |
| NOT_EQUAL `not_equal_kernel` | 0x1A (0x3A / 0xBA); fill 0 (bit 24); fill 1 |
| LOGICAL_NOT `logical_not_kernel` | 0xD2 on in; fill 0 (bit 24); fill 1 |
| LOGICAL_AND `logical_and_kernel` | same shape: 0xDA on in0, 0xDA with bit 24 on in1, fill 1 (bit 24), fill 0; broadcast: the constant is first expanded into out by **0x23 / 0xA3** (w4 = 8 / 5), then the same four steps on in0 and out |
| MAXIMUM `maximum_kernel` | same shape: 0x12 (in0, in1), then **0x13** with bit 24 (A = in1, B = in0, C = out) = "B where true, A elsewhere"; broadcast: 0x32 / 0xB2, then **0x3F** with bit 24 writing in0 (w4 = 6), then 0x23 / 0xA3 without bit 24 writing the constant (w4 = 8 / 5) |
| MINIMUM `minimum_kernel` | same shape: 0x12, then 0x13 with bit 24 (A = in0, B = in1); broadcast: 0x32 / 0xB2, 0x3F **without** bit 24 (in0), then 0x23 / 0xA3 **with** bit 24 (constant) |

Fills: class 0xC3, A = out, w1 15:0 = value, 22:21 = 2, w2 26:13 = out line, w3 29:14 = *n* lines. All results are 0 / 1
in the s16 domain.

**ABS** `abs_kernel` = one **0xED** and **NEG** `neg_kernel` = one **0x6D** (4 words): A = in, C = out, w1 22:21 =
the element format (`_SGS_OPS_GetHwDataFormat`, §7.1) \| bit 23, w2 26:13 = out line, w3 = in line \| *n* << 14.

### 10.5 CAST and ROUND [R unless marked]
**CAST** `SGS_IPU_IPUExeTile` 0x2003_654C (`Math/op_cast.c`; CastOptions type 37: +0x00 `in_data_type`, +0x04
`out_data_type`, +0x08 field 2 (i16), +0x0A field 3 (i16), *E* = `OpParameterConvert` at +0x0C: scale +0x10, rcshift
+0x20):
- `out_data_type` = 6 (BOOL): **0xDA** on in with w1 15:0 = field 2 (A = in; w2 bit 29; w3 = in line \| *n* << 14), then
  fill 0xC3 with bit 24, value = field 2, then fill 0xC3 without bit 24, value = field 3 — out = field 2 on the lanes the
  bit-24 write selects, field 3 on the others; field 2 is also the 0xDA operand. Which lanes these are follows the
  predicate polarity (§7.4) [R: fields; I: roles];
- `out_data_type` = 7 (INT16): one **0xE0** (6 words, B = 5): A = in, C = out, w1 = *n* \| operation 7 \| *E.rcshift* <<
  19 \| 0x02 << 24, w5 = *E.scale* (u16) — a requantising multiply;
- any other output type: assert `op_cast.c` line 0x1AC. (Casts to u8 happen in the domain's narrowing step, §10.2.)

**ROUND** `round_kernel` (RoundOptions type 205: *E0* at +0x00 — scale +0x04, rcshift +0x14; *E1* at +0x18 — scale
+0x1C, rcshift +0x2C): two **0xE0** (6 words, B = 5, operation 7, 0x02 << 24, §7.7): in → out with *E0* (`× E0.scale >>
E0.rcshift`), then out → out with *E1*; the second keeps in's L2 line in w2 and out's in w3, as written. Reading [I]: the
first instruction drops the fractional bits by a right shift (rounding inside the engine), the second scales back — so
ROUND's rounding mode is the engine's right-shift rounding, not known (§12).

### 10.6 RELU family, PRELU, LEAKY_RELU (`Activation/op_relu.c`) [R unless marked]
**RELU / RELU6 / RELU_N1_TO_1** share `relu_kernel`: no fused-activation logic in the firmware — the bounds are the
model's. ReluOptions (type 201): field 0 (i16 +0x00), field 1 (i16 +0x02), *E* = `OpParameterConvert` at +0x04. One
**0xB0** instruction (6 words, B = 5): A = in, C = out, w1 = *n* \| operation 7 \| *E.rcshift* << 19, **w4 = field 1
(15:0) \| field 0 (31:16)**, w5 = *E.scale* (u16). With the LUT instruction's use of the same word (31:16 = lower, 15:0 =
upper, §8.1) field 0 is the lower and field 1 the upper clamp [I]; output ≈ clamp((x · scale) >> rcshift) [I].

**PRELU** `_sgs_PRelu_IPUExeTile` 0x2005_361A: PReluOptions (type 202) *E0* at +0x00 (scale +0x04, rcshift +0x14), *E1*
at +0x18 (scale +0x1C, rcshift +0x2C). Three instructions per tile, all with the register-form bit 30 where noted:

| step | alpha is a tensor | alpha is a constant (`s32BufElementSize` checked; per-channel slice of 8 as in §10.2) |
|---|---|---|
| 1 | **0xD2** (4 words, 0x81C0_00D2): A = in; w1 = 0x40_0000; w2 bit 29 = 1; w3 = in line \| *n* << 14 — the sign test | same |
| 2 | **0xA0** (4 words, bit 30): A = in, B = alpha, C = out; w1 = *n* \| operation 7 \| *E0.rcshift* << 19 \| 0x02 << 24; w2 = in line \| alpha line << 14; w3 = out line | **0x60** (10 words): same w1, w6–w9 = eight u16 alphas |
| 3 | **0xE0** (6 words, bit 30): A = in, B = 5, C = out; w1 = *n* \| operation 7 \| *E1.rcshift* << 19 \| 0x02 << 24; w3 bit 30 set; w5 = *E1.scale* | same |

Reading [I]: step 1 marks the negative lanes, step 2 writes `(x · alpha) >> E0.rcshift` there, step 3 writes `(x ·
E1.scale) >> E1.rcshift` into the others (w3 bit 30 = "write the complementary lanes").

**LEAKY_RELU** — `leaky_relu_kernel`, reached from the op table's 0x2005_3616 through two `j` (§2.3). LeakyReluOptions (type
75): +0x00 `alpha` (f32 bits), *E0* at +0x04 (scale +0x08, rcshift +0x18), *E1* at +0x1C (scale +0x20, rcshift +0x30).
One instruction of class **0xD8** (0x58 with bit 7; 6 words; B = 5): A = in, C = out, w1 = *n* \| 0x0207 << 16 \| *s1*
<< 19 \| *s2* << 26, w4 = 0x7FFF, w5 = *m1* (15:0) \| *m2* (31:16), where
- `alpha > 1.0` (`__gesf2(alpha, 1.0) ≥ 1`): (*m1*, *s1*) = (*E0.scale*, *E0.rcshift*), (*m2*, *s2*) = (*E1.scale*,
  *E1.rcshift*);
- otherwise the two pairs are swapped.

Reading [I]: the engine forms both `(x · m) >> s` products and keeps the larger (or the one selected by the sign), and
the swap keeps the slope order the engine expects; the value of `alpha` itself never reaches the engine.

### 10.7 LUT-based ops [R unless marked]
"Clamp" = `SGS_Lut_Impl` arguments 9 / 10 = LUT instruction w4 31:16 / 15:0 (lower / upper output bound, §8.1). *E* = the
option's `OpParameterConvert` entry: ExpOptions 33, LogisticOptions 200 and SqrtOptions 206 carry it as field 1 → +0x2F0
(type +0x2F0, scale +0x2F4, scale2 +0x2F8, shift +0x2FC, offset +0x300, rcshift +0x304; part 1 §7.3). Instruction forms:
"0x28-14" = the 14-word class-0x28 form (B = 0xD, w1 = *n* \| operation 7 \| *E.rcshift* << 19 \| *E.shift* << 26 \| bit
31, w5 = *E.scale* (u16), **w6–w13 = *E.offset* repeated eight times**); "0xE0-6" = the 6-word class-0xE0 form (§7.7). Every
op loads its LUT block first (`SGS_Lut_SetTableConfig`, §8.2).

| op (tile function) | types | steps |
|---|---|---|
| TANH `SGS_Tanh_IPUExeTile` 0x2006_618A | — | LUT in → out, clamp [−32767, 32767] |
| Reciprocal `SGS_Reciprocal_IPUExeTile` 0x2006_624C | — | LUT in → out, clamp [−32767, 32767] |
| LOGISTIC `SGS_Logistics_IPUExeTile` 0x2006_5162 | — | LUT in → out, clamp [−32767, 32767]; then 0x28-14 on out in place with *E* at option +0x2F0 (`_SGS_Lut_Logisticsadd`) |
| EXP `SGS_Exp_IPUExeTile` 0x2006_53D8 | in, out INT16 (asserted) | 0x28-14 with *E* (+0x2F0), source in, destination register = in, destination line = out's line; then LUT in → out, clamp [0, 32767] |
| SQRT `SGS_Sqrt_IPUExeTile` 0x2006_5B58 | in, out INT16 | 0xE0-6 with multiplier *E.scale* (+0x2F4), shift *E.rcshift* (+0x304), register in → in, lines in → out; then LUT, clamp [0, 32767] |
| RSQRT `SGS_RSqrt_IPUExeTile` 0x2006_5DC8 | — | 0xE0-6 in place on in with *E0* (+0x5E0: scale +0x5E4, rcshift +0x5F4); LUT block 0 in → out, clamp [0, 32767]; 0xE0-6 in place on out with *E1* (+0x5F8: scale +0x5FC, rcshift +0x60C); LUT block 1 out → out, clamp [0, 32767]. The option's third field (+0x610 … +0x618) is not read |
| Atan2 `SGS_Atan2IPUExeHeader` 0x2003_81E4 | — | LUT on VR 2 → VR 2, clamp [−32767, 32767] (inside its own fixed sequence, §10.20) |
| WiggleErr helpers 0x2006_6C3C / 0x2006_6D82 | — | two LUTs with the caller's block indices, clamp [−32767, 32767] |
| reduce / softmax helpers 0x2006_40C6, 0x2006_459A, 0x2006_48BA, 0x2006_630E | — | LUT with clamp [0, 32767] (0x2006_630E: VR 4 → VR 0, [−32767, 32767]) |

Reading [I]: EXP and LOGISTIC rescale the s16 input into the table's domain (or the table output into the output scale)
with `(x · scale + offset) >> shifts`; SQRT / RSQRT pre-scale with a single multiply and shift. The arithmetic of the
0x28 / 0xE0 operations and the LUT interpolation are the engine's (§12).

### 10.8 SOFTMAX — `SGS_Softmax_IPUExeHeader` 0x2006_56CC (`Lut/lut_ipu.c`) [R unless marked]
Input and output must have DDR type 0xC (asserts 0x4C0 / 0x4C1). The LUT block sits at option +0x04 (SoftmaxOptions:
+0x00 beta) and is loaded by `SGS_Lut_SetTableConfig` (§8.2). L2 set-up (`_SGS_Softmax_InitL2Buffer`): VR 0 gets a
customer table of 16 one-hot entries (§4.4), VR 1 one entry 0x880 (or a different one for a single-row tensor). Variant
by shape (*R* = product of all but the innermost dimension, *C* = innermost): *R* = 1 → `softmax_sub_max`; *C* < 9 →
`_SGS_Softmax2DOptimizeForSmallIn…` (per row group `softmax_not_innermost`); else `softmax_innermost`
(`_SGS_Softmax2DOptimizeForLargeIn…`). Afterwards the firmware **rewrites the output tensor's quantisation**: scale =
3.0517578125e−5 (= 2^−15, f32 at 0x2008_FFE0), zero point = 0 (both words of the i64) — the softmax output is Q15
whatever the model says.

All three variants do, per row (*C* values): **max** → **exp(x − max)** by the LUT → **sum** → **reciprocal of the sum**
on the SR → **multiply** → store:
- **Single row** `softmax_sub_max`: requires the DDR / element types of its asserts (5 or 2) and *C* ≤ the L2 capacity
  (else return −1); pad mask **0x1E** when *C* mod 8 ≠ 0; **0x82** lane maximum into a register; SR move `sr_move_b`
  (opcode 0x06, bit 7) placing the maximum where the subtract reads it; **0xA5** (x − max, w4 = 5 = SR operand); LUT
  (exp table, clamp [0, 32767]); **0xC6** lane sum into register 6; `SGS_Softmax_RecipOfAcc_OneLane`; then the
  normalising multiply and the store.
- **`SGS_Softmax_RecipOfAcc_OneLane`** 0x2006_FF0E (`Lut/IPU200M/drv_lut_ops_ipu200.c`): SR *r0* ← **0x4000_0000**
  (opcode 0x48, imm32 = 2^30); **0xA3** broadcasts the sum (register 6, w4 = 6) to a line; two **0x66** load the line
  into SR *r1*, *r1* + 1 (32-bit, w2 26:13 = 8); **SR opcode 0x65 bit 7** (`sr_three_reg_op`: rd = *r2*, ra = *r0*, rb =
  *r1*) — *r2* = 2^30 / sum (§5.3).
- **Per-row helper** `softmax_recip_step` does the same reciprocal for an arbitrary register triple; `softmax_kernel` emits
  the normalising multiply as class **0x00** (§7.8).
- **Small innermost** (log tag `_SGS_Softmax_Innermost_IPUImpl`, `softmax_not_innermost` per group) and **large**
  (`softmax_innermost`): DMA-load a group of rows; **0xD3** (sub-op 1, `reduce_d3_helper`) brackets; per row **0x02 / 0x25**
  (`softmax_max_step`: running maximum, then subtract); LUT exp; per row **0x46** (`softmax_sum_step`, lane reduction) +
  `softmax_kernel`; **0xD3**; DMA store. The large variant loads and stores each row group in several DMA pieces (`do …
  while` over the pieces) and runs one extra LUT pass before the loads [R: order].

### 10.9 Convolution and FULLY_CONNECTED
CONV_2D (0x2002_C60C) and DEPTHWISE_CONV_2D (0x2002_C624) enter `conv_exec_layer_callback_helper` 0x2005_B83A, which
reads the partition and loop configuration, builds the post-quantisation record and runs one of the loop nests — all of
§6. FULLY_CONNECTED has no function (§2.3).

### 10.10 Pooling and MEAN [R unless marked]
**Pooling.** `pooling_exec_layer_callback_helper` 0x2005_C06C / `AveragePool_Domain1HeaderExecuteLayerFunc` 0x2005_C326
call `Pooling_operation_8bitInput` 0x2006_EF96, `Pooling_operation_16bitInput` 0x2006_E4D8 or
`Pooling_operation_16bit_out1x1` 0x2006_F9D6 (global pooling to 1×1) with the conv partition machinery (`sdk_pool.c`):
three nested loops over OB software tiles (do, x, y; steps from the OB partition), per tile `get_ib_hwtile_fromob` with
the pooling window as the kernel, `load_vp_ib_fromemi`, one pooling instruction (§7.5; tile depth ≤ 16 lanes per pass,
asserted `need_loop_in_depth16`, `tile_do_aligned % 16 == 0`), and a DMA store (`pool_dma_store` (0x2000bdf6), type 10). Averages over
more than 0xFFFF taps / pixels are refused (`f_w * f_h < AVE_POOL_U8_W_H_MAX`). The 16-bit variants make the same
instruction and partition calls [R: call list]; their per-tile arithmetic beyond the loop markers and calls is not
known.

**`MeanRunAsAvePool_CheckCollapse2OneConditions`** 0x2006_DF76 (used by `AveragePool_Domain1HeaderExecuteLayerFunc`, not
by MEAN): an average pool collapses to the 1 × 1 kernel `Pooling_operation_16bit_out1x1` when input H·W > 0x2E00, output
H = W = 1, input W < 0x10000, the filter equals the input W × H, input and output are INT16 and their C are equal.

**MEAN** `MeanRunAsAvePool_Domain1HeaderExecuteLayerFunc` 0x2005_C556 (`CONV_POOL/IPU200M/sdk_mean.c`): input 4-D; the
axes tensor 1-D with 2 elements and element size 2; axes must be {1, 2} in either order (`SGS_Mean_CheckMeanRunAsAvePool`);
`keep_dims` (option byte +0) or an output whose leading dimensions are all 1 (assert `pstOpt->bKeepDim == TRUE || …`).
`MeanRunAsAvePool_CheckHwCapability` (0x2006_DF10): input UINT8 or INT16 (else the op does nothing and the header
asserts), and W·H ≤ 0x3E00 (else `Implement by ACC` + assert — not implemented). It then runs as an **average pool of
type 10 (AvgMean)** with window W × H on VR 9 → VR 10 (`Pooling_operation_8bitInput` for UINT8 with 16 lanes,
`Pooling_operation_16bitInput` for INT16 with 8 lanes; instruction 0xDB with w3 bit 30, §7.5); `MeanRunAsAvePool_SetupVr`:
when ⌈W·H / 512⌉ slots ≠ 1 (≤ 31 asserted, `MAPPING_TABLE_VECTOR_NUM`) VR 10 gets one slot and VR 9 that many (tables
from 0x2009_0E54), after an op-3 barrier 0, restored afterwards (`vr_restore` (0x2006df00)). The header returns −1 after the pool.

### 10.11 Resize — RESIZE_BILINEAR / RESIZE_NEAREST_NEIGHBOR (`DMA/op_resize*.c`, `DMA/sdk_resizebilinear.c`, `DMA/IPU200M/drv_resizebilinear_ipu200.c`) [R unless marked]
**Entry and path choice.** Op functions `op_resize_bilinear` (RESIZE_BILINEAR → `_SGS_ResizeBilinear` 0x2005_3FF0) and
`op_resize_nearest` (RESIZE_NEAREST, `_SGS_ResizeNearest` by its asserts) convert output / input / size tensors into IPU
tensor records (§9.2), assert ≥ 3 dimensions and that the size tensor's two values equal the output's H and W, and call
`SGS_OPS_ResizeBilinear` 0x2005_A412 / `SGS_OPS_ResizeNearest` 0x2005_AE32 with {`align_corners` (option byte +0),
**phase type 5**}. Both extend the shapes to 10 dimensions and assert that the product of dims 0–6 is 1 (batch 1 only,
`SGS_OPS_GetAccumulatedProduct(shape, 6, 0) == 1`). Then by phase type (`enPhaseType`, printed with `bAlignCorners`):

| phase type | path |
|---|---|
| 1, 2 | nothing is computed; the function returns 1 — unused (callers pass 5) |
| 3 | CPU path (bilinear on s16 data / nearest) — unused (callers pass 5) |
| other (5) | input H·W = 1 → **repeat path**; else **engine path** `_SGS_ConvBilinear` 0x2005_D46E with argument 5 = 0 (bilinear, through the stub 0x2005_DFE0) or 1 (nearest, stub 0x2005_DFE6) |

**Scales** — `SGS_OPS_GetResizeBilinearScaleValue` 0x2002_E08C (RESIZE_NEAREST has its own copy,
`_SGS_ResizeNearest_GetScaleValue` log tag, identical formulas). Single precision: sH = (float)inH / (float)outH, sW =
(float)inW / (float)outW; with `align_corners` ≠ 0: sH = (float)(inH − 1) / (float)(outH − 1) if outH > 1 (likewise sW).
Two integer ratios are also computed — align_corners = 1: (outH − 1)/(inH − 1) truncated (outH when inH = 1), else
(int)(outH / inH); the same for W — but they are only printed (`s32HeightScale = %d, s32WidthScale = %d`), never used [R:
no other reader in either caller]. No half-pixel offset exists in this firmware: source = destination · scale (TF's
legacy `align_corners = False` mapping [S: TensorFlow resize semantics; I: equivalence]).

**Engine path** — `_SGS_ConvBilinear` 0x2005_D46E. Constants by input kind: pixel mode (input DDR type 1–5 or 15–17,
`is_pixel_mode` (0x2005d19a)) → *Cap* = 2048.0, *L* = 0x8000, *D* = 0x800; otherwise *Cap* = 512.0, *L* = 0x2000, *D* = 0x200. Lines
per pixel *g* = ceil(*k* · C / 128) with *k* = 8 if the input element size (record +0x0C) is 1, else 16; channel-group
blocks *nb* = ceil(*g* / 16). Tile search (float arithmetic as written, `floor` / `ceil` on the widened double):

```
best = (tw 1, th 1, cg 1)
for cg16 in 16, 32, … 16·nb:   cg = min(cg16, g)
  for tw in 1 .. min(L, outW + 1):        inTw = min(inW, floor(tw·sW + 3 − sW))
    for th in 1 .. min(L, outH + 1):      inTh = min(inH, floor(th·sH + 3 − sH))
      cost = inTw·inTh·cg + ceil(tw·th·cg / Cap) · D
      if cost ≤ L and ceil(outW/tw)·ceil(outH/th)·ceil(g/cg) < ceil(outW/bestTw)·ceil(outH/bestTh)·ceil(g/bestCg):
        best = (tw, th, cg)                # strictly fewer tiles; ties keep the earlier
(g = 0: tw = th = cg = 1)
```

(The `+ 3 − s` term is the input window of *t* output pixels, `floor((t − 1)·s) + 2`, written in float [I: algebra].)
Bytes per pixel of a channel tile *P* = 16·cg (`iStack_1d4`). **L2 split:** *o* = ceil(tw·th·cg / *Cap*) slots for the
output; for each half *h* ∈ {0, 16} (ping, pong) the input VR (8 / 0xC) gets slots *h* … *h* + 15 − *o* and the output VR
(0xE / 0xF) the next *o* slots (mod 32), all via `vrbuf_init_bufid(id, 1, n, list, 1)` (§4.4). **Tile loop:** for n in
batch (out +0x28) → channel tile *c* (count ceil(out +0x5C / *P*)) → row tile *y* (step th) → column tile *x* (step tw),
alternating ping / pong; the last row / column tile is outH − *y* / outW − *x*. Per tile:

| value | formula |
|---|---|
| inX0, inY0 | `(int)floor((float)x · sW)`, `(int)floor((float)y · sH)` |
| inX1, inY1 | min(inW − 1, `floor((float)(tw' + x − 1) · sW) + 1.0`), the same in y with th', inH |
| input window | (inX1 − inX0 + 1) × (inY1 − inY0 + 1) pixels, DMA from `data + inY0·rowStride + inX0·pixStride + c·P` |
| fractional start | fX = x·sW − (float)inX0, fY = y·sH − (float)inY0 (float) |
| fixed point | {qsW, qfX}, shiftX = `fixq31`({sW, fX}); {qsH, qfY}, shiftY = `fixq31`({sH, fY}) |

then one **0xAC** instruction (§6.6) per 16-byte channel line of the tile (input / output L2 offsets advanced by the input
/ output window areas; w1 / w2 = qfX / qfY, w6 / w7 = qsW / qsH, w8 = shiftX \| shiftY << 6, bit 12 pixel mode, bit 13 =
nearest, bit 14 = more than one channel), followed by the DMA store of the output tile. The nearest mode takes the same
fixed-point scale and start fraction as bilinear; how the engine interpolates, and how its nearest mode selects the
source pixel (truncation or rounding of the fixed-point position), are not known (§12).

**`fixq31`** (`fixq31`, used nowhere else): for an array *a*[*n*] of floats —
1. *m* = *a*[0]; for *i* ≥ 1, if (double)|*a*[*i*]| − (double)|*m*| > 1.1920929e−7 (FLT_EPSILON, 0x2008_9580) then *m*
   = *a*[*i*];
2. shift = (int)floor(log(2147483647.0 / |*m*|) / 0.6931471805599453) — the largest *s* with |*m*|·2^*s* ≤ 2^31 − 1;
3. qm = (int)round(*m* · 2^shift) (`pow(2.0, shift)`, double);
4. for each *i*: if |*a*[*i*] − *m*| (float) < FLT_EPSILON then *q*[*i*] = qm, else *q*[*i*] = (unsigned)round(*a*[*i*] ·
   2^shift) (`__fixunsdfsi`: negative values become 0).

**CPU bilinear** (phase type 3, unused): for every output (y, x) and channel: fy = (float)y · sH, y0 = (int)fy
(truncation), y1 = min(y0 + 1, inH − 1); fx, x0, x1 likewise with sW; dy = fy − y0, dx = fx − x0 (float); weights (float)
w00 = (1 − dy)(1 − dx), w10 = dy(1 − dx), w01 = (1 − dy)dx, w11 = dy·dx; {q00, q10, q01, q11}, s = `fixq15`(w00, w10,
w01, w11); output s16 = (s16)((in[y1,x0]·q10 + in[y0,x0]·q00 + in[y0,x1]·q01 + in[y1,x1]·q11) >> (s & 31)) in 32-bit
int arithmetic (arithmetic shift, no rounding term, no clamp). `fixq15` (0x2002dec6) = `fixq31` with 32767.0
instead of 2^31 − 1 and a signed result (`__fixdfsi`, stored as 16 bits). **CPU nearest** (phase type 3 of
`SGS_OPS_ResizeNearest`, unused): source row = min((int)((float)y · sH), inH − 1) (truncation), column likewise, the
element (record +0x00 bytes) copied with `memcpy`.

**Repeat path** (input 1 × 1, identical code in both ops): *S* = input bytes (C · element size), *N* = out H · out W. If
*S* ≤ 0x4_0000 (256 KiB): DMA the input vector into L2 at 0, replicate it *r* = min(0x4_0000 / *S* − 1, *N*) times behind
itself with `SGS_OPS_CopyDataInL2` (255-line vectors, §7.2), then store (*r* + 1)-copy blocks to the output until *N*
copies are written (the last block holds the rest). If *S* > 256 KiB: in 256 KiB chunks, load a chunk and store it *N*
times (`_SGS_ResizeBilinear_RepeatInputTensor` log line).

### 10.12 TRANSPOSE — `SGS_OPS_Transpose` 0x2002_3DEC (`DMA/sdk_transpose.c`) [R unless marked]
Entry 0x2001_E80A → `op_transpose` (0x2001e6ec) → `SGS_OPS_Transpose`. With the tile maths of `sdk_tile.c` it is the largest of the
DMA-layout ops: one 11 436-byte function with its helpers inlined; the log tags
name the inlined steps: `_SGS_OPS_IsDestSameAsSource`, `_SGS_OPS_SaveSourceTensorInfo`, `_SGS_OPS_SaveDestTensorInfo`,
`_SGS_OPS_StorePermInfo`, `_SGS_OPS_SplitDimensionInL2`, `_SGS_OPS_SplitL2Perm`, `_SGS_OPS_CalculateTensorInforInL2`,
`_SGS_OPS_CalculateDimensionAndStride…`, `_SGS_OPS_GenerateDimensionBefore…`, `_SGS_OPS_CalculteSrcDdrAddrOffset`,
`_SGS_OPS_TransposeByDma`, `_SGS_OPS_DoTransposeInL2`, `_SGS_OPS_VectorTransposeAction`, `_SGS_OPS_CopyTensorByDma`.
Records are IPU tensor records (§9.2).

1. **Checks**: equal dimension counts (≤ 10) and element sizes, `dst.shape[i] == src.shape[perm[i]]` for every *i*
   (asserts at `sdk_transpose.c` 0x63C / 0x640).
2. **`SGS_OPS_AlignTensor(0xC, src, phase, 1)`** before and `(0xC, dst, phase, 0)` after: innermost-dimension
   re-alignment of the tensor in DDR when its innermost dimension is 1 and the tensor is stored with the 64-byte-rounded
   size, or when tensor extension flag bit 4 is set; its re-layout arithmetic is not known.
3. Phase type (record word 10 of the parameter block) 1–3, or an element size other than 1 or 2 → nothing more is done.
4. **Same-as-source test**: counting the source dimensions ≠ 1 and testing whether `perm[i] == i` for all *i*: a pure
   identity, or a tensor with only one dimension ≠ 1, is **copied**: VR 0 gets all 32 slots (customer table 0 … 31,
   `dma_op_start(0, list, 32, 0)`), *Lb* = (depth(VR 0) − 1)·16 bytes, and the tensor goes DDR → L2 → DDR in ceil(total
   / *Lb*) linear chunks (the last one the remainder).
5. **Otherwise** (`SGS_DMA_OpsStart(0xC, 0)`, VR 0, *Lb* as above): dimensions of size 1 are removed from the shape and
   the permutation (every permutation index above a removed index is decremented); if nothing is left the tensor is taken
   as 2-D [1, *n*]. Per remaining dimension *d* (innermost last) the record keeps {size, element stride, byte span},
   where the innermost span is rounded up to a multiple of 16 bytes (`ceil(x/16)·16`). **Splitting**
   (`_SGS_OPS_SplitL2DimByShape`): walking from the innermost dimension outwards, a dimension whose byte span is below 16
   while the running element size is below 16 is split by the factor 16 / size, so that every DMA row moves at least
   one 16-byte L2 line; the permutation indices above the split dimension are incremented and the new dimension inserted
   after it. The same is done for the destination side (`split for dest dim inx …`).
6. The data then move in L2-sized batches: per batch a strided DMA load (`load_dma_loop`, `gas32LoadDmaDimLoop` …
   `s32L2BatchDim`), the permutation inside L2 (`_SGS_OPS_DoTransposeInL2` / `_SGS_OPS_VectorTransposeAction`), and a
   strided store (`save_dma_loop`), with the source DDR offset of each batch from `_SGS_OPS_CalculteSrcDdrAddrOffset`;
   `dma_op_end` restores the default VR tables at the end (§6.5).

The split arithmetic beyond the rule stated and the batch / loop computation of step 6 are not specified: they exist
only as the vendor's call / instruction order (about a thousand lines of it).

### 10.13 Other data-movement ops and the alias entries [R unless marked]
**Alias entries** — the op-table pointers are separate stubs [R: disassembly]: `Reshape_Alias` 0x2005_3EA8, `Tile_Alias`
0x2001_DDD2, `Pad_Alias` 0x2004_998E, `Split_Alias` 0x2001_D86C, `Unpack_Alias` 0x2001_E848, `Slice_Alias` 0x2001_D688,
`Concatenation_Alias` / `Concatenation_Skip` 0x2003_6AA6, `ResizeBilinear_Alias` 0x2005_3FEC, `DummyOp` 0x2004_76B8,
`Align2` / `Align2_Alias` 0x2003_3E96 are all **`li a0, 0; ret`** (nothing runs: the converter placed the data already —
the way the converter expresses in-place views [I]); **`StridedSlice_Alias` 0x2001_DB7E returns −1** (an error: a model
using it fails at that op); **`Transpose_Alias` 0x2001_E830 is `j 0x2001_E80A`**, i.e. the real TRANSPOSE runs;
`DmaCoefMatrix_Alias` (0x2003_CA6E → `j` → 0x2005_CE8A) returns from inside the DmaCoefMatrix function (§10.19).

**Real data-movement ops** — op function → body (`DMA/*.c`: `op_*.c` front ends, `sdk_*.c` bodies, `dma_ops_common.c`,
`drv_dma_ops_common.c`, IPU200M `drv_dma_ops_ipu200.c`, `drv_tile_ipu200.c`); all go through the shared DMA mover
(`SGS_DMA_OpsStart`, `SGS_DMA_MoveData`, `SGS_DMA_AdjustEmiHeight`, `SGS_DMA_AdjustL2Width`, `SGS_OPS_AlignTensor`,
`SGS_OPS_RemoveInnerDimensionWithShapeValueIs1`, `SGS_OPS_CalculateStrideWithAlignment`) with 10-dimension extended shapes
and strides, log lines `in: DimSize … ElementSize …`, `input shape / stride`, `enPhaseType`:

| op | entry → body | firmware-specific behaviour read |
|---|---|---|
| RESHAPE | 0x2005_3FB2 → `_SGS_Reshape` → `SGS_OPS_Reshape` | element sizes equal (assert); copy only when the aligned layouts differ [I] |
| CONCATENATION / Customized_Concatenation | 0x2003_6AAA → `SGS_OPS_Concat` (7.5 KB) | per input a strided move to its offset along the axis; `s32AccProduct`, `s32Axis` logs; uses the changed-input list of ConcatenationOptions (part 1 §7.3) [I] |
| SLICE / STRIDED_SLICE | 0x2001_D830 / 0x2001_DD96 → `_SGS_Slice` / `_SGS_StridedSlice` → `SGS_OPS_StridedSlice` | both bodies are `SGS_OPS_StridedSlice` (SLICE with stride 1 [I]) |
| SPLIT / SPLIT_V | 0x2001_DB42 → `op_split` (0x2001d870) → `SGS_OPS_Split` | |
| PACK / UNPACK | 0x2004_9798 / 0x2001_E84C → `SGS_OPS_Pack` / `SGS_OPS_Unpack` | fast path when the innermost stride is a multiple of the slot width (`SGS_OPS_IsInnermostStrideOfTensorDivisibleBySlotWidth`) |
| TILE | 0x2001_DEFA → `op_tile` (0x2001ddd6) → `SGS_OPS_Tile` | CPU fallbacks `_SGS_OPS_TileByCpu`, `_SGS_OPS_TileCopyMultipleTimesByCpu` (log tags) |
| PAD | 0x2004_9B48 → `_SGS_Pad` → `SGS_OPS_Pad` | element sizes equal |
| GATHER | 0x2004_79B8 → `_SGS_Gather` → `SGS_OPS_Gather` | input alignment asserted |
| FILL / ZEROS_LIKE | 0x2004_7824 → `op_fill` (0x2004781c) → `fill_dispatch(…, 5, …)` (0x2004775c); 0x2001_EA22 → `SGS_OPS_Fill` (value 0) | `SGS_DMA_FillMemory` (class 0xC3 fill) over the aligned buffer, `value = 0x%x` log |
| Dilation | 0x2004_7686 → `_SGS_Dilation` → `SGS_OPS_Dilation` | strided scatter with zero fill [I] |
| TRANSPOSE | §10.12 | §10.12 |
| **SHAPE** | 0x2001_D62C | **CPU**: output[*i*] = (int16) input dimension *i* — dimensions are written as 16-bit values whatever the output type |

The index arithmetic of these ops is the TFLite definition of each op [S: TFLite reference kernels]; the firmware's own
part is the stride / alignment bookkeeping of the shared mover (its call order is not specified).

### 10.14 SUM — `SGS_ReduceSum_IPUExeHeader` (0x20053112) (`Custom/op_reducesum.c`) [R unless marked]
**Header.** Input and output DDR type 0xC (asserts 0x9A6 / 0x9A7); ≤ 10 dimensions. Axes (input 1's int32 data): a
negative axis gets + rank, then 0 ≤ axis < rank is asserted (`_SGS_ReduceSum_ResolvedAxis`), duplicates are dropped, the
list is sorted ascending (`qsort` 0x2007_8408, comparator 0x2004_DCF6) and must be **consecutive** (else `The resolved
axis is invalid` + assert). Option (ReducerOptions 27 → ReduceSumOptions, part 1 §7.3): byte +0x04 = first shift *s0*,
`OpParameterConvert` *E* at +0x08 (scale +0x0C, rcshift +0x1C). Variant (C = input innermost dimension; *P* = product
of the reduced dimensions with C rounded up to 8):

| condition | variant |
|---|---|
| last axis ≠ rank − 1 (innermost dimension kept) | `_SGS_ReduceSumOnnotInnermostDim` 0x2005_03BE |
| innermost reduced, output has 1 element | `_SGS_ReduceOnInnermostDim_ResolveAllAxis` 0x2004_E214 |
| innermost reduced, the slot split of `_SGS_ReduceOnInnermost_AllocVRSlot` fits, and (C is a multiple of 8, or only the innermost axis) | `reduce_sum_kernel` (single axis) |
| the same, else | `_SGS_ReduceOnInnermostDim_ResolveMultiAxis` 0x2005_0F84 |
| split does not fit, C not a multiple of 8 and not (single innermost axis) | `_SGS_ReduceOnInnermostDim_SplitLane_MultiAxis` 0x2004_F788 |
| split does not fit, else | `_SGS_ReduceOnInnermostDim_SplitLane_SingleAxis` 0x2004_EB96 |

**`_SGS_ReduceOnInnermost_AllocVRSlot`** (*P* must be a multiple of 8, assert `s32ResolvedAlignInnerDim % ONE_SLOT…`),
*D* = output innermost dimension, *g* = ceil(*D* / 8): *s* = (int)ceilf((float)(256·*g*) / (float)(*P*·*D* + 16·*g*));
input gets 32 − 2*s* slots, output 2*s* slots; accepted when 32 − 2*s* > 0, *s* > 0, (*s*·8192)/(16*g*) > 0 and
((32 − 2*s*)·4096)/(*P*·*D*) > 0; otherwise 0 / 0 and the SplitLane variants run.

**L2 set-up** `_SGS_ReduceSum_InitL2Buffer(n0, n1)`: VR 0 gets slots 0 … n0 − 1 (≤ 31), VR 1 the next n1 (n0 + n1 ≤ 32),
and the MR (accumulator) tables MR 0 / MR 1 are installed (`set_customer_mr_tbl`); `mr_tables_restore` (0x2004e156) restores the
defaults.

**Building blocks** (classes of §7.8):
- `reduce_mask_step_b` / `reduce_mask_step(C)`: **0x1E** lane masks — all lanes, then for C mod 8 ≠ 0 the mask `(−1 << (C mod
  8)) & 0xFF` on the last line of each row (w2 26:13 = ceil(C/8) − 1), so the pad lanes of a row are excluded;
- pad clearing: **0xC3** fill 0 over each row's padding (ResolveAllAxis: when C is not a multiple of 8);
- lane reduction **0xC6** (w2 26:13 = 6: into register 6; w3 29:14 = lines);
- accumulation: **0xA3** (0xCA04_00A3, w4 = 6, w2 bit 27) copies register 6 into the accumulator on the first chunk,
  **0xA1** (0xC204_01A1, w4 = 6) adds it on the later ones (ResolveAllAxis); the SplitLane / not-innermost variants use
  **0xD3** (sub-op 1) + **0x46** (register reduction) and the pair **0x21** (0x8204_0121 / 0x8204_2121) for the
  accumulator, with **0xC3** fills (0x8984_00C3 / 0xC184_00C3) clearing the accumulator on the first pass;
- final requantisation, two **0x10** (6 words, B = SR 1 or the accumulator, w1 15:0 = 1 / length, operation 7): first
  `× 1 >> s0`, clamp [−32767, 32767] (w4 = 0x7FFF \| 0x8001 << 16), w3 bit 31; then `× E.scale >> E.rcshift`, same
  clamp; then the DMA store of the result (`result_dma_store` (0x2000486a) → `hal_dma_store`).

**ResolveAllAxis**, the shape read in full: *R* = product of the dimensions from the first axis to the last but one, C8
= C rounded up to 8 (assert C8 < 0xF000, `s32AlignInnerDim < MR_ACC_SLOT_NUM…`), slots (30, 2); rows per chunk *r* =
(depth(VR 0)·8) / C8, chunks ceil(*R* / *r*); an SR register is first set by `drv_ipu_sr_smove128_immed` (2 words); per
chunk: DMA load *r'*·elem·C8 bytes from input + offset, clear pads (0xC3 per row), 0xC6 over the chunk's lines into
register 6, 0xA3 (first) or 0xA1 (others); after the loop the two 0x10 and a store of one element·8 bytes.

The other four variants follow the same pattern with an outer loop over output rows (`s32ProcessOutTensorOuterDimCnt…`,
`s32RealProcessOuterDimCnt`, `s32OffsetOnOuterDim` asserts) and ≤ 16 or ≤ 8 lines per accumulation group; their chunk-size
formulas exist only as the call / instruction order and are not specified.

### 10.15 REDUCE_MAX, ARG_MAX, TOPK_V2, RANGE [R unless marked]
**REDUCE_MAX** `_SGS_ReduceMax_IPUExeHeader` 0x2004_94AE (`Custom/op_mean.c`): DDR types 0xC; **one axis** (input 1's
first value, + rank if negative, asserted < rank; the dimension on it ≤ 0x7FFF, `s32DimOnAxis < INT16_MAX`). VR 0 gets a
16-entry one-hot customer table (§4.4), MR 1 an 8-entry table (entry *i* = 1 << *i*). Innermost axis →
`_SGS_ReduceMaxOnInnermostDim` 0x2004_8210: per row chunk a DMA load, the **0x1E** pad mask when C mod 8 ≠ 0, then per
output **0x82** (running max of two vectors) into a register, a CPU read-back / SR step (`sr_move_b`) and **0xA3**
expanding the result, stores at the end of each output group. Other axis → `_SGS_ReduceMaxOnNotInnermostDim` 0x2004_7A3A
(input INT16 or DDR 0xC): along the axis, step 0 copies the slice (**0x23**), every later step compares (**0x36**,
vector register 8 = running max) and writes the larger with **0x23** under the predicate; chunks of ≤ 0x3FF lines; store
per outer index. No requantisation: the output keeps the input scale [R: no scale write].

**ARG_MAX** `SGS_ArgMax_IPUExeHeader` 0x2003_6422 (`Custom/op_arg_min_max.c`): DDR types 0xC, one axis (as above,
dimension < 0x8000); innermost → `_SGS_ArgmaxOnInnermostDim` 0x2003_3FE8, else `_SGS_ArgmaxOnNotInnermostDim` 0x2003_540C,
which keeps a running maximum and a running index: step 0 copies the values and writes index 0 (**0x23**), every later
step increments the index register by an immediate (**0xC1**), compares (**0x36**), writes the new maximum (**0x23**) and
selects the index (**0x13**) under the predicate. The innermost variant uses 0x02 / 0x13 (w4 = 1 \| 1 << 16) / 0x36 /
0xD3. Afterwards the **output scale is set to 1.0** (f32 0x2007_D454): indices are plain integers. ARG_MIN is not
registered.

**TOPK_V2** `OPS_IPU_TopK` 0x2001_E4B2 (`Custom/op_topkv2.c`): input 0 INT16, input 1 (*k*) INT32, output 0 (values)
INT16, output 1 (indices) INT32 (asserts 0x32B–0x32E); rank ≤ 4, right-aligned into {N, H, W, C}; *k* > C → return 0
(failure). Per (n, h, w) row `topk_kernel`: DMA the row of C values into L2 (`_TopK_IPU_vLoad_EMItoL2`), mask the
padding lanes of both work registers (**0x1E**, `topk_mask_step`), then *k* times: **0x82** (`topk_max_step`: lane maximum
with its index into SR register 6, w2 26:13 = 6, count ceil(C/8) + 1), wait for the queue to drain (`dec0_drain(0)`:
polls the decoder level, then op 3 / op 4 / 0), **read SR 6 by CPU** (`hal_ipu_sr_get_value`): index = low 16 bits, value
= high 16 bits, written to the outputs; then **0x1E** (`topk_mask_step_b`: one-lane mask of the chosen element) and **0x7E**
(`topk_select_step`) to knock the chosen element out of the next round. Output row strides: values align8(*k*)·2 bytes,
indices align8(*k*)·4 bytes. So TOPK is *k* rounds of "max + index, read back, exclude"; ties resolve as the engine's 0x82
resolves them (§12). The VR / MR tables (`range_vr_mr_tables` (0x2001df34)) come from the constants at 0x2008_5DA4 … 0x2008_5DD0.

**RANGE** `_SGS_Range_GetOutputSize` 0x2004_D722 (op table entry 0x2004_DCDC; `Custom/op_range.c`, log tag
`_SGS_Range_FloatImpl`), **all on the CPU**: start, limit, delta and the output must each have one dimension; their
types FLOAT32, INT32, INT16 or INT8. delta ≠ 0 and the direction must agree (start < limit with delta > 0, or start >
limit with delta < 0), else assert. Expected count *m* = ceil(|limit − start| / |delta|) (integer types, integer
arithmetic) or (int)ceil(|(limit − start) / delta|) (FLOAT32, single precision). The output is then filled for **the
output tensor's own length** *L*: out[0] = start, out[*i*+1] = out[*i*] + delta — FLOAT32 by repeated `__addsf3`, INT32
exactly, INT16 and INT8 wrapping to 16 / 8 bits. If *m* ≠ *L* and the output is INT16, the last element is saturated:
when out[*m*−1] > 0 and out[*L*−1] < 0 it becomes 0x7FFF, when out[*m*−1] < 0 and out[*L*−1] > 0 it becomes −0x8000.
Another input type logs `Unknown data type: %d` and falls through to the type dispatch again with *m* = 0.

### 10.16 TFLite_Detection_NMS and the YOLOv3 NMS (`Custom/op_custom_ssd_nms_ipu_offline.c` / `_online.c`, `Custom/IPU200M/op_custom_ipu_implement.c`) [R unless marked]
81 functions are reachable from the op function and from the YOLOv3 kernel [R].

**Parameter record.** The op function `op_nms` runs the parser `nms_parse_params` (0x2003a72a) on every invoke (record at
0x200A_A4E4, **0x88 bytes = 34 words**, copied to the stack; words 0–22 are the FlexBuffer keys of part 1 §7.4) and then
the dispatcher `nms_dispatch` (0x2004351c). Besides the FlexBuffer keys, the parser fills from the **NMSOptions table** (type 203,
option struct = node +0x1C; part 1 §7.3) — first, for both forms:

| word | value |
|---|---|
| 0x21 (byte) | NMSOptions field 15 (+0x38) ≠ 0 — "the table carries the parameters" |
| 0x1E | 8 (default detection type) |
| 0x17 / 0x18 | field 13 (+0x30) / field 14 (+0x34) |
| 0x20 | field 1 (+0x04) read as **f32** *v*: *v* > 0 → `(unsigned)(1.0 / v)` (double division, truncated); else 0x7FFF |
| 0x19 | field 2 (+0x08) as f32 → `(int)` (truncation) |
| 0x0D (13, `nms`) / 0x0E (14, `clip`) | field 4 (+0x10) / field 3 (+0x0C) as f32 → `(float)(int)v` (truncated to a whole number, kept as float) |

and, when field 15 = 1, instead of the FlexBuffer: word 0x1E = field 0 (detection type), 0x0F–0x12 (15–18:
`max_detections`, `max_classes_per_detection`, `detections_per_class`, `num_classes`) = fields 7–10, 0x15 / 0x16
(`nms_score_threshold` / `nms_iou_threshold`, f32 bits) = fields 5 / 6, 0x13 (`bmax_score`) = byte field 12 (+0x2D);
field 11 (+0x2C) is not read. Otherwise the FlexBuffer keys are read (missing root / not a map → return with the
defaults). So NMSOptions field names follow from the words they fill [R: positions; I: names by the FlexBuffer key of the
same word]. Field 1 is the coordinate scale (the clip bound word 0x20 = ⌊1 / field 1⌋, used by the YOLOv3 clip below);
field 2 is the s16 score threshold of the CPU pre-filter (word 0x19). The roles of fields 13 / 14 (words 0x17 / 0x18,
read by the stage functions as SR immediates) are not known.

**Variant choice — `nms_dispatch`.** *off* = word 0x14 (`offline`), *t* = word 0x1E. A score / confidence /
face-coordinate index of −1 means "absent".
- FlexBuffer form (word 0x21 = 0): any of the four coordinate indices (words 0–3) = −1 → return 0. Then by presence of
  the score (word 5), confidence (6) and face-coordinate (7) inputs: face present → *t* = 5; else score and confidence →
  *t* = 7; confidence only → *t* = 4; score only, or neither, with *t* = 0 → type 0; neither with the default *t* = 8 →
  nothing is run.
- Table form: *t* from the table.
- *t* = 0 → `nms_online_t0` (0x2004529a) (with *off* forced to 0); *t* ∈ {1, 2, 3, 4, 7}: `bmax_score` = 1 → `nms_online_t1` (0x200459c8) (*off* =
  0) or `nms_offset_t1` (0x200444dc) (*off* ≠ 0), else `nms_online_nomax` (0x20046690); *t* = 5 → `nms_online_t5_entry` (0x20046f4a) (*off* = 0) or `nms_offset_t5` (0x200446f2) (*off*
  ≠ 0); other *t* → nothing. All are bracketed by `cache_op(1, 1, 1)` and the IPU start / stop calls (`ipu_start` (0x200317f0),
  `ipu_start_b` (0x200317fe), `ipu_stop` (0x2003180c), `ipu_stop_b` (0x200318be)).

**Engine programme, not CPU arithmetic.** Every variant except `nms_online_nomax` runs on the vector unit and the SR file:
the NMS helper encoders 0x2006_8xxx–0x2006_Axxx (classes 0x02, 0x05, 0x16, 0x1A, 0x1E, 0x20, 0x26, 0x3A, 0x3F, 0x4A, 0x4E,
0x60, 0x7E, 0xA1, 0xAA, 0xAE, 0xBA, 0xBE, 0xCA, 0xCE, §7.8), the SR operations `NMS_IPU_sAlu_add` / `and` / `leftShift` /
`rightShift` and 32-bit immediates (`nms_load_imm_step` = SR opcode 0x48), the loaders `NMS_IPU_vLoad_EMItoL2_S1` … `S3` /
`NMS_IPU_vStore_200`; the CPU only reads SR registers back (`sr_read_value` → `hal_ipu_sr_get_value`) after draining the
queue (`nms_drain` (0x2004495a) → `dec0_drain(0)`), and decides the loop exits (`drv_jump_CheckJumpCondition`, §5.2).

**Offline variants** `nms_offset_t1` (*t* ∈ {1, 2, 3, 4, 7}, `bmax_score` = 1) and `nms_offset_t5` (*t* = 5): input 0
must have rank 7 (`stInList … size` print otherwise). Table form: fixed tensor slots — inputs: coordinates 1, 2, 3, 0 (x1,
y1, x2, y2 order of words 0–3 = 2, 1, 3, 0 as written), score 4 (or 5 when *t* ≠ 4), class 6; outputs 0–3 boxes / classes
/ scores / count, 4 the box-index output when `output_detection_boxes_index_idx` ≠ −1. Then `nms_offline` (0x20044406):
- set-up `nms_setup` (0x200443d8) (VR / MR tables, `nms_vr_setup(0, 0, 0x3E00, 0)` (0x20068b8a), `nms_mr_setup(0, offline)` (0x2006a300));
- **stage 1** `nms_stage1(ceil(N/8))` (0x20043a7a): SR registers 0, 2, 3, 4, 5, 10, 11, 14, 15 loaded (word 0xC into 15); lane masks
  `nms_mask_step(0|1, 0xC00, 0xFF)`; SR 0 = 0x1000, 1 = word 0x19 (score threshold), 2 = argument, 3 = 5, 4 = 0, 5 = −1
  (`nms_load_sr` (0x2005cf06)); per block of 1024 anchors (`(N + 0x3FF) >> 10`): load the seven inputs (or only score / class for
  *t* = 5 / 0) in 0x400-element pieces, then per 0x2000-byte step `nms_move_step` / class 0x26 / `nms_online_kernel_first` (the
  per-candidate score test and compaction [I]: 8 × 0x3F / 0x1E / 0xA1 …) and SR counters advanced by `NMS_IPU_sAlu_add`;
- **stage 2** `nms_stage2(0x200)` (0x20043d44): box geometry — 0x4E / 0x4A / 0x05 / 0x60 (with option bytes +0x60 / +0x5C of the
  record) / 0x20 into registers 0xE, 0xF, 10, then two 0x3A compares on 0x400 lanes (areas and the threshold mask) [I:
  roles from the register flow];
- **stage 3** `nms_stage3(0x200, 0x1000)`: up to `max_detections` (word 0xF) rounds of `nms_select_best` (select the best
  remaining candidate [I]: 0x1E / 0x02 / 0x7E; IoU of every candidate with it [I]: 0x05, 0x4E, 0x20, 0x4A, 0xAA, 0xAE,
  0xA1, 0xCA / 0xCE via the SR shifts and adds; suppression by `nms_iou_threshold` [I]: 0xBA / 0x3F), then the count into
  SR 4 (`NMS_IPU_sAlu_add(0xC, 4, 2000)`) and five `NMS_IPU_vStore_200` stores from L2 offsets 0, 0x640 (1600), 0x708
  (1800), 2000 and 0x960 (2400) to the five outputs (boxes, scores, classes, count, box indices [I: order]).

**Online variants** `nms_online_t0` → `nms_online_t0_body` (0x20044f72) (*t* = 0), `nms_online_t1` → `nms_online_t1_body` (0x2004563e) (*t* ∈ {1, 2, 3, 4,
7}), `nms_online_t5_entry` → `nms_online_t5` (*t* = 5): the same steps per class with smaller pieces (loads of 0x400), the best
candidate read back by CPU each round (`sr_read_value`), and the next round masked with 0x1E / 0x7E.

**CPU pre-filter + YOLOv3** `nms_online_nomax` (*t* ∈ {1, 2, 3, 4, 7} with `bmax_score` ≠ 1; input 0 rank 6): over the *N*
anchors (score / confidence tensor, s16): every anchor whose value ≥ word 0x19 (as s16) is **compacted** — its x1, y1,
x2, y2 (s16) and its row of class scores are copied to the next free position, its value and its original index (to the
table 0x200A_A580, 0xC000 bytes cleared first) recorded — until 0x400 candidates. No candidate → `unsupported` log and
return. The five tensors' shapes are then temporarily set to *k* rows (sizes `k·dim·2`, aligned to 16), and if word 13
(`nms`, f32) ≠ 0 `sgs_NMS_Yolov3_Normal_Ipu(k, num_classes, …)` runs; the shapes are restored afterwards.

**`sgs_NMS_Yolov3_Normal_Ipu`** 0x2004_5FBA (≤ 0x50 classes, record +0x48): `yolov3_geometry` loads boxes and computes
their geometry (0x05 / 0x4E / 0x20 / 0x4A / 0x60 / 0x3A); two scratch tensors are described in place (`nms_scratch_tensors` (0x200434e0):
s16, DDR type 0xC, 0x6000 and 0xC000 bytes at 0x200B_6580 / 0x200C_E580); per class: load that class's scores, lane mask
(0x1E), **0x02** maximum → read SR 8 by CPU (`0x200E_66C0`); score test (0x1A) and threshold (0x16); then rounds of
**0x02** (best) → read SR 6 by CPU (index; −1 = none left) → mask the chosen lane (0x1E, 0x7E) → write the box
(`nms_write_box` (0x20044efe): entry = output base + 16·*n*) → **clip** (`nms_clip_box` (0x20044f06), when word 14 `clip` ≠ 0: x1, y1 raised to
0, x2, y2 lowered to word 0x20 = ⌊1 / NMSOptions field 1⌋) → suppress by IoU [I] (`yolov3_geometry_b`: 0x05 / 0x4E / 0x20 /
0x4A / 0xA1 against `nms_iou_threshold`), until `max_detections` (record +0x3C) or no candidate is left; the per-class
output count limit is `detections_per_class`. The engine side of the private opcodes (IoU product, comparisons, the 0x02
tie order) is not known (§12).

### 10.17 RoiPooling and PSRoiPooling [R unless marked]


**RoiPooling** (`Custom/IPU200M/drv_roipooling_ipu200.c`). Op function `op_roipool`: drains the queue, installs VR 0
= option word +0x04 slots and VR 1 = option +0x08 slots (one-hot slot entries, §4.4; RoiPoolingOptions fields 1 / 2 =
`in_vbslot_count` and the output slot count, part 1 §7.3), runs `SGS_OPS_RoiPooling_IPUImpl` (0x2005b836), restores
the tables. **Strategy** (option +0x10, RoiPoolingStrategy): only **0 = BestPolicy** is implemented; 1 SubOptimal, 2
Medium, 3 Worst assert, other values log `The strategy %d is not support` and assert. RoiPoolingOptions field 3 is not
read here; its role is not known.

BestPolicy: input feature map UINT8 or INT16, DDR type 7 or 0xC (input and output); pooled H, W > 0 (output dims 1, 2);
*C* = input innermost; channels per L2 pass *cp* = (depth(VR 0) / (H·W))·(16 >> (elemsize − 1)), passes ceil(C / cp),
channels rounded up to a multiple of 16 / elemsize. Per pass: one strided DMA load of the whole H × W × *cp* block; per
ROI (input 1: s16, 8 values per ROI, values 1–4 used):
- ROI corners: x1 = (int)((float)roi[1] · *s* + 0.5), y1 from roi[2], x2 from roi[3], y2 from roi[4] (`__fixsfsi`, so
  truncation after adding 0.5), *s* = the **ROI tensor's own quantisation scale** (f32 at `*(rois + 0x44)`) — the
  FlexBuffer `spatial_scale` of part 1 §7.4 is not read here;
- roi_w = max(x2 − x1, 0) + 1, roi_h = max(y2 − y1, 0) + 1; bin_w = (float)roi_w / pooled_W, bin_h = (float)roi_h /
  pooled_H (single precision);
- pooled H, W ≤ 16: one attribute set per ROI (computed once, cached in a 7-word record per ROI with a "done" flag) and
  one instruction; larger: tiles of 16 × 16 output bins, each with its own attribute set and instruction;
- instruction: header **0x8404_005B** (the RoiPooling form of class 0x5B, 9 words, `vp1_vRoiPooling_hal`), w1 22:21 =
  data format (u8 0 / s16 2) \| bit 25, w3 29:14 = *cp* lines, the other words from the attribute record; assert
  `roipoolingAttr->image_width * roipoolingAttr->image_height < 2 << 14` (w4 13:0 = W·H); then a strided DMA store.

**`_SGS_OPS_RoiPooling_RetrieveAttrs`** 0x2005_DFEC (fmap H, W, bin_w, bin_h, x1, y1, tile origin) — the Caffe RoiPool bin
rule [S: Caffe RoiPool] in single precision with an ε = 1e−6 ceiling:
- *aw* = (int)bin_w + (frac(bin_w) > 1e−6), *ah* likewise (the "average" bin width / height);
- for output column *pw*: wstart = clamp((int)(pw·bin_w) + x1, 0, W), wend = clamp((int)((pw+1)·bin_w) + (frac > 1e−6) +
  x1, 0, W) (the same for rows with bin_h, y1, H);
- the first non-empty bin gives the region start `s32RoiRegionStartPos` = hstart·W + wstart (< 16384 asserted) and the
  first / last bin extents; each bin's width is either *aw* (bit clear) or *aw* + 1 (bit set in a 16-bit mask per axis),
  any other width with wend < W is an error (`The s32WEnd %d s32FmapWidth %d`); the bit of the "starts on the previous
  end" relation is a second mask (`s32WStart %d s32WEndPrev %d` otherwise);
- asserts: start / end extents < 256, *aw*, *ah* < 256, (out − 1 − index) ≤ 15, extents ≤ avg + 1. Record (0x18 bytes):
  word 0 nibbles = (W_out − 1 − first w), (W_out − 1 − last w), (H_out − 1 − first h), (H_out − 1 − last h); +2 fmap W;
  +4 fmap H, +6 region start (14 bits); +8 W mask "starts at the previous bin's end", +0xA the same for H; +0xC pooled W,
  +0xD pooled H, +0xE *aw*, +0xF *ah*; +0x10 W mask "wide bin (*aw* + 1)", +0x12 the same for H; +0x14 / +0x15 first /
  last bin width, +0x16 / +0x17 first / last bin height. An all-empty ROI logs `The s32RoiRegionStartPos …` and gets a
  zero record with only the pooled sizes set (the output is then the engine's empty-bin value, not known).

**PSRoiPooling** — `_SGS_PSRoiPooling_IPUImpl_Path2` 0x2004_123C (the op function, 8.6 KB;
`Custom/op_custom_psroi_pooling.c`). Header part (`SGS_PSRoiPooling_IPUExeHeader` log tag): input and output types equal
(`eTensorTypeInEmi`), DDR types 0xC or 7; queue drained; VR 0 = option +0x04 slots, VR 1 = option +0x08 slots (one-hot
entries). Then:
- **option +0x08 = 0**: the output is only filled with the **output zero point** (0xC3, w1 15:0 = zp, w1 22 = element
  format ≠ u8, w3 count = depth(VR 0)) and stored in pieces (`dma_store_utils`, `stOutTensor elementsize_inbits % 8 ==
  0` asserted);
- otherwise by L2 capacity *Lc* = depth(VR 0) against the input's last three dimensions H × W × C: *Lc* ≥ H·W·C → **Path
  1** (whole map resident), H·W ≤ *Lc* < H·W·C → **Path 2** (channel chunks), *Lc* < H·W → **Path 3** (per-bin loads).
  All three use the same bin arithmetic and instructions; they differ in what is loaded when.

Bin arithmetic (single precision; *s* = option +0x00 f32, the `spatial_scale`; ROIs are int32 rows {batch, x1, y1, x2,
y2}, batch index must be 0 — `s32InputFmapIdx == 0`, and only one input map, `s32InputFmapCount == 1`):

```
x1s = x1·s,  y1s = y1·s
roi_w = fmaxf((x2 + 1)·s − x1s, 0.1),  roi_h = fmaxf((y2 + 1)·s − y1s, 0.1)        (fmaxf = 0x2007_0934)
bin_w = roi_w / pooled_W,  bin_h = roi_h / pooled_H
for ph, pw:  hstart = clamp((int)floorf(ph·bin_h + y1s), 0, H),  hend = clamp((int)ceilf((ph+1)·bin_h + y1s), 0, H)
             wstart = clamp((int)floorf(pw·bin_w + x1s), 0, W),  wend = clamp((int)ceilf((pw+1)·bin_w + x1s), 0, W)
```

A non-empty bin (hstart < hend and wstart < wend; height and width < 256 asserted) is DMA-loaded (its H × W window of the
channel chunk, strided) and averaged by one **0x8404_00DB** instruction (§7.5: image = bin width × height,
`image_width * image_height < 2 << 14` asserted, w3 count = channel lines) written to the bin's output line; an **empty
bin** gets the output zero point by a **0xC3** fill instead. The input offset of bin (ph, pw) advances by H·W per pw and
pooled_W·H per ph within the channel chunk — the position-sensitive channel grouping of R-FCN [I: from the offset
arithmetic; S: R-FCN definition]; the averaging and its rounding are the engine's (§12).

### 10.18 PostProcess_Unpack / TFLite_YoloV3_Unpack and PostProcess_Max [R unless marked]


**PostProcess_Unpack** (`_SGS_PostProcessUnpack_LoadTensorDataToL2` 0x2004_B004 is the whole op, 10 KB,
`Custom/op_postprocess_unpack.c`; log tags `_SGS_PostProcessUnpack_IPUImpl`, `_SGS_PostProcessUnpack_ConvertPa…`): most
of the body is the FlexBuffer reader of part 1 §7.4 (keys `x_offset` … `scores_lengh`, `max_score`, component records
{type, offset, length}). The computation is pure data movement: the input (DDR type asserted, `eDDRType ==
E_TF_TENSOR_DDR_TYPE_…`) is the detection head (YOLO head boxes) with the components interleaved along its innermost
dimension; per output component and per group of anchor rows the rows are DMA-loaded into VR 8 and every lane of the
component's [offset, offset + length) range is moved to the output layout with **0x23** (`_SGS_PostProcessUnpack_LoadTensorDataToL2`: register 0 →
1 / 1 → 0, w4 = 8), after a **0xD3** sub-op 1 pass on VR 8 over the loaded lines (`roipool_max_pass` (0x2004ab8a), header 0x81A0_08D3:
VR 8 in place, w1 22:16 = 0x41); outputs are stored per component. No arithmetic on the values; `max_score` selects the
variant that also keeps the maximum score [I: by name].

**PostProcess_Max** `op_postprocess_max` (`Custom/op_postprocess_max.c`): one input (scores, rows × `scores_lengh`), two
outputs (max score, class index), asserted counts 1 / 2. FlexBuffer key `skip` (int, default 0). After an op-3 barrier 0
and VR set-up (`postprocess_vr_setup` (0x200689de)), lanes masked (0x1E 0xC00); per group of 8 rows (ceil(rows / 8)): load the group
(`NMS_IPU_vLoad_EMItoL2_S1`), **0xD3** over align8(classes) lanes, if `skip` = 1 a **0xC3** fill of lane 0 (the
background class is excluded) [R: condition; I: role], **0x02** maximum with index, two **0x23** moving the maximum to
register 1 and the index to register 0; at the end two `NMS_IPU_vStore_200` stores (index, maximum). Ties resolve as the
engine's 0x02 resolves them (§12).

### 10.19 DmaCoefMatrix, Fix2Float, Align, Clip, SGS_LSTM [R unless marked]


**DmaCoefMatrix** (op entry 0x2003_CA6A → `j` → `SGS_OPS_DmaCoefMatrix_IPUImpl` 0x2005_CA34,
`drv_dmacoefmatrix_ipu200.c`; the alias entry, §10.13): input DDR type 1–5 or 0x11 (image formats), output DDR type 3 or
4, both 4-D with N = 1; VR 0 gets all 32 slots. The **colour matrix** is programmed by `dma_color_matrix(option, b0, b1,
b2)` (the same call `conv_exec_layer_callback_helper` makes for a first conv layer whose option +0x20 is set, §6.7): op-3
barrier 0, then `hal_ipu_dma_set_coefbias2buf` packs the option's 12 × i16 (`as16ConvertMartix`, the TensorMatrix of
part 1 §6.3) into six DMA registers — coefficients *c*0 … *c*8 as **13-bit sign-magnitude** (bit 12 = sign, 11:0 =
|*c*|; a coefficient ≤ 0 is written as −*c* \| 0x1000, so 0 becomes 0x1000) at +0x40 [12:0] *c*0, +0x40 [28:16] *c*1,
+0x44 [12:0] *c*2, +0x48 [12:0] *c*3, +0x48 [28:16] *c*4, +0x4C [12:0] *c*5, +0x50 [12:0] *c*6, +0x50 [28:16] *c*7, +0x54
[12:0] *c*8, and the three biases (i16 9, 10, 11 = option +0x12 / +0x14 / +0x16) as **9-bit** fields at +0x44 / +0x4C /
+0x54 [24:16] — and the 24 bytes are sent as **op 1 to decoder address 0xC040, 24 bytes** (header 0x1017_C040). The image
is then moved in row chunks (rows per chunk = depth(VR 0)·4 / W pixels, ≤ H) through DMA queues 3 (load, which applies
the matrix) and 8 (store). Anjoy's matrix (0, 0, 1024, 0, 1024, 0, 1024, 0, 0, 256, 256, 256, part 1 §6.3) is thus a B ↔
R swap in Q10 with bias 256 in each row [I: Q10 and the bias role]. The matrix is applied by the DMA engine, not by the
firmware.

**Fix2Float** — `_SGS_Fixed2Float_CModelFixedImpl` 0x2003_CD34 (`Custom/IPU200M/op_custom_fixed2float.c`). Despite the
name this is not a CPU model: it streams the tensor through the vector unit. `op_fix2float` (op function, log tag
`_SGS_Fixed2Float_InitL2Buffer`) installs customer VR tables for VR 0 (one slot) and VR 1 (two slots)
(`set_customer_vr_tbl2`), runs the implementation and restores the defaults (`vrbuf_reset_all_b`). The input must be INT16
(assert line 0x15F); its **f32 scale** (`*(tensor + 0x44)`) is the only parameter. Per pass: DMA-load *k* lines of s16
(8 elements per 16-byte line) into VR 0 (`fix2float_load` (0x2003cc0c) → `hal_dma_load`), one **0xF8** instruction (§8.4: w1 = *k* \|
0x0207 << 16, w4 = the f32 scale's bit pattern), DMA-store 2*k* lines (8 f32 = 32 bytes per input line, `fix2float_store` (0x2003cb06) →
`hal_dma_store`). The whole is wrapped in a CMDQ barrier pair (`0x7004_0201 | q·0x2000 | q·16 | 0x100` before,
`0x7044_0201 | …` after) with *q* = `conv_ob_buffer_id` = 0xF, the conv OB buffer id of §4.2 [R: the function returns the constant 0xF; the
`ipu_vrbuf.c` asserts call it `get_conv_ob_bufid()`]. The DMA queue of type 6 is a separate value, `hal_dma_getqid(6)`
(0x2000_9FBE), passed to every load and store of the pass [R].
Pass planning (*C* = innermost dimension, *C8* = *C* rounded up to 8, *R* = product of the outer dimensions,
*E* = element count):

| case | passes |
|---|---|
| *C* = *C8* (no row padding) | ceil(*E* / 4096) passes of up to 4096 elements (0x200 lines), input offset +0x2000 B, output +0x4000 B per pass |
| *C* < *C8*, *C* ≤ 4096 | rows in groups of ⌊4096 / *C*⌋: each pass loads *g* rows of *C* elements with the source row pitch (row stride − *C*·element size skipped), ceil(*g*·*C*/8) lines, stores them densely (4·*C* bytes per row) |
| *C* < *C8*, *C* > 4096 | per row: ceil(*C* / 4096) passes, 4096 elements each, the last the remainder; the next row starts after the padding (`(C8 − C)·2` bytes) |

**Align** (op entry 0x2003_3FA4 → `_SGS_Align` 0x2003_3E9A → `SGS_OPS_Align` 0x2005_4320, `DMA/op_align.c`,
`sdk_align.c`): equal dimension counts; `SGS_DMA_OpsStart(0xE, 1)`; size-1 inner dimensions removed from both shapes,
strides recomputed with `SGS_OPS_CalculateStrideWithAlignment` (innermost pitch aligned; output alignment 16 checked),
`SGS_OPS_AlignTensor`, and a strided DMA copy — data movement only. **Align2 / Align2_Alias** (0x2003_3E96) are `return
0`: pure relabelling.

**Clip** `SGS_Clip_IPUExeHeader` 0x2003_92F8 (`Custom/op_custom_clip.c`): input, lower and upper bound all INT16, the
bounds one element each (asserts 0x162–0x167); VR 0 / VR 1 one-hot tables of 16 slots each, MR 0 16 entries; chunks of
depth(VR 0)·(16 / elemsize) elements; per chunk: DMA load, an SR 128-bit immediate (`drv_ipu_sr_smove128_immed`), one
**0x10** with w1 15:0 = lines rounded **up to a multiple of 512**, operation 7, no shift, w5 = 1, **w4 = upper (15:0) \|
lower (31:16)** → clamp(*x*, lower, upper) [R: fields; I: that the engine clamps with w4 as for the LUT], DMA store.

**SGS_LSTM** `_SGS_Lstm_Domain1HeaderExecuteLayerFunc` 0x2004_06EC (`Custom/op_custom_lstm.c`): runs the LSTM
**sub-network** named by the option (FlexBuffer keys `C0` and `SubNet`, part 1 §7.4) once per time step: the sub-graph
must have 4 inputs (`u32SubGraphInSize == E_SGS_LSTM…`) and 1 output; the cell state tensor must exist (`pstCn !=
NULL`); the time-step input slices are typed UINT8 or INT16 with DDR type 0xC (else `UnSupport type`); per step the in /
out buffers of the sub-graph are swapped (`SGS_TF_InterpreterFlipInOutBuffer(…, 4, …, 1)`) and its operators executed
(only those with run mode 1, node +0x280). The arithmetic is that of the sub-network's ordinary ops [R: structure].

### 10.20 Radar-phase customs (`Custom/op_custom_*.c`) [R unless marked]
All INT16 in / out, chunked through L2 with customer VR / MR tables and restored afterwards; sequences below give the
instruction classes in push order (immediates and register numbers left out); the operation names of the private classes are [I] as in §7.8. The
`JointMicro*` / `Phase*` names are the vendor's; "radar-phase" is a grouping by those names [I].

| op (header fn) | inputs | engine sequence per chunk |
|---|---|---|
| PhaseGen `SGS_PhaGen_IPUExeHeader` 0x2003_6E96 | input 1, input 2, temp, output | 0xED 0x01 0xDA 0xD6 0x05 0x01 0x09 0xC3 0x5E 0x3E 0xD6 0x3E 0x6D 0xC1 0xD6 0x7E 0xC1 0x6D 0xD6 0x7E 0xC1 0xD6 0x3E 0xC1 (fixed list, `_SGS_PhaseGen_InitL2Buffer` / `RestoreL2Buffer`) |
| Atan2 `SGS_Atan2IPUExeHeader` 0x2003_81E4 | X, Y | 0xED×2 0x12 0x09×2 0xD2, **LUT** (VR 2 → 2, clamp ±32767), 0x78 0x6D 0xC3×2 0x01 0xDA 0x1E 0xDA 0xC3 0x1E 0xD2 0xC3 0xD6×2 0xC3×2 0x01 — octant reduction by sign / magnitude compares, LUT arctangent, octant fix-up [I] |
| PhaseModify `SGS_JointMicroPhaseModifyIPUExeHeader` 0x2003_FE40 | input, input 1 | 0xD3, then per chunk 0x66 0xB2 0x66 0xA1 0x66 0xB6 0x66 0xA1 (compare with ±limits and add a correction — phase unwrap [I]) |
| WiggleErr `SGS_WiggleErrIPUExeHeader` 0x2006_75D2 | input, input 1 | per block of 5 LUTs / 3 converts (option 0xF90 bytes: 5 LUT blocks, 6 + 3 `OpParameterConvert`): 0x60 / 0x6D, 0x41 0x52 0x60 0x6D, 0x60, 0x20 and the LUT pairs of §10.7 |
| CondLess `SGS_JointMicroCondLessIPUExeHeader` 0x2003_F458 | in0 … in3, out (same aligned size and type for in0 / in2) | in2, in3 loaded once; per chunk: load in0, **0xB6** (in0 ≥ scalar, predicate), load in1, **0xA3** with bit 24 writing the scalar on the predicate lanes, store [R: order; I: which inputs are the scalars] |
| CondGreat `SGS_CondGreatIPUExeHeader` 0x2003_9A42 | as CondLess | the same with **0xB2** (>) |
| CustomNotEqual `SGS_CustomNotEqual_IPUExeHeader` 0x2004_0B4A | in0, in1 (1 element), in2 | per chunk one **0x3F** (compare-and-select against the scalar, w1 22:16 = sub-op) |
| CustomFloorMod `SGS_CustomFloorMod_IPUExeHeader` 0x2003_D20E | input 1, divisor (option: 5 `OpParameterConvert`) | 0xD2 0xC5 0xD6 0xC1 0xE0×2 0x20 0xD2 0xDA 0x3E 0x28 0x13 (`_SGS_CustomFloorMod_CalFloorMod`: quotient by multiply-and-shift with the converter's reciprocal, floor correction on the sign, remainder = x − q·d, buffers "Quotient" / "Remainder" / "maskbuf1") [I: roles from the buffer names and classes] |
| FormatConvert `SGS_FormatConvertIPUExeHeader` 0x2003_E224 | input, output | nested loops of 0x47 0x1E 0xE3 0x53 0x47 0x1E 0xE3 (lane gather / scatter between layouts) |
| FourBytesFlip `SGS_FourBytesFlipIPUExeHeader` 0x2003_EC00 | input DDR `IPU_U8_NHWX` or `IPU_S16_NHW…` | 0x47 0x53×2 0x1E 0x13 (byte order reversal within 4-byte groups by lane select [I]) |

Source files: `op_custom_formatconvert.c`, `op_custom_fourbytesflip.c` and the Custom files listed in part 1 §2.4.

## 11. The Anjoy build [R unless marked]
<!-- ev:ipu-firmware-2-engines.11 -->

**Function-level diff** [R: mnemonic-sequence diff, + a difflib pass on the diverging
window, output; byte compare is useless here because every `auipc`
immediate shifts when code moves]. Both code regions disassembled to mnemonic sequences (Anjoy 170 300 instructions,
this build 169 761) and aligned. The whole low half of the image — start-up, FreeRTOS, CCIF, host commands, model /
flatbuffer parsing up to ≈0x2000_C000 — matches instruction for instruction at the **same relative addresses** (the two
builds are the same source). Divergence is confined to three clusters, and even across the diverging window
(0x2000_C000–0x2002_A000) the structural match is **0.968**:
- **vrbuf / L2 allocator** (≈0x2000_C9A0–0x2000_D1E0, `ipu_vrbuf.c`): this build has a ≈137-instruction function that
  Anjoy lacks (and Anjoy a ≈128-instruction one this build lacks) — the ping-pong buffer-id allocator was reworked;
- **tensor parser** (≈0x2001_3980–0x2001_4E40, `parseTensorVector` / `model_parser.c`): a cluster of small insert /
  replace edits — this build reads the newer Tensor fields (11–16, part 1 §6.4) that the `Q_0.9` build did not;
- **element-wise fusion** (≈0x2002_86A0–0x2002_8F70, `sgs_firmware.c`): this build adds the ping-pong helpers
  (`_sgs_ElementwiseOp_getOutputBufLen`, `_sgs_ElementwiseOp_getVecAddrByPingPongType`).

Everything else (the ≈38 k-instruction span the anchor alignment could not line up) is repetitive helper code that is in
fact ≈identical.

**String-level diff** (`strings -n 6`, unique lines) of the Anjoy MC-H12S firmware (`config/dla/ipu_firmware.bin`)
(602 280 B, `Q_0.9_verified_21091510`; same layout, entry sequence, data-region start and protocol hash, part 1 §2.6):
this build adds `MedianFilterOptions` (schema), `_SGS_Fixed2Float_InitL2Buffer`, the element-wise ping-pong asserts
(`eHeaderVrType/eInput1VrType/eOutputVrType==EN_IPU_ISA_PING_BUFFER`, `_sgs_ElementwiseOp_getOutputBufLen`,
`_sgs_ElementwiseOp_getVecAddrByPingPongType`, `s32OutputVecLen …`, `u32Vecaddr==…`) and `s32InputSlotCount <=
CONV_SLOT_TOTAL_NUM`; Anjoy has `pstOpt->s32PartitionInput0SlotCounts <= CONV_SLOT_TOTAL_NUM` and
`stOpExeContext.exec_tile_info.this_tileinfo.tileBufInfo.buf_id == pstVrTmpIn->buf_id` instead.

So this build (`Q_0.1.0_verified_21110517`) is a **later build of the same firmware** as Anjoy, differing only in the
vrbuf allocator, a few newer model-tensor fields, the element-wise fusion path and one conv check; the host protocol,
model format, op set and HAL are the same [I: from the two diffs], consistent with the same `579e2d5f…` protocol hash and
with the camera run of a `Q_0.9` model on this `Q_0.1.0` firmware (`ipu-host.md` §6).

## 12. What the firmware does not show: the engines' own definitions

The firmware gives every engine instruction's bit positions and the values it puts there (§4–§10). What the hardware
does with them is not in the firmware — no C model of the engines exists in it (the `CModel` names mark IPU
implementations, §10.19) — and no hardware reference is on hand. Not known, therefore:

- **predicate polarity** of w1 bit 24 — reading (A) or (A′) of §7.4 — and with it which relation 0x12 / 0x16 / 0x1A,
  0xD2 / 0xDA test, the lanes CAST to BOOL and LOGICAL_AND write (§10.4, §10.5);
- the **requantising operations** 7 / 3 / 4 (§7.3, §7.6): order of multiply and add, which of the two shifts applies
  where, right-shift rounding, saturation; hence also ROUND's rounding (§10.5), the RELU / Clip clamps (§10.6, §10.19)
  and the conv output stage's rounding (§6.4);
- the **LUT interpolation** from `table_first` / `table_second`, the `intp*` slopes and the priorities (§8);
- the **0xAC resize** interpolation and its nearest-mode pixel selection (§10.11), the **0xDB / 0x5B pooling** rounding,
  the RoiPooling empty-bin value (§10.17);
- the **0x02 lane-maximum tie order** (TOPK, ARG_MAX, NMS, PostProcess_Max);
- **SR opcode 0x65** (division assumed): the rounding of 2^30 / sum (§5.3, §10.8); SR opcode 0x09; the sign of the SR
  right shift; whether the SR 8 / 16 / 32-bit views alias the 128-bit register (§5.1);
- the **0xF8 / 0x00 / 0xD8** operations (§8.4, §7.8, §10.6) and the private opcodes of PhaseGen, Atan2, PhaseModify,
  WiggleErr, FloorMod, FormatConvert, FourBytesFlip, NMS and TopK (classes 0x01, 0x05, 0x09, 0x21, 0x26, 0x2A, 0x2E, 0x3E,
  0x3F, 0x41, 0x47, 0x4A, 0x4E, 0x52, 0x56, 0x5A, 0x5E, 0x66, 0x6D, 0x7E, 0xAA, 0xAE, 0xC1, 0xC5, 0xCA, 0xCE, 0xD6, 0xE3,
  0xED as far as no use names them), the meaning of w4 13:0 codes other than the operand sources, of w1 bits 23 / 31, w2
  bits 27–28 and header bits 29:27 / 30 (§7.1–§7.2);
- the **conv engine**: the names of conv modes 0, 1, 3, 5 (§6.2); what it does with the fifth (32-bit) per-channel
  vector of header bit 30 (§6.3); the meaning of Conv2D field 38 (§6.4);
- the **pooling form**: the four paddings' order and w1 bits 22:21 (§7.5);
- the registers `ipu_global_ctrl` +0x48 (§5.2) and decoder address 0x1908 (§6.5) — their uses are known, their names
  are not.

Unnamed model values (no reader, or no reader that names them): the DataConvertParameters "merge" slot (§10.2), the
NMSOptions fields 13 / 14 (§10.16), RoiPoolingOptions field 3 (§10.17).
