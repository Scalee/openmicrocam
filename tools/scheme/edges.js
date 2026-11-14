// The arrows of a scheme whose blocks are placed by hand (tools/scheme/README.md). An arrow runs between the borders of
// the top-level blocks: an end inside a block gets a pin on that block's border, on the side facing the other end, and
// a thin lead inside from its port to the pin, drawn under the other ports; an arrow between two blocks of one
// top-level block stays inside it. Routes go round the blocks (libavoid), each on a lane of its own. The pointer over
// an arrow lights its whole way, leads and pins included. Anything going wrong leaves D2's own arrows.

const dec = s => { try { return decodeURIComponent(escape(atob(s))).replace(/&gt;/g, '>').replace(/&lt;/g, '<'); } catch (e) { return null; } };
const ARROW = /^(?:(.*?)\.)?\((.+?) (->|<-|--|<->) (.+?)\)\[\d+\]$/;
const NS = 'http://www.w3.org/2000/svg', GAP = 20, IN = 6;   // pins on one side at least GAP apart; a lead's bend IN from the border

let A = null;   // libavoid, loaded once from the page's own copy of its wasm (the same as live.js)
async function avoid() {
  if (A) return A;
  if (window.AvoidLib.avoidLib) return A = window.AvoidLib.getInstance();
  const b64 = document.getElementById('avoid-wasm').textContent.trim(), bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  await window.AvoidLib.load(URL.createObjectURL(new Blob([bin], {type: 'application/wasm'})));
  return A = window.AvoidLib.getInstance();
}

function router() {
  const R = new A.Router(A.RouterFlag.OrthogonalRouting.value);
  R.setRoutingParameter(A.RoutingParameter.shapeBufferDistance, 14);
  R.setRoutingParameter(A.RoutingParameter.idealNudgingDistance, 18);   // room between arrows side by side
  R.setRoutingParameter(A.RoutingParameter.segmentPenalty, 30);
  R.setRoutingParameter(A.RoutingParameter.crossingPenalty, 80);
  R.setRoutingOption(A.RoutingOption.nudgeOrthogonalSegmentsConnectedToShapes, true);
  R.setRoutingOption(A.RoutingOption.nudgeSharedPathsWithCommonEndPoint, true);
  return R;
}
const rect = (x, y, w, h) => { const p = new A.Point(x + w / 2, y + h / 2), r = new A.Rectangle(p, w, h); p.delete(); return r; };
function shape(R, b) {   // a block as an obstacle with a pin in the middle of each side, any number of arrows on one
  const r = rect(b.x, b.y, b.w, b.h), s = new A.ShapeRef(R, r); r.delete();
  for (const [px, py, d] of [[0, .5, 4], [1, .5, 8], [.5, 0, 1], [.5, 1, 2]]) new A.ShapeConnectionPin(s, 1, px, py, true, 0, d).setExclusive(false);
  return s;
}
// a route ends OUT beyond a pin, straight out from its side (this libavoid takes no direction for a free end): the
// stub from the pin to there is drawn by us, so the arrow always leaves the border square to it
const OUT = 18, STEP = {l: [-1, 0], r: [1, 0], t: [0, -1], b: [0, 1]};
const beyond = (x, y, side) => [x + STEP[side][0] * OUT, y + STEP[side][1] * OUT];
function at(x, y, side) { const [bx, by] = beyond(x, y, side), p = new A.Point(bx, by), c = new A.ConnEnd(p); p.delete(); return c; }
// the side of block b that faces point (ox, oy)
function facing(b, ox, oy) {
  const dx = ox - (b.x + b.w / 2), dy = oy - (b.y + b.h / 2);
  return Math.abs(dx) * b.h >= Math.abs(dy) * b.w ? (dx > 0 ? 'r' : 'l') : (dy > 0 ? 'b' : 't');
}
const mid = b => [b.x + b.w / 2, b.y + b.h / 2];
const edgeOf = (b, side) => ({l: [b.x, null], r: [b.x + b.w, null], t: [null, b.y], b: [null, b.y + b.h]}[side]);
function el(tag, attrs) { const e = document.createElementNS(NS, tag); for (const k in attrs) e.setAttribute(k, attrs[k]); return e; }
const d = pts => pts.map((p, i) => (i ? 'L' : 'M') + p[0] + ' ' + p[1]).join(' ');
// libavoid's nudging moves a route's ends a few pixels off the points asked for; where two points meet at a slant, a
// corner goes between them, keeping the way the line was going
function square(pts) {
  const out = [pts[0]];
  for (let i = 1; i < pts.length; i++) {
    const a = out[out.length - 1], b = pts[i];
    if (Math.abs(a[0] - b[0]) > .5 && Math.abs(a[1] - b[1]) > .5) {
      const p = out[out.length - 2], vert = p ? Math.abs(p[0] - a[0]) < .5 : false;
      out.push(vert ? [a[0], b[1]] : [b[0], a[1]]);
    }
    out.push(b);
  }
  // a point between two on one line with it goes: it is a bend that is none, or a turn back (a fork's hub past the
  // branch nearer the trunk) that would draw a stub
  const line = (a, b, c) => (Math.abs(a[0] - b[0]) < .5 && Math.abs(b[0] - c[0]) < .5) || (Math.abs(a[1] - b[1]) < .5 && Math.abs(b[1] - c[1]) < .5);
  for (let i = 1; i < out.length - 1;) if (line(out[i - 1], out[i], out[i + 1])) out.splice(i, 1); else i++;
  return out;
}

