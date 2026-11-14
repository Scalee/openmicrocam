//! The scene the menu program sends: a retained tree of groups, paths and text on a
//! virtual screen, independent of any output. What it means in one output's pixels and how it is drawn live in
//! `microcam-draw`; this module only says what the tree is and refuses a tree a renderer should not see.
//!
//! Two spaces for the top-level nodes. `screen`: units of 1/1080 of the output's height, the origin at a point of the
//! output named by `anchor` (a corner, an edge's middle or the centre), so one scene fits 16:9, 16:10 and 4:3. `live`:
//! geometry in fractions of the live picture's window (0..1 across, 0..1 down), clipped to it, for what is tied to
//! what the camera sees (crosshair, grid, guides); stroke widths, dashes and text lengths (size, wrap width) stay in
//! screen units there, so a line keeps its thickness and text its shape whatever the window's shape.
//!
//! Where the live picture lies is the scene's too: a top-level `live` node (at most one; without it, the whole screen)
//! is the window the live picture fills, its centre at the window's aspect. An image marked `full_colour` is shown in
//! the video frame itself in full colour, so it must stay what a frame can carry — an
//! opaque, axis-aligned rectangle: in screen space, under groups with no rotation, mask, path clip, kept drawing or
//! opacity below 1. One marked `prepare` as well is not shown: its picture is made ready for that place ahead of a
//! scene that shows it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// A child's place in the tree from its parent's (a top-level node's is its index): how the renderer and the frame's
/// layout ([`crate::frame`]) name a node that has no `id`.
pub fn child_path(parent: u64, i: u64) -> u64 {
    parent.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(i + 1)
}

/// The scene format this daemon reads.
pub const VERSION: u32 = 2;
/// Display levels: 0 = the picture alone … 3 = the full menu. A node shows on an output whose level is at least its own.
pub const MAX_LEVEL: u8 = 3;
/// Limits checked before anything is drawn.
pub const MAX_NODES: usize = 4096;
pub const MAX_DEPTH: usize = 32;
pub const MAX_SEGMENTS: usize = 65_536;
pub const MAX_TEXT: usize = 1024;
pub const MAX_TEXT_TOTAL: usize = 16_384;
/// The largest text, in screen units (the output's height).
pub const MAX_TEXT_SIZE: f32 = 1080.0;
pub const MAX_STOPS: usize = 16;
pub const MAX_DASHES: usize = 16;
/// Any coordinate or length, in its own units.
const SANE: f32 = 100_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub version: u32,
    /// bottom to top
    #[serde(default)]
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// stable name for edits (unique in the scene when given)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// the lowest output level this node shows at; a child never shows below its parent's level. Top-level default 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// shown only on an output whose live picture is paused (`protocol::OutputInfo::paused`): where the menu says why
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
    #[serde(flatten)]
    pub kind: Kind,
}

impl Node {
    /// The level this node shows from under a parent showing from `base`, or None when it does not show on an output
    /// at `level`, `paused` or not.
    pub fn shows(&self, base: u8, level: u8, paused: bool) -> Option<u8> {
        let l = self.level.map_or(base, |own| own.max(base));
        (self.visible && l <= level && (paused || !self.paused)).then_some(l)
    }
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Kind {
    Group(Group),
    Rect(RectNode),
    Ellipse(EllipseNode),
    Arc(ArcNode),
    Path(PathNode),
    Text(TextNode),
    Image(ImageNode),
    Live(LiveNode),
}

/// The live picture's window in screen space (top-level only): the live picture fills it, its centre shown at the
/// window's aspect.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveNode {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// the output point the origin sits at
    #[serde(default)]
    pub anchor: Anchor,
}

