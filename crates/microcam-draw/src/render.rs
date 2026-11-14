//! The scene (`microcam_core::scene`) for one output: cut into items, each rasterised by the CPU into its own tile
//! (§4). An item is a leaf node, or a group that must be composited as a whole (opacity,
//! mask, path clip, a kept drawing); its tile is premultiplied RGBA at the output's pixels. Items are drawn over each
//! other in order by whoever owns the output's layer (GE on the camera, [`compose`] here).
//!
//! An item carries its drawing as paths already in the tile's pixels, so two items with the same [`Item::key`] draw the
//! same tile wherever they are: an element moved by whole pixels keeps its tile.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use microcam_core::scene::{self, Align, Cap, Clip, Colour, Fit, FillRule, Gradient, Join, Kind, Node, Paint, Scene, Seg, Space, Style};
use microcam_core::protocol::{Measure, MeasuredLine};
use microcam_core::ui::OutputUi;
use microcam_core::Rect;
use tiny_skia as sk;

/// The built-in font: Noto Sans Regular, Latin + Cyrillic (fonts/OFL.txt).
const FONT: &[u8] = include_bytes!("../fonts/NotoSans-microcam.ttf");
/// Arrows, triangles and check marks Noto Sans lacks: DejaVu Sans (fonts/DejaVu-LICENSE.txt).
const SYMBOLS: &[u8] = include_bytes!("../fonts/DejaVuSans-symbols.ttf");

/// The built-in faces and their glyph outlines in font units, kept once read.
pub struct Fonts {
    faces: [ttf_parser::Face<'static>; 2],
    /// line box height and ascent, in ems of the main face
    line: f32,
    ascent: f32,
    outlines: HashMap<(u8, u16), Option<sk::Path>>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let main = ttf_parser::Face::parse(FONT, 0).expect("built-in font");
        let symbols = ttf_parser::Face::parse(SYMBOLS, 0).expect("built-in symbols font");
        let upem = main.units_per_em() as f32;
        let line = (main.ascender() as f32 - main.descender() as f32 + main.line_gap() as f32) / upem;
        let ascent = main.ascender() as f32 / upem;
        Self { faces: [main, symbols], line, ascent, outlines: HashMap::new() }
    }

    /// The face that has `c` (the main one, else the symbols, else the main one's empty box) and its glyph.
    fn glyph(&self, c: char) -> (u8, ttf_parser::GlyphId) {
        if let Some(g) = self.faces[0].glyph_index(c) {
            return (0, g);
        }
        match self.faces[1].glyph_index(c) {
            Some(g) => (1, g),
            None => (0, ttf_parser::GlyphId(0)),
        }
    }

    /// A glyph's advance in ems (each face by its own units).
    fn advance(&self, c: char) -> f32 {
        let (f, g) = self.glyph(c);
        let face = &self.faces[f as usize];
        face.glyph_hor_advance(g).unwrap_or(0) as f32 / face.units_per_em() as f32
    }

    /// A glyph's outline in ems, y up; None for an empty glyph.
    fn outline(&mut self, c: char) -> Option<sk::Path> {
        let (f, g) = self.glyph(c);
        let face = &self.faces[f as usize];
        self.outlines
            .entry((f, g.0))
            .or_insert_with(|| {
                let mut b = Outline(sk::PathBuilder::new());
                face.outline_glyph(g, &mut b)?;
                let k = 1.0 / face.units_per_em() as f32;
                b.0.finish()?.transform(sk::Transform::from_scale(k, k))
            })
            .clone()
    }

    /// Text width in ems.
    fn width(&self, text: &str) -> f32 {
        text.chars().map(|c| self.advance(c)).sum()
    }
}

struct Outline(sk::PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// One drawing step in a tile's pixels.
#[derive(Clone, Debug)]
enum Op {
    Fill { path: sk::Path, paint: Fill, rule: sk::FillRule },
    /// a group composited as a whole: its ops drawn apart, cut by the clip and the mask's alpha, laid over at `opacity`
    Layer { ops: Vec<Op>, opacity: f32, clip: Option<LayerClip>, mask: Option<Vec<Op>> },
}

/// A layer's clip in tile pixels; a rectangle of an axis-aligned group is whole pixels with hard edges, as the same
/// clip is outside a layer.
#[derive(Clone, Debug)]
struct LayerClip {
    path: sk::Path,
    rule: sk::FillRule,
    anti_alias: bool,
}

#[derive(Clone, Debug)]
enum Fill {
    Solid(Colour),
    /// the scene's gradient and the transform from its node's coordinates to the tile's pixels
    Gradient(Gradient, sk::Transform),
    /// a picture's pixels, its key, the transform from them to the tile's pixels, and the part of them sampled (a crop
    /// takes nothing from beyond its edges)
    Picture(Arc<dyn Pixels>, u64, sk::Transform, Option<sk::IntRect>),
}

/// A picture's pixels wherever they are kept (the media heap on the camera): premultiplied R, G, B, A, rows of exactly
/// four bytes a pixel, and the physical address the 2D engine reads them at, when it can.
pub trait Pixels: Send + Sync + std::fmt::Debug {
    fn size(&self) -> (u32, u32);
    fn rgba(&self) -> &[u8];
    fn phy(&self) -> Option<u64> {
        None
    }
}

impl Pixels for sk::Pixmap {
    fn size(&self) -> (u32, u32) {
        (self.width(), self.height())
    }
    fn rgba(&self) -> &[u8] {
        self.data()
    }
}

fn pixmap(p: &dyn Pixels) -> Result<sk::PixmapRef<'_>, String> {
    let (w, h) = p.size();
    sk::PixmapRef::from_bytes(p.rgba(), w, h).ok_or_else(|| format!("a picture of {w}x{h} with {} bytes", p.rgba().len()))
}

/// A resource's picture as the renderer gets it: the source's own size, which `crop` refers to, and its pixels
/// (premultiplied, perhaps a scaled-down decode of the source; none yet while it is being decoded) with a key that
/// changes whenever they do.
#[derive(Clone)]
pub struct Picture {
    pub full: (u32, u32),
    pub pix: Option<(Arc<dyn Pixels>, u64)>,
}

/// The most output pixels a picture pixel may span for the 2D engine to lay the picture itself; a picture shown larger
/// is drawn into a tile like any other item.
pub const REACH: u32 = 8;

/// An image laid straight from its picture, scaled, with no tile of its own: the part of the picture taken (its pixels)
/// and where that part lands relative to the item's rect, which cuts it (it may reach past the rect by less than
/// [`REACH`] pixels each side).
#[derive(Clone, Debug)]
pub struct Placed {
    pub pix: Arc<dyn Pixels>,
    pub src: Rect,
    pub dst: (i32, i32, u32, u32),
}

/// One image node shown on the output: its resource, the output pixels per source pixel it is shown at (what a decode
/// should reach; 0 when the resource has no picture), and whether it was drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub res: String,
    pub scale: f32,
    pub drawn: bool,
}

/// What the walk needs besides the scene: the fonts, the resources' pictures, and what it records of their use.
struct Cx<'a> {
    fonts: &'a mut Fonts,
    pictures: &'a dyn Fn(&str) -> Option<Picture>,
    /// the image nodes the output shows in its frame, by their place in the tree, and their windows
    frame: &'a [microcam_core::frame::Picture],
    uses: Vec<Use>,
}

/// One item of an output's scene: a tile at a place.
#[derive(Clone, Debug)]
pub struct Item {
    /// the tile's content: equal keys draw equal tiles
    pub key: u64,
    /// the tile's place in the output frame (whole pixels, inside it)
    pub rect: Rect,
    /// the group asking for a hardware window of its own this item belongs to (`scene::Window::Own`), by an id stable
    /// across revisions: the group's `id`, or its place in the tree
    pub own: Option<u64>,
    /// the item is one opaque colour over all of its rect (a plain rectangle on whole pixels): that colour, RGB
    pub solid: Option<[u8; 3]>,
    /// the opacity its tile is laid over at (a group's `opacity`, kept out of the tile so a change costs no rasterising)
    pub fade: f32,
    /// the tile is opaque over all of its rect
    pub opaque: bool,
    /// an image node laid from its picture: no tile
    pub image: Option<Placed>,
    /// a full-colour image shown in the video frame under the layer (`microcam_core::frame`): the layer is cleared to
    /// transparent over its rect at its place in z, so what lies below it in the layer does not show over the picture
    pub hole: bool,
    /// a flat shape whose tile is small, laid over its rect in pieces ([`Item::pieces`])
    pub flat: Option<Flat>,
    ops: Arc<Vec<Op>>,
}

/// A flat shape's small tile (.3): a rectangle of one colour, its corners rounded or not, upright. Along
/// each axis the tile holds the shape's two ends and between them three equal middle columns (rows), the middle one
/// stretched over the rest of the rect — three, so a filtered stretch reads only its equal neighbours; or, along an axis
/// too short for that (a pill's height), all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flat {
    /// the tile's size
    pub tile: (u32, u32),
    /// the left and top ends' size: the middle columns and rows start there; the whole length along an axis kept whole
    pub corner: (u32, u32),
}

/// The middle columns (rows) of a flat tile.
const FLAT_MIDDLE: u32 = 3;

impl Item {
    /// The size of its tile: its rect's, or a flat shape's small one.
    pub fn tile_size(&self) -> (u32, u32) {
        self.flat.map_or((self.rect.w, self.rect.h), |f| f.tile)
    }

    /// What draws `part` of the item (item pixels): pairs of its tile's part and where that lands in the item, the
    /// same size or a single middle column, row or pixel stretched. One pair, `part` itself, for a tile the rect's size.
    pub fn pieces(&self, part: Rect) -> Vec<(Rect, Rect)> {
        let Some(f) = self.flat else {
            return vec![(part, part)];
        };
        // along one axis: the three spans of the item and of the tile they come from (start, length)
        let spans = |n: u32, tile: u32, c0: u32| {
            if tile == n {
                return vec![((0, n), (0, n))];
            }
            let c1 = tile - c0 - FLAT_MIDDLE;
            vec![((0, c0), (0, c0)), ((c0, n - c0 - c1), (c0 + 1, 1)), ((n - c1, c1), (c0 + FLAT_MIDDLE, c1))]
        };
        let mut out = Vec::new();
        let rows = spans(self.rect.h, f.tile.1, f.corner.1);
        for ((dx, dw), (sx, sw)) in spans(self.rect.w, f.tile.0, f.corner.0) {
            for &((dy, dh), (sy, sh)) in &rows {
                let d = Rect { x: dx, y: dy, w: dw, h: dh }.intersect(part);
                if d.is_empty() {
                    continue;
                }
                // a corner's span is the tile's one to one; a middle one is the one column (row)
                let (x, w) = if sw == dw { (sx + d.x - dx, d.w) } else { (sx, 1) };
                let (y, h) = if sh == dh { (sy + d.y - dy, d.h) } else { (sy, 1) };
                out.push((Rect { x, y, w, h }, d));
            }
        }
        out
    }
}

impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key && self.rect == o.rect && self.fade == o.fade
    }
}

/// Where a node's coordinates land in the output.
#[derive(Clone, Copy)]
struct Ctx {
    /// node coordinates → output pixels, for geometry
    t: sk::Transform,
    /// screen units of the node → pixels, linear only: text and stroke lengths (in screen space the linear part of `t`;
    /// in live space without the window's non-uniform stretch)
    lin: sk::Transform,
    /// pixels per screen unit of the node for stroke widths and dashes
    s: f32,
    level: u8,
    /// the output's live picture is paused: its `paused` nodes show
    paused: bool,
    /// the part of the output this node may touch
    clip: sk::IntRect,
    /// the node's place in the tree, for the id of an own-window group without an `id`
    path: u64,
    /// the own-window group this node is in
    own: Option<u64>,
}

/// The scene's items for `out`, bottom to top: what shows at its level, in its pixels (a scene without images).
pub fn items(scene: &Scene, out: &OutputUi, fonts: &mut Fonts) -> Vec<Item> {
    shown(scene, out, fonts, &|_| None, &[]).0
}

/// The scene's items for `out` with the resources' pictures, and the image nodes shown in the layer with what each
/// needs; the images of `frame` (the output's frame, `microcam_core::frame::plane`) are holes in the layer.
pub fn shown(scene: &Scene, out: &OutputUi, fonts: &mut Fonts, pictures: &dyn Fn(&str) -> Option<Picture>, frame: &[microcam_core::frame::Picture]) -> (Vec<Item>, Vec<Use>) {
    let mut cx = Cx { fonts, pictures, frame, uses: Vec::new() };
    let (w, h) = (out.raster.w as f32, out.raster.h as f32);
    let unit = h / 1080.0;
    let paused = out.paused.is_some();
    let Some(full) = sk::IntRect::from_xywh(0, 0, out.raster.w, out.raster.h) else {
        return (Vec::new(), Vec::new());
    };
    let mut items = Vec::new();
    for (k, n) in scene.nodes.iter().enumerate() {
        if matches!(n.kind, Kind::Live(_)) {
            continue;
        }
        let lin = sk::Transform::from_scale(unit, unit);
        let path = k as u64;
        // a top-level node without a level shows with the full menu only
        let base = if n.level.is_some() { 0 } else { scene::MAX_LEVEL };
        let ctx = match &n.kind {
            Kind::Group(g) if g.space == Space::Live => {
                let l = out.live;
                let Some(clip) = sk::IntRect::from_xywh(l.x as i32, l.y as i32, l.w, l.h).and_then(|c| c.intersect(&full)) else {
                    continue;
                };
                let t = sk::Transform::from_row(l.w as f32, 0.0, 0.0, l.h as f32, l.x as f32, l.y as f32);
                Ctx { t, lin, s: unit, level: base, paused, clip, path, own: None }
            }
            Kind::Group(g) => {
                let (ax, ay) = g.anchor.point(w, h);
                Ctx { t: lin.post_translate(ax, ay), lin, s: unit, level: base, paused, clip: full, path, own: None }
            }
            _ => Ctx { t: lin, lin, s: unit, level: base, paused, clip: full, path, own: None },
        };
        walk(n, ctx, out.level, &mut cx, &mut items);
    }
    (items, cx.uses)
}

/// The node's own level and visibility applied: the context it draws in, or None when it does not show.
fn shows(n: &Node, ctx: Ctx, level: u8) -> Option<Ctx> {
    n.shows(ctx.level, level, ctx.paused).map(|l| Ctx { level: l, ..ctx })
}

fn group_ctx(g: &scene::Group, ctx: Ctx) -> Ctx {
    let (sx, sy) = g.scale.xy();
    let local = sk::Transform::from_translate(g.x, g.y).pre_rotate(g.rotate).pre_scale(sx, sy);
    let lin_local = sk::Transform::from_rotate(g.rotate).pre_scale(sx, sy);
    let det = (sx * sy).abs().sqrt();
    Ctx { t: ctx.t.pre_concat(local), lin: ctx.lin.pre_concat(lin_local), s: ctx.s * det, ..ctx }
}

fn walk(n: &Node, ctx: Ctx, level: u8, cx: &mut Cx, out: &mut Vec<Item>) {
    let Some(ctx) = shows(n, ctx, level) else {
        return;
    };
    // a picture prepared ahead is drawn nowhere
    if matches!(&n.kind, Kind::Image(m) if m.prepare) {
        return;
    }
    if let Some(p) = cx.frame.iter().find(|p| p.node == ctx.path && matches!(n.kind, Kind::Image(_))) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ("hole", p.window).hash(&mut h);
        out.push(Item { key: h.finish(), rect: p.window, own: ctx.own, solid: None, fade: 1.0, opaque: false, image: None, hole: true, flat: None, ops: Arc::new(Vec::new()) });
        return;
    }
    // a full-colour picture shows only in the frame: not there yet (its form coming) or not at all (failed), it draws
    // nothing — in the layer it would cost more than its form in the frame
    if matches!(&n.kind, Kind::Image(m) if m.full_colour) {
        return;
    }
    let Kind::Group(g) = &n.kind else {
        if let Some((ops, rect, f)) = flat(n, ctx) {
            let before = out.len();
            push(ops, ctx.clip, ctx.own, out);
            if let Some(it) = out.get_mut(before) {
                it.rect = rect;
                it.flat = Some(f);
                it.solid = solid(n, ctx);
                it.opaque = it.solid.is_some();
            }
            return;
        }
        let ops = leaf(n, ctx, cx);
        let before = out.len();
        push(ops, ctx.clip, ctx.own, out);
        if let Some(it) = out.get_mut(before) {
            it.solid = solid(n, ctx);
            it.opaque = it.solid.is_some();
            if matches!(n.kind, Kind::Image(_)) {
                place(it);
            }
        }
        return;
    };
    let mut inner = group_ctx(g, ctx);
    if g.window == scene::Window::Own {
        inner.own = Some(own_id(n, ctx.path));
    }
    let axis_aligned = inner.t.kx == 0.0 && inner.t.ky == 0.0;
    let isolated = g.opacity < 1.0 || g.mask.is_some() || g.keep || matches!(g.clip, Some(Clip::Path { .. })) || (g.clip.is_some() && !axis_aligned);
    if isolated {
        // a drawing to keep stays at no opacity too, an item showing nothing whose tile is kept
        if g.opacity <= 0.0 && !g.keep {
            return;
        }
        // drawn whole: the group's opacity is laid over when its tile is composed
        let mut ops = layer(g, inner, level, cx);
        if let Some(Op::Layer { opacity, .. }) = ops.first_mut() {
            *opacity = 1.0;
        }
        let before = out.len();
        push(ops, ctx.clip, inner.own, out);
        if let Some(it) = out.get_mut(before) {
            it.fade = g.opacity.clamp(0.0, 1.0);
            it.opaque = opaque_under(g, inner, level, it.rect);
        }
        return;
    }
    if let Some(Clip::Rect { x, y, w, h }) = g.clip {
        match pixel_rect(x, y, w, h, inner.t).and_then(|r| r.intersect(&inner.clip)) {
            Some(c) => inner.clip = c,
            None => return,
        }
    }
    for (i, c) in g.children.iter().enumerate() {
        walk(c, Ctx { path: mix(inner.path, i as u64), ..inner }, level, cx, out);
    }
}

/// A rectangle of one colour (any alpha), its corners rounded or not, upright and whole inside its clip, whose tile
/// would be larger than a flat one: the ops of the same shape cut down to the flat tile by whole pixels (its corners
/// and edges where they were in their pixels), its rect in the output, its [`Flat`].
fn flat(n: &Node, ctx: Ctx) -> Option<(Vec<Op>, Rect, Flat)> {
    let Kind::Rect(r) = &n.kind else {
        return None;
    };
    if !matches!(r.style.fill, Some(Paint::Solid(_))) || r.style.stroke.is_some() || ctx.t.kx != 0.0 || ctx.t.ky != 0.0 {
        return None;
    }
    let d = sk::Rect::from_xywh(r.x, r.y, r.w, r.h)?.transform(ctx.t)?;
    let (sx, sy) = (ctx.t.sx.abs(), ctx.t.sy.abs());
    let rad = r.r.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    // along one axis: the rect's pixels, the end before the middle, the tile's length (all of it when too short)
    let axis = |lo: f32, hi: f32, rad: f32| {
        let (p0, p1) = ((lo + 1e-3).floor() as i64, (hi - 1e-3).ceil() as i64);
        let c0 = ((lo + rad - 1e-3).ceil() as i64 - p0).max(0);
        let c1 = (p1 - (hi - rad + 1e-3).floor() as i64).max(0);
        let t = c0 + FLAT_MIDDLE as i64 + c1;
        if t < p1 - p0 { (p0, p1 - p0, c0, t) } else { (p0, p1 - p0, p1 - p0, p1 - p0) }
    };
    let (x0, w, cl, tw) = axis(d.left(), d.right(), rad * sx);
    let (y0, h, ct, th) = axis(d.top(), d.bottom(), rad * sy);
    let (c, rect) = (ctx.clip, Rect { x: x0.max(0) as u32, y: y0.max(0) as u32, w: w.max(0) as u32, h: h.max(0) as u32 });
    let inside = x0 >= c.x() as i64 && y0 >= c.y() as i64 && x0 + w <= c.right() as i64 && y0 + h <= c.bottom() as i64;
    if !inside || tw * th >= w * h {
        return None;
    }
    let (cw, ch) = (r.w - (w - tw) as f32 / sx, r.h - (h - th) as f32 / sy);
    let ops = styled(rounded(r.x, r.y, cw, ch, rad), &r.style, ctx);
    Some((ops, rect, Flat { tile: (tw as u32, th as u32), corner: (cl as u32, ct as u32) }))
}