function board(bel) {
  const outer = bel.querySelector(':scope > svg'), inner = outer && (outer.querySelector('svg') || outer);
  if (!inner) return;
  const S = {}, edges = [];
  for (const g of inner.querySelectorAll('g[class]')) {
    const id = dec(g.classList[0]); if (!id) continue;
    const m = id.match(ARROW);
    if (m) {
      const pre = m[1] ? m[1] + '.' : '', [a, b] = m[3] === '<-' ? [pre + m[4], pre + m[2]] : [pre + m[2], pre + m[4]];
      edges.push({g, a, b, path: g.querySelector(':scope > path'), label: g.querySelector(':scope > text')});
      continue;
    }
    const r = g.querySelector(':scope > .shape > rect');
    if (!r) continue;
    S[id] = {id, g, x: +r.getAttribute('x'), y: +r.getAttribute('y'), w: +r.getAttribute('width'), h: +r.getAttribute('height'), kids: 0};
  }
  for (const id in S) { const p = id.split('.').slice(0, -1).join('.'); S[id].parent = S[p] ? p : null; if (S[p]) S[p].kids++; }
  const root = id => id.split('.')[0];
  const live = edges.filter(e => S[e.a] && S[e.b] && e.path);
  if (!live.length) return;
  for (const m of inner.querySelectorAll('[mask]')) m.removeAttribute('mask');   // D2's label cut-outs sit at its own places

  // pins: an end inside a top-level block, on the side facing the block at the other end. Arrows leaving one end there
  // share its pin and fork; each arrow reaching an end gets a pin of its own, so arrows coming together stay apart
  const pins = {};   // root + side -> [{end, out, want, at}]
  const pinOf = (end, other, out) => {
    // facing the nearest point of the block at the other end: a long block faces by its part across from this one
    const R = S[root(end)], O = S[root(other)], [rx, ry] = mid(R), side = facing(R, Math.min(Math.max(rx, O.x), O.x + O.w), Math.min(Math.max(ry, O.y), O.y + O.h)), [cx, cy] = mid(S[end]);
    const k = R.id + ':' + side, list = pins[k] = pins[k] || [];
    let p = out ? list.find(q => q.end === end && q.out) : null;
    const [ox, oy] = mid(S[other]);
    if (!p) { p = {end, out, R, side, want: side === 'l' || side === 'r' ? cy : cx, there: side === 'l' || side === 'r' ? oy : ox}; list.push(p); }
    return p;
  };
  const ext = [], inside = {};
  for (const e of live) {
    if (root(e.a) === root(e.b)) { (inside[root(e.a)] = inside[root(e.a)] || []).push(e); continue; }
    e.pa = e.a !== root(e.a) ? pinOf(e.a, e.b, true) : null;
    e.pb = e.b !== root(e.b) ? pinOf(e.b, e.a, false) : null;
    ext.push(e);
  }
  // pins on one side kept GAP apart, as near as they can be to where their ports are
  for (const k in pins) {
    // pins wanting about one place stand in the order of the other ends along the border, so their arrows do not cross
    const list = pins[k].sort((p, q) => Math.abs(p.want - q.want) < GAP ? p.there - q.there || p.want - q.want : p.want - q.want), R = list[0].R, vert = list[0].side === 'l' || list[0].side === 'r';
    const lo = (vert ? R.y : R.x) + 8, hi = (vert ? R.y + R.h : R.x + R.w) - 8;
    let last = -1e9; for (const p of list) last = p.at = Math.max(p.want, last + GAP, lo);
    let next = 1e9; for (const p of list.slice().reverse()) next = p.at = Math.min(p.at, next - GAP, hi);
    for (const p of list) { const [ex, ey] = edgeOf(R, p.side); p.x = vert ? ex : p.at; p.y = vert ? p.at : ey; }
  }

  // the routes between top-level blocks
  const R = router(), ref = {};
  for (const id in S) if (!S[id].parent) ref[id] = shape(R, S[id]);
  // arrows from one place into several pins of one block fork rather than run side by side: one trunk is routed
  // to a hub OUT beyond the middle of their pins, and each goes on from there along the border to its own pin
  const groups = {};
  for (const e of ext) { const k = (e.pa ? e.pa.end + ':' + e.pa.side : e.a) + '>' + root(e.b); (groups[k] = groups[k] || []).push(e); }
  const conns = [];
  for (const k in groups) {
    const g = groups[k], e = g[0], fork = g.length > 1 && g.every(x => x.pb && x.pb.side === e.pb.side);
    const s = e.pa ? at(e.pa.x, e.pa.y, e.pa.side) : new A.ConnEnd(ref[e.a], 1);
    let t, hub = null;
    if (fork) {
      const vert = e.pb.side === 'l' || e.pb.side === 'r', m = g.reduce((a, x) => a + (vert ? x.pb.y : x.pb.x), 0) / g.length;
      hub = beyond(vert ? e.pb.x : m, vert ? m : e.pb.y, e.pb.side);
      const p = new A.Point(hub[0], hub[1]); t = new A.ConnEnd(p); p.delete();
    } else t = e.pb ? at(e.pb.x, e.pb.y, e.pb.side) : new A.ConnEnd(ref[e.b], 1);
    const c = new A.ConnRef(R, s, t); s.delete(); t.delete();
    conns.push([fork ? g : [e], c, hub]);
    if (!fork) for (const x of g.slice(1)) {   // ends in no common pin side: each its own way
      const s2 = x.pa ? at(x.pa.x, x.pa.y, x.pa.side) : new A.ConnEnd(ref[x.a], 1), t2 = x.pb ? at(x.pb.x, x.pb.y, x.pb.side) : new A.ConnEnd(ref[x.b], 1);
      conns.push([[x], new A.ConnRef(R, s2, t2), null]); s2.delete(); t2.delete();
    }
  }
  // the arrows inside one top-level block: its plain blocks are the obstacles
  const inners = [];
  for (const r in inside) {
    const Ri = router(), refs = {};
    for (const id in S) if (id.startsWith(r + '.') && !S[id].kids) refs[id] = shape(Ri, S[id]);
    const end = (id, other) => {
      if (refs[id]) return new A.ConnEnd(refs[id], 1);
      const b = S[id], [ox, oy] = mid(S[other]), side = facing(b, ox, oy), [ex, ey] = edgeOf(b, side), [cx, cy] = mid(b);
      return at(ex ?? cx, ey ?? cy, side);
    };
    for (const e of inside[r]) { const s = end(e.a, e.b), t = end(e.b, e.a); inners.push([e, new A.ConnRef(Ri, s, t)]); s.delete(); t.delete(); }
    // (a container's end starts OUT beyond its border; the gap is too small to matter inside a block)
    Ri.processTransaction(); inners.push([null, Ri]);
  }
  R.processTransaction();
  const route = c => { const r = c.displayRoute(), pts = []; for (let i = 0; i < r.size(); i++) { const p = r.at(i); pts.push([p.x, p.y]); } return pts; };
  // labels: of an arrow between blocks only in the room between them; of one inside a block off its plain blocks
  // a container's own title is taken too: a label over it hides the block's name
  const titles = Object.values(S).filter(s => s.kids).map(s => s.g.querySelector(':scope > text')).filter(Boolean)
    .map(t => { const b = t.getBBox(); return [b.x - 2, b.y - 1, b.x + b.width + 2, b.y + b.height + 1]; });
  const busy = Object.values(S).filter(s => !s.kids).map(s => [s.x, s.y, s.x + s.w, s.y + s.h]).concat(titles);
  const outside = busy.concat(Object.values(S).filter(s => !s.parent).map(s => [s.x, s.y, s.x + s.w, s.y + s.h]));
  const put = (e, pts, room) => {
    if (pts.length < 2) return;
    e.path.setAttribute('d', d(pts));
    e.g.querySelector(':scope > path.hit')?.setAttribute('d', d(pts));
    if (e.label && room) jobs.push([e.label, pts, room, e]);
  };
  const jobs = [];   // labels go once every route is drawn, off the other arrows' lines too
  const trunks = new Map(conns.map(([g, c]) => [c, route(c)]));
  spread([...trunks.values()], Object.values(S).filter(s => !s.parent));
  for (const [g, c, hub] of conns) {
    const trunk = trunks.get(c);
    for (const e of g) {
      const pts = trunk.slice();
      if (e.pa) pts.unshift([e.pa.x, e.pa.y]);
      if (hub) pts.push(beyond(e.pb.x, e.pb.y, e.pb.side));
      else if (e.pb && pts.length > 1) {
        // the pin an arrow reaches is its own: it moves to where the nudged route comes in, so the arrow meets the
        // border square on, its head not turned aside by a step along the border
        const [q, r] = pts.slice(-2), vert = e.pb.side === 'l' || e.pb.side === 'r';
        if (vert && Math.abs(q[1] - r[1]) < .5) e.pb.y = r[1];
        if (!vert && Math.abs(q[0] - r[0]) < .5) e.pb.x = r[0];
      }
      if (e.pb) pts.push([e.pb.x, e.pb.y]);
      put(e, square(pts), g.length > 1 ? null : outside);
    }
    // a fork has one label on its trunk, a line per branch
    const t = g.length > 1 && g.find(e => e.label)?.label, texts = [...new Set(g.map(e => e.label?.textContent.trim()).filter(Boolean))];
    if (t && texts.length) {
      for (const e of g) if (e.label && e.label !== t) e.label.style.display = 'none';
      t.textContent = '';
      texts.forEach((x, i) => { const ts = el('tspan', {dy: i ? '1.15em' : '0', x: t.getAttribute('x') || 0}); ts.textContent = x; t.append(ts); });
      jobs.push([t, g[0].pa ? [[g[0].pa.x, g[0].pa.y], ...trunk] : trunk, outside, g]);
    }
  }
  for (const [e, c] of inners) if (e) put(e, route(c), busy);
  const ways = live.map(e => [e, (e.path.getAttribute('d').match(/[\d.-]+ [\d.-]+/g) || []).map(p => p.split(' ').map(Number))]);
  for (const [t, pts, room, own] of jobs) {
    const mine = new Set([].concat(own)), lines = [];
    for (const [e, ps] of ways) if (!mine.has(e)) for (let i = 1; i < ps.length; i++)
      lines.push([Math.min(ps[i - 1][0], ps[i][0]) - 2, Math.min(ps[i - 1][1], ps[i][1]) - 2, Math.max(ps[i - 1][0], ps[i][0]) + 2, Math.max(ps[i - 1][1], ps[i][1]) + 2]);
    const b = label(t, pts, room.concat(lines)); if (b) { busy.push(b); outside.push(b); }
  }

  // the leads inside the blocks and the pins on their borders; they belong to their arrows (data-of)
  const top = el('g', {class: 'pins'}); inner.append(top);
  for (const k in pins) for (const p of pins[k]) {
    const users = ext.filter(e => e.pa === p || e.pb === p), first = users[0], color = first.path.getAttribute('stroke') || '#8fa6c0';
    const of = users.map(e => e.g.classList[0]).join(' ');
    const b = S[p.end], [cx, cy] = mid(b), vert = p.side === 'l' || p.side === 'r';
    const from = vert ? [p.side === 'r' ? b.x + b.w : b.x, cy] : [cx, p.side === 'b' ? b.y + b.h : b.y];
    const bend = p.side === 'r' ? p.x - IN : p.side === 'l' ? p.x + IN : p.side === 'b' ? p.y - IN : p.y + IN;
    const pts = vert ? [from, [bend, from[1]], [bend, p.y], [p.x, p.y]] : [from, [from[0], bend], [p.x, bend], [p.x, p.y]];
    const lead = el('path', {d: d(pts), class: 'lead', fill: 'none', stroke: color, 'data-of': of});
    const holder = S[b.parent] || S[p.R.id];
    holder.g.after(lead);   // over the fills of the blocks round the port, under the ports themselves
    // a pin an arrow leaves from is a square on the border; where one comes in, its head marks the place
    if (p.out) top.append(el('rect', {x: p.x - 3.5, y: p.y - 3.5, width: 7, height: 7, fill: color, class: 'pin', 'data-of': of}));
  }
  R.delete(); for (const [e, c] of inners) if (!e) c.delete();
}

