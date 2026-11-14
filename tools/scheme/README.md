# Schemes: D2 sources shown as pages

The project's schemes — the hardware map of the YIZHAN 4K camera (SigmaStar SSC8836 / Mercury6) — are D2 files in `docs/artifacts/` (the language: d2lang.com/tour; the project's styles and conventions:
`docs/artifacts/_style.d2`, imported by every scheme with `...@_style`). `scheme.py` turns one into a single HTML page
that opens in any browser.

    tools/scheme/scheme.py docs/artifacts/hardware-map.d2 [--lang ru] -o hardware-map.html

The page, or D2's error with exit 1.

    tools/scheme/scheme.py docs/artifacts/hardware-map.d2 --svg -o docs/artifacts/hardware-map.svg

The first board as a picture, for a place that shows no page (GitHub): taken from the page in headless Chrome, so a
scheme whose blocks are placed keeps the page's arrows; any other is as D2 draws it.

## Languages

A scheme is written in several languages in one source: the structure — ids, classes, places, arrows — once, every text
once per language. A leading comment names the languages, `# languages: en ru`; each text then holds one part per
language in that order, parted by `‖` (U+2016): a label, a tooltip, a `vars` string, the title line and the `# tab:`
line. In a block string (`|md … |`) the `‖` stands on a line of its own. A part left out is the first language's. A text
that reads the same in every language (`HDMI-TX`, `3 · HW3`) is written once.

`--lang` picks the language; without it, or for a language the scheme does not list, the first. Adding a language is
adding a part to each text and its code to the `languages` line. The page's own words (button titles) come in English
and Russian (`UI` in `scheme.py`); another language gets English until it is added there. A scheme with no `languages`
line is drawn as written.

## What the page adds to D2