/// A rectangle drawn as one opaque colour with hard edges on whole pixels: that colour.
fn solid(n: &Node, ctx: Ctx) -> Option<[u8; 3]> {
    let Kind::Rect(r) = &n.kind else {
        return None;
    };
    let Some(Paint::Solid(Colour([cr, cg, cb, 255]))) = r.style.fill else {
        return None;
    };
    if r.r > 0.0 || r.style.stroke.is_some() || ctx.t.kx != 0.0 || ctx.t.ky != 0.0 {
        return None;
    }
    let d = sk::Rect::from_xywh(r.x, r.y, r.w, r.h)?.transform(ctx.t)?;
    let whole = |v: f32| (v - v.round()).abs() < 1e-3;
    [d.left(), d.top(), d.right(), d.bottom()].into_iter().all(whole).then_some([cr, cg, cb])
}

/// Whether an isolated group's tile is opaque over all of `rect`: a child of it is a solid rectangle covering `rect`,
/// and no mask or soft clip thins it.
fn opaque_under(g: &scene::Group, inner: Ctx, level: u8, rect: Rect) -> bool {
    let axis_aligned = inner.t.kx == 0.0 && inner.t.ky == 0.0;
    if g.mask.is_some() || matches!(g.clip, Some(Clip::Path { .. })) || (g.clip.is_some() && !axis_aligned) {
        return false;
    }
    g.children.iter().any(|c| {
        let (Some(cc), Kind::Rect(r)) = (shows(c, inner, level), &c.kind) else {
            return false;
        };
        solid(c, cc).is_some()
            && pixel_rect(r.x, r.y, r.w, r.h, cc.t).is_some_and(|d| {
                let (x0, y0) = (d.x().max(0) as u32, d.y().max(0) as u32);
                Rect { x: x0, y: y0, w: (d.right().max(0) as u32).saturating_sub(x0), h: (d.bottom().max(0) as u32).saturating_sub(y0) }.contains(rect)
            })
    })
}

/// An own-window group's id: its `id` when it has one, else its place in the tree.
fn own_id(n: &Node, path: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match &n.id {
        Some(id) => id.hash(&mut h),
        None => path.hash(&mut h),
    }
    h.finish()
}

use microcam_core::scene::child_path as mix;

/// A rectangle of node coordinates on an axis-aligned group as whole pixels of the output (edges rounded).
fn pixel_rect(x: f32, y: f32, w: f32, h: f32, t: sk::Transform) -> Option<sk::IntRect> {
    let r = sk::Rect::from_xywh(x, y, w, h)?.transform(t)?;
    sk::IntRect::from_ltrb(r.left().round() as i32, r.top().round() as i32, r.right().round() as i32, r.bottom().round() as i32)
}

/// A group drawn as one: its children (nested groups as layers of their own), its clip and mask.
fn layer(g: &scene::Group, inner: Ctx, level: u8, cx: &mut Cx) -> Vec<Op> {
    let mut ops = Vec::new();
    for c in &g.children {
        ops.extend(node_ops(c, inner, level, cx));
    }
    let axis_aligned = inner.t.kx == 0.0 && inner.t.ky == 0.0;
    let clip = match &g.clip {
        None => None,
        Some(c) => {
            let (device, anti_alias) = match c {
                Clip::Rect { x, y, w, h } if axis_aligned => {
                    (pixel_rect(*x, *y, *w, *h, inner.t).map(|r| sk::PathBuilder::from_rect(r.to_rect())), false)
                }
                Clip::Rect { x, y, w, h } => (sk::Rect::from_xywh(*x, *y, *w, *h).map(sk::PathBuilder::from_rect).and_then(|p| p.transform(inner.t)), true),
                Clip::Path { d } => (path(&d.segs).and_then(|p| p.transform(inner.t)), true),
            };
            // an empty clip shows nothing
            match device {
                Some(path) => Some(LayerClip { path, rule: sk::FillRule::Winding, anti_alias }),
                None => return Vec::new(),
            }
        }
    };
    let mask = g.mask.as_ref().map(|m| node_ops(m, Ctx { level: 0, paused: true, ..inner }, scene::MAX_LEVEL, cx));
    vec![Op::Layer { ops, opacity: g.opacity, clip, mask }]
}

/// Any node as ops in output pixels (inside a layer).
fn node_ops(n: &Node, ctx: Ctx, level: u8, cx: &mut Cx) -> Vec<Op> {
    let Some(ctx) = shows(n, ctx, level) else {
        return Vec::new();
    };
    match &n.kind {
        Kind::Group(g) => {
            let inner = group_ctx(g, ctx);
            if g.opacity < 1.0 || g.mask.is_some() || g.clip.is_some() {
                layer(g, inner, level, cx)
            } else {
                g.children.iter().flat_map(|c| node_ops(c, inner, level, cx)).collect()
            }
        }
        _ => leaf(n, ctx, cx),
    }
}

fn leaf(n: &Node, ctx: Ctx, cx: &mut Cx) -> Vec<Op> {
    let shape = |p: Option<sk::Path>, st: &Style| styled(p, st, ctx);
    match &n.kind {
        Kind::Group(_) => unreachable!("groups are walked"),
        Kind::Rect(r) => shape(rounded(r.x, r.y, r.w, r.h, r.r), &r.style),
        Kind::Ellipse(e) => shape(sk::Rect::from_xywh(e.cx - e.rx, e.cy - e.ry, 2.0 * e.rx, 2.0 * e.ry).and_then(|o| {
            let mut b = sk::PathBuilder::new();
            b.push_oval(o);
            b.finish()
        }), &e.style),
        Kind::Arc(a) => shape(arc(a.cx, a.cy, a.r, a.start, a.sweep, a.style.fill.is_some()), &a.style),
        Kind::Path(p) => shape(path(&p.d.segs), &p.style),
        Kind::Text(t) => text(t, ctx, cx.fonts),
        Kind::Image(m) => image(m, ctx, cx),
        // where the live picture lies: a window of the frame, nothing of the overlay
        Kind::Live(_) => Vec::new(),
    }
}

/// An image node: its source rectangle fitted into its own, filled with the picture's pixels; nothing while the
/// resource has no picture. Its use is recorded either way.
fn image(m: &scene::ImageNode, ctx: Ctx, cx: &mut Cx) -> Vec<Op> {
    let Some(p) = (cx.pictures)(&m.res) else {
        cx.uses.push(Use { res: m.res.clone(), scale: 0.0, drawn: false });
        return Vec::new();
    };
    let (fw, fh) = (p.full.0 as f32, p.full.1 as f32);
    let [cx0, cy0, cw, ch] = m.crop.unwrap_or([0.0, 0.0, fw, fh]);
    let (sx, sy) = match m.fit {
        Fit::Stretch => (m.w / cw, m.h / ch),
        Fit::Contain => (m.w / cw).min(m.h / ch).pipe(|s| (s, s)),
        Fit::Cover => (m.w / cw).max(m.h / ch).pipe(|s| (s, s)),
    };
    // the source rectangle's place in the node's rectangle, centred
    let (dw, dh) = (cw * sx, ch * sy);
    let (dx, dy) = (m.x + (m.w - dw) / 2.0, m.y + (m.h - dh) / 2.0);
    // what shows: the placed source cut by the node's rectangle
    let (l, t) = (dx.max(m.x), dy.max(m.y));
    let (r, b) = ((dx + dw).min(m.x + m.w), (dy + dh).min(m.y + m.h));
    let (ax, ay) = ((ctx.t.sx * ctx.t.sx + ctx.t.ky * ctx.t.ky).sqrt(), (ctx.t.kx * ctx.t.kx + ctx.t.sy * ctx.t.sy).sqrt());
    let scale = (sx * ax).max(sy * ay);
    let (Some((pix, key)), Some(path)) = (p.pix, sk::Rect::from_ltrb(l, t, r, b).map(sk::PathBuilder::from_rect).and_then(|p| p.transform(ctx.t))) else {
        cx.uses.push(Use { res: m.res.clone(), scale, drawn: false });
        return Vec::new();
    };
    cx.uses.push(Use { res: m.res.clone(), scale, drawn: true });
    // the decode's pixels → the source's → the node's coordinates → the output's
    let (pw, ph) = pix.size();
    let k = pw as f32 / fw;
    let to_node = sk::Transform::from_translate(dx, dy).pre_scale(sx, sy).pre_translate(-cx0, -cy0).pre_scale(1.0 / k, 1.0 / k);
    let part = m.crop.and_then(|_| {
        let (x0, y0) = ((cx0 * k).floor() as i32, (cy0 * k).floor() as i32);
        sk::IntRect::from_ltrb(x0, y0, (((cx0 + cw) * k).ceil() as i32).min(pw as i32), (((cy0 + ch) * k).ceil() as i32).min(ph as i32))
    });
    vec![Op::Fill { path, paint: Fill::Picture(pix, key, ctx.t.pre_concat(to_node), part), rule: sk::FillRule::Winding }]
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}

impl Pipe for f32 {}

/// A shape's fill, then its stroke, in output pixels.
fn styled(local: Option<sk::Path>, st: &Style, ctx: Ctx) -> Vec<Op> {
    let Some(local) = local else {
        return Vec::new();
    };
    let mut ops = Vec::new();
    let Some(device) = local.transform(ctx.t) else {
        return ops;
    };
    if let Some(f) = &st.fill {
        let rule = match st.fill_rule {
            FillRule::NonZero => sk::FillRule::Winding,
            FillRule::EvenOdd => sk::FillRule::EvenOdd,
        };
        ops.push(Op::Fill { path: device.clone(), paint: fill(f, ctx.t), rule });
    }
    if let Some(k) = &st.stroke {
        let width = k.width * ctx.s;
        if width > 0.0 {
            let dashed = if k.dash.is_empty() {
                Some(device)
            } else {
                // an odd list repeats once, as in SVG
                let mut d: Vec<f32> = k.dash.iter().map(|v| v * ctx.s).collect();
                if d.len() % 2 == 1 {
                    d.extend_from_slice(&d.clone());
                }
                sk::StrokeDash::new(d, k.dash_offset * ctx.s).and_then(|dash| device.dash(&dash, 1.0))
            };
            let stroke = sk::Stroke {
                width,
                miter_limit: k.miter_limit,
                line_cap: match k.cap {
                    Cap::Butt => sk::LineCap::Butt,
                    Cap::Round => sk::LineCap::Round,
                    Cap::Square => sk::LineCap::Square,
                },
                line_join: match k.join {
                    Join::Miter => sk::LineJoin::Miter,
                    Join::Round => sk::LineJoin::Round,
                    Join::Bevel => sk::LineJoin::Bevel,
                },
                dash: None,
            };
            if let Some(p) = dashed.and_then(|p| p.stroke(&stroke, 1.0)) {
                ops.push(Op::Fill { path: p, paint: fill(&k.paint, ctx.t), rule: sk::FillRule::Winding });
            }
        }
    }
    ops
}

