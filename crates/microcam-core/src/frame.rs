//! What of the scene an output shows in its video frame rather than in its menu layer, and
//! what the frame holds. The one place that decides it: the pipeline lays the frame out from it, and the renderer leaves
//! those images out of the menu layer (a hole at their place).
//!
//! An image goes into the frame when it is marked `full_colour` and shows at the output's level. Pictures may overlap
//! each other, lie over the live picture's window L or cover it. Derived from that ([`assemble`]): the visible live part
//! V = L − pictures and what the frame holds ([`Way`]). The frame is a canvas of a pool: the scaler port writes L into
//! it, GE lays the still content around.

use crate::scene::{Anchor, Clip, Kind, Node, Scene, MAX_LEVEL};
use crate::{Rect, Size};

/// One output's frame: where the live picture lies and the pictures in it, in the output's pixels, even-aligned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub raster: Size,
    pub live: Rect,
    /// bottom to top, overlapping as the scene has them
    pub pictures: Vec<Picture>,
    /// pictures laid out as if shown but prepared ahead (`prepare`): in no frame
    pub prepared: Vec<Picture>,
    /// the resources of pictures it would show whose size is not known yet (no window: not in `pictures`)
    pub sizing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// the image node's place in the tree ([`crate::scene::child_path`])
    pub node: u64,
    pub res: String,
    /// where in the output
    pub window: Rect,
    /// the part of the resource's picture shown, in its own pixels; scaled to the window
    pub crop: Rect,
}

/// An axis-aligned map from node coordinates to output pixels: `p = o + k · v` per axis.
#[derive(Clone, Copy)]
struct Map {
    o: (f32, f32),
    k: (f32, f32),
}

impl Map {
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        [self.o.0 + self.k.0 * x, self.o.1 + self.k.1 * y, self.k.0 * w, self.k.1 * h]
    }
}

/// The frame of an output of `raster` pixels at display `level`, its live picture `paused` or not; `full` gives a
/// resource's own size once known (an image whose size is not known yet takes no window).
pub fn plane(scene: &Scene, raster: Size, level: u8, paused: bool, full: &dyn Fn(&str) -> Option<Size>) -> Plane {
    let (w, h) = (raster.w as f32, raster.h as f32);
    let unit = h / 1080.0;
    let screen = [0.0, 0.0, w, h];
    let live = scene
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            Kind::Live(l) => {
                let (ax, ay) = l.anchor.point(w, h);
                Some(even(Map { o: (ax, ay), k: (unit, unit) }.rect(l.x, l.y, l.w, l.h), raster))
            }
            _ => None,
        })
        .unwrap_or(Rect::full(raster));
    let mut p = Plane { raster, live, pictures: Vec::new(), prepared: Vec::new(), sizing: Vec::new() };
    for (k, n) in scene.nodes.iter().enumerate() {
        let base = if n.level.is_some() { 0 } else { MAX_LEVEL };
        let anchor = match &n.kind {
            Kind::Group(g) => g.anchor,
            _ => Anchor::TopLeft,
        };
        let (ax, ay) = match &n.kind {
            Kind::Group(_) => anchor.point(w, h),
            _ => (0.0, 0.0),
        };
        walk(n, k as u64, Map { o: (ax, ay), k: (unit, unit) }, screen, base, (level, paused), raster, full, &mut p);
    }
    p
}

