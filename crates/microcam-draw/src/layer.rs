//! How an output's layer is brought from the items it shows to new ones: what changed
//! ([`damage`]), and the work that redraws it ([`plan`]) — each damaged area in bands, every band composed in a small
//! scratch from the tiles that touch it, bottom to top, then copied into the layer whole. Nothing is ever erased in the
//! layer itself, so an erased state is never scanned out. The plan is pure data; the GE (`mi::layer`) or the CPU
//! ([`run_cpu`], the reference) carries it out.

use microcam_core::Rect;
use tiny_skia as sk;

use crate::render::Item;

/// Above this many cells the middle of a diff is not matched item by item: all of it is damage.
const LCS_CELLS: usize = 65_536;

/// One tile part composed into the scratch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blit {
    /// the item whose tile it is
    pub item: usize,
    /// the part of the tile
    pub src: Rect,
    /// where it goes in the scratch
    pub x: u32,
    pub y: u32,
}

/// One band: the scratch cleared over `rect`'s size, `blits` laid over it in order, then copied to `rect` of the layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Band {
    pub rect: Rect,
    pub blits: Vec<Blit>,
}

/// The layer's areas whose pixels differ between showing `old` and showing `new`: the bounds of every item outside
/// their longest common subsequence (both where it was and where it is), merged so no pixel is redrawn twice, inside
/// `clip` (the part of the output frame the layer holds). A pixel whose items all lie in the common part keeps the same
/// items in the same order, so it is unchanged.
pub fn damage(old: &[Item], new: &[Item], clip: Rect) -> Vec<Rect> {
    // the common ends cost nothing to match
    let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suf = old[pre..].iter().rev().zip(new[pre..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (o, n) = (&old[pre..old.len() - suf], &new[pre..new.len() - suf]);
    let mut out = Vec::new();
    if (o.len() + 1) * (n.len() + 1) > LCS_CELLS {
        out.extend(o.iter().chain(n).map(|i| i.rect));
    } else {
        let (a, b) = (o.len(), n.len());
        let at = |i: usize, j: usize| i * (b + 1) + j;
        let mut lcs = vec![0u16; (a + 1) * (b + 1)];
        for i in (0..a).rev() {
            for j in (0..b).rev() {
                lcs[at(i, j)] = if o[i] == n[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a || j < b {
            if i < a && j < b && o[i] == n[j] {
                i += 1;
                j += 1;
            } else if j < b && (i == a || lcs[at(i, j + 1)] >= lcs[at(i + 1, j)]) {
                out.push(n[j].rect);
                j += 1;
            } else {
                out.push(o[i].rect);
                i += 1;
            }
        }
    }
    merge(out.into_iter().map(|r| r.intersect(clip)).filter(|r| !r.is_empty()).collect())
}

/// The work that redraws `damage` with `items`: bands at most `rows` tall (a scratch as wide as the layer), each with
/// the tile parts that touch it.
pub fn plan(items: &[Item], damage: &[Rect], rows: u32) -> Vec<Band> {
    let mut bands = Vec::new();
    for d in damage {
        let rows = rows.clamp(1, d.h.max(1));
        let mut y = d.y;
        while y < d.y + d.h {
            let rect = Rect { x: d.x, y, w: d.w, h: rows.min(d.y + d.h - y) };
            let blits = items
                .iter()
                .enumerate()
                .filter_map(|(k, it)| {
                    let common = it.rect.intersect(rect);
                    (!common.is_empty()).then(|| Blit {
                        item: k,
                        src: Rect { x: common.x - it.rect.x, y: common.y - it.rect.y, w: common.w, h: common.h },
                        x: common.x - rect.x,
                        y: common.y - rect.y,
                    })
                })
                .collect();
            bands.push(Band { rect, blits });
            y += rect.h;
        }
    }
    bands
}

/// Carry a plan out on the CPU into a premultiplied `layer`, with each item's tile laid at its fade: the reference for
/// the GE.
pub fn run_cpu(bands: &[Band], items: &[Item], tiles: &[&sk::Pixmap], layer: &mut sk::Pixmap) {
    for b in bands {
        let Some(mut scratch) = sk::Pixmap::new(b.rect.w, b.rect.h) else {
            continue;
        };
        for bl in b.blits.iter() {
            let Some(part) = crate::render::spread(&items[bl.item], tiles[bl.item], bl.src) else {
                continue;
            };
            let paint = sk::PixmapPaint { opacity: items[bl.item].fade, ..Default::default() };
            scratch.draw_pixmap(bl.x as i32, bl.y as i32, part.as_ref(), &paint, sk::Transform::identity(), None);
        }
        // the copy replaces what the layer held there
        let w = layer.width() as usize;
        let data = layer.data_mut();
        for row in 0..b.rect.h as usize {
            let dst = ((b.rect.y as usize + row) * w + b.rect.x as usize) * 4;
            let src = row * b.rect.w as usize * 4;
            data[dst..dst + b.rect.w as usize * 4].copy_from_slice(&scratch.data()[src..src + b.rect.w as usize * 4]);
        }
    }
}

/// Where an encoder output's menu lies over still content: each shown
/// item's rect (holes and items at no opacity show nothing of their own) inside each of the `still` rectangles (the
/// frame outside the visible live part, disjoint), widened to even columns and rows (NV12) as far as that rectangle
/// goes, and merged within it so no pixel is baked twice. The patches to bake.
pub fn bake_areas(items: &[Item], still: &[Rect]) -> Vec<Rect> {
    let mut out = Vec::new();
    for s in still {
        let (sx0, sy0) = (s.x.next_multiple_of(2), s.y.next_multiple_of(2));
        let (sx1, sy1) = ((s.x + s.w) & !1, (s.y + s.h) & !1);
        let parts: Vec<Rect> = items
            .iter()
            .filter(|i| !i.hole && i.fade > 0.0)
            .map(|i| i.rect.intersect(*s))
            .filter(|r| !r.is_empty())
            .filter_map(|r| {
                let (x0, y0) = ((r.x & !1).max(sx0), (r.y & !1).max(sy0));
                let (x1, y1) = ((r.x + r.w).next_multiple_of(2).min(sx1), (r.y + r.h).next_multiple_of(2).min(sy1));
                (x1 > x0 && y1 > y0).then(|| Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
            })
            .collect();
        out.extend(merge(parts));
    }
    out
}

/// Overlapping or touching rectangles joined, so no pixel is drawn twice.
pub fn merge(mut rs: Vec<Rect>) -> Vec<Rect> {
    let touch = |a: &Rect, b: &Rect| a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h;
    let mut i = 0;
    while i < rs.len() {
        let mut j = i + 1;
        let mut grew = false;
        while j < rs.len() {
            if touch(&rs[i], &rs[j]) {
                let b = rs.swap_remove(j);
                rs[i] = rs[i].union(b);
                grew = true;
            } else {
                j += 1;
            }
        }
        if !grew {
            i += 1;
        }
    }
    rs
}

/// One hardware window's worth of an output's items: items whose rectangles overlap are one
/// cluster, so every pixel lies under one window (windows do not blend with each other); the items of an own-window
/// group are one cluster whether they overlap or not, so the group moves as one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cluster {
    /// in the output frame, inside the clip
    pub rect: Rect,
    /// indices into the items, bottom to top
    pub items: Vec<usize>,
    /// the own-window group whose window it is: it floats over the other windows and carries, inside its rectangle,
    /// every item under and over the group too (its window hides theirs there)
    pub own: Option<u64>,
}

/// The clusters of `items` inside `clip` (the part of the output frame the layer holds), at most `max` (the GOP's
/// windows per port). Own-window groups float first: each its own window (overlapping ones share one), the topmost in
/// z keeping theirs when they outnumber the windows; the other items cluster in the windows left, and while there are
/// too many, the pair whose one window would fetch the least extra area goes together.
pub fn clusters(items: &[Item], clip: Rect, max: usize) -> Vec<Cluster> {
    let max = max.max(1);
    let mut floating: Vec<Cluster> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let (Some(o), r) = (it.own, it.rect.intersect(clip)) else {
            continue;
        };
        if r.is_empty() {
            continue;
        }
        match floating.iter().position(|c| c.own == Some(o)) {
            Some(k) => {
                floating[k].rect = floating[k].rect.union(r);
                floating[k].items.push(i);
            }
            None => floating.push(Cluster { rect: r, items: vec![i], own: Some(o) }),
        }
    }
    // overlapping own groups share a window, named by the topmost
    'own: loop {
        for i in 0..floating.len() {
            for j in i + 1..floating.len() {
                if !floating[i].rect.intersect(floating[j].rect).is_empty() {
                    let b = floating.swap_remove(j);
                    let a = &mut floating[i];
                    a.rect = a.rect.union(b.rect);
                    if b.items.last() > a.items.last() {
                        a.own = b.own;
                    }
                    a.items.extend(b.items);
                    a.items.sort_unstable();
                    continue 'own;
                }
            }
        }
        break;
    }
    // one window at least for the rest; the lowest own groups beyond that are drawn with it
    floating.sort_by_key(|c| std::cmp::Reverse(c.items.last().copied()));
    let sunk: Vec<usize> = floating.drain(floating.len().min(max - 1)..).flat_map(|c| c.items).collect();
    let mut v: Vec<Cluster> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if it.own.is_some() && !sunk.contains(&i) {
            continue;
        }
        let r = it.rect.intersect(clip);
        if !r.is_empty() {
            v.push(Cluster { rect: r, items: vec![i], own: None });
        }
    }
    fn join(v: &mut Vec<Cluster>, i: usize, j: usize) {
        let b = v.swap_remove(j);
        let a = &mut v[i];
        a.rect = a.rect.union(b.rect);
        a.items.extend(b.items);
        a.items.sort_unstable();
    }
    let room = max - floating.len();
    loop {
        'overlap: loop {
            for i in 0..v.len() {
                for j in i + 1..v.len() {
                    if !v[i].rect.intersect(v[j].rect).is_empty() {
                        join(&mut v, i, j);
                        continue 'overlap;
                    }
                }
            }
            break;
        }
        if v.len() <= room {
            break;
        }
        let mut best = (u64::MAX, 0, 0);
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                let cost = v[i].rect.union(v[j].rect).area() - v[i].rect.area() - v[j].rect.area();
                if cost < best.0 {
                    best = (cost, i, j);
                }
            }
        }
        join(&mut v, best.1, best.2);
    }
    // a floating window carries everything inside its rectangle, in z order
    for f in &mut floating {
        f.items = (0..items.len()).filter(|&i| !items[i].rect.intersect(f.rect).is_empty()).collect();
    }
    v.extend(floating);
    v
}