- the first line `# Title` is the page title; a `# tab: …` line among the leading comments names the root board's tab
- every board is a tab: the root first, then the top-level `scenarios` and `layers` in source order
- `vars: {intro: |md … |}` of a board is its text in the panel under the picture (a scenario without one keeps the
  root's); a board with no blocks shows its intro as a page of text
- an arrow's tooltip lives in the board's `vars: {arrows: {"a -> b": |md … |}}`, keyed by the full ids of its ends
  (D2 draws no arrow tooltip, and TALA refuses one on an arrow without a label); a scenario adds to the root's. The
  page shows it like a block's; a wide invisible stroke along the arrow takes the pointer
- the pointer over a block or an arrow with a `tooltip` shows it in the panel, a click keeps it there; markdown-lite:
  `#`/`###` headings, `- ` items, `**bold**`, `` `code` ``, `[R]`/`[S]`/`[I]` origin tags; D2's own tooltip icons are
  hidden, and a tooltip is shown only in the panel (taken out of the SVG, so the browser shows nothing by the pointer)
- every plain block (a rectangle that holds no other) is drawn tight round its label: D2 leaves 20 px of room on each
  side, so the page draws each board twice, the second time with each such block's `width`/`height` set to its label
  plus a narrow margin; a scheme that sets any `width`/`height` itself sizes all its blocks itself. elk's spacing is
  tightened too (the `--elk-*` flags in `scheme.py`)
- small buttons over the picture's top right corner: the live layout on/off (kept in the browser; off by default, which
  is D2's own picture), the live layout's direction ⇢ across / ⇣ down (kept; across by default), and 1:1 (the zoom back)
- the live layout (`live.js`): every scheme whose blocks the author did not place (no `top`/`left`, no
  grid) is laid out in the page itself, on a copy of D2's picture, again on every change of the panel's size or the
  zoom, gliding from the layout before. Bottom up: a zone packs its blocks in chains along the arrows written between
  them, in columns as high as the room allows, each chain next to the blocks outside the zone its arrows come from; a
  zone is then one block for the level above; the top level lines its units up in the order of the arrows — across: at
  the least height that still fits the width, wrapping into rows like text; down: stacked, each zone's chains in rows as
  wide as the panel. libavoid routes the arrows round the blocks (a pin in the middle of each side, any number of
  arrows on one); an edge's label goes to the first free spot along its route. If anything fails, D2's own picture stays
- the arrows of a scheme whose blocks are placed by hand (`top`/`left`) are routed by the page (`edges.js`, libavoid):
  between the borders of the top-level blocks, each on a lane of its own; an end inside a block gets a pin on that
  block's border facing the other end and a thin lead inside to its port, under the other ports; an arrow between two
  blocks of one top-level block stays inside it. A pin faces the nearest point of the block at the other end; arrows
  from one place into several pins of one block run as one trunk and fork at its border; labels of arrows between
  blocks go only into the room between them. Lanes libavoid packs side by side are spread 18 px apart afterwards, each
  within its own room (up to the nearest block across its span), a fork's stretches as one lane. The pointer over an
  arrow lights its whole way and dims the rest
- `vars: {links: {"a": "b; c; d -> e"}}` ties an element to what it goes with (the hardware map: a memory block
  and the ports, blocks and arrows its buffers pass): the pointer over either side lights the other
- the pointer over an element lights its chain — itself, its arrows (a container's: those of the blocks inside it
  too), the blocks at their other ends, what `vars.links` ties it to — and darkens the rest. A click pins an element's
  chain or lets it go, a double click picks an element (a magenta outline) or drops it, any number of each; the panel
  heads with what is pinned (📌) and picked (◆) and ✕ for all of it (Esc too)
- zooming moves the picture drawn last by a transform and draws it again sharp once it rests (big pictures)
- a picture keeps its own size and scrolls when larger — one only too wide sideways under the plain wheel; Ctrl + wheel
  zooms the picture, not the page (a live one is laid out again for the zoom), Ctrl+0 resets
- shown in a frame whose element carries `data-fit`, zoom 1 is the picture fitted to the frame, the page's title is
  hidden, and the plain wheel zooms

## Extending the page

A host page that embeds schemes can add to them without changing this tool: `render_file(path, lang, ext)` takes an
extension, any object (a module will do) with some of these attributes.

| attribute | what it is |
|---|---|
| `boards(info)` | called once the boards are drawn, with a dict: `path`, `text` (the source as written), `lang`, `targets` (each board's tab name, D2 target, intro, arrow tooltips, links), `svgs`, `placed`. It may change `info['svgs']` and returns HTML of boards of its own (`<div class="board …">`, hidden until its script shows one); the page's tabs stay its own boards (`data-board`) |
| `css` | CSS added after the page's |
| `tools` | HTML of buttons added to the tool row over the picture, after 1:1 |
| `js` | a script run inside the page's own, before the page starts; it works through `window.scheme` |

`window.scheme` holds the page's state and functions: `stage`, `boards`, `keys` (each SVG group → its D2 id, an arrow
as `a -> b`), `norm`, `chainOf`, `linkedTo`, `ways` (an arrow's leads and pins), `pins`, `chosen`, `last`, `say` (an
element's id and name), `panel`, `relight`, `fit`, `size`, `baseOf`, `hidden`, `svgOn`, `unit`, `look`, `sharp`, `zoom()`,
`FIT`, and `head` — a list of functions whose HTML is added to the panel's row of pins and picks. `scheme.on(event, f)`
hears `zoom` (the user zooms or resets the zoom), `tab` (a tab is chosen), `change` (pins or picks change, and once at
the start), `panel` (the panel is drawn again); `scheme.emit('change')` tells the others of a change the extension made.

## Installing

D2 is one binary: v0.9.0 from github.com/terrastruct/d2 (the release's `d2-v0.9.0-linux-amd64.tar.gz`, its `bin/d2`),
on `PATH` or in `~/.local/bin`. Its layout engines elk, dagre and tala are built in.

libavoid-js 0.5.0-beta.5 (LGPL-2.1; `npm pack libavoid-js@0.5.0-beta.5`, the `dist/index.js` and `dist/libavoid.wasm`
inside) goes into `~/.local/lib/libavoid-js/0.5.0-beta.5/`, or the folder `LIBAVOID_JS` names; each page carries its own
copy of both. Without it the page shows D2's layout and D2's own arrows.
