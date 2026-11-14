//! Laptop or camera preview: a scene's items for a raster, rasterised and composed; writes straight RGBA and prints the
//! times. usage: render <scene.json> <w> <h> [out.rgba]
use std::sync::Arc;
use std::time::Instant;

use microcam_core::scene::{Scene, MAX_LEVEL};
use microcam_core::ui::OutputUi;
use microcam_core::{Rect, Size};
use microcam_draw::render::{compose, items, rasterise, Fonts};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let scene: Scene = serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    scene.validate().unwrap();
    let (w, h): (u32, u32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let out = OutputUi { raster: Size::new(w, h), live: Rect::full(Size::new(w, h)), level: MAX_LEVEL, paused: None };
    let mut fonts = Fonts::new();
    for round in 0..2 {
        let t0 = Instant::now();
        let list = items(&scene, &out, &mut fonts);
        let t1 = Instant::now();
        let tiles: Vec<_> = list.iter().map(|i| (i.clone(), Arc::new(rasterise(i).unwrap()))).collect();
        let t2 = Instant::now();
        let px = compose(&tiles, Rect::full(out.raster)).unwrap();
        let t3 = Instant::now();
        let kpx: u64 = list.iter().map(|i| i.rect.area()).sum::<u64>() / 1000;
        eprintln!("round {round}: {} items, {kpx} kpx of tiles; items {:?}, rasterise {:?}, compose {:?}", list.len(), t1 - t0, t2 - t1, t3 - t2);
        if round == 1 {
            if let Some(path) = a.get(4) {
                let straight: Vec<u8> = px.pixels().iter().flat_map(|p| { let c = p.demultiply(); [c.red(), c.green(), c.blue(), c.alpha()] }).collect();
                std::fs::write(path, straight).unwrap();
            }
        }
    }
}
