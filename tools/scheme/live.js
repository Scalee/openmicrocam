// The live layout of a scheme page (tools/scheme/README.md): D2 drew the blocks, this places them for the panel's size
// and the zoom and routes the arrows round them (libavoid), and a change of either glides instead of jumping.
// The layout goes bottom up: a zone packs its blocks as tight as it can, in chains along its arrows, each chain next to
// where its arrows come from; a zone is then one block for the level above; the top level lines its units up across
// the panel at the least height that still fits, wrapping into rows like text, or stacks them when the panel is narrow.
// Anything going wrong leaves D2's own picture.

const GAP = 16, COL = 30, UNIT = 30, PAD = {t: 30, l: 12, r: 12, b: 12}, GLIDE = 280;
const dec = s => { try { return decodeURIComponent(escape(atob(s))).replace(/&gt;/g, '>').replace(/&lt;/g, '<'); } catch (e) { return null; } };

let A = null;   // libavoid, loaded once from the page's own copy of its wasm
async function avoid() {
  if (A) return A;
  const b64 = document.getElementById('avoid-wasm').textContent.trim(), bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  await window.AvoidLib.load(URL.createObjectURL(new Blob([bin], {type: 'application/wasm'})));
  return A = window.AvoidLib.getInstance();
}

class Board {
  constructor(el, A) {
    this.el = el; this.A = A;
    this.outer = el.querySelector(':scope > svg'); this.inner = this.outer.querySelector('svg') || this.outer;
    for (const m of this.inner.querySelectorAll('[mask]')) m.removeAttribute('mask');   // D2's label cut-outs sit at its own places
    const S = this.S = {}, order = this.order = [], edges = this.edges = [];
    for (const g of this.inner.querySelectorAll('g[class]')) {
      const cls = g.getAttribute('class').split(' '), id = dec(cls[0]); if (!id) continue;
      const em = id.match(/^(?:(.*?)\.)?\((.+?) (->|<-|--|<->) (.+?)\)\[\d+\]$/);   // an edge in a zone carries the zone's name first
      if (em) { const pre = em[1] ? em[1] + '.' : '';
        edges.push({g, a: pre + em[2], b: pre + em[4], dir: em[3], tie: cls.includes('tie'), path: g.querySelector('path'), label: g.querySelector('text')});
        continue; }
      const bb = g.getBBox(); if (!bb.width) continue;
      S[id] = {id, g, x0: bb.x, y0: bb.y, w: bb.width, h: bb.height, kids: []}; order.push(id);
    }
    for (const id of order) { const p = id.split('.').slice(0, -1).join('.'); S[id].parent = S[p] ? p : null; if (S[p]) S[p].kids.push(id); }
    this.roots = order.filter(id => !S[id].parent);
    this.leaves = order.filter(id => !S[id].kids.length);
    for (const e of edges) e.lost = !S[e.a] || !S[e.b];
    this.live = edges.filter(e => !e.lost);
    for (const e of edges) if (e.tie || e.lost) e.g.style.display = 'none';
    for (const e of this.live) if (e.label) { const b = e.label.getBBox(); e.lw = b.width; e.lh = b.height;
      e.label.style.paintOrder = 'stroke'; e.label.style.stroke = '#0f1216'; e.label.style.strokeWidth = '4px'; e.label.style.strokeLinejoin = 'round'; }
    // the router: a shape per block with a pin in the middle of each side; an arrow to a whole zone ends on its border
    const R = this.router = new A.Router(A.RouterFlag.OrthogonalRouting.value);
    R.setRoutingParameter(A.RoutingParameter.shapeBufferDistance, 4);
    R.setRoutingParameter(A.RoutingParameter.idealNudgingDistance, 5);
    R.setRoutingParameter(A.RoutingParameter.segmentPenalty, 20);
    R.setRoutingParameter(A.RoutingParameter.crossingPenalty, 60);
    R.setRoutingOption(A.RoutingOption.nudgeOrthogonalSegmentsConnectedToShapes, true);
    R.setRoutingOption(A.RoutingOption.nudgeSharedPathsWithCommonEndPoint, true);
    this.ref = {};
    for (const id of this.leaves) {
      const s = S[id]; this.ref[id] = new A.ShapeRef(R, this.rect(s.x0, s.y0, s.w, s.h));
      for (const [px, py, d] of [[0, .5, 4], [1, .5, 8], [.5, 0, 1], [.5, 1, 2]])   // a pin takes any number of arrows
        new A.ShapeConnectionPin(this.ref[id], 1, px, py, true, 0, d).setExclusive(false);
    }
    this.conns = this.live.filter(e => !e.tie).map(e => {
      const [from, to] = e.dir === '<-' ? [e.b, e.a] : [e.a, e.b];
      if (e.dir === '<-' && e.path) { const ms = e.path.getAttribute('marker-start'); if (ms) { e.path.setAttribute('marker-end', ms); e.path.removeAttribute('marker-start'); } }
      return {e, from, to, c: null};   // made on the first drawing, with its ends
    });
    this.drawn = null; this.box = null; this.anim = 0;
  }
  rect(x, y, w, h) { const A = this.A, p = new A.Point(x + w / 2, y + h / 2), r = new A.Rectangle(p, w, h); p.delete(); return r; }
  under(c, id) { const S = this.S; while (id && S[id].parent !== c) id = S[id].parent; return id; }   // c's child holding id