// libavoid packs lanes running side by side into a few pixels where they end near one point (they merge into one).
// Stretches of the routes — not the first, which leaves a pin square on (the last's joints are squared again) — that run side by side closer than LANE and
// overlap are spread LANE apart, the order kept, from the one nearest the blocks they lead to; the stretches of one
// fork, at one place, move as one lane. A cluster whose new places would touch a block stays as it was.
const LANE = 18;
function spread(routes, blocks) {
  for (const axis of [0, 1]) {   // 0: vertical stretches (x shared), 1: horizontal (y shared)
    // a lane: the stretches at one place (a fork's move as one), with the span they cover
    const lanes = new Map();
    for (const r of routes) for (let i = 1; i < r.length - 1; i++) {
      const a = r[i], b = r[i + 1];
      if (Math.abs(a[axis] - b[axis]) >= .5) continue;
      const k = Math.round(a[axis] * 2) / 2, l = lanes.get(k) || {at: a[axis], lo: Infinity, hi: -Infinity, segs: []};
      l.lo = Math.min(l.lo, a[1 - axis], b[1 - axis]); l.hi = Math.max(l.hi, a[1 - axis], b[1 - axis]); l.segs.push([r, i]); lanes.set(k, l);
    }
    // the room a lane has: up to the nearest block on each side within its own span
    const across = (b, l) => axis === 0 ? l.lo < b.y + b.h && b.y < l.hi : l.lo < b.x + b.w && b.x < l.hi;
    const near = b => axis === 0 ? [b.x, b.x + b.w] : [b.y, b.y + b.h];
    for (const l of lanes.values()) {
      l.min = -Infinity; l.max = Infinity;
      for (const b of blocks) if (across(b, l)) { const [p, q] = near(b); if (q <= l.at) l.min = Math.max(l.min, q + 8); else if (p >= l.at) l.max = Math.min(l.max, p - 8); }
    }
    // from the far side down: each lane LANE short of the lanes beside it that it runs along, never past its room
    const placed = [];
    for (const l of [...lanes.values()].sort((p, q) => q.at - p.at)) {
      let to = l.at;
      for (const o of placed) if (l.lo < o.hi && o.lo < l.hi && to > o.to - LANE) to = o.to - LANE;
      to = Math.max(to, Math.min(l.min, l.at));
      l.to = to; placed.push(l);
      for (const [r, i] of l.segs) { r[i][axis] = to; r[i + 1][axis] = to; }
    }
  }
}

