#!/usr/bin/env python3
"""A D2 scheme (d2lang.com) as one HTML page: every board a tab, the pointer over a block or an arrow with a tooltip
shows it in the panel under the picture, a click keeps it there. What the page adds to D2: tools/scheme/README.md.

usage: scheme.py SRC.d2 [--lang LANG] [--svg] [-o OUT]
       the HTML to stdout (or OUT); D2's errors to stderr and exit 1. A scheme in several languages (a `# languages:`
       line) is drawn in LANG, by default in the first it lists. --svg: the first board as a picture instead, taken
       from the page in headless Chrome (a placed scheme with the page's arrows, any other as D2 draws it).
"""
import base64, html, json, os, re, shutil, subprocess, sys, textwrap

E = lambda s: html.escape(str(s), quote=True)
D2 = shutil.which('d2') or os.path.expanduser('~/.local/bin/d2')
# libavoid-js routes the arrows of the live layout and of a placed scheme (README «Installing»)
LIBAVOID = os.environ.get('LIBAVOID_JS') or os.path.expanduser('~/.local/lib/libavoid-js/0.5.0-beta.5')
LIVE = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'live.js')
EDGES = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'edges.js')   # arrows of a placed scheme

# the page's own words, per language; a language not here gets English
UI = {
    'en': {'live': 'live layout: fits the panel and the zoom (off: as D2 draws it)', 'dirX': 'laid out left to right (click: top to bottom)',
           'dirY': 'laid out top to bottom (click: left to right)', 'zoom': 'zoom 1:1 (Ctrl+0)', 'unpin': 'drop all',
           'tab': 'scheme', 'err': 'Scheme «{}» is not drawn'},
    'ru': {'live': 'живая раскладка: по размеру панели и зуму (выключено — как рисует D2)',
           'dirX': 'строить слева направо (нажать — сверху вниз)', 'dirY': 'строить сверху вниз (нажать — слева направо)',
           'zoom': 'масштаб 1:1 (Ctrl+0)', 'unpin': 'снять всё', 'tab': 'схема', 'err': 'Схема «{}» не нарисована'},
}
# the picture's switch of the live layout: an icon of blocks being rearranged
LIVE_BTN = ('<button id="t-live" hidden title="{}">'
            '<svg width="16" height="14" viewBox="0 0 16 14" fill="none" stroke="currentColor" stroke-width="1.6">'
            '<rect x="1" y="1" width="5" height="4" rx="1"/><rect x="10" y="1" width="5" height="4" rx="1"/>'
            '<rect x="1" y="9" width="5" height="4" rx="1"/><path d="M8 11h4.5m0 0-1.6-1.6m1.6 1.6-1.6 1.6"/></svg></button>')

# A scheme in several languages names them in a leading comment, `# languages: en ru`; every text in it — a label, a
# tooltip, a `vars` string, the title and tab lines — is written once per language in that order, the parts parted by
# SEP. A part left out is the first language's.
SEP = '\u2016'


def languages(text):
    """The languages a scheme lists in its leading comments, [] for a scheme in one."""
    for line in text.splitlines():
        if not line.startswith('#'):
            break
        m = re.match(r'#\s*languages:\s*(.+)', line)
        if m:
            return m.group(1).split()
    return []


def localized(text, lang=None):
    """The source with each text in `lang` (or the first language listed): what D2 and the page are given."""
    langs = languages(text)
    if not langs:
        return text
    k = langs.index(lang) if lang in langs else 0
    pick = lambda v: (lambda ps: (ps[k] if k < len(ps) and ps[k].strip() else ps[0]))(v.split(SEP))
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c == '#' and (i == 0 or text[i - 1] in ' \t\n;{'):   # a comment: the title and tab lines are texts too
            j = text.find('\n', i)
            j = n if j < 0 else j
            line = text[i:j]
            if SEP in line:
                m = re.match(r'(#\s*(?:tab:\s*)?)(.*)', line)
                line = m.group(1) + pick(m.group(2)).strip()
            out.append(line)
            i = j
            continue
        if c in '"\'':
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == '\\' else 1
            body = text[i + 1:j]
            out.append(c + (pick(body).strip() if SEP in body else body) + c)
            i = j + 1
            continue
        if c == '|':   # a block string: |md … |, |||md … |||
            bar = re.match(r'\|+', text[i:]).group(0)
            tag = re.match(r'[a-z]*', text[i + len(bar):]).group(0)
            start = i + len(bar) + len(tag)
            end = text.find(bar, start)
            end = n if end < 0 else end
            body = text[start:end]
            if SEP in body:
                lines = pick(body).strip('\n').split('\n')
                body = '\n' + textwrap.dedent('\n'.join(lines)).strip() + '\n'
            out.append(bar + tag + body + bar)
            i = end + len(bar)
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def head(text):
    """The page title (first line «# …») and the root board's tab name («# tab: …» among the leading comments)."""
    title, tab = '', ''
    for line in text.splitlines():
        if not line.startswith('#'):
            break
        m = re.match(r'#\s*tab:\s*(.+)', line)
        if m:
            tab = m.group(1).strip()
        elif not title:
            title = line.lstrip('#').strip()
    return title, tab


