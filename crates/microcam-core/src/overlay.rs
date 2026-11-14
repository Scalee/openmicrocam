//! The `ui` on the RGN outputs: the output's layer — ARGB4444 regions over the drawn
//! parts of the port's frames, which the `ui` thread makes and draws with the GE ([`LayerRgn`] says when it may).
//!
//! The pipeline thread decides when an output's layer may be on its port: while the output wants one, and never across a
//! transition that takes the port away ([`touches`]) — before such a step, or when the output goes, it has the `ui`
//! thread take its regions off (microcamd's `Ui::release_layer`).


use crate::reconcile::{ChnPort, Module, Op};
use crate::ui::OutputUi;
use crate::hdmi::Timing;
use crate::{Codec, Rect};

/// One output (an encoder's, or HDMI's DISP) as the pipeline built it, and where its menu goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgnOutput {
    /// the whole output frame the `ui` is laid out in, its live window and display level
    pub ui: OutputUi,
    /// the SCL port whose frames carry the layer; None: no live part shows (covered), the port is off and the output
    /// has no layer — its whole menu goes into its frames as patches
    pub port: Option<ChnPort>,
    /// the part of the output frame the port's frames are, at the same pixels: all of it, or the live window (widened
    /// to the port's grid) when the compositor puts them there; the whole frame when covered
    pub area: Rect,
    pub fps: u32,
    /// what the menu is told of the output beside its frame (`protocol::OutputInfo`), as built: HDMI's timing, the USB
    /// stream's codec (its rate is `fps`)
    pub timing: Option<Timing>,
    pub codec: Option<Codec>,
    /// the pool of another output whose canvases this output's encoder reads (its frame is that output's): no port, layer or bake of its own; it shows what that output's layer and patches show, and
    /// presents with it
    pub joined: Option<u32>,
    /// the built frame's visible live part in output pixels, as its compositor has it (empty when covered): the menu's
    /// layer shows only what touches it
    pub visible: Vec<Rect>,
}

impl RgnOutput {
    /// Whether a layer drawn for `o` fits this output unchanged (the rate alone does not matter).
    pub fn same_drawing(&self, o: &RgnOutput) -> bool {
        self.ui == o.ui && self.area == o.area
    }

    /// Whether a layer on `o` is this output's layer: the same port and drawing.
    pub fn same_layer(&self, o: &RgnOutput) -> bool {
        self.port == o.port && self.same_drawing(o)
    }
}

/// Whether a transition's steps take this port, its channel or its device down or change the port's frames.
pub fn touches(ops: &[Op], p: ChnPort) -> bool {
    p.module == Module::Scl
        && ops.iter().any(|o| match o {
            Op::SclConfigurePort { dev, chn, port, .. } | Op::SclEnablePort { dev, chn, port } | Op::SclDisablePort { dev, chn, port } => (*dev, *chn, *port) == (p.dev, p.chn, p.port),
            Op::SclCreateChannel { dev, chn } | Op::SclDestroyChannel { dev, chn } | Op::SclStartChannel { dev, chn } | Op::SclStopChannel { dev, chn } => (*dev, *chn) == (p.dev, p.chn),
            Op::SclCreateDevice { dev, .. } | Op::SclDestroyDevice { dev } => *dev == p.dev,
            _ => false,
        })
}

/// The layer on a port, as the `ui` thread is told of it: the output's generation and the port's frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub generation: u64,
    /// this layer's own life: a new one every time it is allowed on, so a layer taken off and allowed again on the same
    /// output is never taken for the one before (whose regions are gone)
    pub life: u64,
    pub out: RgnOutput,
}

/// One RGN output's layer as the pipeline allows it on its port.
#[derive(Default)]
pub struct LayerRgn {
    /// the output and the generation the pipeline named it by
    out: Option<(u64, RgnOutput)>,
    shown: Option<Attached>,
    /// ports whose regions were left in place because the 2D engine may still write them: never taken away
    abandoned: Vec<ChnPort>,
    /// layers allowed so far, numbering each one's life
    made: u64,
}

impl LayerRgn {
    pub fn new() -> Self {
        Self::default()
    }

    /// The output as the pipeline built it now (None: there is none). Whether the layer on it must go first (another
    /// port, or drawn for another frame): then the caller has the `ui` thread take it off and calls [`Self::retire`].
    pub fn must_go(&self, out: Option<RgnOutput>) -> bool {
        self.shown.as_ref().is_some_and(|s| !matches!(out, Some(o) if o.same_layer(&s.out)))
    }

    pub fn set_output(&mut self, generation: u64, out: Option<RgnOutput>) {
        self.out = out.map(|o| (generation, o));
        if let (Some(s), Some((g, o))) = (self.shown.as_mut(), self.out.clone()) {
            // the same drawing at another rate
            if o.same_layer(&s.out) {
                s.generation = g;
                s.out = o;
            }
        }
    }

    /// The layer allowed now.
    pub fn shown(&self) -> Option<Attached> {
        self.shown.clone()
    }