// an arrow's label goes to the first free spot along its route, the longest stretches first
function label(t, pts, busy) {
  const b = t.getBBox(), w = b.width + 4, h = b.height, spots = [];
  const segs = pts.slice(1).map((p, i) => [pts[i], p]).sort((u, v) => (Math.abs(v[1][0] - v[0][0]) + Math.abs(v[1][1] - v[0][1])) - (Math.abs(u[1][0] - u[0][0]) + Math.abs(u[1][1] - u[0][1])));
  // beside the line first, then a little further off it when the room by the line is taken
  for (const k of [0, 12, 24]) for (const [a, c] of segs) for (const f of [.5, .3, .7]) {
    const mx = a[0] + (c[0] - a[0]) * f, my = a[1] + (c[1] - a[1]) * f;
    if (a[1] === c[1]) spots.push([mx - w / 2, my - h - 3 - k], [mx - w / 2, my + 3 + k]); else spots.push([mx + 5 + k, my - h / 2], [mx - 5 - k - w, my - h / 2]);
  }
  // the first free spot along the route; with none, the one overlapping least
  const over = r => busy.reduce((a, q) => a + Math.max(0, Math.min(r[2], q[2]) - Math.max(r[0], q[0])) * Math.max(0, Math.min(r[3], q[3]) - Math.max(r[1], q[1])), 0);
  let s = null, least = Infinity;
  for (const [x, y] of spots) { const o = over([x, y, x + w, y + h]); if (o < least) { least = o; s = [x, y]; } if (!o) break; }
  if (!s) return null;
  const lines = t.querySelectorAll('tspan');
  t.setAttribute('text-anchor', 'start'); t.setAttribute('x', s[0] + 2); t.setAttribute('y', s[1] + (lines.length ? h / lines.length : h) * .8);
  for (const ts of lines) ts.setAttribute('x', s[0] + 2);
  t.style.paintOrder = 'stroke'; t.style.stroke = '#0f1216'; t.style.strokeWidth = '4px'; t.style.strokeLinejoin = 'round';
  return [s[0], s[1], s[0] + w, s[1] + h];
}

// a board is drawn when it is first shown: a hidden one has no sizes to place labels by
window.edgesDraw = b => { if (!A || !b || b.dataset.drawn || !b.classList.contains('placed')) return; b.dataset.drawn = 1;
  try { board(b); } catch (e) { console.warn('edges:', e); } };
(async () => {
  try { await avoid(); } catch (e) { console.warn('edges: no libavoid', e); return; }
  window.edgesDraw(document.querySelector('.board.on'));
  window.dispatchEvent(new Event('edges'));
})();
