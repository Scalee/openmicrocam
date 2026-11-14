# OpenMicroCam — agent rules

Open firmware for the YIZHAN 4K HDMI/USB microscope camera (SigmaStar SSC8836 / Mercury6, GC8613 sensor). `microcamd` is the
daemon that replaces the vendor's app; it runs on the vendor's kernel modules and libraries until ours replace them.

## Docs
- Every claim in the docs carries its origin: [R] read or measured, [S] stated by a source, [I] inference. An inference is
  never upgraded by being repeated.
- Anything stale noticed in the docs is corrected at once; wrong data is deleted, not marked; the docs stay compact:
  before a doc is handed over, every sentence, the inherited ones too, is held to what its reader needs from it, and one
  the reader would not miss goes.
- A doc reads as one text: the same names and words in every part, none of our working words; a new part is written
  into the whole doc, not beside it.
- Whatever a change makes obsolete — testers, scripts, code, notes — is deleted in the same step.

## Schemes
A scheme is a D2 file in `docs/artifacts/` starting with `...@_style`; read `docs/artifacts/_style.d2` and
`tools/scheme/README.md` before drawing. A scheme's texts are written in every language its `languages` line lists: a text
added or changed is written in each. A change to microcamd's blocks, ports or data paths updates `docs/artifacts/hardware-map.d2` in
the same step. `tools/scheme/scheme.py SRC.d2 [--lang ru] -o OUT.html` renders one. `docs/artifacts/hardware-map.svg` is the map as GitHub shows it:
`scheme.py docs/artifacts/hardware-map.d2 --svg -o docs/artifacts/hardware-map.svg` in the commit that changes the map.

## Guides
A guide (install, tools, bench) is written for a builder: someone who took this repository to build it and run it on their
own camera, on their own computer.
- Its sections are what the builder does, in the order it is done; each opens with the reason in one sentence.
- Every action is a command block to copy; what it does is said in the text before it.
- A sentence stays when the command beside it cannot be chosen or understood without it. How a thing works inside lives
  in the architecture page and the specification.
- A step that needs explaining is the tool's defect: the tool takes the step over, or says it in its own message, and the
  guide loses the step.
- A passage a reader misread is written again whole.
- A claim covers what was tried: the tested camera, the tested system.
- The guide reads the same for any builder: it holds nothing of our own setup.