def boards(text):
    """The boards in source order, the root first: [(tab name or None for the root, --target, intro, arrows, links)]; a
    board's intro is its `vars.intro` (a scenario without one keeps the root's), its arrows' tooltips `vars.arrows`
    (a scenario adds to the root's), and its `vars.links`: an element and what it goes with, `a; b; c -> d`."""
    out, stack, i, n = [('', '', '')], [], 0, len(text)
    arrows, links = {'': {}}, {'': {}}
    cur, last = '', ''
    def value(v):   # a string value read at the current path
        path = stack + [last]
        if path[-3:-1] in (['vars', 'arrows'], ['vars', 'links']):
            where = path[:-3]
            store = arrows if path[-2] == 'arrows' else links
            store.setdefault(f'{where[0]}."{where[1]}"' if len(where) == 2 else '', {})[path[-1]] = v
        elif path[-2:] == ['vars', 'intro']:
            where = path[:-2]
            if not where:
                out[0] = out[0][:2] + (v,)
            elif len(where) == 2 and where[0] in ('scenarios', 'layers'):
                for k, (name, t, _) in enumerate(out):
                    if name == where[1]:
                        out[k] = (name, t, v)
    while i < n:
        c = text[i]
        if c == '#' and (i == 0 or text[i - 1] in ' \t\n;{'):
            i = text.find('\n', i)
            i, cur, last = (n if i < 0 else i), '', ''
            continue
        if c in '"\'':
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == '\\' else 1
            if last:
                value(text[i + 1:j].replace('\\n', '\n'))
            else:
                cur += text[i + 1:j]
            i = j + 1
            continue
        if c == '|':   # a block string: |md … |, |||md … |||
            bar = re.match(r'\|+', text[i:]).group(0)
            start = i + len(bar)
            end = text.find(bar, start)
            body = text[start:n if end < 0 else end]
            if last:
                value(textwrap.dedent(re.sub(r'^[a-z]*\n', '', body)).strip())
            i, cur, last = (n if end < 0 else end + len(bar)), '', ''
            continue
        if c == ':':
            last, cur = cur.strip(), ''
        elif c == '{':
            k = last or cur.strip()
            if len(stack) == 1 and stack[0] in ('scenarios', 'layers') and k:
                out.append((k, f'{stack[0]}."{k}"', None))
            stack.append(k)
            cur = last = ''
        elif c == '}':
            stack and stack.pop()
            cur = last = ''
        elif c in ';\n':
            cur = last = ''
        else:
            cur += c
        i += 1
    root = out[0][2]
    inherit = lambda d, t: {**d[''], **d.get(t, {})} if t.startswith('scenarios') else d.get(t, {})
    return [(name, t, root if intro is None and t.startswith('scenarios') else intro or '', inherit(arrows, t),
             inherit(links, t)) for name, t, intro in out]


ARROW = re.compile(r'^(?:(.*)\.)?\((.+) (<->|->|<-|--) (.+)\)\[\d+\]$')
ENDS = re.compile(r'^(.+?)\s*(<->|->|<-|--)\s*(.+)$')


def arrow_key(src, op, dst):
    """An arrow as `a -> b` with full ids at its ends (`<-` turned round), as `vars.arrows` and the page's marks name it."""
    return f'{dst} -> {src}' if op == '<-' else f'{src} {op} {dst}'


def titled(svg, arrows):
    """The SVG with each arrow's tooltip (`vars.arrows`) as its group's <title>, the way D2 marks a block's: D2 refuses a
    tooltip on some arrows, so a scheme keeps them there, keyed `a -> b` by the full ids of the ends."""
    tips = {}
    for k, v in arrows.items():
        m = ENDS.match(k.strip())
        if m:
            tips[arrow_key(m.group(1), m.group(2), m.group(3))] = v

    def add(m):
        try:
            a = ARROW.match(html.unescape(base64.b64decode(m.group(1)).decode()))
        except ValueError:
            a = None
        at = (lambda x: f'{a.group(1)}.{x}' if a.group(1) else x) if a else None
        tip = a and tips.get(arrow_key(at(a.group(2)), a.group(3), at(a.group(4))))
        return m.group(0) + (f'<title>{E(tip)}</title>' if tip else '')
    return re.sub(r'<g class="([A-Za-z0-9+/=]+)(?: [^"]*)?">', add, svg)   # an arrow's class follows its name


def d2(path, target, salt, text=None):
    """One board as SVG; `text` (when given) is drawn in place of the file, its imports still beside the file."""
    # elk's own spacing is loose for the artifact panel; tighter here (the flags are ignored by the other engines)
    r = subprocess.run([D2, '--omit-version', '--no-xml-tag', '--scale=1', '--elk-nodeNodeBetweenLayers=20',
                        '--elk-edgeNodeBetweenLayers=8', '--elk-padding=[top=30,left=8,bottom=8,right=8]',
                        f'--salt={salt}', f'--target={target}', '-' if text is not None else os.path.basename(path), '-'],
                       input=text, cwd=os.path.dirname(os.path.abspath(path)), capture_output=True, text=True, timeout=60)
    if r.returncode:
        raise RuntimeError(r.stderr.strip() or f'd2 exited {r.returncode}')
    return r.stdout