#[allow(clippy::too_many_arguments)]
fn walk(n: &Node, path: u64, map: Map, clip: [f32; 4], base: u8, out: (u8, bool), raster: Size, full: &dyn Fn(&str) -> Option<Size>, p: &mut Plane) {
    let Some(l) = n.shows(base, out.0, out.1) else { return };
    match &n.kind {
        Kind::Group(g) => {
            // only what a full-colour image may lie under goes on (validation keeps them there)
            let (sx, sy) = g.scale.xy();
            if g.rotate != 0.0 || sx <= 0.0 || sy <= 0.0 || g.opacity < 1.0 || g.mask.is_some() || g.keep || matches!(g.clip, Some(Clip::Path { .. })) {
                return;
            }
            let inner = Map { o: (map.o.0 + map.k.0 * g.x, map.o.1 + map.k.1 * g.y), k: (map.k.0 * sx, map.k.1 * sy) };
            let clip = match g.clip {
                Some(Clip::Rect { x, y, w, h }) => cut(clip, inner.rect(x, y, w, h)),
                _ => clip,
            };
            for (i, c) in g.children.iter().enumerate() {
                walk(c, crate::scene::child_path(path, i as u64), inner, clip, l, out, raster, full, p);
            }
        }
        Kind::Image(m) if m.full_colour => {
            let Some(size) = full(&m.res) else {
                if !m.prepare {
                    p.sizing.push(m.res.clone());
                }
                return;
            };
            let (fw, fh) = (size.w as f32, size.h as f32);
            let [cx0, cy0, cw, ch] = m.crop.unwrap_or([0.0, 0.0, fw, fh]);
            let (sx, sy) = match m.fit {
                crate::scene::Fit::Stretch => (m.w / cw, m.h / ch),
                crate::scene::Fit::Contain => {
                    let s = (m.w / cw).min(m.h / ch);
                    (s, s)
                }
                crate::scene::Fit::Cover => {
                    let s = (m.w / cw).max(m.h / ch);
                    (s, s)
                }
            };
            // the source rectangle placed centred in the node's, cut by it, by the clips and by the output
            let (dw, dh) = (cw * sx, ch * sy);
            let (dx, dy) = (m.x + (m.w - dw) / 2.0, m.y + (m.h - dh) / 2.0);
            let shown = cut(map.rect(dx, dy, dw, dh), cut(map.rect(m.x, m.y, m.w, m.h), clip));
            let window = even(shown, raster);
            if window.w < 2 || window.h < 2 {
                return;
            }
            let [px, py, pw, ph] = map.rect(dx, dy, dw, dh);
            // the window's edges back in the source's pixels
            let at = |v: f32, o: f32, len: f32, c0: f32, c: f32| c0 + (v - o) / len * c;
            let (x0, x1) = (at(window.x as f32, px, pw, cx0, cw), at((window.x + window.w) as f32, px, pw, cx0, cw));
            let (y0, y1) = (at(window.y as f32, py, ph, cy0, ch), at((window.y + window.h) as f32, py, ph, cy0, ch));
            let crop = even([x0, y0, x1 - x0, y1 - y0], size);
            if !crop.is_empty() {
                let to = if m.prepare { &mut p.prepared } else { &mut p.pictures };
                to.push(Picture { node: path, res: m.res.clone(), window, crop });
            }
        }
        _ => {}
    }
}

/// How finely to decode (in eighths, the JPEG decoder's steps) a picture whose part `crop` (its own pixels) is copied
/// into `window`: the coarsest whose copy shrinks by 1.2 or more on both axes — SCL dev 3 filters a shrink only from
/// there and lets anything between 1:1 and 1.2 alias — else the finest that does not shrink.
pub fn eighths(crop: Size, window: Size) -> u32 {
    let ratio = |n: u32| {
        let rw = (crop.w * n) as f32 / (8 * window.w.max(1)) as f32;
        let rh = (crop.h * n) as f32 / (8 * window.h.max(1)) as f32;
        (rw.min(rh), rw.max(rh))
    };
    (1..=8).find(|&n| ratio(n).0 >= 1.2).or_else(|| (1..=8).rev().find(|&n| ratio(n).1 <= 1.01)).unwrap_or(8)
}