/// The items a port's covers can show instead of its windows: solid ones, inside `clip`, that no
/// item below them in z touches (a cover lies under every window), at most `max`, the largest; indices, bottom to top.
pub fn covers(items: &[Item], clip: Rect, max: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..items.len())
        .filter(|&i| {
            let r = items[i].rect.intersect(clip);
            items[i].solid.is_some() && !r.is_empty() && items[..i].iter().all(|b| b.rect.intersect(clip).intersect(r).is_empty())
        })
        .collect();
    v.sort_by_key(|&i| std::cmp::Reverse(items[i].rect.intersect(clip).area()));
    v.truncate(max);
    v.sort_unstable();
    v
}

/// The items an RGN layer shows: those touching the `visible` live part of the frame, bottom to top. One wholly over
/// still content lies in the patches only — the compositor puts the still content back over every port write there.
pub fn on_live(items: &[Item], visible: &[Rect]) -> Vec<Item> {
    items.iter().filter(|i| visible.iter().any(|v| !i.rect.intersect(*v).is_empty())).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{compose, items, rasterise, Fonts};
    use microcam_core::scene::{Scene, MAX_LEVEL};
    use microcam_core::Size;
    use microcam_core::ui::OutputUi;
    use std::sync::Arc;

    fn scene(nodes: &str) -> Scene {
        let s: Scene = serde_json::from_str(&format!(r#"{{"version":2,"nodes":[{nodes}]}}"#)).unwrap();
        s.validate().unwrap();
        s
    }

    /// Show each scene in turn through damage + plan + CPU run, and check the layer equals a full composition each time.
    fn replay(scenes: &[Scene], size: Size, rows: u32) -> Vec<u64> {
        let out = OutputUi { raster: size, live: Rect::full(size), level: MAX_LEVEL, paused: None };
        let mut f = Fonts::new();
        let mut layer = sk::Pixmap::new(size.w, size.h).unwrap();
        let mut shown: Vec<Item> = Vec::new();
        let mut drawn = Vec::new();
        for s in scenes {
            let new = items(s, &out, &mut f);
            let tiles: Vec<sk::Pixmap> = new.iter().map(|i| rasterise(i).unwrap()).collect();
            let d = damage(&shown, &new, Rect::full(size));
            drawn.push(d.iter().map(|r| r.area()).sum());
            run_cpu(&plan(&new, &d, rows), &new, &tiles.iter().collect::<Vec<_>>(), &mut layer);
            let pairs: Vec<_> = new.iter().cloned().zip(tiles.into_iter().map(Arc::new)).collect();
            let want = compose(&pairs, Rect::full(size)).unwrap();
            assert!(layer.data() == want.data(), "the layer differs from the full composition");
            shown = new;
        }
        drawn
    }

    #[test]
    fn moves_overlaps_reorders_and_removals_leave_the_layer_right() {
        let a = r##"{"kind":"rect","x":10,"y":10,"w":60,"h":40,"r":8,"fill":"#ff000080"}"##;
        let b = |x: u32| format!(r##"{{"kind":"ellipse","cx":{x},"cy":40,"rx":30,"ry":20,"fill":"#00ff00a0"}}"##);
        let t = r##"{"kind":"text","x":20,"y":60,"size":30,"text":"Снимок","fill":"#ffffff"}"##;
        let scenes = [
            scene(&format!("{a},{},{t}", b(50))),
            scene(&format!("{a},{},{t}", b(90))),
            scene(&format!("{},{a},{t}", b(90))),
            scene(&format!("{a},{t}")),
            scene(""),
        ];
        // a small scratch splits every area into many bands
        let drawn = replay(&scenes, Size::new(320, 180), 7);
        assert!(drawn.iter().all(|&d| d > 0));
    }

    #[test]
    fn a_fade_costs_no_new_tile_and_only_its_group_is_redrawn() {
        let g = |o: f32| scene(&format!(r##"{{"kind":"rect","x":0,"y":0,"w":40,"h":40,"fill":"#00ff00"}},{{"kind":"group","opacity":{o},"children":[{{"kind":"rect","x":20,"y":20,"w":60,"h":40,"fill":"#ff0000"}},{{"kind":"text","x":24,"y":24,"size":20,"text":"Ок","fill":"#ffffff"}}]}}"##));
        // screen units are output pixels at 1080 rows
        let size = Size::new(1920, 1080);
        let out = OutputUi { raster: size, live: Rect::full(size), level: MAX_LEVEL, paused: None };
        let (a, b) = (items(&g(0.3), &out, &mut Fonts::new()), items(&g(0.7), &out, &mut Fonts::new()));
        assert_eq!((a[1].key, a[1].fade, b[1].fade), (b[1].key, 0.3, 0.7));
        assert!(a[1].opaque && a[0].opaque);
        let drawn = replay(&[g(0.3), g(0.7), g(1.0), g(0.0)], size, 16);
        assert_eq!(drawn[1], 60 * 40);
    }

    #[test]
    fn an_unchanged_scene_costs_nothing_and_a_selection_move_costs_only_its_items() {
        let menu = |sel: u32| {
            let rows: Vec<String> = (0..6)
                .map(|k| format!(r##"{{"kind":"text","x":20,"y":{},"size":28,"text":"Строка {k}","fill":"{}"}}"##, 20 + 40 * k, if k == sel { "#ffb300" } else { "#ededed" }))
                .collect();
            scene(&format!(r##"{{"kind":"rect","x":10,"y":{},"w":200,"h":36,"fill":"#ffb30038"}},{}"##, 16 + 40 * sel, rows.join(",")))
        };
        let drawn = replay(&[menu(0), menu(0), menu(3)], Size::new(640, 360), 100);
        assert_eq!(drawn[1], 0);
        // the highlight and two rows, not the whole menu
        assert!(drawn[2] < 4 * 200 * 40, "{drawn:?}");
    }

    #[test]
    fn bake_areas_lie_inside_the_still_part_on_even_edges() {
        use crate::render::test_item as item;
        let r = |x, y, w, h| Rect { x, y, w, h };
        // a panel across the live window's left edge, a cross item over two still rectangles, a hole, a faded-out item
        let mut hole = item(4, r(0, 0, 400, 400), None);
        hole.hole = true;
        let mut gone = item(5, r(900, 900, 50, 50), None);
        gone.fade = 0.0;
        let its = [hole, item(1, r(301, 101, 200, 99), None), item(2, r(310, 150, 20, 20), None), item(3, r(50, 590, 40, 30), None), gone];
        // the still part: left of the live window (x 400…) and below it
        let still = [r(0, 0, 400, 600), r(400, 600, 600, 400)];
        let b = bake_areas(&its, &still);
        assert_eq!(b, vec![r(300, 100, 100, 100), r(50, 590, 40, 10)]);
        assert!(b.iter().all(|p| p.x % 2 == 0 && p.y % 2 == 0 && p.w % 2 == 0 && p.h % 2 == 0));
    }

    #[test]
    fn the_layer_takes_only_items_touching_the_visible_live_part() {
        use crate::render::test_item as item;
        let r = |x, y, w, h| Rect { x, y, w, h };
        // the compare scene at 4K: a frozen picture over the left half, the live picture visible in the right half
        let visible = [r(1920, 0, 1920, 2160)];
        let mut hole = item(1, r(0, 0, 1920, 2160), None);
        hole.hole = true;
        let mut chip = item(4, r(2400, 100, 300, 80), None);
        chip.solid = Some([0, 0, 0]);
        let mut over_picture = item(5, r(100, 100, 300, 80), None);
        over_picture.solid = Some([0, 0, 0]);
        // the line across the seam touches the live part; the label over the picture does not
        let its = [hole, item(2, r(1916, 0, 8, 2160), None), item(3, r(200, 1900, 400, 60), None), chip, over_picture];
        let on = on_live(&its, &visible);
        assert_eq!(on.iter().map(|i| i.key).collect::<Vec<_>>(), vec![2, 4]);
        // a hole reaching into the live part stays, so it keeps cutting what lies under it there
        let mut wide = item(6, r(0, 0, 2000, 2160), None);
        wide.hole = true;
        assert_eq!(on_live(&[wide], &visible).len(), 1);
        // a frame all live: every item; covered: none
        assert_eq!(on_live(&its, &[r(0, 0, 3840, 2160)]).len(), its.len());
        assert!(on_live(&its, &[]).is_empty());
    }

    #[test]
    fn bands_cover_their_area_and_stay_within_the_scratch() {
        let s = scene(r##"{"kind":"rect","x":0,"y":0,"w":300,"h":170,"fill":"#ffffff"}"##);
        let out = OutputUi { raster: Size::new(320, 180), live: Rect::full(Size::new(320, 180)), level: MAX_LEVEL, paused: None };
        let it = items(&s, &out, &mut Fonts::new());
        let d = damage(&[], &it, Rect::full(out.raster));
        let bands = plan(&it, &d, 10);
        assert!(bands.iter().all(|b| b.rect.h <= 10));
        assert_eq!(bands.iter().map(|b| b.rect.area()).sum::<u64>(), d.iter().map(|r| r.area()).sum::<u64>());
    }

    #[test]
    fn clusters_hold_everything_without_overlap_and_within_the_limit() {
        use crate::render::test_item as item;
        let r = |x, y, w, h| Rect { x, y, w, h };
        let clip = r(0, 0, 1920, 1080);
        // two overlapping become one, a far one stays apart, the part outside the clip is cut off
        let its = [item(1, r(10, 10, 100, 50), None), item(2, r(90, 40, 100, 50), None), item(3, r(1800, 1000, 200, 200), None)];
        let c = clusters(&its, clip, 8);
        assert_eq!(c.iter().map(|c| (c.rect, c.items.clone())).collect::<Vec<_>>(), vec![(r(10, 10, 180, 80), vec![0, 1]), (r(1800, 1000, 120, 80), vec![2])]);
        // a column of menu lines: down to the limit, the closest pairs first, every line still inside one cluster
        let lines: Vec<Item> = (0..20).map(|k| item(10 + k, r(100, 100 + 60 * k as u32, 400, 40), None)).chain([item(99, r(1500, 900, 300, 100), None)]).collect();
        for max in [1, 3, 8] {
            let c = clusters(&lines, clip, max);
            assert!(c.len() <= max, "{max}: {c:?}");
            for (a, x) in c.iter().enumerate() {
                for y in &c[a + 1..] {
                    assert!(x.rect.intersect(y.rect).is_empty(), "{x:?} {y:?}");
                }
            }
            for (k, l) in lines.iter().enumerate().filter(|(_, l)| !l.rect.intersect(clip).is_empty()) {
                let one = c.iter().find(|x| x.items.contains(&k)).unwrap();
                assert!(one.rect.contains(l.rect.intersect(clip)), "{l:?} in {one:?}");
            }
        }
        assert!(clusters(&[item(1, r(2000, 0, 10, 10), None)], clip, 8).is_empty());
    }

    #[test]
    fn an_own_window_group_is_one_cluster_and_keeps_it_longest() {
        use crate::render::test_item as item;
        let r = |x, y, w, h| Rect { x, y, w, h };
        let clip = r(0, 0, 1920, 1080);
        // the group's two far-apart items are one cluster (its window is their box), apart from the menu line beside it
        let its = [item(1, r(1200, 100, 400, 40), None), item(2, r(10, 10, 20, 20), Some(7)), item(3, r(1000, 800, 20, 20), Some(7))];
        let c = clusters(&its, clip, 8);
        assert_eq!(c.len(), 2);
        let own = c.iter().find(|c| c.own == Some(7)).unwrap();
        assert_eq!((own.rect, own.items.clone()), (r(10, 10, 1010, 810), vec![1, 2]));
        // over the line, the group floats in a window of its own carrying the line's part under it; the line keeps
        // its window
        let its = [item(1, r(100, 100, 400, 40), None), item(2, r(120, 110, 20, 20), Some(7))];
        let c = clusters(&its, clip, 8);
        assert_eq!(c.len(), 2);
        assert!(c.iter().any(|c| c.own.is_none() && c.items == vec![0] && c.rect == r(100, 100, 400, 40)), "{c:?}");
        assert!(c.iter().any(|c| c.own == Some(7) && c.items == vec![0, 1] && c.rect == r(120, 110, 20, 20)), "{c:?}");
        // two own groups overlapping share one floating window, named by the topmost
        let its = [item(1, r(100, 100, 20, 20), Some(7)), item(2, r(110, 110, 20, 20), Some(8))];
        let c = clusters(&its, clip, 8);
        assert_eq!(c, vec![Cluster { rect: r(100, 100, 30, 30), items: vec![0, 1], own: Some(8) }]);
        // ten lines and two own groups above them, three windows: the lines merge among themselves first, then the
        // lower own group goes in, the topmost keeps its own window
        let mut its: Vec<Item> = (0..10).map(|k| item(10 + k, r(100, 100 + 60 * k as u32, 400, 40), None)).collect();
        its.push(item(50, r(1500, 100, 20, 20), Some(1)));
        its.push(item(51, r(1500, 900, 20, 20), Some(2)));
        let c = clusters(&its, clip, 3);
        assert_eq!(c.len(), 3);
        assert!(c.iter().any(|c| c.own == Some(2) && c.items == vec![11]), "{c:?}");
        let c = clusters(&its, clip, 2);
        assert!(c.iter().any(|c| c.own == Some(2) && c.items == vec![11]), "{c:?}");
        assert!(c.iter().any(|c| c.items == (0..11).collect::<Vec<_>>()), "{c:?}");
    }

    #[test]
    fn covers_are_solid_items_nothing_below_touches_the_largest_first() {
        use crate::render::test_item as item;
        let r = |x, y, w, h| Rect { x, y, w, h };
        let clip = r(0, 0, 1920, 1080);
        let solid = |key, rect| {
            let mut i = item(key, rect, None);
            i.solid = Some([1, 2, 3]);
            i
        };
        let its = [
            solid(1, r(0, 0, 100, 100)),
            // over the first: something lies under it
            solid(2, r(50, 50, 100, 100)),
            // text over a solid one does not stop it, and is no cover itself
            item(3, r(10, 10, 20, 20), None),
            solid(4, r(500, 0, 10, 10)),
            solid(5, r(800, 0, 300, 300)),
            solid(6, r(1300, 0, 50, 50)),
            solid(7, r(1500, 0, 60, 60)),
            // outside the clip
            solid(8, r(2000, 0, 60, 60)),
        ];
        assert_eq!(covers(&its, clip, 8), vec![0, 3, 4, 5, 6]);
        assert_eq!(covers(&its, clip, 4), vec![0, 4, 5, 6]);
        assert!(covers(&its, clip, 0).is_empty());
    }
}