  // ---- the layout
  unit(id, axis, room) {   // -> {w, h, put(x, y)}
    const S = this.S, s = S[id], placed = this.placed;
    if (!s.kids.length) return {w: s.w, h: s.h, put: (x, y) => { placed[id] = {x: x + s.w / 2, y: y + s.h / 2, X: x, Y: y}; }};
    const kids = s.kids, tt = s.g.querySelector('text'), top = Math.max(PAD.t, tt ? tt.getBBox().height + 14 : 0);
    const inner = room - (axis === 'x' ? top + PAD.b : PAD.l + PAD.r);
    const succ = {}, indeg = {}; kids.forEach(k => { succ[k] = []; indeg[k] = 0; });
    for (const e of this.live) {   // arrows between the children, in the order written
      const a = this.under(id, e.a), b = this.under(id, e.b);
      if (a && b && a !== b && !succ[a].includes(b)) { succ[a].push(b); indeg[b]++; }
    }
    const used = new Set(), chains = [];   // chains: from each start, the first child not taken yet
    for (const k of [...kids.filter(k => !indeg[k]), ...kids]) {
      if (used.has(k)) continue;
      const ch = []; let c = k;
      while (c && !used.has(c)) { used.add(c); ch.push(c); c = succ[c].find(n => !used.has(n)); }
      chains.push(ch);
    }
    const key = ch => {   // a chain goes next to the blocks outside this zone its arrows come from, if placed already
      let best = Infinity;
      for (const e of this.live) for (const [mine, other] of [[e.b, e.a], [e.a, e.b]]) {
        if (ch.includes(this.under(id, mine)) && !this.under(id, other) && placed[other]) best = Math.min(best, axis === 'x' ? placed[other].y : placed[other].x);
      }
      return best;
    };
    const keyed = chains.map((ch, i) => ({ch, k: key(ch), i})).sort((p, q) => (p.k - q.k) || (p.i - q.i));
    const L = {}; kids.forEach(k => L[k] = this.unit(k, axis, inner));
    const along = k => axis === 'x' ? L[k].h : L[k].w, across = k => axis === 'x' ? L[k].w : L[k].h;
    const cols = [[]]; let fill = 0;   // columns (rows when the flow goes down) of whole chains, as high as the room
    for (const {ch} of keyed) for (const k of ch) {
      if (fill && fill + GAP + along(k) > inner) { cols.push([]); fill = 0; }
      cols[cols.length - 1].push(k); fill += (fill ? GAP : 0) + along(k);
    }
    const colW = cols.map(c => Math.max(...c.map(across))), colH = cols.map(c => c.reduce((t, k) => t + along(k), 0) + GAP * (c.length - 1));
    const a = colW.reduce((t, w) => t + w, 0) + COL * (cols.length - 1), b = Math.max(...colH);
    const titleW = tt ? tt.getBBox().width + 24 : 0;
    const w = Math.max(axis === 'x' ? a : b, titleW) + PAD.l + PAD.r, h = (axis === 'x' ? b : a) + top + PAD.b;
    return {w, h, put: (x, y) => {
      placed[id] = {X: x, Y: y, W: w, H: h};
      let c0 = 0;
      cols.forEach((c, i) => {
        let r = 0;
        for (const k of c) {
          if (axis === 'x') L[k].put(x + PAD.l + c0, y + top + r); else L[k].put(x + PAD.l + r, y + top + c0);
          r += along(k) + GAP;
        }
        c0 += colW[i] + COL;
      });
    }};
  }
  arrangeAt(W, axis, room) {
    this.placed = {};
    const top = this.roots.slice(), rank = {}; top.forEach(r => rank[r] = 0);
    for (let pass = 0; pass < top.length; pass++) for (const e of this.live) {   // longest path; a loop stops at the count
      const a = this.under(null, e.a), b = this.under(null, e.b);
      if (a && b && a !== b) rank[b] = Math.max(rank[b], Math.min(rank[a] + 1, top.length));
    }
    top.sort((p, q) => (rank[p] - rank[q]) || (this.order.indexOf(p) - this.order.indexOf(q)));
    let x = 0, y = 0, line = 0, width = 0, span = 0;
    for (const r of top) {
      const u = this.unit(r, axis, room); span += u.w + UNIT;
      if (axis === 'x') { if (x && x + u.w > W) { x = 0; y += line + UNIT; line = 0; } u.put(x, y); x += u.w + UNIT; line = Math.max(line, u.h); width = Math.max(width, x - UNIT); }
      else { u.put(x, y); y += u.h + UNIT; width = Math.max(width, u.w); }
    }
    return {axis, width, height: axis === 'x' ? y + line : y - UNIT, at: this.placed, span: span - UNIT};
  }
  arrange(W, H, axis) {   // across: the least height at which the units still stand in one row; down: stacked
    const full = (axis === 'x' ? H : W) - 20;
    let room = full;
    if (axis === 'x' && this.arrangeAt(W, axis, full).span <= W) {
      let lo = Math.max(...this.leaves.map(id => this.S[id].h)) + 60, hi = full;
      while (hi - lo > 8) { const mid = (lo + hi) / 2; if (this.arrangeAt(W, axis, mid).span <= W) hi = mid; else lo = mid; }
      room = hi;
    }
    return this.arrangeAt(W, axis, room);
  }