/// The size of a `full`-size picture's NV12 form whose part `crop` (its own pixels) is copied into `window`: the size
/// that makes the copy 1:1, made from the picture by SCL dev 3's filtered shrink. Per axis: no larger than the picture
/// (the copy enlarges the rest); where it would be smaller than the picture by less than 1.2 — a shrink SCL dev 3 lets
/// alias — the picture shrunk by 1.2, the copy enlarging by the rest. Even
/// sides (NV12).
pub fn form_size(full: Size, crop: Size, window: Size) -> Size {
    let axis = |full: u32, crop: u32, window: u32| {
        let t = if crop == 0 { full } else { (full as u64 * window as u64).div_ceil(crop as u64).min(full as u64) as u32 };
        let v = if t + 1 >= full {
            full
        } else if full * 5 < t * 6 {
            full * 5 / 6
        } else {
            t
        };
        (v & !1).max(2)
    };
    Size::new(axis(full.w, crop.w, window.w), axis(full.h, crop.h, window.h))
}

/// A scaler port writes a window only on this grid of columns: its start rounded down, its width up.
pub const PORT_GRID: u32 = 16;

/// What an output's frame holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Way {
    /// the live picture alone, over the whole frame
    #[default]
    Live,
    /// the live picture's visible part beside or under still content
    Canvas,
    /// no live part shows: still content only
    Covered,
}

/// What one output's frame holds, from a [`Plane`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembly {
    pub way: Way,
    /// the live picture's visible part V, disjoint rectangles
    pub visible: Vec<Rect>,
}

impl Plane {
    /// What this frame holds.
    pub fn assemble(&self) -> Assembly {
        assemble(self.raster, self.live, &self.pictures.iter().map(|p| p.window).collect::<Vec<_>>())
    }
}

/// What a frame of `raster` holds with the live picture in `live` and pictures at `windows`, bottom to top.
pub fn assemble(raster: Size, live: Rect, windows: &[Rect]) -> Assembly {
    let visible = region(&[live], windows, &[]);
    let way = match () {
        _ if visible.is_empty() => Way::Covered,
        _ if live == Rect::full(raster) && windows.iter().all(|w| w.is_empty()) => Way::Live,
        _ => Way::Canvas,
    };
    Assembly { way, visible }
}