fn fill(p: &Paint, t: sk::Transform) -> Fill {
    match p {
        Paint::Solid(c) => Fill::Solid(*c),
        Paint::Gradient(g) => Fill::Gradient(g.clone(), t),
    }
}

/// Text laid out in its node's units, each glyph's outline placed in output pixels.
fn text(t: &scene::TextNode, ctx: Ctx, fonts: &mut Fonts) -> Vec<Op> {
    let em = t.size / fonts.line;
    let lines = lay_out(t, em, fonts);
    // the text's own frame: its point mapped by `t`, lengths by `lin`
    let (ox, oy) = map(ctx.t, t.x, t.y);
    let frame = ctx.lin.post_translate(ox, oy);
    let mut b = sk::PathBuilder::new();
    let mut any = false;
    for (i, (line, width)) in lines.iter().enumerate() {
        let left = line_left(t, *width);
        let base = i as f32 * t.size * t.line_height + fonts.ascent * em;
        let mut pen = left;
        for c in line.chars() {
            if let Some(o) = fonts.outline(c) {
                let ts = frame.pre_translate(pen, base).pre_scale(em, -em);
                if let Some(p) = o.transform(ts) {
                    b.push_path(&p);
                    any = true;
                }
            }
            pen += fonts.advance(c) * em;
        }
    }
    if !any {
        return Vec::new();
    }
    match b.finish() {
        Some(path) => vec![Op::Fill { path, paint: fill(&t.fill, ctx.t), rule: sk::FillRule::Winding }],
        None => Vec::new(),
    }
}

/// Where a line `width` wide starts, from the text's `x`, by its alignment.
fn line_left(t: &scene::TextNode, width: f32) -> f32 {
    match (t.width, t.align) {
        (Some(_), Align::Left) | (None, Align::Left) => 0.0,
        (Some(w), Align::Centre) => (w - width) / 2.0,
        (Some(w), Align::Right) => w - width,
        (None, Align::Centre) => -width / 2.0,
        (None, Align::Right) => -width,
    }
}

/// The text as [`text`] lays it out, in its node's units: the box from its (`x`, `y`), each line's place and glyphs.
pub fn measure(t: &scene::TextNode, fonts: &Fonts) -> Measure {
    let em = t.size / fonts.line;
    let step = t.size * t.line_height;
    let lines: Vec<MeasuredLine> = lay_out(t, em, fonts)
        .into_iter()
        .enumerate()
        .map(|(i, (text, w))| {
            let x = t.x + line_left(t, w);
            let mut pen = x;
            let glyphs = text
                .chars()
                .map(|c| {
                    let at = pen;
                    pen += fonts.advance(c) * em;
                    at
                })
                .collect();
            let y = t.y + i as f32 * step;
            MeasuredLine { text, x, y, baseline: y + fonts.ascent * em, w, glyphs }
        })
        .collect();
    let left = lines.iter().map(|l| l.x).fold(f32::INFINITY, f32::min);
    let right = lines.iter().map(|l| l.x + l.w).fold(f32::NEG_INFINITY, f32::max);
    let w = t.width.unwrap_or(if lines.is_empty() { 0.0 } else { right - left });
    // the last line's box is the text's size tall, the ones above it a line step each
    let h = if lines.is_empty() { 0.0 } else { (lines.len() - 1) as f32 * step + t.size };
    Measure { w, h, lines }
}

fn map(t: sk::Transform, x: f32, y: f32) -> (f32, f32) {
    let mut p = [sk::Point::from_xy(x, y)];
    t.map_points(&mut p);
    (p[0].x, p[0].y)
}