  // ---- drawing a layout: blocks moved, zones resized, arrows routed, labels where they are free
  draw(at) {
    const {A, S, router: R} = this;
    for (const id of this.order) {
      const s = S[id], p = at[id]; if (!p) continue;
      if (s.kids.length) {
        const r = s.g.querySelector('rect'), t = s.g.querySelector('text');
        if (r) { r.setAttribute('x', p.X); r.setAttribute('y', p.Y); r.setAttribute('width', p.W); r.setAttribute('height', p.H); }
        if (t) { t.setAttribute('x', p.X + p.W / 2); t.setAttribute('y', p.Y + t.getBBox().height * .8 + 6); }
      } else {
        s.g.setAttribute('transform', `translate(${p.X - s.x0},${p.Y - s.y0})`);
        const r = this.rect(p.X, p.Y, s.w, s.h); R.moveShape_poly(this.ref[s.id], r, false); r.delete();
      }
    }
    const end = (id, other) => {   // a block's pins, or the middle of the side of a zone that faces the other end
      if (this.ref[id]) return new A.ConnEnd(this.ref[id], 1);
      const p = at[id], q = at[other] || {}, ox = q.x ?? (q.X + q.W / 2), oy = q.y ?? (q.Y + q.H / 2);
      const cx = p.X + p.W / 2, cy = p.Y + p.H / 2, dx = ox - cx, dy = oy - cy;
      const pt = Math.abs(dx) * p.H > Math.abs(dy) * p.W ? new A.Point(dx > 0 ? p.X + p.W : p.X, cy) : new A.Point(cx, dy > 0 ? p.Y + p.H : p.Y);
      const ce = new A.ConnEnd(pt); pt.delete(); return ce;
    };
    for (const k of this.conns) {
      const s = end(k.from, k.to), t = end(k.to, k.from);
      if (k.c) { k.c.setSourceEndpoint(s); k.c.setDestEndpoint(t); } else k.c = new A.ConnRef(R, s, t);
      s.delete(); t.delete();
    }
    R.processTransaction();
    const busy = this.leaves.map(id => { const p = at[id]; return p ? [p.X, p.Y, p.X + S[id].w, p.Y + S[id].h] : null; }).filter(Boolean);
    for (const id of this.order) if (S[id].kids.length && at[id]) { const t = S[id].g.querySelector('text'); if (t) { const b = t.getBBox(); busy.push([b.x, b.y, b.x + b.width, b.y + b.height]); } }
    const free = r => !busy.some(b => r[0] < b[2] && b[0] < r[2] && r[1] < b[3] && b[1] < r[3]);
    for (const {e, c} of this.conns) {
      const route = c.displayRoute(), pts = [];
      for (let i = 0; i < route.size(); i++) { const p = route.at(i); pts.push([p.x, p.y]); }
      if (e.path) e.path.setAttribute('d', pts.map((p, i) => (i ? 'L' : 'M') + p[0] + ' ' + p[1]).join(' '));
      if (!e.label || pts.length < 2) continue;
      const w = e.lw + 4, h = e.lh, spots = [];
      const segs = pts.slice(1).map((p, i) => [pts[i], p]).sort((u, v) => (Math.abs(v[1][0] - v[0][0]) + Math.abs(v[1][1] - v[0][1])) - (Math.abs(u[1][0] - u[0][0]) + Math.abs(u[1][1] - u[0][1])));
      for (const [a, b] of segs) for (const f of [.5, .3, .7]) {
        const mx = a[0] + (b[0] - a[0]) * f, my = a[1] + (b[1] - a[1]) * f;
        if (a[1] === b[1]) spots.push([mx - w / 2, my - h - 3], [mx - w / 2, my + 3]);
        else spots.push([mx + 5, my - h / 2], [mx - 5 - w, my - h / 2]);
      }
      const at0 = spots.find(([x, y]) => free([x, y, x + w, y + h])) || spots[0];
      busy.push([at0[0], at0[1], at0[0] + w, at0[1] + h]);
      e.label.setAttribute('text-anchor', 'start'); e.label.setAttribute('x', at0[0] + 2); e.label.setAttribute('y', at0[1] + h * .8);
    }
  }
  frame(box, zoom) {
    const pad = 12, w = box.width + 2 * pad, h = box.height + 2 * pad, vb = `${-pad} ${-pad} ${w} ${h}`;
    for (const el of new Set([this.outer, this.inner])) {
      el.setAttribute('viewBox', vb); el.setAttribute('width', w * zoom); el.setAttribute('height', h * zoom);
      el.removeAttribute('x'); el.removeAttribute('y'); el.style.width = '';
    }
    this.inner.querySelectorAll(':scope > rect').forEach(r => { r.setAttribute('x', -pad); r.setAttribute('y', -pad); r.setAttribute('width', w); r.setAttribute('height', h); });
  }
  relayout(W, H, zoom, axis, glide) {   // a new layout glides from the one drawn now
    // the next frame; where frames do not come (a page in the background), the end after the glide's time
    const next = f => { let done = false; requestAnimationFrame(n => { done = true; f(n); });
      setTimeout(() => { if (!done) f(performance.now() + GLIDE); }, 120); };
    const target = this.arrange(W / zoom, H / zoom, axis), from = glide ? this.drawn : null, box0 = this.box || target;
    const t0 = performance.now(), my = ++this.anim, lerp = (a, b, t) => a + (b - a) * t;
    const step = now => {
      if (my !== this.anim) return;
      const t = from ? Math.min(1, (now - t0) / GLIDE) : 1, k = t < .5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2, at = {};
      for (const id in target.at) { const p = target.at[id], q = from && from[id] || p; at[id] = {};
        for (const f of ['x', 'y', 'X', 'Y', 'W', 'H']) if (p[f] !== undefined) at[id][f] = lerp(q[f] ?? p[f], p[f], k); }
      this.box = {width: lerp(box0.width, target.width, k), height: lerp(box0.height, target.height, k)};
      this.frame(this.box, zoom); this.draw(at); this.drawn = at;
      if (t < 1) next(step);
    };
    if (from) next(step); else step(t0);
  }
}