# D2 leaves 20 px round the label of every block (a two-line block of 63 × 44 px of text comes out 103 × 84); the
# second drawing gives each plain block its label's size with a narrow margin instead
D2_PAD, KEEP_X, KEEP_Y = 20, 9, 7
BLOCK = re.compile(r'<g class="([A-Za-z0-9+/=]+)((?: [\w-]+)*)"><g class="shape" ><rect x="[\d.-]+" y="[\d.-]+" '
                   r'width="([\d.]+)" height="([\d.]+)"')


def tight(text, sizes):
    """The source with every plain block of every board sized to its label: `sizes` holds each board's first drawing.
    A scheme that sets sizes itself (`width:`/`height:`) keeps them all."""
    if re.search(r'^[^#\n]*(?<![\w-])(?:width|height)\s*:', text, re.M):
        return None
    out = []
    for (name, target, *_), svg in sizes:
        found = {}
        for m in BLOCK.finditer(svg):
            try:
                bid = base64.b64decode(m.group(1)).decode()
            except ValueError:
                continue
            if re.fullmatch(r'[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*', bid):
                found[bid] = (float(m.group(3)), float(m.group(4)), 'place' in m.group(2).split())
        lines = []
        for bid, (w, h, double) in found.items():
            if any(o.startswith(bid + '.') for o in found) or w <= 2 * D2_PAD or h <= 2 * D2_PAD:
                continue   # a container, or a block with no room of D2's to take
            extra = 5 if double else 0   # the inner line of a double border
            lines.append(f'{bid}.width: {round(w - 2 * (D2_PAD - KEEP_X - extra))}')
            lines.append(f'{bid}.height: {round(h - 2 * (D2_PAD - KEEP_Y - extra))}')
        if lines and not target:
            out += lines
        elif lines:
            kind = target.split('.')[0]
            out.append(f'{kind}: {{"{name}": {{' + '; '.join(lines) + '}}')
    return text + '\n# sizes for tight blocks, added by tools/scheme\n' + '\n'.join(out) + '\n'


CSS = '''*{box-sizing:border-box} body{margin:0;background:#0f1216;color:#d8dde4;font:14px/1.45 system-ui,sans-serif;height:100vh;
display:grid;grid-template-rows:auto 1fr auto;overflow:hidden}
header{padding:8px 18px 6px;border-bottom:1px solid #2a313b;display:flex;align-items:center;gap:22px;flex-wrap:wrap}
h1{font-size:17px;margin:0} .tabs{display:flex;gap:6px;flex-wrap:wrap}
.tab{padding:5px 13px;border:1px solid #2a313b;border-radius:6px;cursor:pointer;background:#171b21;color:#d8dde4;font:inherit}
.tab.on{background:#25303d;border-color:#4b6077}
#view{position:relative;min-height:0} #stage{position:absolute;inset:0;overflow:auto;padding:6px 10px}
.board{display:none} .board.on{display:block}
#tools{position:absolute;top:6px;right:18px;display:flex;gap:4px;z-index:2}
#tools button{min-width:28px;height:26px;padding:0 6px;border:1px solid #2a313b;border-radius:6px;background:#171b21d9;
color:#8a94a3;font:600 13px system-ui,sans-serif;cursor:pointer;display:flex;align-items:center;justify-content:center}
#tools button:focus,.tab:focus{outline:none} #tools button:hover{color:#d8dde4} #tools button.on{color:#d8dde4;background:#25303d;border-color:#4b6077} #tools [hidden]{display:none}
.board svg{height:auto;display:block}
.board .appendix-icon{display:none} .board g.tip{cursor:pointer} .board g.tip:hover>.shape{filter:brightness(1.35)}
#side{border-top:1px solid #2a313b;background:#171b21;overflow:auto;max-height:36vh;padding:10px 20px;display:none}
#side.on{display:block} #side h2{font-size:16px;margin:0 0 6px} #side p,#side li{margin:4px 0;max-width:1100px}
#side ul{padding-left:18px;margin:4px 0} #side code{background:#232a33;padding:0 4px;border-radius:3px;font-size:12.5px}
#side hr{border:0;border-top:1px solid #2a313b;margin:6px 0} #side h3{font-size:12.5px;text-transform:uppercase;letter-spacing:.04em;color:#8a94a3;margin:12px 0 3px}
.tag{font-size:11px;color:#9fb0c3;background:#25303d;border-radius:3px;padding:0 4px;margin-left:3px}
.doc{max-width:1000px;padding:10px 18px} .doc p{margin:6px 0} .doc code{background:#232a33;padding:0 4px;border-radius:3px}
.board path.hit{stroke:transparent;stroke-width:14;fill:none;pointer-events:stroke} .board g.edge:hover>path{filter:brightness(1.6)}
.fitted #side{position:fixed;left:0;right:0;bottom:0;max-height:45vh;background:#171b21f2;z-index:3}
.board g.linked>.shape>*,.board g.linked>.shape{stroke:#ffffff!important;filter:brightness(1.35)}
/* picked: a colour no block or arrow has, with a glow, seen on any fill */
.board g.chosen>.shape>*,.board g.chosen>.shape,.board g.edge.chosen>path:not(.hit){stroke:#ff4fd8!important;stroke-width:4px!important}
.board g.chosen>.shape,.board g.edge.chosen>path:not(.hit){filter:drop-shadow(0 0 5px #ff4fd8)!important}
#side{position:relative} #held{display:flex;flex-wrap:wrap;gap:4px 10px;align-items:center;margin-bottom:6px;font-size:13px} #held .pin{color:#f0f0f0} #held .pick{color:#ff4fd8}
#unpin{margin-left:auto;background:#262626;color:#bbb;border:1px solid #555;border-radius:4px;font:inherit;padding:0 7px;cursor:pointer}
.board.linking g[class]:not(.linked):not(.hovered):not(.edge)>.shape,.board.linking g[class]:not(.linked):not(.hovered):not(.edge)>text{opacity:.25}
.board .lead{stroke-width:1.3;opacity:.55} .board .lead.hot{opacity:1;stroke-width:2}
.board.hovering g.edge:not(.hot)>path:not(.hit),.board.hovering g.edge:not(.hot)>text,.board.hovering .lead:not(.hot),.board.hovering .pin:not(.hot){opacity:.15}
.fitted h1{display:none} .fitted header:not(:has(.tabs)){display:none} body.fitted:not(:has(.tabs)){grid-template-rows:1fr auto}
.err{padding:20px} .err h2{color:#ff6b5c} pre{white-space:pre-wrap;background:#171b21;padding:10px;border-radius:6px}'''