/// The text's lines and their widths, in the node's units: hard breaks, wrapping in `width` at spaces (and inside a
/// word wider than a line), at most `lines`, the last kept line ending in "…" when something was cut and `ellipsis`.
pub fn lay_out(t: &scene::TextNode, em: f32, fonts: &Fonts) -> Vec<(String, f32)> {
    let w = |s: &str| fonts.width(s) * em;
    let mut lines: Vec<String> = Vec::new();
    for para in t.text.split('\n') {
        let Some(max) = t.width else {
            lines.push(para.to_string());
            continue;
        };
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if w(&candidate) <= max || line.is_empty() && w(word) <= max {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // a word wider than the line is broken where it must
            for c in word.chars() {
                if !line.is_empty() && w(&format!("{line}{c}")) > max {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    let limit = t.lines.map_or(usize::MAX, |l| l as usize);
    let mut cut = lines.len() > limit;
    lines.truncate(limit);
    // one line wider than its box (no room to wrap) is cut too
    if let (Some(max), Some(last)) = (t.width, lines.last()) {
        cut |= w(last) > max;
    }
    if cut && t.ellipsis {
        if let Some(last) = lines.last_mut() {
            let max = t.width.unwrap_or(f32::INFINITY);
            let mut s: Vec<char> = last.trim_end().chars().collect();
            while !s.is_empty() && w(&format!("{}…", s.iter().collect::<String>())) > max {
                s.pop();
            }
            *last = format!("{}…", s.iter().collect::<String>().trim_end());
        }
    }
    lines.into_iter().map(|l| (l.clone(), w(&l))).collect()
}

/// Ops placed as one item: their bounds in whole pixels inside `clip`, moved so the tile's corner is the origin.
fn push(ops: Vec<Op>, clip: sk::IntRect, own: Option<u64>, out: &mut Vec<Item>) {
    let Some(b) = bounds(&ops) else {
        return;
    };
    // a bound a rounding error past a whole pixel stays on it
    let (x0, y0) = ((b.left() + 1e-3).floor() as i32, (b.top() + 1e-3).floor() as i32);
    let (x1, y1) = ((b.right() - 1e-3).ceil() as i32, (b.bottom() - 1e-3).ceil() as i32);
    let Some(r) = sk::IntRect::from_ltrb(x0, y0, x1, y1).and_then(|r| r.intersect(&clip)) else {
        return;
    };
    let shift = sk::Transform::from_translate(-r.x() as f32, -r.y() as f32);
    let ops: Vec<Op> = ops.into_iter().filter_map(|o| moved(o, shift)).collect();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (r.width(), r.height()).hash(&mut h);
    hash_ops(&ops, &mut h);
    out.push(Item { key: h.finish(), rect: Rect { x: r.x() as u32, y: r.y() as u32, w: r.width(), h: r.height() }, own, solid: None, fade: 1.0, opaque: false, image: None, hole: false, flat: None, ops: Arc::new(ops) });
}

/// An image item the 2D engine can lay from its picture (upright, unrotated, at most [`REACH`] pixels a picture pixel):
/// the picture's whole pixels that land on the item and where they land, rounded, in place of its drawing.
fn place(it: &mut Item) {
    let [Op::Fill { paint: Fill::Picture(pix, key, t, part), .. }] = it.ops.as_slice() else {
        return;
    };
    let reach = REACH as f32;
    if t.kx != 0.0 || t.ky != 0.0 || !(t.sx > 0.0 && t.sx <= reach && t.sy > 0.0 && t.sy <= reach) {
        return;
    }
    let (pw, ph) = pix.size();
    let (x0, y0, x1, y1) = part.map_or((0, 0, pw as i32, ph as i32), |r| (r.x(), r.y(), r.right(), r.bottom()));
    // the picture's pixels over 0..len of the tile, inside lo..hi, and where their edges land
    let span = |len: u32, lo: i32, hi: i32, s: f32, o: f32| {
        let (a, b) = ((((0.0 - o) / s).floor() as i32).max(lo), (((len as f32 - o) / s).ceil() as i32).min(hi));
        (a, b, (s * a as f32 + o).round() as i32, (s * b as f32 + o).round() as i32)
    };
    let (u0, u1, d0, d1) = span(it.rect.w, x0, x1, t.sx, t.tx);
    let (v0, v1, e0, e1) = span(it.rect.h, y0, y1, t.sy, t.ty);
    if u1 <= u0 || v1 <= v0 || d1 <= d0 || e1 <= e0 {
        return;
    }
    let placed = Placed {
        pix: pix.clone(),
        src: Rect { x: u0 as u32, y: v0 as u32, w: (u1 - u0) as u32, h: (v1 - v0) as u32 },
        dst: (d0, e0, (d1 - d0) as u32, (e1 - e0) as u32),
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (it.rect.w, it.rect.h, *key, placed.src, placed.dst).hash(&mut h);
    it.key = h.finish();
    it.image = Some(placed);
    it.ops = Arc::new(Vec::new());
}

fn bounds(ops: &[Op]) -> Option<sk::Rect> {
    let mut acc: Option<sk::Rect> = None;
    for o in ops {
        let b = match o {
            Op::Fill { path, .. } => Some(path.bounds()),
            Op::Layer { ops, clip, mask, .. } => {
                let mut b = bounds(ops);
                if let Some(c) = clip {
                    b = b.and_then(|b| b.intersect(&c.path.bounds()));
                }
                if let Some(m) = mask {
                    b = b.and_then(|b| bounds(m).and_then(|m| b.intersect(&m)));
                }
                b
            }
        };
        acc = match (acc, b) {
            (Some(a), Some(b)) => Some(sk::Rect::from_ltrb(a.left().min(b.left()), a.top().min(b.top()), a.right().max(b.right()), a.bottom().max(b.bottom()))?),
            (a, b) => a.or(b),
        };
    }
    acc
}

fn moved(o: Op, by: sk::Transform) -> Option<Op> {
    Some(match o {
        Op::Fill { path, paint, rule } => {
            let paint = match paint {
                Fill::Gradient(g, t) => Fill::Gradient(g, t.post_concat(by)),
                Fill::Picture(p, k, t, part) => Fill::Picture(p, k, t.post_concat(by), part),
                s => s,
            };
            Op::Fill { path: path.transform(by)?, paint, rule }
        }
        Op::Layer { ops, opacity, clip, mask } => Op::Layer {
            ops: ops.into_iter().filter_map(|o| moved(o, by)).collect(),
            opacity,
            clip: match clip {
                Some(c) => Some(LayerClip { path: c.path.transform(by)?, ..c }),
                None => None,
            },
            mask: mask.map(|m| m.into_iter().filter_map(|o| moved(o, by)).collect()),
        },
    })
}

/// Every list, option and path is counted or tagged, so no two different drawings run together into one stream.
fn hash_ops(ops: &[Op], h: &mut impl Hasher) {
    h.write_usize(ops.len());
    for o in ops {
        match o {
            Op::Fill { path, paint, rule } => {
                h.write_u8(0);
                (*rule as u8).hash(h);
                hash_path(path, h);
                match paint {
                    Fill::Solid(c) => {
                        h.write_u8(1);
                        c.0.hash(h);
                    }
                    Fill::Gradient(g, t) => {
                        // the gradient's own numbers and where they land
                        h.write_u8(2);
                        format!("{g:?}").hash(h);
                        for v in [t.sx, t.kx, t.ky, t.sy, t.tx, t.ty] {
                            quantised(v, h);
                        }
                    }
                    Fill::Picture(_, key, t, part) => {
                        h.write_u8(3);
                        key.hash(h);
                        part.map(|r| (r.x(), r.y(), r.width(), r.height())).hash(h);
                        for v in [t.sx, t.kx, t.ky, t.sy, t.tx, t.ty] {
                            quantised(v, h);
                        }
                    }
                }
            }
            Op::Layer { ops, opacity, clip, mask } => {
                h.write_u8(6);
                quantised(*opacity, h);
                hash_ops(ops, h);
                match clip {
                    Some(c) => {
                        h.write_u8(1);
                        (c.rule as u8, c.anti_alias).hash(h);
                        hash_path(&c.path, h);
                    }
                    None => h.write_u8(0),
                }
                match mask {
                    Some(m) => {
                        h.write_u8(1);
                        hash_ops(m, h);
                    }
                    None => h.write_u8(0),
                }
            }
        }
    }
}

/// To 1/256 px: the same drawing moved by whole pixels differs only in rounding errors.
fn quantised(v: f32, h: &mut dyn Hasher) {
    h.write_i64((v as f64 * 256.0).round() as i64);
}

fn hash_path(path: &sk::Path, h: &mut impl Hasher) {
    let f = quantised;
    h.write_usize(path.len());
    {
        {
            {
                for s in path.segments() {
                    match s {
                        sk::PathSegment::MoveTo(p) => {
                            h.write_u8(1);
                            f(p.x, h);
                            f(p.y, h);
                        }
                        sk::PathSegment::LineTo(p) => {
                            h.write_u8(2);
                            f(p.x, h);
                            f(p.y, h);
                        }
                        sk::PathSegment::QuadTo(a, p) => {
                            h.write_u8(3);
                            for q in [a, p] {
                                f(q.x, h);
                                f(q.y, h);
                            }
                        }
                        sk::PathSegment::CubicTo(a, b, p) => {
                            h.write_u8(4);
                            for q in [a, b, p] {
                                f(q.x, h);
                                f(q.y, h);
                            }
                        }
                        sk::PathSegment::Close => h.write_u8(5),
                    }
                }
            }
        }
    }
}

/// The Linux memory one tile's drawing may hold at once (a pixmap per nested layer, clip and mask; the tile itself is
/// in the media heap): bounds what a valid scene can ask of the camera's RAM, of which about 21 MB is available in the
/// worst case (memory in the worst case).
pub const LAYER_BYTES: u64 = 12 << 20;

static LAYER_PEAK: AtomicU64 = AtomicU64::new(0);

/// The most Linux memory one tile's drawing asked for since the last [`reset_layer_peak`] (`microcam mem`).
pub fn layer_peak() -> u64 {
    LAYER_PEAK.load(Ordering::SeqCst)
}

pub fn reset_layer_peak() {
    LAYER_PEAK.store(0, Ordering::SeqCst);
}

/// An item's tile drawn into `px` (its [`Item::tile_size`], rows of exactly four bytes a pixel), cleared first: premultiplied
/// RGBA; an error when its nested layers would hold more than [`LAYER_BYTES`] at once or memory is refused.
pub fn draw_tile(item: &Item, px: &mut sk::PixmapMut) -> Result<(), String> {
    let need = cost(&item.ops);
    LAYER_PEAK.fetch_max(need, Ordering::SeqCst);
    if need > LAYER_BYTES {
        let (w, h) = item.tile_size();
        return Err(format!("the tile {w}x{h} needs {} MiB to draw, at most {}", need >> 20, LAYER_BYTES >> 20));
    }
    px.fill(sk::Color::TRANSPARENT);
    match &item.image {
        Some(p) => draw_placed(px, p),
        None => draw(px, (0, 0), &item.ops),
    }
}

/// An item's tile in Linux memory: the reference the layer is checked against, and the host's.
pub fn rasterise(item: &Item) -> Result<sk::Pixmap, String> {
    let (w, h) = item.tile_size();
    let mut px = sk::Pixmap::new(w, h).ok_or("no memory for the tile")?;
    draw_tile(item, &mut px.as_mut())?;
    Ok(px)
}

/// `part` of an item (item pixels) as the layer gets it from its tile: the tile's [`Item::pieces`], a middle column or
/// row stretched by repeating it. The CPU's reference for the 2D engine's stretch.
pub fn spread(item: &Item, tile: &sk::Pixmap, part: Rect) -> Option<sk::Pixmap> {
    if item.flat.is_none() {
        return sk::IntRect::from_xywh(part.x as i32, part.y as i32, part.w, part.h).and_then(|r| tile.clone_rect(r));
    }
    let mut px = sk::Pixmap::new(part.w, part.h)?;
    let (tw, pw) = (tile.width() as usize, part.w as usize);
    for (s, d) in item.pieces(part) {
        for row in 0..d.h {
            let sy = s.y + if s.h == d.h { row } else { 0 };
            for col in 0..d.w {
                let sx = s.x + if s.w == d.w { col } else { 0 };
                let v = tile.pixels()[sy as usize * tw + sx as usize];
                px.pixels_mut()[(d.y - part.y + row) as usize * pw + (d.x - part.x + col) as usize] = v;
            }
        }
    }
    Some(px)
}

/// A placed picture drawn as the 2D engine lays it: its part stretched over its destination, filtered.
fn draw_placed(px: &mut sk::PixmapMut, p: &Placed) -> Result<(), String> {
    let r = sk::IntRect::from_xywh(p.src.x as i32, p.src.y as i32, p.src.w, p.src.h).ok_or("an empty picture part")?;
    let part = pixmap(&*p.pix)?.clone_rect(r).ok_or("a picture part outside the picture")?;
    let (x, y, w, h) = p.dst;
    let t = sk::Transform::from_row(w as f32 / p.src.w as f32, 0.0, 0.0, h as f32 / p.src.h as f32, x as f32, y as f32);
    let paint = sk::Paint { shader: sk::Pattern::new(part.as_ref(), sk::SpreadMode::Pad, sk::FilterQuality::Bilinear, 1.0, t), ..Default::default() };
    if let Some(r) = sk::Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
        px.fill_rect(r, &paint, sk::Transform::identity(), None);
    }
    Ok(())
}

/// The most memory the nested layers of `ops` hold at once, each at its own bounds.
fn cost(ops: &[Op]) -> u64 {
    ops.iter()
        .map(|o| match o {
            Op::Fill { .. } => 0,
            // drawn straight into what holds it
            Op::Layer { ops: inner, opacity, clip: None, mask: None } if *opacity >= 1.0 => cost(inner),
            Op::Layer { ops: inner, mask, .. } => {
                let area = layer_bounds(o).map_or(0, |r| r.width() as u64 * r.height() as u64);
                // the layer, its clip or mask (one byte a pixel) and the mask's own pixmap, then the deepest inside
                4 * area + area + mask.as_ref().map_or(0, |m| 4 * area + cost(m)) + cost(inner)
            }
        })
        .max()
        .unwrap_or(0)
}

/// A layer's pixels: its content cut by its clip and mask, in whole pixels.
fn layer_bounds(o: &Op) -> Option<sk::IntRect> {
    let b = bounds(std::slice::from_ref(o))?;
    sk::IntRect::from_ltrb(b.left().floor() as i32, b.top().floor() as i32, b.right().ceil() as i32, b.bottom().ceil() as i32)
}

/// Draw `ops` (in tile pixels) into `px`, whose corner lies at `at` of the tile.
fn draw(px: &mut sk::PixmapMut, at: (i32, i32), ops: &[Op]) -> Result<(), String> {
    let to_px = sk::Transform::from_translate(-at.0 as f32, -at.1 as f32);
    for o in ops {
        match o {
            Op::Fill { path, paint, rule } => {
                let mut p = sk::Paint { anti_alias: true, ..Default::default() };
                match paint {
                    Fill::Solid(Colour([r, g, b, a])) => p.set_color_rgba8(*r, *g, *b, *a),
                    Fill::Gradient(g, t) => match shader(g, t.post_concat(to_px)) {
                        Some(s) => p.shader = s,
                        None => continue,
                    },
                    Fill::Picture(pic, _, t, part) => {
                        let cut;
                        let whole = pixmap(&**pic)?;
                        let (src, t) = match part {
                            // the crop alone, so its edges repeat rather than blend in what lies beyond them
                            Some(r) => {
                                cut = whole.clone_rect(*r).ok_or("an empty crop")?;
                                (cut.as_ref(), t.pre_translate(r.x() as f32, r.y() as f32))
                            }
                            None => (whole, *t),
                        };
                        p.shader = sk::Pattern::new(src, sk::SpreadMode::Pad, sk::FilterQuality::Bilinear, 1.0, t.post_concat(to_px));
                        px.fill_path(path, &p, *rule, to_px, None);
                        continue;
                    }
                }
                px.fill_path(path, &p, *rule, to_px, None);
            }
            // a whole group with nothing to lay over it: its children drawn straight in
            Op::Layer { ops: inner, opacity, clip: None, mask: None } if *opacity >= 1.0 => draw(px, at, inner)?,
            Op::Layer { ops: inner, opacity, clip, mask } => {
                // the layer at its own bounds inside this pixmap
                let here = sk::IntRect::from_xywh(at.0, at.1, px.width(), px.height()).ok_or("an empty pixmap")?;
                let Some(r) = layer_bounds(o).and_then(|r| r.intersect(&here)) else {
                    continue;
                };
                let origin = (r.x(), r.y());
                let mut sub = sk::Pixmap::new(r.width(), r.height()).ok_or("no memory for a layer")?;
                draw(&mut sub.as_mut(), origin, inner)?;
                let sub_px = sk::Transform::from_translate(-origin.0 as f32, -origin.1 as f32);
                if let Some(c) = clip {
                    let mut m = sk::Mask::new(r.width(), r.height()).ok_or("no memory for a clip")?;
                    m.fill_path(&c.path, c.rule, c.anti_alias, sub_px);
                    sub.apply_mask(&m);
                }
                if let Some(mops) = mask {
                    let mut mp = sk::Pixmap::new(r.width(), r.height()).ok_or("no memory for a mask")?;
                    draw(&mut mp.as_mut(), origin, mops)?;
                    sub.apply_mask(&sk::Mask::from_pixmap(mp.as_ref(), sk::MaskType::Alpha));
                }
                let paint = sk::PixmapPaint { opacity: *opacity, ..Default::default() };
                px.draw_pixmap(origin.0 - at.0, origin.1 - at.1, sub.as_ref(), &paint, sk::Transform::identity(), None);
            }
        }
    }
    Ok(())
}

fn shader(g: &Gradient, t: sk::Transform) -> Option<sk::Shader<'static>> {
    let stops = g.stops().iter().map(|s| {
        let [r, gr, b, a] = s.1 .0;
        sk::GradientStop::new(s.0, sk::Color::from_rgba8(r, gr, b, a))
    });
    match g {
        Gradient::Linear { from, to, .. } => {
            sk::LinearGradient::new(sk::Point::from_xy(from[0], from[1]), sk::Point::from_xy(to[0], to[1]), stops.collect(), sk::SpreadMode::Pad, t)
        }
        Gradient::Radial { centre, radius, .. } => {
            let c = sk::Point::from_xy(centre[0], centre[1]);
            sk::RadialGradient::new(c, c, *radius, stops.collect(), sk::SpreadMode::Pad, t)
        }
    }
}

/// A rectangle with corners rounded by `r` (at most half its smaller side).
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<sk::Path> {
    let rect = sk::Rect::from_xywh(x, y, w, h)?;
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Some(sk::PathBuilder::from_rect(rect));
    }
    // a quarter circle as one cubic
    let k = r * (1.0 - 0.552_284_8);
    let (x1, y1) = (x + w, y + h);
    let mut b = sk::PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x1 - r, y);
    b.cubic_to(x1 - k, y, x1, y + k, x1, y + r);
    b.line_to(x1, y1 - r);
    b.cubic_to(x1, y1 - k, x1 - k, y1, x1 - r, y1);
    b.line_to(x + r, y1);
    b.cubic_to(x + k, y1, x, y1 - k, x, y1 - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + k, x + k, y, x + r, y);
    b.close();
    b.finish()
}

/// A circle's arc in cubics of at most a quarter turn; closed by its chord when it is to be filled.
fn arc(cx: f32, cy: f32, r: f32, start: f32, sweep: f32, closed: bool) -> Option<sk::Path> {
    let sweep = sweep.clamp(-360.0, 360.0);
    let n = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
    let step = sweep.to_radians() / n as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |a: f32| (cx + r * a.cos(), cy + r * a.sin());
    let mut a = start.to_radians();
    let mut b = sk::PathBuilder::new();
    let (x, y) = at(a);
    b.move_to(x, y);
    for _ in 0..n {
        let (x0, y0) = at(a);
        let (x3, y3) = at(a + step);
        b.cubic_to(x0 - k * r * a.sin(), y0 + k * r * a.cos(), x3 + k * r * (a + step).sin(), y3 - k * r * (a + step).cos(), x3, y3);
        a += step;
    }
    if closed {
        b.close();
    }
    b.finish()
}

fn path(segs: &[Seg]) -> Option<sk::Path> {
    let mut b = sk::PathBuilder::new();
    for s in segs {
        match *s {
            Seg::Move(x, y) => b.move_to(x, y),
            Seg::Line(x, y) => b.line_to(x, y),
            Seg::Quad(a, c, x, y) => b.quad_to(a, c, x, y),
            Seg::Cubic(a, c, d, e, x, y) => b.cubic_to(a, c, d, e, x, y),
            Seg::Close => b.close(),
        }
    }
    b.finish()
}

/// `items` (with their tiles, in order) composed over a transparent `area` of the output by the CPU: premultiplied
/// RGBA rows of `area.w` pixels. The reference the layer's composition is checked against.
pub fn compose(items: &[(Item, Arc<sk::Pixmap>)], area: Rect) -> Option<sk::Pixmap> {
    let mut px = sk::Pixmap::new(area.w, area.h)?;
    for (item, tile) in items {
        if item.rect.intersect(area).is_empty() {
            continue;
        }
        let paint = sk::PixmapPaint { opacity: item.fade, ..Default::default() };
        let Some(whole) = spread(item, tile, Rect { x: 0, y: 0, w: item.rect.w, h: item.rect.h }) else {
            continue;
        };
        px.draw_pixmap(item.rect.x as i32 - area.x as i32, item.rect.y as i32 - area.y as i32, whole.as_ref(), &paint, sk::Transform::identity(), None);
    }
    Some(px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use microcam_core::Size;

    fn out(w: u32, h: u32) -> OutputUi {
        OutputUi { raster: Size::new(w, h), live: Rect::full(Size::new(w, h)), level: scene::MAX_LEVEL, paused: None }
    }

    fn scene(nodes: &str) -> Scene {
        let s: Scene = serde_json::from_str(&format!(r#"{{"version":2,"nodes":[{nodes}]}}"#)).unwrap();
        s.validate().unwrap();
        s
    }

    /// A flat shape laid from its small tile matches the same shape drawn whole (in a kept group, which keeps it one
    /// tile) pixel for pixel: whole and fractional edges, rounded corners up to a pill, translucent, scaled, mirrored.
    #[test]
    fn a_flat_shape_from_its_small_tile_is_the_shape_drawn_whole() {
        let cases = [
            r##"{"kind":"rect","x":0,"y":0,"w":1920,"h":1080,"fill":"#0c0e1260"}"##,
            r##"{"kind":"rect","x":10.3,"y":20.7,"w":300.4,"h":100.2,"r":12,"fill":"#20406080"}"##,
            r##"{"kind":"rect","x":7.5,"y":3.25,"w":211.1,"h":57.9,"fill":"#ff8000c0"}"##,
            r##"{"kind":"rect","x":40,"y":40,"w":400,"h":100,"r":50,"fill":"#ffffffff"}"##,
            r##"{"kind":"rect","x":40.6,"y":40.2,"w":61,"h":400,"r":30.5,"fill":"#30303040"}"##,
            r##"{"kind":"group","x":100,"y":50,"scale":[1.37,0.61],"children":[{"kind":"rect","x":3.3,"y":9.1,"w":500,"h":300,"r":20,"fill":"#11223399"}]}"##,
            r##"{"kind":"group","x":900,"y":600,"scale":[-1.0,1.0],"children":[{"kind":"rect","x":20.25,"y":0.5,"w":333.3,"h":111,"r":8,"fill":"#5060708f"}]}"##,
        ];
        for (w, h) in [(1920, 1080), (960, 540), (1280, 720)] {
            let o = out(w, h);
            for c in cases {
                let flat = items(&scene(c), &o, &mut Fonts::new());
                assert!(flat.len() == 1 && flat[0].flat.is_some(), "{c} at {w}x{h}: {:?}", flat.first().map(|i| (i.rect, i.flat)));
                let (tw, th) = flat[0].tile_size();
                assert!(((tw * th) as u64) < flat[0].rect.area(), "{c}: a tile {tw}x{th} for {:?}", flat[0].rect);
                let whole = items(&scene(&format!(r#"{{"kind":"group","keep":true,"children":[{c}]}}"#)), &o, &mut Fonts::new());
                assert!(whole[0].flat.is_none() && whole[0].rect == flat[0].rect, "{c}: {:?} {:?}", whole[0].rect, flat[0].rect);
                let draw = |v: &[Item]| {
                    let t: Vec<_> = v.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
                    compose(&t, Rect::full(o.raster)).unwrap()
                };
                let (a, b) = (draw(&flat), draw(&whole));
                let diff = a.pixels().iter().zip(b.pixels()).filter(|(x, y)| x != y).count();
                assert_eq!(diff, 0, "{c} at {w}x{h}: {diff} pixels differ");
            }
        }
    }

    fn render(s: &Scene, o: &OutputUi) -> (Vec<Item>, sk::Pixmap) {
        let mut f = Fonts::new();
        let items = items(s, o, &mut f);
        let tiles: Vec<_> = items.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
        let px = compose(&tiles, Rect::full(o.raster)).unwrap();
        (items, px)
    }

    fn at(px: &sk::Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = px.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn measured_text_is_where_it_is_drawn() {
        let node = r##"{"kind":"text","x":100,"y":50,"size":40,"width":300,"align":"centre","text":"Яркость и контраст картинки","fill":"#ffffff"}"##;
        let s = scene(node);
        let Kind::Text(t) = &s.nodes[0].kind else { panic!() };
        let m = measure(t, &Fonts::new());
        assert!(m.lines.len() >= 2, "{m:?}");
        assert_eq!((m.w, m.h), (300.0, (m.lines.len() - 1) as f32 * 40.0 + 40.0));
        for l in &m.lines {
            assert_eq!(l.glyphs.len(), l.text.chars().count());
            assert!(l.glyphs.windows(2).all(|g| g[0] <= g[1]) && l.x >= 100.0 && l.x + l.w <= 400.0 + 0.01);
        }
        // the ink of each line lies inside its measured box (1080p: one unit a pixel)
        let (_, px) = render(&s, &out(1920, 1080));
        let px = &px;
        for l in &m.lines {
            let ink: Vec<u32> = (l.y as u32..(l.y + 40.0) as u32).flat_map(|y| (0..1920).filter(move |&x| at(px, x, y)[3] > 0)).collect();
            let (lo, hi) = (*ink.iter().min().unwrap() as f32, *ink.iter().max().unwrap() as f32);
            assert!(lo >= l.x - 2.0 && hi <= l.x + l.w + 2.0, "{l:?} ink {lo}..{hi}");
        }
    }

    /// A 2×1 picture, red then blue, shown as resource `p`.
    fn two_pixels(id: &str) -> Option<Picture> {
        if id != "p" {
            return None;
        }
        let mut pix = sk::Pixmap::new(2, 1).unwrap();
        pix.pixels_mut()[0] = sk::ColorU8::from_rgba(255, 0, 0, 255).premultiply();
        pix.pixels_mut()[1] = sk::ColorU8::from_rgba(0, 0, 255, 255).premultiply();
        Some(Picture { full: (2, 1), pix: Some((Arc::new(pix) as Arc<dyn Pixels>, 7)) })
    }

    #[test]
    fn an_image_is_fitted_into_its_rectangle_and_its_use_recorded() {
        let o = out(1920, 1080);
        let draw = |node: &str| {
            let (items, uses) = shown(&scene(node), &o, &mut Fonts::new(), &two_pixels, &[]);
            let tiles: Vec<_> = items.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
            (compose(&tiles, Rect::full(o.raster)), uses)
        };
        // contain: 2:1 in a square → 400×200 centred, rows 100..300
        let (px, uses) = draw(r#"{"kind":"image","res":"p","x":0,"y":0,"w":400,"h":400}"#);
        let px = px.unwrap();
        assert_eq!(uses, vec![Use { res: "p".into(), scale: 200.0, drawn: true }]);
        // the picture is interpolated: a pixel's colour, not exactly
        let red = |c: [u8; 4]| c[0] > 200 && c[2] < 60 && c[3] == 255;
        let blue = |c: [u8; 4]| c[2] > 200 && c[0] < 60 && c[3] == 255;
        assert!(red(at(&px, 40, 200)) && blue(at(&px, 360, 200)));
        assert_eq!(at(&px, 200, 50)[3], 0);
        // cover: the square full, the picture's middle
        let (px, _) = draw(r#"{"kind":"image","res":"p","x":0,"y":0,"w":400,"h":400,"fit":"cover"}"#);
        let px = px.unwrap();
        assert_eq!(at(&px, 200, 30)[3], 255);
        assert!(red(at(&px, 10, 200)));
        // crop: the right pixel alone
        let (px, _) = draw(r#"{"kind":"image","res":"p","x":0,"y":0,"w":100,"h":100,"crop":[1,0,1,1]}"#);
        assert!(blue(at(&px.unwrap(), 10, 10)));
        // not there yet: nothing drawn, the use recorded
        let (px, uses) = draw(r#"{"kind":"image","res":"q","x":0,"y":0,"w":100,"h":100}"#);
        assert!(px.is_none() || at(&px.unwrap(), 10, 10)[3] == 0);
        assert_eq!(uses, vec![Use { res: "q".into(), scale: 0.0, drawn: false }]);
    }

    #[test]
    fn a_placed_image_draws_like_the_same_picture_in_a_tile() {
        // a 40×20 ramp shown ×1.5 and at a fractional place: laid from the picture
        let mut pix = sk::Pixmap::new(40, 20).unwrap();
        for (k, p) in pix.pixels_mut().iter_mut().enumerate() {
            *p = sk::ColorU8::from_rgba((k % 40 * 6) as u8, (k / 40 * 12) as u8, 90, 255).premultiply();
        }
        let pic = Picture { full: (40, 20), pix: Some((Arc::new(pix) as Arc<dyn Pixels>, 3)) };
        let pictures = |id: &str| (id == "p").then(|| pic.clone());
        let o = out(1920, 1080);
        let node = r#"{"kind":"image","res":"p","x":10.3,"y":20,"w":60,"h":30}"#;
        let draw = |s: &Scene| {
            let (items, _) = shown(s, &o, &mut Fonts::new(), &pictures, &[]);
            let tiles: Vec<_> = items.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
            (items, compose(&tiles, Rect::full(o.raster)).unwrap())
        };
        let (placed, a) = draw(&scene(node));
        let p = placed[0].image.as_ref().expect("laid from the picture");
        assert_eq!((p.src.w, p.src.h), (40, 20));
        // the same inside a group that keeps its drawing: a tile drawn by tiny-skia
        let (tiled, b) = draw(&scene(&format!(r#"{{"kind":"group","keep":true,"children":[{node}]}}"#)));
        assert!(tiled[0].image.is_none());
        for y in 21..49 {
            for x in 12..69 {
                let (p, q) = (at(&a, x, y), at(&b, x, y));
                assert!(p.iter().zip(q).all(|(u, v)| u.abs_diff(v) <= 12), "({x},{y}) {p:?} {q:?}");
            }
        }
    }

    #[test]
    fn only_a_plain_opaque_rectangle_on_whole_pixels_is_solid() {
        let o = out(1920, 1080);
        let solid = |node: &str| items(&scene(node), &o, &mut Fonts::new())[0].solid;
        assert_eq!(solid(r##"{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#102030"}"##), Some([0x10, 0x20, 0x30]));
        // inside a moved, clipped group too: the clip cuts the rect on whole pixels
        assert_eq!(solid(r##"{"kind":"group","x":5,"y":5,"clip":{"x":0,"y":0,"w":20,"h":20},"children":[{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#102030"}]}"##), Some([0x10, 0x20, 0x30]));
        for node in [
            r##"{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#10203080"}"##,
            r##"{"kind":"rect","x":10,"y":10,"w":40,"h":20,"r":4,"fill":"#102030"}"##,
            r##"{"kind":"rect","x":10.5,"y":10,"w":40,"h":20,"fill":"#102030"}"##,
            r##"{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#102030","stroke":{"paint":"#ffffff","width":2}}"##,
            r##"{"kind":"group","rotate":10,"children":[{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#102030"}]}"##,
            r##"{"kind":"group","opacity":0.5,"children":[{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":"#102030"}]}"##,
            r##"{"kind":"ellipse","cx":30,"cy":30,"rx":10,"ry":10,"fill":"#102030"}"##,
        ] {
            assert_eq!(solid(node), None, "{node}");
        }
    }

    #[test]
    fn screen_units_and_anchors_land_in_output_pixels() {
        let s = scene(r##"{"kind":"group","anchor":"bottom-right","children":[{"kind":"rect","x":-100,"y":-50,"w":100,"h":50,"fill":"#ff0000"}]}"##);
        let (items, px) = render(&s, &out(3840, 2160));
        assert_eq!(items[0].rect, Rect { x: 3640, y: 2060, w: 200, h: 100 });
        assert_eq!(at(&px, 3700, 2100), [255, 0, 0, 255]);
        assert_eq!(at(&px, 3600, 2100), [0; 4]);
    }

    #[test]
    fn a_whole_pixel_move_keeps_the_tile_and_a_half_pixel_one_does_not() {
        let rect = |x: f32| scene(&format!(r##"{{"kind":"rect","x":{x},"y":10,"w":40,"h":20,"r":6,"fill":"#ffffff"}}"##));
        let mut f = Fonts::new();
        let o = out(1920, 1080);
        let a = &items(&rect(10.0), &o, &mut f)[0];
        let b = &items(&rect(30.0), &o, &mut f)[0];
        let c = &items(&rect(30.5), &o, &mut f)[0];
        assert_eq!(a.key, b.key);
        assert_ne!(a.rect, b.rect);
        assert_ne!(a.key, c.key);
    }

    #[test]
    fn a_translucent_group_is_composited_whole() {
        // two overlapping opaque rects in a half-opacity group: the overlap is not darker than the rest
        let s = scene(
            r##"{"kind":"group","opacity":0.5,"children":[
                {"kind":"rect","x":0,"y":0,"w":60,"h":20,"fill":"#ffffff"},
                {"kind":"rect","x":30,"y":0,"w":60,"h":20,"fill":"#ffffff"}]}"##,
        );
        let (items, px) = render(&s, &out(1920, 1080));
        assert_eq!(items.len(), 1);
        assert_eq!(at(&px, 10, 10), at(&px, 45, 10));
        assert!((at(&px, 45, 10)[3] as i32 - 128).abs() <= 1);
        // without the group's opacity, two half-alpha rects do add up where they overlap
        let s = scene(
            r##"{"kind":"rect","x":0,"y":0,"w":60,"h":20,"fill":"#ffffff80"},
                {"kind":"rect","x":30,"y":0,"w":60,"h":20,"fill":"#ffffff80"}"##,
        );
        let (items, px) = render(&s, &out(1920, 1080));
        assert_eq!(items.len(), 2);
        assert!(at(&px, 45, 10)[3] > at(&px, 10, 10)[3]);
    }

    #[test]
    fn levels_hide_nodes_and_children_never_go_below_their_parent() {
        let s = scene(r##"{"kind":"group","level":2,"children":[{"kind":"rect","level":0,"x":0,"y":0,"w":10,"h":10,"fill":"#ffffff"}]},{"kind":"rect","level":1,"x":20,"y":0,"w":10,"h":10,"fill":"#ffffff"}"##);
        let mut o = out(1920, 1080);
        let mut f = Fonts::new();
        assert_eq!(items(&s, &o, &mut f).len(), 2);
        o.level = 1;
        assert_eq!(items(&s, &o, &mut f).len(), 1);
        o.level = 0;
        assert!(items(&s, &o, &mut f).is_empty());
    }

    #[test]
    fn paused_nodes_show_only_on_a_paused_output() {
        let s = scene(r##"{"kind":"group","level":0,"paused":true,"children":[{"kind":"rect","x":0,"y":0,"w":10,"h":10,"fill":"#ffffff"}]},{"kind":"rect","level":0,"x":20,"y":0,"w":10,"h":10,"fill":"#ffffff"}"##);
        let mut o = out(1920, 1080);
        o.level = 0;
        let mut f = Fonts::new();
        assert_eq!(items(&s, &o, &mut f).len(), 1);
        o.paused = Some(microcam_core::desired::Pause::Recording);
        assert_eq!(items(&s, &o, &mut f).len(), 2);
    }

    #[test]
    fn an_image_in_the_frame_is_a_hole_in_the_layer_at_its_place_in_z() {
        let s = scene(r##"{"kind":"group","children":[
            {"kind":"rect","x":0,"y":0,"w":200,"h":200,"fill":"#ff0000"},
            {"kind":"image","res":"p","x":0,"y":0,"w":100,"h":100,"full_colour":true},
            {"kind":"rect","x":50,"y":50,"w":10,"h":10,"fill":"#00ff00"}]}"##);
        let o = out(1080, 1080);
        let window = Rect { x: 0, y: 0, w: 100, h: 100 };
        let frame = [microcam_core::frame::Picture { node: microcam_core::scene::child_path(0, 1), res: "p".into(), window, crop: window }];
        let (items, uses) = shown(&s, &o, &mut Fonts::new(), &|_| None, &frame);
        // below it the red square, the hole over the picture's window, the green one above it; the image draws nothing
        assert_eq!(items.iter().map(|i| (i.hole, i.rect)).collect::<Vec<_>>(), vec![
            (false, Rect { x: 0, y: 0, w: 200, h: 200 }),
            (true, window),
            (false, Rect { x: 50, y: 50, w: 10, h: 10 }),
        ]);
        assert!(uses.is_empty(), "a frame's picture is no use of the layer's form");
    }

    /// A picture prepared ahead draws nothing: no hole, no image, no use of a form.
    #[test]
    fn a_prepared_picture_is_in_no_layer() {
        let s = scene(r##"{"kind":"rect","x":0,"y":0,"w":200,"h":200,"fill":"#ff0000"},
            {"kind":"image","res":"p","x":0,"y":0,"w":100,"h":100,"full_colour":true,"prepare":true}"##);
        let (items, uses) = shown(&s, &out(1080, 1080), &mut Fonts::new(), &|_| None, &[]);
        assert_eq!(items.iter().map(|i| (i.hole, i.rect)).collect::<Vec<_>>(), vec![(false, Rect { x: 0, y: 0, w: 200, h: 200 })]);
        assert!(uses.is_empty());
    }

    /// A full-colour picture not in the frame (its form coming, or failed) draws nothing in the layer and asks no form of
    /// it.
    #[test]
    fn a_full_colour_picture_out_of_the_frame_is_in_no_layer() {
        let s = scene(r##"{"kind":"image","res":"p","x":0,"y":0,"w":1080,"h":1080,"full_colour":true}"##);
        let (items, uses) = shown(&s, &out(1080, 1080), &mut Fonts::new(), &|_| None, &[]);
        assert!(items.is_empty() && uses.is_empty());
    }

    #[test]
    fn live_space_is_the_live_window_and_strokes_keep_screen_widths() {
        let s = scene(r##"{"kind":"group","space":"live","children":[{"kind":"path","d":"M0 0.5 H2","stroke":{"paint":"#ffffff","width":4}}]}"##);
        let mut o = out(3840, 2160);
        o.live = Rect { x: 0, y: 540, w: 1920, h: 1080 };
        let (items, px) = render(&s, &o);
        // a line across the window's middle, 8 px thick at 4K, cut at the window's right edge
        assert_eq!(items[0].rect, Rect { x: 0, y: 1076, w: 1920, h: 8 });
        assert_eq!(at(&px, 100, 1080)[3], 255);
        assert_eq!(at(&px, 2000, 1080)[3], 0);
    }

    #[test]
    fn a_rect_clip_crops_and_a_path_clip_isolates() {
        let s = scene(r##"{"kind":"group","clip":{"x":0,"y":0,"w":50,"h":50},"children":[{"kind":"ellipse","cx":50,"cy":50,"rx":40,"ry":40,"fill":"#00ff00"}]}"##);
        let (items, _) = render(&s, &out(1080 * 16 / 9, 1080));
        assert_eq!(items[0].rect, Rect { x: 10, y: 10, w: 40, h: 40 });
        let s = scene(r##"{"kind":"group","clip":{"d":"M0 0 L100 0 L0 100 Z"},"children":[{"kind":"rect","x":0,"y":0,"w":100,"h":100,"fill":"#00ff00"}]}"##);
        let (_, px) = render(&s, &out(1920, 1080));
        assert_eq!(at(&px, 20, 20)[3], 255);
        assert_eq!(at(&px, 80, 80)[3], 0);
    }

    #[test]
    fn a_mask_shows_the_group_only_where_it_has_alpha() {
        let s = scene(
            r##"{"kind":"group","mask":{"kind":"rect","x":0,"y":0,"w":50,"h":100,"fill":"#000000"},"children":[
                {"kind":"rect","x":0,"y":0,"w":100,"h":100,"fill":{"linear":{"from":[0,0],"to":[100,0],"stops":[[0,"#ff0000"],[1,"#0000ff"]]}}}]}"##,
        );
        let (_, px) = render(&s, &out(1920, 1080));
        let left = at(&px, 10, 50);
        assert_eq!(left[3], 255);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert_eq!(at(&px, 75, 50)[3], 0);
    }

    #[test]
    fn dashes_and_arcs_draw() {
        let s = scene(r##"{"kind":"path","d":"M0 10 H100","stroke":{"paint":"#ffffff","width":4,"dash":[10]}},{"kind":"arc","cx":200,"cy":200,"r":50,"start":0,"sweep":90,"stroke":{"paint":"#ffffff","width":4}}"##);
        let (_, px) = render(&s, &out(1920, 1080));
        assert_eq!(at(&px, 5, 10)[3], 255);
        assert_eq!(at(&px, 15, 10)[3], 0);
        // 45° along the arc lies on it, the opposite side does not
        let d = 50.0 / 2f32.sqrt();
        assert!(at(&px, (200.0 + d) as u32, (200.0 + d) as u32)[3] > 128);
        assert_eq!(at(&px, (200.0 - d) as u32, (200.0 - d) as u32)[3], 0);
    }

    #[test]
    fn text_sits_in_its_line_box_and_aligns() {
        let s = scene(r##"{"kind":"text","x":960,"y":100,"size":40,"text":"Снимок","align":"centre","fill":"#ffffff"}"##);
        let (items, _) = render(&s, &out(1920, 1080));
        let r = items[0].rect;
        assert!(r.y >= 100 && r.y + r.h <= 142, "{r:?}");
        assert!(((r.x + r.w / 2) as i32 - 960).abs() <= 3, "{r:?}");
    }

    #[test]
    fn text_wraps_cuts_and_ends_in_an_ellipsis() {
        let f = Fonts::new();
        let node = |json: &str| -> scene::TextNode {
            let s = scene(json);
            match &s.nodes[0].kind {
                Kind::Text(t) => t.clone(),
                _ => unreachable!(),
            }
        };
        let t = node(r##"{"kind":"text","x":0,"y":0,"size":30,"text":"один два три четыре пять шесть семь восемь","fill":"#ffffff","width":150,"lines":2,"ellipsis":true}"##);
        let em = t.size / f.line;
        let lines = lay_out(&t, em, &f);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|(_, w)| *w <= 150.0));
        assert!(lines[1].0.ends_with('…'));
        let t = node(r##"{"kind":"text","x":0,"y":0,"size":30,"text":"а\nб","fill":"#ffffff"}"##);
        assert_eq!(lay_out(&t, em, &f).len(), 2);
    }

    #[test]
    fn a_child_and_a_clip_of_the_same_shape_draw_different_keys() {
        let tri = r##"M0 0 L100 0 L0 100 Z"##;
        let child = scene(&format!(r##"{{"kind":"group","keep":true,"children":[{{"kind":"rect","x":0,"y":0,"w":100,"h":100,"fill":"#ffffff"}},{{"kind":"path","d":"{tri}","fill":"#00000000"}}]}}"##));
        let clip = scene(&format!(r##"{{"kind":"group","keep":true,"clip":{{"d":"{tri}"}},"children":[{{"kind":"rect","x":0,"y":0,"w":100,"h":100,"fill":"#ffffff"}}]}}"##));
        let mut f = Fonts::new();
        let o = out(1920, 1080);
        assert_ne!(items(&child, &o, &mut f)[0].key, items(&clip, &o, &mut f)[0].key);
    }

    #[test]
    fn a_kept_group_clips_a_rectangle_as_a_plain_one_does() {
        let g = |keep: bool| scene(&format!(r##"{{"kind":"group","keep":{keep},"clip":{{"x":10.3,"y":10.6,"w":40.4,"h":30.2}},"children":[{{"kind":"rect","x":0,"y":0,"w":100,"h":100,"fill":"#ffffff"}}]}}"##));
        let (_, a) = render(&g(false), &out(1920, 1080));
        let (_, b) = render(&g(true), &out(1920, 1080));
        assert!(a.data() == b.data());
    }

    #[test]
    fn a_tile_too_costly_to_draw_is_refused() {
        // nested half-opaque groups over most of a 4K frame
        let inner = (0..6).fold(String::from(r##"{"kind":"rect","x":0,"y":0,"w":1900,"h":1000,"fill":"#ffffff"}"##), |c, _| format!(r##"{{"kind":"group","opacity":0.9,"children":[{c}]}}"##));
        let s = scene(&inner);
        let it = items(&s, &out(3840, 2160), &mut Fonts::new());
        assert!(rasterise(&it[0]).is_err());
    }

    #[test]
    fn the_demo_menu_renders() {
        let s: Scene = serde_json::from_str(include_str!("../../../tools/ui/demo-menu.json")).unwrap();
        let (items, px) = render(&s, &out(1920, 1080));
        assert!(items.len() > 20);
        // the panel at its opacity over nothing
        let p = at(&px, 40, 300);
        assert!((p[3] as i32 - 189).abs() <= 2, "{p:?}");
    }
}

/// An item of `key` at `rect` with no drawing, for the layer's tests.
#[cfg(test)]
pub(crate) fn test_item(key: u64, rect: Rect, own: Option<u64>) -> Item {
    Item { key, rect, own, solid: None, fade: 1.0, opaque: false, image: None, hole: false, flat: None, ops: Arc::new(Vec::new()) }
}