// ---- the page: lay out the board shown, again on a change of size or zoom — while the picture's switch is on (kept,
// off by default: D2's own picture), across or down as its direction button says (kept, across by default). The live
// layout works on a copy of D2's picture, so switching off puts D2's own back.
const stage = document.getElementById('stage'), made = new Map(), d2pic = new Map(), KEY = 'microcam-scheme-live', DIR = 'microcam-scheme-dir';
const btn = document.getElementById('t-live'), dirBtn = document.getElementById('t-dir');
let zoom = 1, lib = null, on = localStorage.getItem(KEY) === '1', axis = localStorage.getItem(DIR) === 'y' ? 'y' : 'x';
async function current(glide) {
  const el = document.querySelector('.board.on.live'); if (!el || !on) return;
  try {
    lib = lib || await avoid();
    if (!made.has(el)) {
      const pic = el.querySelector(':scope > svg'), copy = pic.cloneNode(true);
      d2pic.set(el, pic); el.replaceChild(copy, pic); el.classList.add('laid');
      made.set(el, new Board(el, lib));
    }
    const cs = getComputedStyle(stage), padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight), padY = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
    made.get(el).relayout(stage.clientWidth - padX - 28, stage.clientHeight - padY - 28, zoom, axis, glide && made.get(el).drawn);   // 28: the picture's frame
  } catch (err) { console.error('live layout', err); off(el); el.classList.remove('live'); }   // D2's own picture stays
}
function off(el) {
  const b = made.get(el); if (!b) return;
  b.anim++; try { b.router.delete(); } catch (e) {}
  el.replaceChild(d2pic.get(el), el.querySelector(':scope > svg')); el.classList.remove('laid'); made.delete(el);
}
const showDir = () => { dirBtn.hidden = !on; dirBtn.textContent = axis === 'x' ? '⇢' : '⇣';
  dirBtn.title = axis === 'x' ? UI.dirX : UI.dirY; };   // UI: the page's words (scheme.py)
function set(v) {
  on = v; localStorage.setItem(KEY, v ? '1' : '0'); btn.classList.toggle('on', v); showDir();
  if (v) current(false); else { for (const el of [...made.keys()]) off(el); window.fit && window.fit(); }
}
if (document.querySelector('.board.live')) {
  btn.hidden = false; btn.classList.toggle('on', on); btn.onclick = () => set(!on); showDir();
  dirBtn.onclick = () => { axis = axis === 'x' ? 'y' : 'x'; localStorage.setItem(DIR, axis); showDir(); current(true); };
}
window.live = {zoom(z) { zoom = z; current(true); }, show() { current(false); }};
new ResizeObserver(() => current(true)).observe(stage);
current(false);
