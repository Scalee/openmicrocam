//! Camera check of the GE layer path: scenes shown one after another through `mi::layer` into an
//! ARGB4444 buffer of the media heap, each result compared pixel by pixel with the CPU reference (`render::compose`,
//! demultiplied, cut to 4 bits). Image nodes show resource `test`, a 600×300 picture of colour and alpha ramps in the
//! media heap; a picture the engine stretches is filtered its own way, so its pixels are counted apart and not judged.
//! usage: gecheck <W> <H> <scene.json>...
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use microcam_core::scene::{Scene, MAX_LEVEL};
use microcam_core::ui::OutputUi;
use microcam_core::{Rect, Size};
use microcam_draw::render::{compose, rasterise, shown, Fonts, Picture, Pixels};
use microcam_draw::tiny_skia as sk;
use mi::gfx::{Fmt, Surface};
use mi::layer::{Composer, Image, Mma, Scratch, Sink, Tiles};

#[cfg(target_arch = "arm")]
fn main() {
    check()
}

/// On the computer the check is only compiled: the MI libraries it calls are the camera's.
#[cfg(not(target_arch = "arm"))]
fn main() {
    eprintln!("gecheck runs on the camera");
    std::process::exit(2);
}

#[cfg_attr(not(target_arch = "arm"), allow(dead_code))]
fn check() {
    let a: Vec<String> = std::env::args().collect();
    let (w, h): (u32, u32) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    mi::sys::init().unwrap();
    mi::gfx::open().unwrap();
    let mem = Mma::new(w * h * 2, mi::layer::Use::Scratch).unwrap();
    let layer = Surface { phy: mem.phy, fmt: Fmt::Argb4444, w, h, stride: w * 2 };
    mi::layer::set_share(32 << 20);
    let mut tiles = Tiles::new();
    let mut comp = Composer::new();
    let mut scratch = Scratch::new(w).unwrap();
    let out = OutputUi { raster: Size::new(w, h), live: Rect::full(Size::new(w, h)), level: MAX_LEVEL, paused: None };
    let mut fonts = Fonts::new();
    // red across, green down, blue in steps; opaque on the left half, alpha falling to 0 across the right
    let mut pix = Image::new(600, 300).unwrap();
    for (k, p) in pix.bytes().chunks_exact_mut(4).enumerate() {
        let (x, y) = (k % 600, k / 600);
        let a = if x < 300 { 255 } else { 255 - (x - 300) * 255 / 299 };
        let c = sk::ColorU8::from_rgba((x * 255 / 599) as u8, (y * 255 / 299) as u8, ((x / 40 + y / 40) % 2 * 255) as u8, a as u8).premultiply();
        p.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    pix.sync().unwrap();
    let test = Picture { full: (600, 300), pix: Some((Arc::new(pix) as Arc<dyn Pixels>, 1)) };
    let pictures = move |id: &str| (id == "test").then(|| test.clone());
    for path in &a[3..] {
        let scene: Scene = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        scene.validate().unwrap();
        let t0 = Instant::now();
        let list = shown(&scene, &out, &mut fonts, &pictures, &[]).0;
        let t1 = Instant::now();
        let keep: HashSet<u64> = comp.shown().iter().map(|i| i.key).collect();
        let px = tiles.admit(&list, &keep).unwrap();
        let t2 = Instant::now();
        let st = comp.show(&mut scratch, &mut Sink::Engine(layer), Rect::full(out.raster), list.clone(), &tiles).unwrap();
        let t3 = Instant::now();
        // the reference
        mem.sync().unwrap();
        let pairs: Vec<_> = list.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
        let want = compose(&pairs, Rect::full(out.raster)).unwrap();
        let got = mem.read();
        let stretched: Vec<Rect> = list.iter().filter(|i| i.image.as_ref().is_some_and(|p| (p.src.w, p.src.h) != (p.dst.2, p.dst.3))).map(|i| i.rect).collect();
        let in_picture = |x: u32, y: u32| stretched.iter().any(|r| r.intersect(Rect { x, y, w: 1, h: 1 }).area() == 1);
        let mut pic = [0u64; 16];
        let mut hist = [0u64; 16];
        let (mut bad, mut bbox) = (Vec::new(), (u32::MAX, u32::MAX, 0u32, 0u32));
        let mut per_band = [0u64; 16];
        for (k, p) in want.pixels().iter().enumerate() {
            let c = p.demultiply();
            let v = u16::from_le_bytes([got[2 * k], got[2 * k + 1]]);
            let g = [(v >> 12) & 15, (v >> 8) & 15, (v >> 4) & 15, v & 15];
            let r = [c.alpha(), c.red(), c.green(), c.blue()].map(|x| (x as u16 * 15 + 127) / 255);
            // colour does not matter where nothing shows
            let n = if r[0] == 0 && g[0] == 0 { 0 } else { (0..4).map(|i| g[i].abs_diff(r[i])).max().unwrap() };
            if in_picture((k as u32) % w, (k as u32) / w) {
                pic[n as usize] += 1;
                continue;
            }
            hist[n as usize] += 1;
            if n >= 2 {
                let (x, y) = ((k as u32) % w, (k as u32) / w);
                per_band[(y / 120).min(15) as usize] += 1;
                bbox = (bbox.0.min(x), bbox.1.min(y), bbox.2.max(x), bbox.3.max(y));
                if bad.len() < 12 && (bad.is_empty() || k % 97 == 0) {
                    bad.push(format!("({x},{y}) got ARGB {g:?} want {r:?}"));
                }
            }
        }
        println!(
            "{path}: {} items, raster {:?} ({} kpx), GE {:?} ({} areas, {} bands, {} blits, {} kpx), tiles {} KiB; diff per pixel (4-bit steps) 0:{} 1:{} 2:{} 3+:{}; in stretched pictures 0:{} 1:{} 2:{} 3+:{}",
            list.len(),
            t2 - t1,
            px / 1000,
            t3 - t2,
            st.areas,
            st.bands,
            st.blits,
            st.pixels / 1000,
            tiles.bytes() / 1024,
            hist[0],
            hist[1],
            hist[2],
            hist[3..].iter().sum::<u64>(),
            pic[0],
            pic[1],
            pic[2],
            pic[3..].iter().sum::<u64>()
        );
        let _ = t0;
        if hist[2..].iter().sum::<u64>() > 0 {
            eprintln!("off by 2+: x {}..{}, y {}..{}; per 120 rows {:?}; {}", bbox.0, bbox.2, bbox.1, bbox.3, &per_band[..9], bad.join("; "));
            eprintln!("FAIL: pixels more than one 4-bit step off the reference");
            std::process::exit(1);
        }
    }
}