/// The part of the union of `include` outside every `exclude`, as disjoint rectangles cut also at the edges of `cuts`:
/// horizontal bands between all edges, the covered spans of each band, a span continuing one of the band above (same
/// columns, no cut between them) joined to it.
pub fn region(include: &[Rect], exclude: &[Rect], cuts: &[Rect]) -> Vec<Rect> {
    let mut ys: Vec<u32> = include.iter().chain(exclude).chain(cuts).filter(|r| !r.is_empty()).flat_map(|r| [r.y, r.y + r.h]).collect();
    ys.sort_unstable();
    ys.dedup();
    let mut xs: Vec<u32> = cuts.iter().filter(|r| !r.is_empty()).flat_map(|r| [r.x, r.x + r.w]).collect();
    xs.sort_unstable();
    xs.dedup();
    let cut_ys: Vec<u32> = cuts.iter().filter(|r| !r.is_empty()).flat_map(|r| [r.y, r.y + r.h]).collect();
    let mut done = Vec::new();
    let mut open: Vec<Rect> = Vec::new();
    for b in ys.windows(2) {
        let (y0, y1) = (b[0], b[1]);
        let across = |r: &&Rect| !r.is_empty() && r.y <= y0 && r.y + r.h >= y1;
        let mut spans: Vec<(u32, u32)> = include.iter().filter(across).map(|r| (r.x, r.x + r.w)).collect();
        spans.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (a, z) in spans {
            match merged.last_mut() {
                Some(m) if a <= m.1 => m.1 = m.1.max(z),
                _ => merged.push((a, z)),
            }
        }
        for r in exclude.iter().filter(across) {
            let (a, z) = (r.x, r.x + r.w);
            merged = merged.into_iter().flat_map(|(s, e)| [(s, e.min(a)), (s.max(z), e)]).filter(|(s, e)| s < e).collect();
        }
        let mut cut: Vec<(u32, u32)> = Vec::new();
        for (s, e) in merged {
            let mut at = s;
            for &x in xs.iter().filter(|&&x| x > s && x < e) {
                cut.push((at, x));
                at = x;
            }
            cut.push((at, e));
        }
        cut.sort_unstable();
        let mut next = Vec::new();
        for (x0, x1) in cut {
            match open.iter().position(|o| o.x == x0 && o.x + o.w == x1 && o.y + o.h == y0 && !cut_ys.contains(&y0)) {
                Some(i) => {
                    let mut o = open.swap_remove(i);
                    o.h = y1 - o.y;
                    next.push(o);
                }
                None => next.push(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }),
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    done.sort_by_key(|r| (r.y, r.x));
    done
}

/// The common part of two `[x, y, w, h]` rectangles (zero-sized when apart).
fn cut(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// A rectangle as whole even pixels inside `bounds` (NV12 windows and crops need even edges).
fn even(r: [f32; 4], bounds: Size) -> Rect {
    let e = |v: f32, max: u32| ((v.round().max(0.0) as u32).min(max)) & !1;
    let (x0, y0) = (e(r[0], bounds.w), e(r[1], bounds.h));
    let (x1, y1) = (e(r[0] + r[2], bounds.w), e(r[1] + r[3], bounds.h));
    Rect { x: x0, y: y0, w: x1.saturating_sub(x0), h: y1.saturating_sub(y0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane_of(json: &str, raster: Size, level: u8) -> Plane {
        let s: Scene = serde_json::from_str(&format!(r#"{{"version":2,"nodes":[{json}]}}"#)).unwrap();
        s.validate().unwrap();
        plane(&s, raster, level, false, &|_| Some(Size::new(4000, 3000)))
    }

    /// A picture prepared ahead is laid out where a scene showing it would put it, in no frame; a shown picture whose
    /// size is not known yet has no window and is named as waiting for it, a prepared one is not.
    #[test]
    fn a_prepared_picture_is_laid_out_but_in_no_frame() {
        let json = r#"{"kind":"image","res":"next","x":0,"y":0,"w":1920,"h":1080,"fit":"cover","full_colour":true,"prepare":true},
                      {"kind":"image","res":"now","x":0,"y":0,"w":1920,"h":1080,"fit":"cover","full_colour":true}"#;
        let uhd = Size::new(3840, 2160);
        let p = plane_of(json, uhd, MAX_LEVEL);
        assert_eq!(p.pictures.iter().map(|q| q.res.as_str()).collect::<Vec<_>>(), ["now"]);
        assert_eq!(p.prepared.len(), 1);
        assert_eq!((p.prepared[0].window, p.prepared[0].crop), (p.pictures[0].window, p.pictures[0].crop));
        assert_eq!(p.assemble().way, Way::Covered);
        let s: Scene = serde_json::from_str(&format!(r#"{{"version":2,"nodes":[{json}]}}"#)).unwrap();
        let unknown = plane(&s, uhd, MAX_LEVEL, false, &|_| None);
        assert_eq!((unknown.pictures.len(), unknown.prepared.len(), unknown.sizing), (0, 0, vec!["now".to_string()]));
    }

    const FHD: Size = Size::new(1920, 1080);

    #[test]
    fn a_decode_is_a_shrink_of_1_2_or_more_or_no_shrink() {
        // a 4K photo into a 1920 window: 5/8 gives 2400 (1.25), the coarsest that shrinks enough
        assert_eq!(eighths(Size::new(3840, 2160), Size::new(1920, 1080)), 5);
        // into a 3500 window: 8/8 would shrink by 1.1 (aliasing), 7/8 enlarges
        assert_eq!(eighths(Size::new(3840, 2160), Size::new(3500, 1968)), 7);
        // the whole picture 1:1
        assert_eq!(eighths(Size::new(3840, 2160), Size::new(3840, 2160)), 8);
        // a thumbnail: 1/8 already shrinks enough
        assert_eq!(eighths(Size::new(3840, 2160), Size::new(320, 180)), 1);
        // larger than the picture: all of it, enlarged
        assert_eq!(eighths(Size::new(1000, 750), Size::new(1920, 1440)), 8);
    }

    #[test]
    fn a_form_is_its_window_s_size_unless_that_aliases() {
        let k4 = Size::new(3840, 2160);
        // the whole 4K photo into a 1080p window: the window's size, the copy 1:1
        assert_eq!(form_size(k4, k4, FHD), FHD);
        // a quarter of it into the 1080p window: the picture at twice the window
        assert_eq!(form_size(k4, Size::new(1920, 1080), Size::new(1920, 1080)), k4);
        assert_eq!(form_size(k4, Size::new(960, 540), FHD), k4);
        // into a 3500 window: a shrink by 1.1 aliases — shrunk by 1.2, the copy enlarging
        assert_eq!(form_size(k4, k4, Size::new(3500, 1968)), Size::new(3200, 1800));
        // exactly 1.2: the window's size
        assert_eq!(form_size(k4, k4, Size::new(3200, 1800)), Size::new(3200, 1800));
        // odd sides rounded down to even
        assert_eq!(form_size(Size::new(1001, 751), Size::new(1001, 751), Size::new(501, 375)), Size::new(500, 374));
    }

    #[test]
    fn without_a_live_node_the_live_picture_takes_the_screen() {
        let p = plane_of(r##"{"kind":"rect","x":0,"y":0,"w":1,"h":1,"fill":"#ffffff"}"##, FHD, 3);
        assert_eq!(p, Plane { raster: FHD, live: Rect::full(FHD), pictures: vec![], prepared: vec![], sizing: vec![] });
    }

    #[test]
    fn a_picture_beside_the_live_window_goes_into_the_frame_in_output_pixels() {
        // the live picture in the right half of a 4K screen (anchored top-right), a 4:3 photo contained in the left half
        let p = plane_of(
            r#"{"kind":"live","anchor":"top-right","x":-960,"y":0,"w":960,"h":1080},
               {"kind":"image","res":"ph","x":0,"y":0,"w":960,"h":1080,"full_colour":true}"#,
            Size::new(3840, 2160),
            3,
        );
        assert_eq!(p.live, Rect { x: 1920, y: 0, w: 1920, h: 2160 });
        // 4000×3000 into 1920×2160: 1920 wide, 1440 high, centred
        assert_eq!(p.pictures, vec![Picture { node: 1, res: "ph".into(), window: Rect { x: 0, y: 360, w: 1920, h: 1440 }, crop: Rect { x: 0, y: 0, w: 4000, h: 3000 } }]);
    }

    #[test]
    fn cover_cuts_the_source_and_clips_cut_the_crop() {
        let p = plane_of(
            r#"{"kind":"live","x":1000,"y":0,"w":920,"h":540},
               {"kind":"group","x":0,"y":0,"clip":{"x":0,"y":0,"w":540,"h":540},"children":[
                 {"kind":"image","res":"ph","x":0,"y":0,"w":1080,"h":540,"fit":"cover","full_colour":true}]}"#,
            FHD,
            3,
        );
        let q = &p.pictures[0];
        // cover: 4000×3000 scaled to 1080 wide (810 high, centred: 135 cut above and below); the clip keeps the left half
        assert_eq!(q.window, Rect { x: 0, y: 0, w: 540, h: 540 });
        assert_eq!(q.crop, Rect { x: 0, y: 500, w: 2000, h: 2000 });
        assert_eq!(q.node, crate::scene::child_path(1, 0));
    }

    #[test]
    fn pictures_over_the_live_window_and_overlapping_go_into_the_frame_unless_below_the_level_or_unmarked() {
        let img = |x: u32, extra: &str| format!(r#"{{"kind":"image","res":"p","x":{x},"y":0,"w":400,"h":300,"full_colour":true{extra}}}"#);
        // over the live picture that takes the screen
        assert_eq!(plane_of(&img(0, ""), FHD, 3).pictures.len(), 1);
        let live = r#"{"kind":"live","x":1400,"y":0,"w":520,"h":300}"#;
        // two that overlap: both, bottom to top
        let p = plane_of(&format!("{live},{},{}", img(0, ""), img(200, "")), FHD, 3);
        assert_eq!(p.pictures.iter().map(|q| q.node).collect::<Vec<_>>(), vec![1, 2]);
        // a top-level node without a level shows with the full menu only
        assert!(plane_of(&format!("{live},{}", img(0, "")), FHD, 2).pictures.is_empty());
        assert_eq!(plane_of(&format!("{live},{}", img(0, r#","level":1"#)), FHD, 1).pictures.len(), 1);
        // not marked: the menu layer
        assert!(plane_of(&format!(r#"{live},{{"kind":"image","res":"p","x":0,"y":0,"w":400,"h":300}}"#), FHD, 3).pictures.is_empty());
    }

    const UHD: Size = Size::new(3840, 2160);

    fn r(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    /// A plane of `raster` with the live window `live` and pictures at `windows`, bottom to top (node = index + 1).
    fn laid(raster: Size, live: Rect, windows: &[Rect]) -> Plane {
        let pictures = windows.iter().enumerate().map(|(i, w)| Picture { node: i as u64 + 1, res: format!("p{i}"), window: *w, crop: Rect::full(w.size()) }).collect();
        Plane { raster, live, pictures, prepared: vec![], sizing: vec![] }
    }


    /// The rectangles are disjoint and cover exactly `area` pixels inside `within`.
    fn tiles(rects: &[Rect], area: u64, within: Rect) {
        for (i, a) in rects.iter().enumerate() {
            assert!(within.contains(*a), "{a:?} outside {within:?}");
            for b in &rects[i + 1..] {
                assert!(a.intersect(*b).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(rects.iter().map(|r| r.area()).sum::<u64>(), area);
    }

    #[test]
    fn a_region_is_disjoint_rectangles_cut_at_the_given_edges() {
        // two overlapping squares: their union, 2·100² − 50²
        let u = region(&[r(0, 0, 100, 100), r(50, 50, 100, 100)], &[], &[]);
        tiles(&u, 2 * 10_000 - 2_500, r(0, 0, 150, 150));
        assert_eq!(u.len(), 3);
        // a frame minus a picture in its middle: four rectangles around it
        let v = region(&[r(0, 0, 300, 300)], &[r(100, 100, 100, 100)], &[]);
        tiles(&v, 90_000 - 10_000, r(0, 0, 300, 300));
        assert_eq!(v.len(), 4);
        // cut at a window's edges: no rectangle crosses them
        let c = region(&[r(0, 0, 200, 100)], &[], &[r(50, 0, 100, 50)]);
        tiles(&c, 20_000, r(0, 0, 200, 100));
        assert!(c.iter().all(|x| r(50, 0, 100, 50).contains(*x) || x.intersect(r(50, 0, 100, 50)).is_empty()));
    }

    #[test]
    fn a_frame_is_live_canvas_or_covered() {
        let full = Rect::full(UHD);
        // nothing: the live picture alone
        assert_eq!(laid(UHD, full, &[]).assemble().way, Way::Live);
        // a quarter picture at the top left: three quarters of the live picture show
        let a = laid(UHD, full, &[r(0, 0, 1920, 1080)]).assemble();
        assert_eq!(a.way, Way::Canvas);
        tiles(&a.visible, UHD.w as u64 * UHD.h as u64 * 3 / 4, full);
        // the whole frame covered: no live part
        let a = laid(UHD, full, &[r(0, 0, 3840, 2160)]).assemble();
        assert_eq!((a.way, a.visible.len()), (Way::Covered, 0));
        // the live picture in the right half, nothing over it
        assert_eq!(laid(UHD, r(1920, 0, 1920, 2160), &[]).assemble().way, Way::Canvas);
    }
}