JS = r'''
const tabs=[...document.querySelectorAll('.tab')], bs=[...document.querySelectorAll('.board[data-board]')], side=document.getElementById('side');
const esc=s=>s.replace(/&/g,'&amp;').replace(/</g,'&lt;');
// window.scheme: what an extension (README «Extending the page») works with; on(ev, f) hears 'zoom' (the user zooms),
// 'tab' (a tab is chosen), 'change' (pins or picks change), 'panel' (the panel is drawn again)
const hears={}, emit=(ev,x)=>(hears[ev]||[]).forEach(f=>f(x));
const S=window.scheme={on:(ev,f)=>(hears[ev]=hears[ev]||[]).push(f), emit, boards:bs, head:[]};
// tooltips are markdown-lite: ### heading, - item, **bold**, `code`, [R]/[S]/[I] tags, blank line = paragraph
function md(t){
  const inl=s=>esc(s).replace(/\*\*(.+?)\*\*/g,'<b>$1</b>').replace(/`([^`]+)`/g,'<code>$1</code>').replace(/\[([RSI])\]/g,'<span class=tag>$1</span>');
  let out='', list=false, para=[];
  const flush=()=>{ if(para.length){out+='<p>'+para.join(' ')+'</p>';para=[];} if(list){out+='</ul>';list=false;} };
  for(const l of t.split('\n')){
    if(/^#{1,3}\s/.test(l)){flush(); out+=(l.startsWith('# ')?'<h2>':'<h3>')+inl(l.replace(/^#+\s/,''))+(l.startsWith('# ')?'</h2>':'</h3>');}
    else if(/^\s*-\s/.test(l)){ if(para.length){out+='<p>'+para.join(' ')+'</p>';para=[];} if(!list){out+='<ul>';list=true;} out+='<li>'+inl(l.replace(/^\s*-\s/,''))+'</li>'; }
    else if(!l.trim()) flush();
    else { if(list){out+='</ul>';list=false;} para.push(inl(l)); }
  }
  flush(); return out;
}
// a picture keeps its own size (wider or taller ones scroll; one only too wide scrolls sideways under the plain wheel);
// Ctrl + wheel zooms the picture about the pointer, Ctrl+0 gives its size back — the page itself never zooms
const stage=document.getElementById('stage'); let zoom=1;   // fit() is global: live.js calls it when switched off
// shown in a frame that carries data-fit, zoom 1 is the picture fitted to the frame, not its size, and the plain wheel zooms
const FIT=!!(window.frameElement&&window.frameElement.dataset.fit);
if(FIT){ document.body.classList.add('fitted'); new ResizeObserver(()=>fit()).observe(stage); }
const size=s=>[+s.getAttribute('width')||s.viewBox.baseVal.width, +s.getAttribute('height')||s.viewBox.baseVal.height];
const baseOf=s=>{ const [n,h]=size(s); return FIT?Math.min((stage.clientWidth-20)/n,(stage.clientHeight-12)/h):1; };
const hidden=()=>FIT&&(stage.clientWidth<40||stage.clientHeight<40);   // the frame closed: nothing to fit to
function fit(){ if(hidden()) return; for(const s of document.querySelectorAll('.board:not(.laid)>svg')) s.style.width=size(s)[0]*baseOf(s)*zoom+'px'; }
// zooming draws the picture again at every step, too slow for a big one: while the wheel turns, the picture drawn last
// is scaled and moved by a transform (the graphics card's work), and drawn again sharp once it rests. zc is the zoom it
// was drawn at, tx/ty the transform's shift.
let zc=1, tx=0, ty=0, settle=0;
const svgOn=()=>document.querySelector('.board.on:not(.laid)>svg');
function unit(px,py){   // the picture's point (in its units) shown now at (px, py) of the stage's box
  const s=svgOn(), b=s.parentElement, k=baseOf(s)*zoom;
  return [(px+stage.scrollLeft-b.offsetLeft-tx)/k, (py+stage.scrollTop-b.offsetTop-ty)/k]; }
function sharp(px,py,ux,uy){   // draw it again at the zoom shown, the same point under (px, py)
  clearTimeout(settle); const s=svgOn(); if(!s) return; const b=s.parentElement;
  s.style.transform=''; tx=ty=0; zc=zoom; fit(); const k=baseOf(s)*zoom;
  stage.scrollLeft=b.offsetLeft+ux*k-px; stage.scrollTop=b.offsetTop+uy*k-py; }
function look(z,px,py,ux,uy){   // show zoom z with the picture's point (ux, uy) at (px, py), by the transform
  const s=svgOn(); if(!s) return; const b=s.parentElement, k=baseOf(s)*z;
  zoom=z; tx=px+stage.scrollLeft-b.offsetLeft-ux*k; ty=py+stage.scrollTop-b.offsetTop-uy*k;
  s.style.transformOrigin='0 0'; s.style.transform=`translate(${tx}px,${ty}px) scale(${z/zc})`;
  clearTimeout(settle); settle=setTimeout(()=>sharp(px,py,ux,uy),180); }
Object.assign(S,{stage, FIT, size, baseOf, hidden, fit, svgOn, unit, sharp, look, zoom:()=>zoom});
stage.addEventListener('wheel',e=>{
  if(!e.ctrlKey&&!FIT){   // a picture that is only too wide scrolls sideways under the plain wheel
    if(stage.scrollWidth>stage.clientWidth && stage.scrollHeight<=stage.clientHeight && Math.abs(e.deltaY)>Math.abs(e.deltaX)){
      stage.scrollLeft+=e.deltaY; e.preventDefault(); }
    return; }
  e.preventDefault(); emit('zoom');   // Ctrl + wheel, and in a fitted frame the plain wheel: zoom about the pointer
  if(document.querySelector('.board.on.laid')){   // a live picture is laid out again at the new zoom
    zoom=Math.min(4,Math.max(.2,zoom*Math.exp(-e.deltaY*.0015))); window.live.zoom(zoom); return; }
  const s=svgOn(); if(!s) return;
  const r=stage.getBoundingClientRect(), px=e.clientX-r.left, py=e.clientY-r.top, [ux,uy]=unit(px,py);
  look(Math.min(Math.max(4,3/baseOf(s)),Math.max(.2,zoom*Math.exp(-e.deltaY*.0015))),px,py,ux,uy); },{passive:false});
function unzoom(){ emit('zoom'); clearTimeout(settle); const s=svgOn(); if(s) s.style.transform=''; tx=ty=0; zoom=zc=1; fit(); window.live&&window.live.zoom(1); }
addEventListener('keydown',e=>{ if(e.ctrlKey&&e.key==='0'){ e.preventDefault(); unzoom(); } });
document.getElementById('t-zoom').onclick=unzoom;
for(const b of document.querySelectorAll('.board.doc')) b.innerHTML=md(b.dataset.intro);
// a tooltip is shown in the panel only: taken out of the SVG, where the browser would show it by the pointer too
for(const b of document.querySelectorAll('.board')) for(const t of b.querySelectorAll('g>title')){ const g=t.parentNode; g.classList.add('tip'); g.dataset.tip=t.textContent; t.remove(); }
// every element by its D2 id; an arrow as `a -> b` with full ids at the ends
const ARROW=/^(?:(.*)\.)?\((.+) (<->|->|<-|--) (.+)\)\[\d+\]$/;
function key(id){ const m=id.match(ARROW); if(!m) return id.trim();
  const at=x=>(m[1]?m[1]+'.':'')+x; return m[3]==='<-'?`${at(m[4])} -> ${at(m[2])}`:`${at(m[2])} ${m[3]} ${at(m[4])}`; }
const norm=x=>{ const m=x.trim().match(/^(.+?)\s*(<->|->|<-|--)\s*(.+)$/); return !m?x.trim():m[2]==='<-'?`${m[3]} -> ${m[1]}`:`${m[1]} ${m[2]} ${m[3]}`; };
const keys=new Map();
for(const g of document.querySelectorAll('.board g[class]')){ const c=g.classList[0];
  try{ const t=document.createElement('textarea'); t.innerHTML=new TextDecoder().decode(Uint8Array.from(atob(c),x=>x.charCodeAt(0)));
    keys.set(g,key(t.value)); }catch(e){} }
// every arrow: a wide invisible stroke along it takes the pointer
for(const [g,k] of keys) if(/ (->|<->|--) /.test(k)){
  g.classList.add('edge'); const p=g.querySelector(':scope>path');
  if(p){ const h=document.createElementNS('http://www.w3.org/2000/svg','path'); h.setAttribute('d',p.getAttribute('d')); h.setAttribute('class','hit'); g.append(h); } }
// the pointer over a block, a port or an arrow shows its tooltip in the panel; leaving gives back the clicked one. An
// arrow under the pointer (or its lead or pin, edges.js) is lit with its whole way, the others dimmed
function ways(g){ return g.closest('.board').querySelectorAll(`[data-of~="${g.classList[0]}"]`); }
function arrowsAt(t){   // the arrows under the pointer: one, or all of a bus's trunk (edges.js)
  if(!t||!t.closest) return []; const b=t.closest('.board'), of=t.closest('[data-of]');
  if(of) return of.dataset.of.split(' ').map(c=>b.querySelector(`g[class~="${c}"]`)).filter(Boolean);
  const g=t.closest('g.edge'); return g?[g]:[]; }
// vars.links: an element and what it goes with (a memory block and the ports and arrows its buffers pass), both ways
// an arrow among them brings the blocks at its ends too: the buffer is there as well
const withEnds=ms=>{ const out=new Set(ms); for(const m of ms){ const e=m.match(/^(.+?) (?:->|<->|--) (.+)$/); if(e){ out.add(e[1]); out.add(e[2]); } } return out; };
function linkedTo(g){ const b=g.closest('.board'), L=JSON.parse(b.dataset.links||'{}'), k=keys.get(g), out=new Set();
  for(const a in L){ const ms=withEnds(L[a].split(';').map(norm).filter(Boolean));
    if(norm(a)===k) ms.forEach(m=>out.add(m)); else if(ms.has(k)) out.add(norm(a)); }
  return [...b.querySelectorAll('g[class]')].filter(x=>out.has(keys.get(x))); }
// an element's chain: itself, its arrows (a container's: those of the blocks inside it too), the blocks at their other
// ends, and what vars.links ties it to. Under the pointer it is lit and the rest darkens. A click pins an element's
// chain or lets it go, a double click picks an element or drops it, any number of each; ✕ on the panel or Esc drops all.
function elAt(t){ const a=arrowsAt(t)[0]; if(a) return a; for(let g=t&&t.closest&&t.closest('g[class]'); g; g=g.parentNode.closest&&g.parentNode.closest('g[class]')) if(keys.has(g)) return g; return null; }
const endsOf=a=>{ const m=(keys.get(a)||'').match(/^(.+?) (?:->|<->|--) (.+)$/); return m?[m[1],m[2]]:[]; };
function chainOf(g){ const b=g.closest('.board'), k=keys.get(g), all=[...b.querySelectorAll('g[class]')].filter(x=>keys.has(x)), out=new Set([g]), ends=new Set();
  if(g.classList.contains('edge')) endsOf(g).forEach(x=>ends.add(x));
  else for(const a of all) if(a.classList.contains('edge')){ const [s,t]=endsOf(a), inside=x=>x===k||x.startsWith(k+'.');
    if(inside(s)||inside(t)){ out.add(a); ends.add(inside(s)?t:s); } }
  for(const x of all) if(ends.has(keys.get(x))) out.add(x);
  for(const x of linkedTo(g)) out.add(x);
  return out; }
const pins=new Map(), chosen=new Set(); let last=null;   // pinned element -> its chain; picked elements; the one touched last
function paint(chain,root){
  const busy=!!(chain&&chain.size)||pins.size>0||chosen.size>0;   // .user: something under the pointer, pinned or picked
  for(const b of document.querySelectorAll('.board')){ b.classList.remove('hovering','linking'); b.classList.toggle('user',busy); }
  for(const x of document.querySelectorAll('.board .hot,.board .linked,.board .hovered')) x.classList.remove('hot','linked','hovered');
  if(!chain||!chain.size) return;
  [...chain][0].closest('.board').classList.add('hovering','linking');
  for(const r of root) r.classList.add('hovered');
  for(const x of chain){ x.classList.add(x.classList.contains('edge')?'hot':'linked'); if(x.classList.contains('edge')) for(const w of ways(x)) w.classList.add('hot'); } }
const pinned=()=>{ const all=new Set(); for(const c of pins.values()) for(const x of c) all.add(x); return all; };
const plain=t=>(t||'').split('\n')[0].replace(/\*\*/g,'').replace(/\s*\[[RSI]\](\[[RSI]\])*/g,'').trim();
// an arrow is named by its label (kept before edges.js rewrites a fork's), a block by its tooltip's first line
for(const [g] of keys) if(g.classList.contains('edge')){ const t=g.querySelector(':scope>text'); if(t) g.dataset.name=t.textContent.trim(); }
const say=g=>({id:keys.get(g), what:(g.classList.contains('edge')?g.dataset.name:plain(g.dataset.tip))||''});
// a pin's short name: the tooltip's first line without its kind («kind · name — a few words»)
const short=g=>(say(g).what.split(' — ')[0]||keys.get(g)).replace(/^[^·]+ · /,'');
// the panel: a row of what is pinned (📌) and picked (◆) with ✕ for all of it, then the tooltip under the pointer, or of
// the element touched last, or the board's intro; an extension adds to the row (S.head: functions giving HTML)
function panel(g){
  const t=(g||last||{dataset:{}}).dataset.tip, more=S.head.map(f=>f()).join(''), head=pins.size||chosen.size||more;
  if(!t&&!head){ const b=document.querySelector('.board.on'), i=b.classList.contains('doc')?'':b.dataset.intro; side.innerHTML=md(i); side.classList.toggle('on',!!i); emit('panel'); return; }
  side.innerHTML=(head?'<div id="held">'+[...pins.keys()].map(x=>`<span class="pin">📌 ${esc(short(x))}</span>`).join('')+
    [...chosen].map(x=>`<span class="pick">◆ ${esc(short(x))}</span>`).join('')+more+
    `<button id="unpin" title="${UI.unpin}">✕</button></div>`:'')+md(t||'');
  side.classList.add('on'); const x=document.getElementById('unpin'); if(x) x.onclick=clearAll; emit('panel'); }
function relight(){ paint(pins.size?pinned():null,[...pins.keys()]); }
const report=()=>emit('change');
function togglePin(g){ if(pins.has(g)) pins.delete(g); else pins.set(g,chainOf(g)); last=pins.has(g)?g:last===g?null:last; relight(); panel(null); report(); }
function choose(g){ g.classList.toggle('chosen'); if(chosen.has(g)) chosen.delete(g); else { chosen.add(g); last=g; } relight(); panel(null); report(); }
function clearAll(){ pins.clear(); for(const x of chosen) x.classList.remove('chosen'); chosen.clear(); last=null; relight(); panel(null); report(); }
Object.assign(S,{keys, norm, ways, chainOf, linkedTo, pins, chosen, say, panel, relight});
Object.defineProperty(S,'last',{get:()=>last, set:v=>{ last=v; }});
stage.addEventListener('mouseover',e=>{ const g=elAt(e.target); if(!g) return; if(!pins.size) paint(chainOf(g),[g]); if(g.dataset.tip) panel(g); });
stage.addEventListener('mouseout',e=>{ const g=elAt(e.target); if(!g||elAt(e.relatedTarget)===g) return; if(!pins.size) paint(null,[]); panel(null); });
// a single click waits a moment: when it is half of a double click, only the double click counts
let clickWait=0;
stage.addEventListener('click',e=>{ const g=elAt(e.target); clearTimeout(clickWait); if(e.detail>1||!g) return; clickWait=setTimeout(()=>togglePin(g),250); });
stage.addEventListener('dblclick',e=>{ clearTimeout(clickWait); const g=elAt(e.target); if(g) choose(g); });
addEventListener('keydown',e=>{ if(e.key==='Escape'&&(pins.size||chosen.size)) clearAll(); });
tabs.forEach((t,i)=>t.addEventListener('click',()=>{ tabs.forEach(x=>x.classList.toggle('on',x===t)); emit('tab',i);
  for(const b of document.querySelectorAll('.board')) b.classList.toggle('on',b===bs[i]); clearAll(); window.edgesDraw&&window.edgesDraw(bs[i]);
  window.live&&window.live.show(); }));
// the page starts once an extension's script has run, so the extension hears the first report too
function start(){ panel(null); fit(); report(); }
'''