    /// Whether a step would take away the port of regions left in place (the engine lost), whatever the output is now.
    pub fn holds_port(&self, op: &Op) -> bool {
        self.abandoned.iter().any(|p| touches(std::slice::from_ref(op), *p))
    }

    /// Whether a layer is allowed on a port.
    pub fn shows(&self) -> bool {
        self.shown.is_some()
    }

    /// The `ui` thread took the layer's regions off.
    pub fn retire(&mut self) {
        self.shown = None;
    }

    /// The layer's regions stay where they are: the 2D engine may still be writing them. Their port is never taken
    /// away again.
    pub fn abandon(&mut self) {
        if let Some(p) = self.shown.take().and_then(|s| s.out.port) {
            self.abandoned.push(p);
        }
    }

    /// Allow the layer on the port when `want` and there is none; returns the layer if it is new. Not `want`: the
    /// caller has had the `ui` thread take it off first.
    pub fn step(&mut self, want: bool) -> Option<Attached> {
        if !want {
            self.shown = None;
            return None;
        }
        if self.shown.is_some() {
            return None;
        }
        // no port, no layer
        let (generation, out) = self.out.clone().filter(|(_, o)| o.port.is_some())?;
        self.made += 1;
        let a = Attached { generation, life: self.made, out };
        self.shown = Some(a.clone());
        Some(a)
    }

    /// For the status: whether a layer is allowed, and on which port.
    pub fn describe(&self) -> Option<String> {
        let (_, out) = self.out.as_ref()?;
        if let Some(pool) = out.joined {
            return Some(format!("pool {pool}'s frames (joined): that output's layer and patches"));
        }
        let Some(p) = out.port else { return Some("no layer: covered, the menu in the frames".into()) };
        Some(match self.shown {
            Some(_) => format!("layer ({}) on SCL {}/{}/{}", out.area.size(), p.dev, p.chn, p.port),
            None => format!("no layer on SCL {}/{}/{}", p.dev, p.chn, p.port),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::topo;
    use crate::Size;

    fn out(fps: u32, w: u32) -> RgnOutput {
        let size = Size::new(w, w * 9 / 16);
        RgnOutput { ui: OutputUi { raster: size, live: Rect::full(size), level: 3, paused: None }, port: Some(topo::SCL_OUT_USB), area: Rect::full(size), fps, timing: None, codec: Some(Codec::Mjpeg), joined: None, visible: vec![Rect::full(size)] }
    }

    #[test]
    fn a_wanted_layer_is_allowed_once_and_goes_when_not_wanted() {
        let mut l = LayerRgn::new();
        l.set_output(1, Some(out(30, 1920)));
        assert_eq!(l.step(true).map(|a| a.generation), Some(1));
        // already there: nothing new
        assert_eq!(l.step(true), None);
        l.step(false);
        assert!(!l.shows());
    }

    #[test]
    fn another_rate_keeps_the_layer_and_another_frame_retires_it() {
        let mut l = LayerRgn::new();
        l.set_output(1, Some(out(30, 1920)));
        l.step(true);
        assert!(!l.must_go(Some(out(60, 1920))));
        l.set_output(2, Some(out(60, 1920)));
        assert_eq!(l.shown().map(|s| s.generation), Some(2));
        assert!(l.must_go(Some(out(60, 3840))));
        assert!(l.must_go(None));
        // another visible part (a picture over the live picture): the same layer, cut otherwise by the `ui`
        assert!(!l.must_go(Some(RgnOutput { visible: Vec::new(), ..out(60, 1920) })));
    }

    #[test]
    fn a_layer_allowed_again_on_the_same_output_is_a_new_life() {
        let mut l = LayerRgn::new();
        l.set_output(1, Some(out(30, 1920)));
        let a = l.step(true).unwrap();
        l.retire();
        let b = l.step(true).unwrap();
        assert_eq!((a.generation, &a.out), (b.generation, &b.out));
        assert_ne!(a, b);
    }

    #[test]
    fn an_abandoned_layer_holds_its_port() {
        let mut l = LayerRgn::new();
        l.set_output(1, Some(out(30, 1920)));
        l.step(true);
        l.abandon();
        assert!(!l.shows());
        let p = topo::SCL_OUT_USB;
        assert!(l.holds_port(&Op::SclDisablePort { dev: p.dev, chn: p.chn, port: p.port }));
        assert!(!l.holds_port(&Op::SclDisablePort { dev: p.dev, chn: p.chn, port: topo::SCL_PORT_HDMI }));
    }

    #[test]
    fn a_transition_touches_the_port_through_its_channel_or_device_but_not_through_another_port() {
        let p = topo::SCL_OUT_USB;
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert!(touches(&[Op::SclDisablePort { dev: 0, chn: 0, port: 2 }], p));
        assert!(touches(&[Op::SclStopChannel { dev: 0, chn: 0 }], p));
        assert!(touches(&[Op::SclDestroyDevice { dev: 0 }], p));
        assert!(!touches(&[Op::SclConfigurePort { dev: 0, chn: 0, port: 0, crop: r(0, 0, 8, 8), size: Size::new(8, 8) }], p));
        assert!(!touches(&[Op::SclDestroyChannel { dev: 1, chn: 0 }], p));
    }
}