/// Children in their own coordinates: `scale`, then `rotate`, then moved by (`x`, `y`) in the parent's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// degrees, clockwise
    #[serde(default)]
    pub rotate: f32,
    #[serde(default)]
    pub scale: Scale,
    /// top-level only: the output point (screen space) the origin sits at
    #[serde(default)]
    pub anchor: Anchor,
    /// top-level only
    #[serde(default)]
    pub space: Space,
    /// in the group's own coordinates
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    /// the group composited as a whole at this opacity (not multiplied into each child)
    #[serde(default = "one")]
    pub opacity: f32,
    /// the group shows only where this node (in the group's coordinates, never drawn itself) has alpha
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Box<Node>>,
    /// a drawing to keep: drawn once per output scale and reused while only its position or opacity changes
    #[serde(default)]
    pub keep: bool,
    /// how the group gets onto an overlay output
    #[serde(default, skip_serializing_if = "is_auto")]
    pub window: Window,
    /// bottom to top
    #[serde(default)]
    pub children: Vec<Node>,
}

/// How a group gets onto an overlay output (USB, recording): with whatever it overlaps, or asking for a hardware
/// window of its own — for what moves or blinks often, so that a move or a show costs no redraw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    #[default]
    Auto,
    Own,
}

fn is_auto(w: &Window) -> bool {
    *w == Window::Auto
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    Uniform(f32),
    Xy([f32; 2]),
}

impl Default for Scale {
    fn default() -> Self {
        Scale::Uniform(1.0)
    }
}