def page(title, body, script='', lang='en', css=''):
    return (f'<!doctype html><html lang="{E(lang)}"><head><meta charset="utf-8"><title>{E(title)}</title><style>{CSS}{css}</style></head>'
            f'<body>{body}{script}</body></html>\n')


def drawn(path, text, pre, lang=None):
    """Every board of a source in `lang`: its title, targets and SVGs (tight blocks, arrow tooltips)."""
    text = localized(text, lang)
    title, tab = head(text)
    targets = boards(text)
    targets[0] = (tab or UI.get(lang, UI['en'])['tab'],) + targets[0][1:]
    svgs = [d2(path, t, f'{pre}{i}', text) for i, (_, t, *_) in enumerate(targets)]
    tighter = tight(text, list(zip(targets, svgs)))
    if tighter:
        svgs = [d2(path, t, f'{pre}{i}', tighter) for i, (_, t, *_) in enumerate(targets)]
    return title, targets, [titled(s, arrows) for s, (_, _, _, arrows, _) in zip(svgs, targets)]


def render_file(path, lang=None, ext=None):
    """D2 source -> (HTML, errors). With errors the HTML is a page showing D2's own message. `lang`: the language to
    draw a scheme in several languages in (default: its first). `ext`: an extension for a host page, any object with
    some of these attributes (README «Extending the page»):
      boards(info) -> str   called with the drawn page (a dict: path, text, lang, targets, svgs, placed); may change
                            info['svgs'] and returns the HTML of boards of its own
      css, js, tools        CSS added to the page's, a script run after the page's own (it finds window.scheme),
                            buttons added to the tool row over the picture"""
    name = os.path.splitext(os.path.basename(path))[0]
    langs = []
    try:
        raw = open(path, encoding='utf-8').read()
        langs = languages(raw)
        lang = lang if lang in langs or not langs else langs[0]
        title, targets, svgs = drawn(path, raw, 'b', lang)
        text = localized(raw, lang)
        # the live layout takes every scheme whose blocks the author did not place: no `top`/`left`, no grid
        avoid = os.path.isfile(os.path.join(LIBAVOID, 'libavoid.wasm'))
        live = avoid and not re.search(r'^[^#\n]*(?<![\w-])(?:top|left|grid-rows|grid-columns)\s*:', text, re.M)
        # a scheme whose blocks the author placed gets its arrows from the page: edge to edge, a lead inside (edges.js)
        placed = avoid and bool(re.search(r'^[^#\n]*(?<![\w-])(?:top|left)\s*:', text, re.M))
        info = {'path': path, 'text': raw, 'lang': lang, 'targets': targets, 'svgs': svgs, 'placed': placed}
        extra = ext.boards(info) if hasattr(ext, 'boards') else ''
        svgs = info['svgs']
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as e:
        msg = str(e)
        ui = UI.get(lang, UI['en'])
        return page(f'⚠ {name}', f'<div class="err"><h2>{E(ui["err"].format(name))}</h2><pre>{E(msg)}</pre></div>',
                    lang=lang or 'en'), [msg]
    ui = UI.get(lang, UI['en'])
    tabs = ''.join(f'<button class="tab{" on" if i == 0 else ""}">{E(t)}</button>' for i, (t, *_) in enumerate(targets))
    body = (f'<header><h1>{E(title or name)}</h1>' + (f'<div class="tabs">{tabs}</div>' if len(targets) > 1 else '') +
            '</header><div id="view"><div id="stage">' +
            ''.join(f'<div class="board{" on" if i == 0 else ""}{" live" if live else ""}{" placed" if placed else ""}" data-board="{i}" data-intro="{E(intro)}" data-links="{E(json.dumps(links, ensure_ascii=False))}">{s}</div>'
                    if 'class="shape"' in s or not intro else   # a board of text only is a page of text
                    f'<div class="board doc{" on" if i == 0 else ""}" data-board="{i}" data-intro="{E(intro)}"></div>'
                    for i, (s, (_, _, intro, _, links)) in enumerate(zip(svgs, targets))) + extra +
            '</div><div id="tools">' + LIVE_BTN.format(E(ui['live'])) + '<button id="t-dir" hidden></button>'
            f'<button id="t-zoom" title="{E(ui["zoom"])}">1:1</button>' + getattr(ext, 'tools', '') +
            '</div></div><div id="side"></div>')
    # an extension's script runs in the page's own, so it hears the page's first report
    script = f'<script>const UI={json.dumps(ui, ensure_ascii=False)};{JS}\n{getattr(ext, "js", "")}\nstart();</script>'
    if live or placed:
        lib = re.sub(r'export\{(\w+) as AvoidLib\}', r'window.AvoidLib=\1;', open(os.path.join(LIBAVOID, 'index.js')).read())
        wasm = base64.b64encode(open(os.path.join(LIBAVOID, 'libavoid.wasm'), 'rb').read()).decode()
        script += (f'<script type="module">{lib}</script><script id="avoid-wasm" type="text/plain">{wasm}</script>'
                   f'<script type="module">{open(LIVE if live else EDGES).read()}</script>')
    return page(title or name, body, script, lang, getattr(ext, 'css', '')), []


class SvgOut:
    """The extension `--svg` renders the page with: the first board's picture without the page's pointer aids,
    serialized into the page for the dump to carry out — a placed scheme's once the page has laid its arrows, any other
    as D2 draws it (the live layout fits a window, a picture has none)."""
    js = ("const svgOut=()=>{const s=document.querySelector('.board.on>svg').cloneNode(true);"
          "s.querySelectorAll('path.hit,.appendix-icon').forEach(e=>e.remove());s.removeAttribute('style');"
          "const st=document.createElementNS('http://www.w3.org/2000/svg','style');st.textContent='.lead{stroke-width:1.3;opacity:.55}';"
          "s.prepend(st);const o=document.createElement('pre');o.id='svg-out';o.textContent=new XMLSerializer().serializeToString(s);"
          "document.body.append(o);};"
          "if(document.querySelector('.board.on.placed')) addEventListener('edges',svgOut); else svgOut();")


def render_svg(path, lang=None):
    """D2 source -> (SVG, errors): the first board as the page draws it."""
    import tempfile
    html_, errs = render_file(path, lang, SvgOut())
    if errs:
        return '', errs
    chrome = next((c for c in ('google-chrome', 'chromium', 'chromium-browser') if shutil.which(c)), None)
    if not chrome:
        return '', ['--svg needs Chrome or Chromium']
    with tempfile.TemporaryDirectory() as d:
        page_ = os.path.join(d, 'page.html')
        open(page_, 'w', encoding='utf-8').write(html_)
        dom = subprocess.run([chrome, '--headless=new', '--disable-gpu', f'--user-data-dir={d}/profile',
                              '--virtual-time-budget=30000', '--dump-dom', 'file://' + page_],
                             capture_output=True, text=True, timeout=180).stdout
    m = re.search(r'<pre id="svg-out">(.*?)</pre>', dom, re.S)
    if not m:
        return '', ['the page gave no picture: a placed scheme needs libavoid-js for its arrows (README «Installing»)']
    return html.unescape(m.group(1)), []


def title_of(path, lang=None):
    try:
        with open(path, encoding='utf-8') as f:
            text = f.read()
    except OSError:
        return ''
    langs = languages(text)
    return head(localized(text, lang if lang in langs else None))[0]


if __name__ == '__main__':
    args, lang, out, svg = sys.argv[1:], None, None, False
    src = []
    while args:
        if args[0] in ('-h', '--help'):
            sys.exit(__doc__)
        if args[0] == '--svg':
            svg, args = True, args[1:]
            continue
        if args[0] in ('--lang', '-o') and len(args) > 1:
            lang, out = (args[1], out) if args[0] == '--lang' else (lang, args[1])
            args = args[2:]
        else:
            src.append(args.pop(0))
    if len(src) != 1:
        sys.exit(__doc__)
    html_, errs = render_svg(src[0], lang) if svg else render_file(src[0], lang)
    if errs:
        sys.exit(f'{src[0]}: {errs[0]}')
    if out:
        open(out, 'w', encoding='utf-8').write(html_)
    else:
        sys.stdout.write(html_)