impl Scale {
    pub fn xy(self) -> (f32, f32) {
        match self {
            Scale::Uniform(s) => (s, s),
            Scale::Xy([x, y]) => (x, y),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Space {
    #[default]
    Screen,
    Live,
}

/// A point of the output: `x` grows right and `y` down from it in every case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Centre,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// The point in a `w` × `h` area.
    pub fn point(self, w: f32, h: f32) -> (f32, f32) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (w / 2.0, 0.0),
            Anchor::TopRight => (w, 0.0),
            Anchor::Left => (0.0, h / 2.0),
            Anchor::Centre => (w / 2.0, h / 2.0),
            Anchor::Right => (w, h / 2.0),
            Anchor::BottomLeft => (0.0, h),
            Anchor::Bottom => (w / 2.0, h),
            Anchor::BottomRight => (w, h),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Clip {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Path { d: PathData },
}

/// How a closed shape or a path is painted: a fill, a stroke, or both (the stroke over the fill).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default)]
    pub fill_rule: FillRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    /// screen units
    pub width: f32,
    #[serde(default)]
    pub join: Join,
    #[serde(default)]
    pub cap: Cap,
    #[serde(default = "miter_limit")]
    pub miter_limit: f32,
    /// on and off lengths in screen units, repeated
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f32>,
    #[serde(default)]
    pub dash_offset: f32,
}

fn miter_limit() -> f32 {
    4.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// A solid colour (`"#rrggbb"` or `"#rrggbbaa"`) or a gradient in the node's own coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Paint {
    Solid(Colour),
    Gradient(Gradient),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gradient {
    Linear { from: [f32; 2], to: [f32; 2], stops: Vec<Stop> },
    Radial { centre: [f32; 2], radius: f32, stops: Vec<Stop> },
}

impl Gradient {
    pub fn stops(&self) -> &[Stop] {
        match self {
            Gradient::Linear { stops, .. } | Gradient::Radial { stops, .. } => stops,
        }
    }
}

/// A gradient's colour at `offset` (0..1 along it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop(pub f32, pub Colour);

/// Straight (not premultiplied) RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Colour(pub [u8; 4]);

impl TryFrom<String> for Colour {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let hex = s.strip_prefix('#').filter(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some(hex) = hex else {
            return Err(format!("colour {s:?} is not #rrggbb or #rrggbbaa"));
        };
        let byte = |i: usize| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        Ok(Colour([byte(0), byte(1), byte(2), if hex.len() == 8 { byte(3) } else { 255 }]))
    }
}

impl From<Colour> for String {
    fn from(c: Colour) -> String {
        let [r, g, b, a] = c.0;
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

/// Corners rounded by `r`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectNode {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub r: f32,
    #[serde(flatten)]
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EllipseNode {
    pub cx: f32,
    pub cy: f32,
    pub rx: f32,
    pub ry: f32,
    #[serde(flatten)]
    pub style: Style,
}

/// A circle's arc from `start` through `sweep` degrees (0 = right, clockwise); filled, it is closed by its chord.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArcNode {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    pub start: f32,
    pub sweep: f32,
    #[serde(flatten)]
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathNode {
    pub d: PathData,
    #[serde(flatten)]
    pub style: Style,
}

/// Lines of text laid out by microcamd. (`x`, `y`) is the top of the first line box: its left edge, middle or right edge as
/// `align` says, or the left edge of a box `width` wide that the lines wrap in and align within.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextNode {
    pub x: f32,
    pub y: f32,
    /// the line box's height, screen units
    pub size: f32,
    pub text: String,
    pub fill: Paint,
    #[serde(default)]
    pub align: Align,
    /// wrap at spaces (and inside a word longer than a line) in this width; a text length like `size`, so screen units
    /// in both spaces
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// at most this many lines; what does not fit is cut
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
    /// a cut line (by `lines`, or a single line wider than `width`) ends in "…"
    #[serde(default)]
    pub ellipsis: bool,
    /// line spacing as a multiple of `size`
    #[serde(default = "one")]
    pub line_height: f32,
}

/// A resource's picture (or the part `crop` of it, in the picture's own pixels) in the rectangle (`x`, `y`, `w`, `h`),
/// fitted as `fit` says. While the resource is not ready it shows nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageNode {
    /// the resource's id
    pub res: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f32; 4]>,
    #[serde(default)]
    pub fit: Fit,
    /// shown in the video frame in full colour, not in the menu's 4-bit layer (see the module's head for where it may be)
    #[serde(default, skip_serializing_if = "is_false")]
    pub full_colour: bool,
    /// a full-colour picture prepared ahead, not shown: laid out as if shown, its form made for that window and held,
    /// so a later scene showing it there finds it ready; in no frame and no layer
    #[serde(default, skip_serializing_if = "is_false")]
    pub prepare: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// How a picture fills its rectangle: whole inside it, centred (`contain`); all of it, centred and cut (`cover`); or
/// stretched to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

/// SVG path data limited to absolute and relative `M L H V Q C Z`, read into absolute segments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PathData {
    pub segs: Vec<Seg>,
    source: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

impl TryFrom<String> for PathData {
    type Error = String;
    fn try_from(source: String) -> Result<Self, String> {
        Ok(PathData { segs: parse_path(&source)?, source })
    }
}

impl From<PathData> for String {
    fn from(p: PathData) -> String {
        p.source
    }
}

fn parse_path(d: &str) -> Result<Vec<Seg>, String> {
    let mut toks = Tokens { s: d.as_bytes(), i: 0 };
    let mut segs = Vec::new();
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    let mut cmd = None;
    loop {
        let c = match toks.command() {
            Some(c) => c,
            // numbers after a command repeat it (after M, as L)
            None if toks.more() => match cmd {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(c) if !matches!(c, b'Z' | b'z') => c,
                _ => return Err(format!("path: a number where a command is expected at byte {}", toks.i)),
            },
            None => break,
        };
        cmd = Some(c);
        let rel = c.is_ascii_lowercase();
        let (ox, oy) = if rel { cur } else { (0.0, 0.0) };
        let pt = |t: &mut Tokens| -> Result<(f32, f32), String> { Ok((t.num()? + ox, t.num()? + oy)) };
        match c.to_ascii_uppercase() {
            b'M' => {
                cur = pt(&mut toks)?;
                start = cur;
                segs.push(Seg::Move(cur.0, cur.1));
            }
            b'L' => {
                cur = pt(&mut toks)?;
                segs.push(Seg::Line(cur.0, cur.1));
            }
            b'H' => {
                cur.0 = toks.num()? + if rel { cur.0 } else { 0.0 };
                segs.push(Seg::Line(cur.0, cur.1));
            }
            b'V' => {
                cur.1 = toks.num()? + if rel { cur.1 } else { 0.0 };
                segs.push(Seg::Line(cur.0, cur.1));
            }
            b'Q' => {
                let (a, b) = (pt(&mut toks)?, pt(&mut toks)?);
                cur = b;
                segs.push(Seg::Quad(a.0, a.1, b.0, b.1));
            }
            b'C' => {
                let (a, b, e) = (pt(&mut toks)?, pt(&mut toks)?, pt(&mut toks)?);
                cur = e;
                segs.push(Seg::Cubic(a.0, a.1, b.0, b.1, e.0, e.1));
            }
            b'Z' => {
                cur = start;
                segs.push(Seg::Close);
            }
            _ => return Err(format!("path: command {:?} is not one of M L H V Q C Z", c as char)),
        }
        if segs.len() > MAX_SEGMENTS {
            return Err(format!("path: more than {MAX_SEGMENTS} segments"));
        }
    }
    if !matches!(segs.first(), None | Some(Seg::Move(..))) {
        return Err("path: does not start with M".into());
    }
    Ok(segs)
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn more(&mut self) -> bool {
        self.skip();
        self.i < self.s.len()
    }
    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() && !matches!(c, b'e' | b'E') {
            self.i += 1;
            return Some(c);
        }
        None
    }
    fn num(&mut self) -> Result<f32, String> {
        self.skip();
        let start = self.i;
        let mut j = self.i;
        let digits = |j: &mut usize, s: &[u8]| {
            while *j < s.len() && s[*j].is_ascii_digit() {
                *j += 1;
            }
        };
        if j < self.s.len() && matches!(self.s[j], b'+' | b'-') {
            j += 1;
        }
        digits(&mut j, self.s);
        if j < self.s.len() && self.s[j] == b'.' {
            j += 1;
            digits(&mut j, self.s);
        }
        if j < self.s.len() && matches!(self.s[j], b'e' | b'E') {
            j += 1;
            if j < self.s.len() && matches!(self.s[j], b'+' | b'-') {
                j += 1;
            }
            digits(&mut j, self.s);
        }
        let v = std::str::from_utf8(&self.s[start..j]).ok().and_then(|t| t.parse::<f32>().ok());
        match v {
            Some(v) if v.is_finite() => {
                self.i = j;
                Ok(v)
            }
            _ => Err(format!("path: a number expected at byte {start}")),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SceneError {
    #[error("scene version {0}, this daemon reads {VERSION}")]
    Version(u32),
    #[error("{0} nodes, at most {MAX_NODES}")]
    TooMany(usize),
    #[error("{0} path segments, at most {MAX_SEGMENTS}")]
    Segments(usize),
    #[error("{0} characters of text, at most {MAX_TEXT_TOTAL}")]
    Text(usize),
    /// the node's position in the tree (indexes from the top) and what is wrong
    #[error("node {0}: {1}")]
    Node(String, String),
}

/// What validation counts on the way down.
#[derive(Default)]
struct Tally<'a> {
    nodes: usize,
    segments: usize,
    text: usize,
    ids: HashSet<&'a str>,
}

impl Scene {
    pub fn empty() -> Self {
        Self { version: VERSION, nodes: Vec::new() }
    }

    /// The resources its image nodes name, shown or not (masks included).
    pub fn resources(&self) -> HashSet<&str> {
        fn walk<'a>(n: &'a Node, out: &mut HashSet<&'a str>) {
            match &n.kind {
                Kind::Image(m) => {
                    out.insert(&m.res);
                }
                Kind::Group(g) => {
                    g.children.iter().chain(g.mask.as_deref()).for_each(|c| walk(c, out));
                }
                _ => {}
            }
        }
        let mut out = HashSet::new();
        self.nodes.iter().for_each(|n| walk(n, &mut out));
        out
    }

    /// Everything a renderer relies on: numbers finite and sane, levels and opacities in range, the tree and its text
    /// bounded, anchors and spaces only where they mean something, ids unique.
    pub fn validate(&self) -> Result<(), SceneError> {
        if self.version != VERSION {
            return Err(SceneError::Version(self.version));
        }
        let mut t = Tally::default();
        for (i, n) in self.nodes.iter().enumerate() {
            check(n, &i.to_string(), 1, true, true, &mut t)?;
        }
        if self.nodes.iter().filter(|n| matches!(n.kind, Kind::Live(_))).count() > 1 {
            return Err(SceneError::Node("live".into(), "more than one live node".into()));
        }
        if t.nodes > MAX_NODES {
            return Err(SceneError::TooMany(t.nodes));
        }
        if t.segments > MAX_SEGMENTS {
            return Err(SceneError::Segments(t.segments));
        }
        if t.text > MAX_TEXT_TOTAL {
            return Err(SceneError::Text(t.text));
        }
        Ok(())
    }
}

/// `frame`: whether a full-colour image may lie here (every group above it keeps it an opaque, axis-aligned rectangle).
fn check<'a>(n: &'a Node, at: &str, depth: usize, top: bool, frame: bool, t: &mut Tally<'a>) -> Result<(), SceneError> {
    let bad = |m: &str| Err(SceneError::Node(at.to_string(), m.to_string()));
    t.nodes += 1;
    if t.nodes > MAX_NODES {
        return Err(SceneError::TooMany(t.nodes));
    }
    if depth > MAX_DEPTH {
        return bad("nested deeper than 32");
    }
    if let Some(id) = &n.id {
        if id.is_empty() || id.len() > 64 {
            return bad("id empty or longer than 64 bytes");
        }
        if !t.ids.insert(id) {
            return bad("id used twice");
        }
    }
    if n.level.is_some_and(|l| l > MAX_LEVEL) {
        return bad("level above 3");
    }
    let sane = |vs: &[f32]| vs.iter().all(|v| v.is_finite() && v.abs() <= SANE);
    let mut nums: Vec<f32> = Vec::new();
    let mut style = None;
    match &n.kind {
        Kind::Group(g) => {
            if !top && (g.anchor != Anchor::default() || g.space != Space::default()) {
                return bad("anchor and space are for top-level groups only");
            }
            if g.space == Space::Live && g.anchor != Anchor::default() {
                return bad("a live-space group has no anchor");
            }
            if !(0.0..=1.0).contains(&g.opacity) {
                return bad("opacity outside 0..1");
            }
            let (sx, sy) = g.scale.xy();
            nums.extend([g.x, g.y, g.rotate, sx, sy]);
            match &g.clip {
                Some(Clip::Rect { x, y, w, h }) => {
                    if *w < 0.0 || *h < 0.0 {
                        return bad("clip with a negative size");
                    }
                    nums.extend([*x, *y, *w, *h]);
                }
                Some(Clip::Path { d }) => path(d, &mut nums, t),
                None => {}
            }
            if let Some(m) = &g.mask {
                check(m, &format!("{at}.mask"), depth + 1, false, false, t)?;
            }
            let frame = frame && g.space == Space::Screen && g.rotate == 0.0 && sx > 0.0 && sy > 0.0 && g.opacity >= 1.0
                && g.mask.is_none() && !g.keep && !matches!(g.clip, Some(Clip::Path { .. }));
            for (i, c) in g.children.iter().enumerate() {
                check(c, &format!("{at}.{i}"), depth + 1, false, frame, t)?;
            }
        }
        Kind::Rect(r) => {
            if r.w < 0.0 || r.h < 0.0 || r.r < 0.0 {
                return bad("negative size or radius");
            }
            nums.extend([r.x, r.y, r.w, r.h, r.r]);
            style = Some(&r.style);
        }
        Kind::Ellipse(e) => {
            if e.rx < 0.0 || e.ry < 0.0 {
                return bad("negative radius");
            }
            nums.extend([e.cx, e.cy, e.rx, e.ry]);
            style = Some(&e.style);
        }
        Kind::Arc(a) => {
            if a.r < 0.0 {
                return bad("negative radius");
            }
            nums.extend([a.cx, a.cy, a.r, a.start, a.sweep]);
            style = Some(&a.style);
        }
        Kind::Path(p) => {
            path(&p.d, &mut nums, t);
            style = Some(&p.style);
        }
        Kind::Text(x) => {
            let chars = x.text.chars().count();
            if chars > MAX_TEXT {
                return bad("text longer than 1024 characters");
            }
            t.text += chars;
            if !(1.0..=MAX_TEXT_SIZE).contains(&x.size) {
                return bad("text size outside 1..1080");
            }
            if x.width.is_some_and(|w| w <= 0.0) {
                return bad("text width not positive");
            }
            if x.lines == Some(0) {
                return bad("text lines 0");
            }
            if !(0.5..=4.0).contains(&x.line_height) {
                return bad("line height outside 0.5..4");
            }
            nums.extend([x.x, x.y, x.size, x.width.unwrap_or(0.0)]);
            paint(&x.fill, &mut nums).or_else(bad)?;
        }
        Kind::Image(m) => {
            if m.res.is_empty() || m.res.len() > 64 {
                return bad("resource id empty or longer than 64 bytes");
            }
            if m.w <= 0.0 || m.h <= 0.0 {
                return bad("image size not positive");
            }
            if let Some(c) = m.crop {
                if c.iter().any(|v| *v < 0.0) || c[2] <= 0.0 || c[3] <= 0.0 {
                    return bad("crop: x, y not negative, w, h positive");
                }
                nums.extend(c);
            }
            if m.full_colour && !frame {
                return bad("a full-colour image lies only in screen space, under no rotation, mask, path clip, kept drawing or opacity below 1");
            }
            if m.prepare && !m.full_colour {
                return bad("only a full-colour image is prepared");
            }
            nums.extend([m.x, m.y, m.w, m.h]);
        }
        Kind::Live(l) => {
            if !top {
                return bad("the live node is top-level only");
            }
            if !n.visible || n.level.is_some() {
                return bad("the live node has no level and is not hidden (a hidden live picture: not yet)");
            }
            if l.w <= 0.0 || l.h <= 0.0 {
                return bad("live window size not positive");
            }
            nums.extend([l.x, l.y, l.w, l.h]);
        }
    }
    if let Some(s) = style {
        if s.fill.is_none() && s.stroke.is_none() {
            return bad("neither fill nor stroke");
        }
        if let Some(f) = &s.fill {
            paint(f, &mut nums).or_else(bad)?;
        }
        if let Some(k) = &s.stroke {
            if !(0.0..=1080.0).contains(&k.width) {
                return bad("stroke width outside 0..1080");
            }
            if !(1.0..=100.0).contains(&k.miter_limit) {
                return bad("miter limit outside 1..100");
            }
            if k.dash.len() > MAX_DASHES || k.dash.iter().any(|d| *d < 0.0) || (!k.dash.is_empty() && k.dash.iter().sum::<f32>() <= 0.0) {
                return bad("dash: at most 16 lengths, none negative, not all zero");
            }
            nums.extend(k.dash.iter().copied());
            nums.extend([k.width, k.dash_offset]);
            paint(&k.paint, &mut nums).or_else(bad)?;
        }
    }
    if !sane(&nums) {
        return bad("a number is not finite or beyond 100000");
    }
    Ok(())
}

fn path(d: &PathData, nums: &mut Vec<f32>, t: &mut Tally) {
    t.segments += d.segs.len();
    for s in &d.segs {
        match *s {
            Seg::Move(x, y) | Seg::Line(x, y) => nums.extend([x, y]),
            Seg::Quad(a, b, c, d) => nums.extend([a, b, c, d]),
            Seg::Cubic(a, b, c, d, e, f) => nums.extend([a, b, c, d, e, f]),
            Seg::Close => {}
        }
    }
}

fn paint(p: &Paint, nums: &mut Vec<f32>) -> Result<(), &'static str> {
    let Paint::Gradient(g) = p else {
        return Ok(());
    };
    match g {
        Gradient::Linear { from, to, .. } => nums.extend(from.iter().chain(to)),
        Gradient::Radial { centre, radius, .. } => {
            if *radius <= 0.0 {
                return Err("gradient radius not positive");
            }
            nums.extend(centre.iter().chain([radius]));
        }
    }
    let stops = g.stops();
    if stops.is_empty() || stops.len() > MAX_STOPS {
        return Err("gradient: 1 to 16 stops");
    }
    if stops.iter().any(|s| !(0.0..=1.0).contains(&s.0)) || stops.windows(2).any(|w| w[1].0 < w[0].0) {
        return Err("gradient stops: offsets in 0..1, not decreasing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(json: &str) -> Result<Scene, String> {
        let s: Scene = serde_json::from_str(json).map_err(|e| e.to_string())?;
        s.validate().map_err(|e| e.to_string())?;
        Ok(s)
    }

    #[test]
    fn a_menu_reads_with_its_defaults() {
        let s = scene(
            r##"{"version":2,"nodes":[
              {"kind":"group","id":"menu","anchor":"bottom-left","x":32,"y":-200,"level":2,"children":[
                {"kind":"rect","x":0,"y":0,"w":560,"h":136,"r":18,"fill":"#0c0e12bd"},
                {"kind":"text","x":32,"y":16,"size":34,"text":"Яркость","fill":"#ededed"},
                {"kind":"path","d":"M0 0 L10 0 l0 10 h-10 z","stroke":{"paint":{"linear":{"from":[0,0],"to":[10,0],"stops":[[0,"#ff0000"],[1,"#0000ff80"]]}},"width":2,"dash":[4,2]}}
              ]},
              {"kind":"group","space":"live","children":[{"kind":"path","d":"M0 0.5 H1","stroke":{"paint":"#ffffff","width":2}}]}
            ]}"##,
        )
        .unwrap();
        let Kind::Group(g) = &s.nodes[0].kind else { panic!() };
        assert_eq!((g.anchor, g.opacity, g.scale), (Anchor::BottomLeft, 1.0, Scale::Uniform(1.0)));
        assert!(s.nodes[0].visible);
        let Kind::Rect(r) = &g.children[0].kind else { panic!() };
        assert_eq!(r.style.fill, Some(Paint::Solid(Colour([12, 14, 18, 0xbd]))));
        let Kind::Path(p) = &g.children[2].kind else { panic!() };
        assert_eq!(p.d.segs, vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 0.0), Seg::Line(10.0, 10.0), Seg::Line(0.0, 10.0), Seg::Close]);
        // it writes back to what it read
        let again: Scene = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn the_demo_menus_are_valid() {
        for json in [include_str!("../../../tools/ui/demo-menu.json"), include_str!("../../../tools/ui/demo-menu-2.json")] {
            scene(json).unwrap();
        }
    }

    #[test]
    fn path_data_repeats_commands_and_reads_compact_numbers() {
        let p = PathData::try_from("m1 1 2 2-1.5.5e1Q0,0 1,1c1 1 2 2 3 3".to_string()).unwrap();
        assert_eq!(
            p.segs,
            vec![Seg::Move(1.0, 1.0), Seg::Line(3.0, 3.0), Seg::Line(1.5, 8.0), Seg::Quad(0.0, 0.0, 1.0, 1.0), Seg::Cubic(2.0, 2.0, 3.0, 3.0, 4.0, 4.0)]
        );
        for bad in ["L 1 1", "M 1", "M 1 1 A 1 1 0 0 0 2 2", "M 1 1 Z 3 3", "M nan 1"] {
            assert!(PathData::try_from(bad.to_string()).is_err(), "{bad}");
        }
    }

    #[test]
    fn validation_refuses_what_a_renderer_should_not_see() {
        let bad = |nodes: &str| scene(&format!(r#"{{"version":2,"nodes":[{nodes}]}}"#)).is_err();
        assert!(scene(r#"{"version":1}"#).is_err());
        assert!(bad(r##"{"kind":"rect","level":4,"x":0,"y":0,"w":1,"h":1,"fill":"#fff000"}"##));
        assert!(bad(r##"{"kind":"rect","x":0,"y":0,"w":1,"h":1}"##), "neither fill nor stroke");
        assert!(bad(r##"{"kind":"rect","x":1e9,"y":0,"w":1,"h":1,"fill":"#ffffff"}"##));
        assert!(bad(r##"{"kind":"rect","x":0,"y":0,"w":1,"h":1,"fill":"white"}"##));
        assert!(bad(r##"{"kind":"text","x":0,"y":0,"size":0,"text":"a","fill":"#ffffff"}"##));
        assert!(bad(r##"{"kind":"group","children":[{"kind":"group","anchor":"top"}]}"##), "anchor below the top");
        assert!(bad(r##"{"kind":"group","space":"live","anchor":"top"}"##));
        assert!(bad(r##"{"kind":"group","opacity":1.5}"##));
        assert!(bad(r##"{"kind":"group","id":"a"},{"kind":"group","id":"a"}"##), "id twice");
        assert!(bad(r##"{"kind":"ellipse","cx":0,"cy":0,"rx":1,"ry":1,"fill":{"radial":{"centre":[0,0],"radius":1,"stops":[[0.5,"#ffffff"],[0.2,"#000000"]]}}}"##));
        assert!(bad(r##"{"kind":"path","d":"M0 0 L1 1","stroke":{"paint":"#ffffff","width":1,"dash":[0,0]}}"##));
        let deep = (0..40).fold(String::from(r#"{"kind":"group"}"#), |inner, _| format!(r#"{{"kind":"group","children":[{inner}]}}"#));
        assert!(bad(&deep));
        assert!(!bad(r##"{"kind":"arc","cx":0,"cy":0,"r":10,"start":-90,"sweep":270,"stroke":{"paint":"#ffffff","width":3,"cap":"round"}}"##));
    }

    #[test]
    fn the_live_window_is_one_top_level_node() {
        let bad = |nodes: &str| scene(&format!(r#"{{"version":2,"nodes":[{nodes}]}}"#)).is_err();
        let s = scene(r#"{"version":2,"nodes":[{"kind":"live","x":-960,"y":0,"w":960,"h":540,"anchor":"top-right"}]}"#).unwrap();
        assert_eq!(s.nodes[0].kind, Kind::Live(LiveNode { x: -960.0, y: 0.0, w: 960.0, h: 540.0, anchor: Anchor::TopRight }));
        assert!(bad(r#"{"kind":"live","x":0,"y":0,"w":1,"h":1},{"kind":"live","x":0,"y":0,"w":1,"h":1}"#), "two");
        assert!(bad(r#"{"kind":"group","children":[{"kind":"live","x":0,"y":0,"w":1,"h":1}]}"#), "not top-level");
        assert!(bad(r#"{"kind":"live","x":0,"y":0,"w":0,"h":1}"#));
        assert!(bad(r#"{"kind":"live","visible":false,"x":0,"y":0,"w":1,"h":1}"#), "hidden: not yet");
    }

    #[test]
    fn a_full_colour_image_stays_an_opaque_straight_rectangle() {
        let ok = |nodes: &str| scene(&format!(r#"{{"version":2,"nodes":[{nodes}]}}"#)).is_ok();
        let img = r#"{"kind":"image","res":"p","x":0,"y":0,"w":10,"h":10,"full_colour":true}"#;
        assert!(ok(img));
        assert!(ok(&format!(r#"{{"kind":"group","x":5,"scale":2,"clip":{{"x":0,"y":0,"w":5,"h":5}},"children":[{img}]}}"#)));
        assert!(!ok(&format!(r#"{{"kind":"group","rotate":10,"children":[{img}]}}"#)));
        assert!(!ok(&format!(r#"{{"kind":"group","opacity":0.5,"children":[{img}]}}"#)));
        assert!(!ok(&format!(r#"{{"kind":"group","space":"live","children":[{img}]}}"#)));
        assert!(!ok(&format!(r#"{{"kind":"group","scale":[-1,1],"children":[{img}]}}"#)), "mirrored");
        assert!(!ok(&format!(r#"{{"kind":"group","mask":{img},"children":[]}}"#)), "inside a mask");
        // only a full-colour picture is prepared ahead
        assert!(ok(r#"{"kind":"image","res":"p","x":0,"y":0,"w":10,"h":10,"full_colour":true,"prepare":true}"#));
        assert!(!ok(r#"{"kind":"image","res":"p","x":0,"y":0,"w":10,"h":10,"prepare":true}"#));
        assert!(!ok(&format!(r#"{{"kind":"group","clip":{{"d":"M0 0 L5 0 L0 5 Z"}},"children":[{img}]}}"#)));
    }
}
