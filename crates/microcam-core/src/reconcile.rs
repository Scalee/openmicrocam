//! The reconciler: `Current` (what is built) + `Desired` → an ordered list of hardware operations ([`Op`]), and the
//! state each successful operation leaves. The daemon executes the list through its `Hal`; the tests execute it
//! through a recording `Hal` and check the orders the hardware demands.
//!
//! Operations are the vocabulary of the pipeline; they carry intent, not MI structs. Fine-grained where the order is
//! what hangs the SoC (sensor, VIF, ISP, SCL, binds); composite where the SDK sequence is fixed and harmless
//! (HDMI/DISP bring-up, VENC channel creation).
//!
//! Every output — USB, the media encoder, HDMI — is fed the same way: its SCL port (behind it an
//! upscaler when zoomed) writes the live picture into the canvases of the output's pool, and microcamd's feed hands each
//! canvas whole to the pool's readers — an encoder's input port, DISP's input port 0 —, each at its own rate. One pool
//! per distinct frame: an encoder whose frame is an earlier output's — USB's HDMI's, the recording's HDMI's or USB's —
//! reads that output's pool at its own rate, a reader added and taken off around its steps, nothing of that output
//! rebuilt for it; the photo during a recording reads the pool the recording reads, handed one canvas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::desired::{Desired, Epoch, FrameOf, HdmiPlan, Input, MediaPlan, OutputFrame, PictureId, UsbPlan};
use crate::frame::Way;
use crate::hdmi::Timing;
use crate::modes::ModeTable;
use crate::tuning::{self, Adjust, Knob, Tuning};
use crate::{Codec, Rect, Size};

// ---- addressing -----------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Module {
    Vif,
    Isp,
    Scl,
    Venc,
    Disp,
    Vdec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChnPort {
    pub module: Module,
    pub dev: u32,
    pub chn: u32,
    pub port: u32,
}

impl ChnPort {
    pub const fn new(module: Module, dev: u32, chn: u32, port: u32) -> Self {
        Self { module, dev, chn, port }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BindKind {
    FrameBase,
    Realtime,
}

/// The fixed topology of this camera's pipeline (one sensor pad, one of each device).
pub mod topo {
    use super::{ChnPort, Module};
    pub const PAD: u32 = 0;
    pub const VIF_GROUP: u32 = 0;
    pub const VIF_DEV: u32 = 0;
    pub const VIF_PORT: u32 = 0;
    pub const ISP_DEV: u32 = 0;
    pub const ISP_CHN: u32 = 0;
    pub const ISP_PORT: u32 = 0;
    pub const SCL_DEV: u32 = 0;
    pub const SCL_CHN: u32 = 0;
    /// SCL output ports: 0 → HDMI's pool (DISP), 1 → media encoder (photo, recording), 2 → USB encoder
    pub const SCL_PORT_HDMI: u32 = 0;
    pub const SCL_PORT_MEDIA: u32 = 1;
    pub const SCL_PORT_USB: u32 = 2;
    /// SCL hardware output mask (HWSCL0..2)
    pub const SCL_HW_MASK: u32 = 7;
    /// the USB upscaler: a frame-mode SCL device fed from SCL port 2 through memory, output port 0 to the encoder
    /// (only dev 0 sits on the realtime ISP link; the SoC has six hardware scalers)
    pub const SCL_UP_DEV: u32 = 1;
    pub const SCL_UP_CHN: u32 = 0;
    pub const SCL_UP_PORT: u32 = 0;
    pub const SCL_UP_HW_MASK: u32 = 8;
    /// the media upscaler: a second channel of the same frame-mode device (only dev 1 takes its input from memory —
    /// a bind into dev 2 is refused with NOT_SUPPORT, 2026-10-25), sharing its hardware scaler by time
    pub const SCL_MEDIA_UP_CHN: u32 = 1;
    /// HDMI's upscaler while zoomed: a third channel of that device. DISP shows the canvas 1:1 (the menu is in it), and
    /// SCL port 0 itself must never scale up (it stalls the ISP, see `HdmiPlan::scl_out`)
    pub const SCL_HDMI_UP_CHN: u32 = 2;
    pub const DISP_DEV: u32 = 0;
    pub const DISP_LAYER: u32 = 0;
    pub const DISP_PORT: u32 = 0;
    /// VENC devices as the SDK numbers them on this SoC: JPEG encoder dev 8, H.26x dev 0; one channel each
    pub const VENC_DEV_JPEG: u32 = 8;
    pub const VENC_DEV_H26X: u32 = 0;
    pub const VENC_CHN_USB: u32 = 0;
    /// the media encoder's channel on the same device as USB's (the vendor's photo used JPEG dev 8 chn 1 beside UVC)
    pub const VENC_CHN_MEDIA: u32 = 1;
    /// a photo during a recording: a JPEG channel of its own (the recording's H.264 is media's channel on dev 0)
    pub const VENC_CHN_SNAP: u32 = 2;
    pub const VIF_OUT: ChnPort = ChnPort::new(Module::Vif, VIF_DEV, 0, VIF_PORT);
    pub const ISP_IN: ChnPort = ChnPort::new(Module::Isp, ISP_DEV, ISP_CHN, 0);
    pub const ISP_OUT: ChnPort = ChnPort::new(Module::Isp, ISP_DEV, ISP_CHN, ISP_PORT);
    pub const SCL_IN: ChnPort = ChnPort::new(Module::Scl, SCL_DEV, SCL_CHN, 0);
    pub const SCL_OUT_HDMI: ChnPort = ChnPort::new(Module::Scl, SCL_DEV, SCL_CHN, SCL_PORT_HDMI);
    pub const SCL_OUT_USB: ChnPort = ChnPort::new(Module::Scl, SCL_DEV, SCL_CHN, SCL_PORT_USB);
    pub const SCL_OUT_MEDIA: ChnPort = ChnPort::new(Module::Scl, SCL_DEV, SCL_CHN, SCL_PORT_MEDIA);
    /// the upscaler's input and its output port 0 (same address; a bind names the direction)
    pub const SCL_UP: ChnPort = ChnPort::new(Module::Scl, SCL_UP_DEV, SCL_UP_CHN, SCL_UP_PORT);
    pub const SCL_MEDIA_UP: ChnPort = ChnPort::new(Module::Scl, SCL_UP_DEV, SCL_MEDIA_UP_CHN, SCL_UP_PORT);
    pub const SCL_HDMI_UP: ChnPort = ChnPort::new(Module::Scl, SCL_UP_DEV, SCL_HDMI_UP_CHN, SCL_UP_PORT);
    /// a path from a dev-0 SCL port to an output's pool, with a frame-mode upscaler behind the port when the crop is
    /// smaller than the live window; the last port writes into the pool's canvases, which microcamd's feed thread hands on
    pub struct Feed {
        pub port: u32,
        pub out: ChnPort,
        /// the feed's channel on the upscaler device
        pub up_chn: u32,
        pub up: ChnPort,
        /// buffer quota of the port and of its upscaler: a frame holds its buffer until the encoder has encoded it, and a
        /// quota that runs out drops every frame that finds none
        pub buffers: u32,
    }
    /// frames of a feed port queued for microcamd's feed thread (one in hand, one queued)
    pub const FEED_USER_FRAMES: u32 = 2;
    /// the canvas pools of our module (kmods/microcam-canvas), one per output
    pub const POOL_USB: u32 = 0;
    pub const POOL_MEDIA: u32 = 1;
    pub const POOL_HDMI: u32 = 2;
    pub const USB_FEED: Feed = Feed { port: SCL_PORT_USB, out: SCL_OUT_USB, up_chn: SCL_UP_CHN, up: SCL_UP, buffers: 3 };
    /// the H.264 encoder holds a frame longer than the USB encoder does; at the USB quota a recording could lock at 3/4
    /// of its rate [R 2026-10-25]
    pub const MEDIA_FEED: Feed =
        Feed { port: SCL_PORT_MEDIA, out: SCL_OUT_MEDIA, up_chn: SCL_MEDIA_UP_CHN, up: SCL_MEDIA_UP, buffers: 5 };
    /// HDMI's: SCL port 0 into its pool, or through its upscaler when zoomed (the port's quota then as USB's)
    pub const HDMI_FEED: Feed = Feed { port: SCL_PORT_HDMI, out: SCL_OUT_HDMI, up_chn: SCL_HDMI_UP_CHN, up: SCL_HDMI_UP, buffers: 3 };
    /// DISP's input port 0 (layer 0; the port is the channel), the HDMI pool's reader
    pub const DISP_IN: ChnPort = ChnPort::new(Module::Disp, DISP_DEV, DISP_LAYER, DISP_PORT);
    /// the memory-to-memory scaler (`MI_SCL_StretchBuf`, its own hardware scaler, dev 3 with mask 0x20): the pictures
    /// scaled to their windows in the outputs' frames, the menu over them baked
    pub const SCL_STRETCH_DEV: u32 = 3;
    pub const SCL_STRETCH_HW_MASK: u32 = 0x20;
    /// the player's decoder: its port 0 (scaled by the VPU), bound into an output's channel of SCL dev 1
    pub const VDEC_OUT: ChnPort = ChnPort::new(Module::Vdec, 0, 0, 0);
    pub const fn venc_in(dev: u32, chn: u32) -> ChnPort {
        ChnPort::new(Module::Venc, dev, chn, 0)
    }
    pub const fn venc_dev(codec: super::Codec) -> u32 {
        match codec {
            super::Codec::Mjpeg => VENC_DEV_JPEG,
            super::Codec::H264 | super::Codec::H265 => VENC_DEV_H26X,
        }
    }
}

// ---- operations -----------------------------------------------------------------------------------------------

/// One hardware operation. Everything the daemon does to the pipeline is one of these, so a recording `Hal` sees the
/// whole truth and the tests can assert order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    // sensor
    SensorSetRes { pad: u32, index: u8, mode: String },
    SensorEnable { pad: u32 },
    SensorDisable { pad: u32 },
    /// the driver's `fps_live` parameter (frame length from the next frame on); 0 = the mode's own rate
    SensorFpsLive { fps: u32 },
    /// `setmirrorflip` through the sensor module's procfs; must follow every sensor (re)start
    SensorOrientation { pad: u32, mirror: bool, flip: bool },
    // VIF (attributes come from the sensor's pad/plane info, read by the Hal at execution)
    VifCreateGroup { group: u32 },
    VifDestroyGroup { group: u32 },
    VifConfigureDev { dev: u32 },
    VifEnableDev { dev: u32 },
    VifDisableDev { dev: u32 },
    VifConfigurePort { dev: u32, port: u32 },
    VifEnablePort { dev: u32, port: u32 },
    VifDisablePort { dev: u32, port: u32 },
    // ISP
    IspCreateDevice { dev: u32 },
    IspDestroyDevice { dev: u32 },
    IspCreateChannel { dev: u32, chn: u32, pad: u32 },
    IspDestroyChannel { dev: u32, chn: u32 },
    IspSetChnParam { dev: u32, chn: u32, nr3d: u8 },
    IspStartChannel { dev: u32, chn: u32 },
    IspStopChannel { dev: u32, chn: u32 },
    /// output port: full input crop, NV12
    IspConfigurePort { dev: u32, chn: u32, port: u32 },
    IspEnablePort { dev: u32, chn: u32, port: u32 },
    IspDisablePort { dev: u32, chn: u32, port: u32 },
    IspLoadIq { dev: u32, chn: u32, path: String },
    /// a picture control; valid on a started channel, undone by the next IQ load. One the ISP refuses does not stop the
    /// steps: it is recorded refused ([`IspState::refused`]) and the build goes on — nothing after it depends on it
    IspAdjust { dev: u32, chn: u32, adjust: Adjust },
    // SCL
    SclCreateDevice { dev: u32, hw_mask: u32 },
    SclDestroyDevice { dev: u32 },
    SclCreateChannel { dev: u32, chn: u32 },
    SclDestroyChannel { dev: u32, chn: u32 },
    SclStartChannel { dev: u32, chn: u32 },
    SclStopChannel { dev: u32, chn: u32 },
    SclConfigurePort { dev: u32, chn: u32, port: u32, crop: Rect, size: Size },
    SclEnablePort { dev: u32, chn: u32, port: u32 },
    SclDisablePort { dev: u32, chn: u32, port: u32 },
    /// scaler clock in Hz (480 MHz needed for clean 3840-wide output at the fast 4K line)
    SclClock { hz: u32 },
    // MI_SYS
    Bind { src: ChnPort, dst: ChnPort, kind: BindKind, src_fps: u32, dst_fps: u32 },
    Unbind { src: ChnPort, dst: ChnPort },
    SetDepth { port: ChnPort, user: u32, buffers: u32 },
    // HDMI + DISP, composite: the SDK's fixed bring-up / tear-down sequences. DISP's input port 0 takes the whole screen
    // at the timing's size, free-running: it shows the HDMI pool's canvases 1:1
    HdmiDispUp { timing: Timing },
    HdmiDispDown,
    // VENC, composite: create channel + input source + start receiving; stop + destroy
    /// `bitrate` 0 = the vendor's size rule, `gop` 0 = 30 frames
    VencUp { dev: u32, chn: u32, codec: Codec, size: Size, fps: u32, mjpeg_quality: u8, bitrate: u32, gop: u32, bound: Option<Ring> },
    VencDown { dev: u32, chn: u32 },
    /// the canvas pool `id` (kmods/microcam-canvas): `n` canvases of `size` (`rows` allocated per plane, whole blocks for the
    /// encoders) lent as views to the port `src` and handed whole to each of its `readers` (an encoder's input port,
    /// DISP's input port 0: its own reader, at its feed's rate); after the readers are made, before the port's first frame
    PoolUp { id: u32, src: ChnPort, readers: Vec<ChnPort>, size: Size, rows: u32, n: u32 },
    /// the pool to `n` canvases, live
    PoolCount { id: u32, n: u32 },
    /// one more reader of the pool, live, handed `fps` of its canvases a second from then on (USB's encoder joining
    /// HDMI's frames at the stream's rate; the photo's encoder, 0, only the canvas named for it); after the reader is
    /// made, which reads no other pool
    PoolReader { id: u32, reader: ChnPort, fps: u32 },
    /// that reader taken off, live (its allocator off while its channel still exists): before its channel goes
    PoolUnreader { id: u32, reader: ChnPort },
    /// the pool's allocators taken off — while its port's channel and its readers still exist — and its canvases freed
    /// once nobody has them
    PoolDown { id: u32 },
    /// microcamd's feed thread hands the frames of `src` to `dst` — an encoder's input or DISP's input port 0, the pool's
    /// own reader — at `fps` a second of them, and to the readers joining the pool at theirs, the `way` given
    FeedStart { src: ChnPort, dst: ChnPort, fps: u32, way: FeedWay },
    /// the feed stopped (the compositor's frames given back); before its destination or its source is touched
    FeedStop { dst: ChnPort },
    // USB pump: starts after the encoder runs; stops through the quiescence barrier before the encoder is touched
    PumpStart { epoch: Epoch, venc_dev: u32, venc_chn: u32 },
    PumpStop,
    // recorder: the media encoder's output into an MP4 file; stops (file closed) before the encoder is touched
    RecordStart { venc_dev: u32, venc_chn: u32, codec: Codec, path: String, size: Size, fps: u32 },
    RecordStop,
    // the player: the decoder's channel (its device made with the first and kept), its port's size, live; the
    // player thread that sends it the file's samples — after the binds from its port, stopped before they go
    VdecUp { codec: Codec, size: Size, ring: u32, port: Size },
    VdecPort { port: Size },
    VdecDown,
    PlayStart { path: String, fps: u32 },
    PlayStop,
}

// ---- current state --------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorState {
    pub mode: String,
    pub index: u8,
    pub enabled: bool,
    pub fps_live: u32,
    pub oriented: Option<(bool, bool)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VifState {
    pub group: bool,
    pub dev_configured: bool,
    pub dev_enabled: bool,
    pub port_configured: bool,
    pub port_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IspState {
    pub device: bool,
    pub channel: bool,
    pub nr3d: Option<u8>,
    pub started: bool,
    pub port_configured: bool,
    pub port_enabled: bool,
    pub iq: Option<String>,
    /// picture knobs in effect since the last IQ load
    pub tuning: Tuning,
    /// picture knobs the ISP refused since the last IQ load: the value asked and why. Not asked again until the value
    /// wanted changes or the next load, so a refused knob does not make every later transition a picture step.
    #[serde(default)]
    pub refused: BTreeMap<Knob, (i32, String)>,
}

impl IspState {
    /// The knobs as last asked of the ISP since the IQ load: those in effect, and those refused at the value refused.
    pub fn asked(&self) -> Tuning {
        let mut t = self.tuning.clone();
        t.extend(self.refused.iter().map(|(k, (v, _))| (*k, *v)));
        t
    }

    /// Why the ISP refused one of `knobs` since the IQ load, if it did.
    pub fn refusal(&self, knobs: &[Knob]) -> Option<&str> {
        knobs.iter().find_map(|k| self.refused.get(k)).map(|(_, why)| why.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SclPortState {
    pub configured: Option<(Rect, Size)>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SclState {
    pub device: bool,
    pub channel: bool,
    pub started: bool,
    pub ports: [SclPortState; 3],
    pub clock_hz: Option<u32>,
}

/// The player's decoder channel as made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VdecState {
    pub codec: Codec,
    pub size: Size,
    pub ring: u32,
    pub port: Size,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindState {
    pub src: ChnPort,
    pub dst: ChnPort,
    pub kind: BindKind,
    pub src_fps: u32,
    pub dst_fps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HdmiState {
    pub timing: Timing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VencState {
    pub dev: u32,
    pub chn: u32,
    pub codec: Codec,
    pub size: Size,
    pub fps: u32,
    pub mjpeg_quality: u8,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub gop: u32,
    /// the SCL port bound into it in a ring (HW_RING, `FrameOf::Ring`): it reads each band as the port writes it;
    /// none when its frames are handed to it. Fixed while the channel runs (its input mode).
    #[serde(default)]
    pub bound: Option<Ring>,
}

/// An SCL port bound into an encoder in a ring. The port writes every frame of the sensor, so the bind declares the
/// sensor mode's rate as its source and the encoder's as its destination: mi_sys's frame-rate control hands the
/// encoder only its rate of them, following the source's measured rate from the first second on; at equal rates
/// it hands every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    pub port: ChnPort,
    pub src_fps: u32,
}

/// How the feed thread hands a port's frames on (nothing is bound into an encoder or DISP, so changing the
/// way never rebuilds them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedWay {
    /// the canvases of pool `pool`, `size` frames, to every reader of the pool. The port writes the live window `live` at `view` (its write
    /// area, `live` widened to the port's 16-column grid; None: covered, the port off, the canvas handed again on the
    /// feed's clock); the pictures' `windows` (bottom to top, black between) hold the still set the `ui` puts in the
    /// output's box, and where the port wrote over them — `view` outside the `visible` live part — it is put back every
    /// frame. Pictures take the memory-to-memory scaler, and the `ui` bakes the menu over the still content with it
    Canvas { pool: u32, size: Size, live: Rect, view: Option<Rect>, visible: Vec<Rect>, windows: Vec<Rect> },
}

impl FeedWay {
    /// The port writes over pictures every frame (they are put back): the canvas it wrote stays with the feed longer.
    fn puts_back(&self) -> bool {
        match self {
            FeedWay::Canvas { view: Some(v), visible, .. } => visible.iter().map(|r| r.area()).sum::<u64>() < v.area(),
            FeedWay::Canvas { view: None, .. } => false,
        }
    }
}

/// A canvas pool as made ([`Op::PoolUp`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolState {
    pub id: u32,
    pub src: ChnPort,
    /// the input ports it hands each canvas to
    #[serde(default)]
    pub readers: Vec<ChnPort>,
    pub size: Size,
    pub rows: u32,
    pub n: u32,
}

/// A running feed (`Op::FeedStart`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedState {
    pub src: ChnPort,
    pub dst: ChnPort,
    pub fps: u32,
    pub way: FeedWay,
}

/// What is built right now. Updated after every successful operation, so after a failure it is the truth about the
/// partial state and the next plan starts from it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Current {
    pub sensor: Option<SensorState>,
    pub vif: VifState,
    pub isp: IspState,
    pub scl: SclState,
    /// the upscaler device (`topo::SCL_UP_DEV`) and its USB channel, present only while a zoomed USB stream or recording
    /// needs it; `device` is the same in both
    #[serde(default)]
    pub scl_up: SclState,
    /// the upscaler's media channel (`topo::SCL_MEDIA_UP_CHN`)
    #[serde(default)]
    pub scl_media_up: SclState,
    /// the upscaler's HDMI channel (`topo::SCL_HDMI_UP_CHN`)
    #[serde(default)]
    pub scl_hdmi_up: SclState,
    /// the memory-to-memory scaler device (`topo::SCL_STRETCH_DEV`, no channels of ours), up while an output is
    #[serde(default)]
    pub scl_stretch: SclState,
    pub binds: Vec<BindState>,
    /// DISP and the HDMI transmitter
    pub hdmi: Option<HdmiState>,
    pub venc: Option<VencState>,
    /// the media encoder (`topo::VENC_CHN_MEDIA`) behind SCL port 1
    #[serde(default)]
    pub media: Option<VencState>,
    /// the JPEG encoder of a photo during a recording (`topo::VENC_CHN_SNAP`)
    #[serde(default)]
    pub snap: Option<VencState>,
    /// the file the recorder writes
    #[serde(default)]
    pub recording: Option<String>,
    pub pump: Option<Epoch>,
    /// the running feeds, at most one per output
    #[serde(default)]
    pub feeds: Vec<FeedState>,
    /// the outputs' canvas pools
    #[serde(default)]
    pub pools: Vec<PoolState>,
    /// the H.26x encoder device exists: made with its first channel, kept after the channel goes (its memory with it)
    #[serde(default)]
    pub codec_device: bool,
    /// the player's decoder channel
    #[serde(default)]
    pub vdec: Option<VdecState>,
    /// the file the player thread sends
    #[serde(default)]
    pub playing: Option<String>,
    /// the Desired this state was last reconciled to (None after a failure)
    pub desired: Option<Desired>,
}

impl Current {
    pub fn bind(&self, src: ChnPort, dst: ChnPort) -> Option<&BindState> {
        self.binds.iter().find(|b| b.src == src && b.dst == dst)
    }

    /// The feed into `dst`.
    pub fn feed(&self, dst: ChnPort) -> Option<&FeedState> {
        self.feeds.iter().find(|f| f.dst == dst)
    }

    /// The feed into the encoder on this channel (of either device).
    pub fn feed_into(&self, chn: u32) -> Option<&FeedState> {
        self.feeds.iter().find(|f| f.dst.module == Module::Venc && f.dst.chn == chn)
    }

    /// A feed uses the memory-to-memory scaler (every one does: its pictures, the menu baked over them).
    fn composing(&self) -> bool {
        !self.feeds.is_empty()
    }

    pub fn pool(&self, id: u32) -> Option<&PoolState> {
        self.pools.iter().find(|p| p.id == id)
    }

    /// The pool `r` reads, if any.
    fn pool_read_by(&self, r: ChnPort) -> Option<&PoolState> {
        self.pools.iter().find(|p| p.readers.contains(&r))
    }

    /// USB's encoder, as the input port a pool hands it canvases on.
    fn usb_reader(&self) -> Option<ChnPort> {
        self.venc.as_ref().map(|v| topo::venc_in(v.dev, topo::VENC_CHN_USB))
    }

    /// USB's encoder reads HDMI's pool (its frame is HDMI's).
    pub fn usb_joined(&self) -> bool {
        self.usb_reader().is_some_and(|r| self.pool(topo::POOL_HDMI).is_some_and(|p| p.readers.contains(&r)))
    }

    /// The media encoder, as the input port a pool hands it canvases on.
    fn media_reader(&self) -> Option<ChnPort> {
        self.media.as_ref().map(|v| topo::venc_in(v.dev, topo::VENC_CHN_MEDIA))
    }

    /// The photo's encoder beside a recording, as a reader.
    fn snap_reader(&self) -> Option<ChnPort> {
        self.snap.as_ref().map(|v| topo::venc_in(v.dev, topo::VENC_CHN_SNAP))
    }

    /// The pool of another output the media encoder reads (its frame is that output's).
    pub fn media_joined(&self) -> Option<u32> {
        self.media_reader().and_then(|r| self.pool_read_by(r)).map(|p| p.id).filter(|id| *id != topo::POOL_MEDIA)
    }

    /// The pool the photo's encoder reads during a recording.
    pub fn snap_pool(&self) -> Option<u32> {
        self.snap_reader().and_then(|r| self.pool_read_by(r)).map(|p| p.id)
    }

    /// The encoder on this channel: USB's or the media one.
    fn encoder(&self, chn: u32) -> &Option<VencState> {
        match chn {
            topo::VENC_CHN_MEDIA => &self.media,
            topo::VENC_CHN_SNAP => &self.snap,
            _ => &self.venc,
        }
    }

    fn encoder_mut(&mut self, chn: u32) -> &mut Option<VencState> {
        match chn {
            topo::VENC_CHN_MEDIA => &mut self.media,
            topo::VENC_CHN_SNAP => &mut self.snap,
            _ => &mut self.venc,
        }
    }

    /// The input port a pool's canvases or a feed go to exists: its encoder, or DISP.
    fn reader_exists(&self, r: ChnPort) -> bool {
        match r.module {
            Module::Venc => self.encoder(r.chn).as_ref().map(|v| v.dev) == Some(r.dev),
            Module::Disp => r == topo::DISP_IN && self.hdmi.is_some(),
            _ => false,
        }
    }

    /// The size of the frames an enabled SCL output port hands on.
    pub fn port_size(&self, p: ChnPort) -> Option<Size> {
        let s = self.scl_of(p.dev, p.chn).ports.get(p.port as usize)?;
        s.configured.filter(|_| s.enabled).map(|(_, size)| size)
    }

    /// The state of one SCL channel (and its device's existence).
    fn scl_of(&self, dev: u32, chn: u32) -> &SclState {
        match (dev, chn) {
            (topo::SCL_UP_DEV, topo::SCL_MEDIA_UP_CHN) => &self.scl_media_up,
            (topo::SCL_UP_DEV, topo::SCL_HDMI_UP_CHN) => &self.scl_hdmi_up,
            (topo::SCL_UP_DEV, _) => &self.scl_up,
            (topo::SCL_STRETCH_DEV, _) => &self.scl_stretch,
            _ => &self.scl,
        }
    }

    fn scl_of_mut(&mut self, dev: u32, chn: u32) -> &mut SclState {
        match (dev, chn) {
            (topo::SCL_UP_DEV, topo::SCL_MEDIA_UP_CHN) => &mut self.scl_media_up,
            (topo::SCL_UP_DEV, topo::SCL_HDMI_UP_CHN) => &mut self.scl_hdmi_up,
            (topo::SCL_UP_DEV, _) => &mut self.scl_up,
            (topo::SCL_STRETCH_DEV, _) => &mut self.scl_stretch,
            _ => &mut self.scl,
        }
    }

    /// Every channel state of a device.
    fn scl_dev_mut(&mut self, dev: u32) -> Vec<&mut SclState> {
        if dev == topo::SCL_UP_DEV {
            vec![&mut self.scl_up, &mut self.scl_media_up, &mut self.scl_hdmi_up]
        } else if dev == topo::SCL_STRETCH_DEV {
            vec![&mut self.scl_stretch]
        } else {
            vec![&mut self.scl]
        }
    }

    fn scl_any_channel(&self, dev: u32) -> bool {
        if dev == topo::SCL_UP_DEV {
            self.scl_up.channel || self.scl_media_up.channel || self.scl_hdmi_up.channel
        } else if dev == topo::SCL_STRETCH_DEV {
            false
        } else {
            self.scl.channel
        }
    }

    /// Does this state have what `op` needs? The hardware refuses (or hangs on) operations against resources that do
    /// not exist or are in the wrong state; planning mistakes of that kind are caught here before any MI call.
    pub fn precondition(&self, op: &Op) -> Result<(), String> {
        use Op::*;
        let fail = |what: &str| Err(format!("{what} ({op:?})"));
        match op {
            SensorEnable { .. } if self.sensor.is_none() => fail("sensor mode not set"),
            SensorDisable { .. } if self.sensor.is_none() => fail("no sensor to disable"),
            SensorFpsLive { .. } | SensorOrientation { .. } if self.sensor.is_none() => fail("no sensor"),
            VifDestroyGroup { .. } if !self.vif.group => fail("no VIF group"),
            VifDisableDev { .. } if !self.vif.dev_enabled => fail("VIF dev not enabled"),
            VifDisablePort { .. } if !self.vif.port_enabled => fail("VIF port not enabled"),
            VifEnableDev { .. } if !self.vif.group => fail("no VIF group"),
            IspCreateChannel { .. } if !self.isp.device => fail("no ISP device"),
            IspDestroyDevice { .. } if !self.isp.device => fail("no ISP device"),
            IspDestroyChannel { .. } | IspStopChannel { .. } | IspDisablePort { .. } if !self.isp.channel => fail("no ISP channel"),
            IspDestroyChannel { .. } if self.isp.started => fail("ISP channel still started"),
            IspStopChannel { .. } if !self.isp.started => fail("ISP channel not started"),
            IspDisablePort { .. } if !self.isp.port_enabled => fail("ISP port not enabled"),
            IspAdjust { .. } if !self.isp.started => fail("ISP channel not started"),
            SclCreateDevice { dev, .. } if self.scl_of(*dev, 0).device => fail("SCL device exists"),
            SclCreateChannel { dev, chn } if !self.scl_of(*dev, *chn).device => fail("no SCL device"),
            SclCreateChannel { dev, chn } if self.scl_of(*dev, *chn).channel => fail("SCL channel exists"),
            SclDestroyDevice { dev } if !self.scl_of(*dev, 0).device => fail("no SCL device"),
            SclDestroyDevice { dev } if self.scl_any_channel(*dev) => fail("SCL channel still exists"),
            SclDestroyChannel { dev, chn } if !self.scl_of(*dev, *chn).channel => fail("no SCL channel"),
            SclDestroyChannel { dev, chn } if self.scl_of(*dev, *chn).ports.iter().any(|p| p.enabled) => fail("an SCL port is still enabled"),
            SclStopChannel { dev, chn } if !self.scl_of(*dev, *chn).started => fail("SCL channel not started"),
            SclConfigurePort { dev, chn, .. } | SclEnablePort { dev, chn, .. } if !self.scl_of(*dev, *chn).channel => fail("no SCL channel"),
            SclDisablePort { dev, chn, port } if !self.scl_of(*dev, *chn).ports[*port as usize].enabled => fail("SCL port not enabled"),
            SclEnablePort { dev, chn, port } if self.scl_of(*dev, *chn).ports[*port as usize].configured.is_none() => fail("SCL port not configured"),
            Unbind { src, dst } if self.bind(*src, *dst).is_none() => fail("no such bind"),
            Bind { src, dst, .. } if self.bind(*src, *dst).is_some() && src.module != Module::Isp && src.module != Module::Scl => fail("already bound"),
            VencDown { dev, chn } if self.encoder(*chn).as_ref().map(|v| (v.dev, v.chn)) != Some((*dev, *chn)) => fail("no such encoder"),
            VencUp { chn, .. } if self.encoder(*chn).is_some() => fail("an encoder already exists"),
            VencUp { bound: Some(Ring { port: p, .. }), .. } if self.scl_of(p.dev, p.chn).ports[p.port as usize].configured.is_none() => fail("a ring from a port not configured"),
            VencUp { bound: Some(Ring { port: p, .. }), .. } if self.scl_of(p.dev, p.chn).ports[p.port as usize].enabled => fail("a ring bound to a port already started (mi_sys BUGs)"),
            SclDisablePort { dev, chn, port } if [&self.venc, &self.media, &self.snap].iter().any(|v| v.as_ref().and_then(|v| v.bound).map(|r| r.port) == Some(ChnPort { module: Module::Scl, dev: *dev, chn: *chn, port: *port })) => fail("the port is bound into an encoder in a ring: it goes with VencDown"),
            PumpStart { .. } if self.venc.is_none() => fail("pump without an encoder"),
            PoolUp { id, .. } if self.pool(*id).is_some() => fail("the pool exists"),
            PoolUp { src, .. } if !self.scl_of(src.dev, src.chn).channel => fail("the pool's port has no channel"),
            SclDestroyChannel { dev, chn } if self.pools.iter().any(|p| (p.src.dev, p.src.chn) == (*dev, *chn)) => fail("a pool lends views to this channel"),
            SclDestroyDevice { dev } if self.pools.iter().any(|p| p.src.dev == *dev) => fail("a pool lends views to this device"),
            PoolUp { readers, .. } if readers.iter().any(|r| !self.reader_exists(*r)) => fail("pool without its reader"),
            PoolUp { readers, .. } if readers.iter().any(|r| self.pool_read_by(*r).is_some()) => fail("a reader reads another pool"),
            PoolReader { id, .. } | PoolUnreader { id, .. } if self.pool(*id).is_none() => fail("no such pool"),
            PoolReader { reader, .. } if !self.reader_exists(*reader) => fail("reader without its channel"),
            PoolReader { reader, .. } if self.pool_read_by(*reader).is_some() => fail("the reader reads a pool already"),
            PoolReader { reader, .. } if self.feed(*reader).is_some() => fail("a feed runs into the reader"),
            PoolUnreader { id, reader } if self.pool(*id).is_some_and(|p| !p.readers.contains(reader)) => fail("not a reader of the pool"),
            PoolUnreader { id, reader } if self.pool(*id).is_some_and(|p| p.readers.first() == Some(reader)) => fail("the pool's own reader"),
            PoolCount { id, .. } | PoolDown { id } if self.pool(*id).is_none() => fail("no such pool"),
            PoolDown { id } if self.pool(*id).is_some_and(|p| p.readers.iter().any(|r| self.feed(*r).is_some())) => fail("a feed still uses the pool"),
            VencDown { dev, chn } if self.pools.iter().any(|p| p.readers.contains(&topo::venc_in(*dev, *chn))) => fail("the encoder's pool is still up"),
            FeedStart { dst, .. } if self.feed(*dst).is_some() => fail("a feed already runs into it"),
            FeedStart { dst, .. } if !self.reader_exists(*dst) => fail("feed into nothing"),
            FeedStart { src, way, .. } if !matches!(way, FeedWay::Canvas { view: None, .. }) && self.port_size(*src).is_none() => fail("feed from a port not enabled"),
            FeedStart { way: FeedWay::Canvas { pool, .. }, dst, .. } if self.pool(*pool).is_none_or(|p| !p.readers.contains(dst)) => fail("feed without its reader's pool"),
            FeedStart { way: FeedWay::Canvas { pool, view: Some(_), .. }, src, .. } if self.pool(*pool).is_some_and(|p| p.src != *src) => fail("the pool lends its views to another port"),
            FeedStart { .. } if !self.scl_stretch.device => fail("no memory-to-memory scaler"),
            FeedStop { dst } if self.feed(*dst).is_none() => fail("no such feed"),
            VencDown { chn, .. } if self.feed_into(*chn).is_some() => fail("a feed still runs into the encoder"),
            HdmiDispDown if self.feed(topo::DISP_IN).is_some() => fail("a feed still runs into DISP"),
            HdmiDispDown if self.pools.iter().any(|p| p.readers.contains(&topo::DISP_IN)) => fail("a pool still hands its canvases to DISP"),
            SclDestroyDevice { dev } if *dev == topo::SCL_STRETCH_DEV && self.composing() => fail("the compositor uses the scaler"),
            SclDisablePort { dev, chn, port } if self.feeds.iter().any(|f| (f.src.dev, f.src.chn, f.src.port) == (*dev, *chn, *port)) => fail("a feed reads this port"),
            PumpStop if self.pump.is_none() => fail("pump not running"),
            RecordStart { .. } if self.media.is_none() => fail("recorder without an encoder"),
            RecordStart { .. } if self.recording.is_some() => fail("already recording"),
            RecordStop if self.recording.is_none() => fail("not recording"),
            HdmiDispDown if self.hdmi.is_none() => fail("HDMI not up"),
            HdmiDispUp { .. } if self.hdmi.is_some() => fail("HDMI already up"),
            VdecUp { .. } if self.vdec.is_some() => fail("the decoder's channel exists"),
            VdecPort { .. } | VdecDown if self.vdec.is_none() => fail("no decoder channel"),
            VdecDown if self.binds.iter().any(|b| b.src == topo::VDEC_OUT) => fail("the decoder's port is still bound"),
            VdecDown if self.playing.is_some() => fail("the player still sends"),
            Bind { src, .. } if *src == topo::VDEC_OUT && self.vdec.is_none() => fail("no decoder channel"),
            PlayStart { .. } if self.vdec.is_none() => fail("player without a decoder"),
            PlayStart { .. } if self.playing.is_some() => fail("the player already sends"),
            PlayStop if self.playing.is_none() => fail("the player is not sending"),
            _ => Ok(()),
        }
    }

    /// Record a successfully executed operation.
    pub fn apply(&mut self, op: &Op) {
        use Op::*;
        match op {
            SensorSetRes { index, mode, .. } => {
                let s = self.sensor.get_or_insert_with(Default::default);
                s.mode = mode.clone();
                s.index = *index;
                s.enabled = false;
                s.oriented = None;
                s.fps_live = 0;
            }
            SensorEnable { .. } => {
                if let Some(s) = &mut self.sensor {
                    s.enabled = true;
                }
            }
            SensorDisable { .. } => self.sensor = None,
            SensorFpsLive { fps } => {
                if let Some(s) = &mut self.sensor {
                    s.fps_live = *fps;
                }
            }
            SensorOrientation { mirror, flip, .. } => {
                if let Some(s) = &mut self.sensor {
                    s.oriented = Some((*mirror, *flip));
                }
            }
            VifCreateGroup { .. } => self.vif.group = true,
            VifDestroyGroup { .. } => self.vif = VifState::default(),
            VifConfigureDev { .. } => self.vif.dev_configured = true,
            VifEnableDev { .. } => self.vif.dev_enabled = true,
            VifDisableDev { .. } => self.vif.dev_enabled = false,
            VifConfigurePort { .. } => self.vif.port_configured = true,
            VifEnablePort { .. } => self.vif.port_enabled = true,
            VifDisablePort { .. } => self.vif.port_enabled = false,
            IspCreateDevice { .. } => self.isp.device = true,
            IspDestroyDevice { .. } => self.isp = IspState::default(),
            IspCreateChannel { .. } => self.isp.channel = true,
            IspDestroyChannel { .. } => {
                let dev = self.isp.device;
                self.isp = IspState { device: dev, ..Default::default() };
            }
            IspSetChnParam { nr3d, .. } => self.isp.nr3d = Some(*nr3d),
            IspStartChannel { .. } => self.isp.started = true,
            IspStopChannel { .. } => self.isp.started = false,
            IspConfigurePort { .. } => self.isp.port_configured = true,
            IspEnablePort { .. } => self.isp.port_enabled = true,
            IspDisablePort { .. } => self.isp.port_enabled = false,
            IspLoadIq { path, .. } => {
                self.isp.iq = Some(path.clone());
                self.isp.tuning.clear();
                self.isp.refused.clear();
            }
            IspAdjust { adjust, .. } => {
                for (k, v) in adjust.knobs() {
                    self.isp.refused.remove(&k);
                    self.isp.tuning.insert(k, v);
                }
            }
            SclCreateDevice { dev, .. } => self.scl_dev_mut(*dev).into_iter().for_each(|s| s.device = true),
            SclDestroyDevice { dev } => {
                for s in self.scl_dev_mut(*dev) {
                    *s = SclState { clock_hz: s.clock_hz, ..Default::default() };
                }
            }
            SclCreateChannel { dev, chn } => self.scl_of_mut(*dev, *chn).channel = true,
            SclDestroyChannel { dev, chn } => {
                let s = self.scl_of_mut(*dev, *chn);
                s.channel = false;
                s.started = false;
                s.ports = Default::default();
            }
            SclStartChannel { dev, chn } => self.scl_of_mut(*dev, *chn).started = true,
            SclStopChannel { dev, chn } => self.scl_of_mut(*dev, *chn).started = false,
            SclConfigurePort { dev, chn, port, crop, size } => self.scl_of_mut(*dev, *chn).ports[*port as usize].configured = Some((*crop, *size)),
            SclEnablePort { dev, chn, port } => self.scl_of_mut(*dev, *chn).ports[*port as usize].enabled = true,
            SclDisablePort { dev, chn, port } => self.scl_of_mut(*dev, *chn).ports[*port as usize].enabled = false,
            PoolUp { id, src, readers, size, rows, n } => {
                self.pools.retain(|p| p.id != *id);
                self.pools.push(PoolState { id: *id, src: *src, readers: readers.clone(), size: *size, rows: *rows, n: *n });
            }
            PoolCount { id, n } => {
                if let Some(p) = self.pools.iter_mut().find(|p| p.id == *id) {
                    p.n = *n;
                }
            }
            PoolDown { id } => self.pools.retain(|p| p.id != *id),
            PoolReader { id, reader, .. } => {
                if let Some(p) = self.pools.iter_mut().find(|p| p.id == *id) {
                    p.readers.push(*reader);
                }
            }
            PoolUnreader { id, reader } => {
                if let Some(p) = self.pools.iter_mut().find(|p| p.id == *id) {
                    p.readers.retain(|r| r != reader);
                }
            }
            SclClock { hz } => self.scl.clock_hz = Some(*hz),
            Bind { src, dst, kind, src_fps, dst_fps } => {
                self.binds.retain(|b| !(b.src == *src && b.dst == *dst));
                self.binds.push(BindState { src: *src, dst: *dst, kind: *kind, src_fps: *src_fps, dst_fps: *dst_fps });
            }
            Unbind { src, dst } => self.binds.retain(|b| !(b.src == *src && b.dst == *dst)),
            SetDepth { .. } => {}
            HdmiDispUp { timing } => self.hdmi = Some(HdmiState { timing: *timing }),
            HdmiDispDown => self.hdmi = None,
            VencUp { dev, chn, codec, size, fps, mjpeg_quality, bitrate, gop, bound } => {
                self.codec_device |= *codec != Codec::Mjpeg;
                *self.encoder_mut(*chn) =
                    Some(VencState { dev: *dev, chn: *chn, codec: *codec, size: *size, fps: *fps, mjpeg_quality: *mjpeg_quality, bitrate: *bitrate, gop: *gop, bound: *bound })
            }
            VencDown { chn, .. } => {
                // a ring's port goes down inside the encoder's teardown (stopped, port off, unbound, destroyed)
                if let Some(p) = self.encoder(*chn).as_ref().and_then(|v| v.bound).map(|r| r.port) {
                    self.scl_of_mut(p.dev, p.chn).ports[p.port as usize].enabled = false;
                }
                *self.encoder_mut(*chn) = None
            }
            VdecUp { codec, size, ring, port } => {
                // the decoder's device is the encoders' Wave5 device: made with it, kept as the encoders' is
                self.codec_device = true;
                self.vdec = Some(VdecState { codec: *codec, size: *size, ring: *ring, port: *port })
            }
            VdecPort { port } => {
                if let Some(v) = &mut self.vdec {
                    v.port = *port;
                }
            }
            VdecDown => self.vdec = None,
            PlayStart { path, .. } => self.playing = Some(path.clone()),
            PlayStop => self.playing = None,
            FeedStart { src, dst, fps, way } => {
                self.feeds.retain(|f| f.dst != *dst);
                self.feeds.push(FeedState { src: *src, dst: *dst, fps: *fps, way: way.clone() });
            }
            FeedStop { dst } => self.feeds.retain(|f| f.dst != *dst),
            PumpStart { epoch, .. } => self.pump = Some(*epoch),
            PumpStop => self.pump = None,
            RecordStart { path, .. } => self.recording = Some(path.clone()),
            RecordStop => self.recording = None,
        }
    }
}

// ---- planning -------------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error(transparent)]
    Modes(#[from] crate::modes::ModesError),
}

/// A transition: the operations in execution order, and why they were chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub rung: Rung,
    pub ops: Vec<Op>,
}

/// The cost ladder (ARCHITECTURE §2), cheapest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    Nothing,
    /// pump restart for a new stream epoch with identical parameters
    PumpOnly,
    /// `fps_live` + bind rates
    FpsOnly,
    /// USB encoder / SCL port 2 only
    UsbOnly,
    /// media encoder / SCL port 1 only
    MediaOnly,
    /// HDMI timing / window only
    HdmiOnly,
    /// image settings only (3DNR, IQ, orientation)
    ImageOnly,
    /// several of the above without a sensor change
    Outputs,
    /// sensor mode change on a running chain: the sensor and the VIF restarted in the new mode, the ISP, the scalers,
    /// the pools and the encoders kept; the outputs follow as on the ladder
    Switch,
    /// sensor mode change with no running chain: everything from the sensor up is built
    Rebuild,
}

/// Compute the operations that take `cur` to `want`.
pub fn plan(cur: &Current, want: &Desired, modes: &ModeTable) -> Result<Plan, PlanError> {
    use topo::*;
    let mut ops = Vec::new();
    // a recording's or a photo's chain that ends goes before anything else: what comes back is never built beside it
    if want.media.is_none() {
        media_down(cur, false, &mut ops);
    }
    let media_first = !ops.is_empty();
    let stopped;
    let cur = if media_first {
        stopped = after(cur, &ops);
        &stopped
    } else {
        cur
    };
    let sensor_same = cur.sensor.as_ref().map(|s| s.mode == want.sensor.mode && s.enabled).unwrap_or(false)
        && cur.vif.dev_enabled
        && cur.vif.port_enabled
        && cur.isp.started
        && cur.scl.started
        && cur.bind(VIF_OUT, ISP_IN).is_some()
        && cur.bind(ISP_OUT, SCL_IN).is_some();

    let switched = !sensor_same && switchable(cur);
    if !sensor_same && !switched {
        teardown_capture(cur, &mut ops);
        build_capture(want, modes, &mut ops)?;
        // the outputs are planned from the state the teardown leaves: nothing but the scaler clock, DISP/HDMI (which
        // stay up across a sensor rebuild, their pool gone with its port) and the memory-to-memory scaler — planning them
        // from `cur` re-issued unbind/destroy of what the teardown had already removed (2026-10-24:
        // `MI_SYS_UnBindChnPort 0xa0092003` at every stream off)
        let mut after = Current::default();
        after.scl.clock_hz = cur.scl.clock_hz;
        after.hdmi = cur.hdmi.clone();
        after.scl_stretch = cur.scl_stretch.clone();
        // the player's decoder and thread go on across a sensor rebuild (its binds went with SCL dev 1's channels)
        after.vdec = cur.vdec;
        after.playing = cur.playing.clone();
        build_outputs(&after, want, &mut ops, true);
        return Ok(Plan { rung: Rung::Rebuild, ops });
    }

    // same sensor mode, or the new one switched to on the running chain: the ladder from there
    let mid;
    let cur = if switched {
        switch_sensor(cur, want, modes, &mut ops)?;
        mid = after(cur, &ops);
        &mid
    } else {
        cur
    };
    let mut rungs = Vec::new();
    if media_first {
        rungs.push(Rung::MediaOnly);
    }

    // image settings
    if cur.isp.nr3d != Some(want.image.nr3d) {
        ops.push(Op::IspSetChnParam { dev: ISP_DEV, chn: ISP_CHN, nr3d: want.image.nr3d });
        rungs.push(Rung::ImageOnly);
    }
    let reload_iq = cur.isp.iq.as_deref() != Some(want.image.iq_bin.as_str());
    if reload_iq {
        ops.push(Op::IspLoadIq { dev: ISP_DEV, chn: ISP_CHN, path: want.image.iq_bin.clone() });
        rungs.push(Rung::ImageOnly);
    }
    // an IQ load resets every picture knob, so after one they are all applied again; a knob refused since the last load
    // is asked again only at another value
    let applied = if reload_iq { Tuning::new() } else { cur.isp.asked() };
    for adjust in tuning::plan(&want.image.tuning, &applied) {
        ops.push(Op::IspAdjust { dev: ISP_DEV, chn: ISP_CHN, adjust });
        rungs.push(Rung::ImageOnly);
    }
    if cur.sensor.as_ref().and_then(|s| s.oriented) != Some((want.image.mirror, want.image.flip)) {
        ops.push(Op::SensorOrientation { pad: PAD, mirror: want.image.mirror, flip: want.image.flip });
        rungs.push(Rung::ImageOnly);
    }

    // outputs
    let hdmi_changed = match &want.hdmi {
        None => cur.hdmi.is_some() || cur.feed(topo::DISP_IN).is_some() || cur.pool(topo::POOL_HDMI).is_some() || cur.scl.ports[topo::SCL_PORT_HDMI as usize].enabled,
        Some(h) => cur.hdmi.as_ref().map(|s| s.timing) != Some(h.timing) || !hdmi_feed_same(cur, h, src_fps(want), &joiners(want, topo::POOL_HDMI)),
    };
    let usb_changed = !usb_encoder_same(cur, want) || want.usb.as_ref().is_some_and(|u| !usb_feed_same(cur, u, src_fps(want), usb_joins(want), &joiners(want, topo::POOL_USB)));
    let player_changed = cur.vdec.map(|v| (v.codec, v.size, v.ring, v.port)) != want.player.as_ref().map(|p| (p.codec, p.size, p.ring, p.port))
        || cur.playing.as_deref() != want.player.as_ref().map(|p| p.path.as_str());
    let epoch_changed = cur.pump != want.usb.as_ref().map(|u| u.epoch);
    let fps_changed = cur.sensor.as_ref().map(|s| effective_fps(s.fps_live, want.sensor.mode_max_fps)).unwrap_or(0) != want.sensor.fps;
    let media_changed = !media_same(cur, want);

    if fps_changed {
        rungs.push(Rung::FpsOnly);
    }
    if hdmi_changed {
        rungs.push(Rung::HdmiOnly);
    }
    if usb_changed {
        rungs.push(Rung::UsbOnly);
    } else if epoch_changed {
        rungs.push(Rung::PumpOnly);
    }
    if media_changed {
        rungs.push(Rung::MediaOnly);
    }
    if player_changed {
        rungs.push(Rung::Outputs);
    }

    if fps_changed || hdmi_changed || usb_changed || epoch_changed || media_changed || player_changed {
        build_outputs(cur, want, &mut ops, false);
    }

    rungs.dedup();
    let rung = match rungs.len() {
        _ if switched => Rung::Switch,
        0 => Rung::Nothing,
        1 => rungs[0],
        _ => Rung::Outputs,
    };
    Ok(Plan { rung, ops })
}

/// The capture chain runs whole: a new sensor mode can be switched to on it.
fn switchable(cur: &Current) -> bool {
    use topo::*;
    cur.sensor.as_ref().is_some_and(|s| s.enabled)
        && cur.vif.dev_enabled
        && cur.vif.port_enabled
        && cur.isp.started
        && cur.scl.started
        && cur.bind(VIF_OUT, ISP_IN).is_some()
        && cur.bind(ISP_OUT, SCL_IN).is_some()
}

/// The sensor and the VIF restarted in the new mode on the running chain. The ISP takes the new frame size from its
/// input by itself (it configures its input on every frame whose size or format changed); the scaler's ports are cut
/// in the old mode's coordinates, so they go off first and the outputs' steps after this configure them anew.
fn switch_sensor(cur: &Current, want: &Desired, modes: &ModeTable, ops: &mut Vec<Op>) -> Result<(), PlanError> {
    use topo::*;
    let index = modes.driver_index(&want.sensor.mode)?;
    let max = want.sensor.mode_max_fps;
    let start = ops.len();
    let now = |ops: &Vec<Op>| after(cur, &ops[start..]);
    // a mode of the same frame size: the ports' cuts hold, everything behind them waits out the sensor's restart
    let resized = cur.sensor.as_ref().and_then(|s| modes.get(&s.mode).ok()).map(|m| m.size()) != Some(want.sensor.size);
    if resized {
        // an encoder bound to its port in a ring goes with the port; the feeds stop before the ports they read
        if cur.media.as_ref().is_some_and(|v| v.bound.is_some()) {
            teardown_media(&now(ops), ops);
        }
        if cur.venc.as_ref().is_some_and(|v| v.bound.is_some()) {
            teardown_usb_encoder(&now(ops), ops);
        }
        for f in now(ops).feeds {
            ops.push(Op::FeedStop { dst: f.dst });
        }
        for (p, st) in now(ops).scl.ports.iter().enumerate() {
            if st.enabled {
                ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: p as u32 });
            }
        }
    }
    ops.push(Op::VifDisablePort { dev: VIF_DEV, port: VIF_PORT });
    ops.push(Op::VifDisableDev { dev: VIF_DEV });
    ops.push(Op::SensorDisable { pad: PAD });
    ops.push(Op::SensorSetRes { pad: PAD, index, mode: want.sensor.mode.clone() });
    ops.push(Op::SensorEnable { pad: PAD });
    ops.push(Op::VifConfigureDev { dev: VIF_DEV });
    ops.push(Op::VifEnableDev { dev: VIF_DEV });
    ops.push(Op::VifConfigurePort { dev: VIF_DEV, port: VIF_PORT });
    ops.push(Op::SetDepth { port: VIF_OUT, user: 0, buffers: 4 });
    ops.push(Op::VifEnablePort { dev: VIF_DEV, port: VIF_PORT });
    ops.push(Op::Bind { src: ISP_OUT, dst: SCL_IN, kind: BindKind::Realtime, src_fps: max, dst_fps: max });
    ops.push(Op::SensorOrientation { pad: PAD, mirror: want.image.mirror, flip: want.image.flip });
    if want.sensor.fps < max {
        ops.push(Op::SensorFpsLive { fps: want.sensor.fps });
    }
    Ok(())
}

fn effective_fps(fps_live: u32, max: u32) -> u32 {
    if fps_live == 0 {
        max
    } else {
        fps_live.min(max)
    }
}

/// The memory-to-memory scaler is needed: an output fed through a pool (its pictures scaled to their windows, the menu
/// over them baked).
fn stretch_wanted(want: &Desired) -> bool {
    want.hdmi.is_some() || want.usb.is_some() || want.media.is_some()
}

/// The screen off: DISP/HDMI down (its feed, pool and port are the caller's). The memory-to-memory scaler goes too
/// unless a compositor uses it.
pub fn teardown_hdmi(cur: &Current, ops: &mut Vec<Op>) {
    if cur.hdmi.is_some() {
        ops.push(Op::HdmiDispDown);
    }
    if cur.scl_stretch.device && !cur.composing() {
        ops.push(Op::SclDestroyDevice { dev: topo::SCL_STRETCH_DEV });
    }
}

/// Everything down: the capture chain with its encoders, then the screen.
pub fn teardown_all(cur: &Current) -> Vec<Op> {
    let mut ops = Vec::new();
    teardown_capture(cur, &mut ops);
    let mid = after(cur, &ops);
    // the player: its binds went with SCL dev 1's channels
    if mid.playing.is_some() {
        ops.push(Op::PlayStop);
    }
    if mid.vdec.is_some() {
        ops.push(Op::VdecDown);
    }
    let mid = after(cur, &ops);
    teardown_hdmi(&mid, &mut ops);
    ops
}

/// Consumer first while the sensor still runs; every SCL port disabled before any SCL destroy; sensor last.
/// Only what exists is torn down, so this is also the rollback from any partial state.
pub fn teardown_capture(cur: &Current, ops: &mut Vec<Op>) {
    use topo::*;
    // encoder chains (each pool's port goes with its pool, the other dev-0 ports below); each step planned from the
    // state the ones before leave
    let start = ops.len();
    let now = |ops: &Vec<Op>| after(cur, &ops[start..]);
    teardown_media(cur, ops);
    teardown_upscaler(&now(ops), &MEDIA_FEED, ops);
    teardown_usb_encoder(&now(ops), ops);
    teardown_upscaler(&now(ops), &USB_FEED, ops);
    // HDMI chain: its feed, pool (while DISP, its reader, still exists) and upscaler; DISP/HDMI stay up across a sensor
    // rebuild
    teardown_hdmi_feed(&now(ops), ops);
    teardown_upscaler(&now(ops), &HDMI_FEED, ops);
    let cur = &now(ops);
    if cur.scl_up.device {
        ops.push(Op::SclDestroyDevice { dev: SCL_UP_DEV });
    }
    // SCL
    for (p, st) in cur.scl.ports.iter().enumerate() {
        if st.enabled {
            ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: p as u32 });
        }
    }
    if cur.scl.started {
        ops.push(Op::SclStopChannel { dev: SCL_DEV, chn: SCL_CHN });
    }
    if cur.scl.channel {
        ops.push(Op::SclDestroyChannel { dev: SCL_DEV, chn: SCL_CHN });
    }
    if cur.bind(ISP_OUT, SCL_IN).is_some() {
        ops.push(Op::Unbind { src: ISP_OUT, dst: SCL_IN });
    }
    if cur.scl.device {
        ops.push(Op::SclDestroyDevice { dev: SCL_DEV });
    }
    // ISP
    if cur.isp.port_enabled {
        ops.push(Op::IspDisablePort { dev: ISP_DEV, chn: ISP_CHN, port: ISP_PORT });
    }
    if cur.bind(VIF_OUT, ISP_IN).is_some() {
        ops.push(Op::Unbind { src: VIF_OUT, dst: ISP_IN });
    }
    if cur.isp.started {
        ops.push(Op::IspStopChannel { dev: ISP_DEV, chn: ISP_CHN });
    }
    if cur.isp.channel {
        ops.push(Op::IspDestroyChannel { dev: ISP_DEV, chn: ISP_CHN });
    }
    if cur.isp.device {
        ops.push(Op::IspDestroyDevice { dev: ISP_DEV });
    }
    // VIF
    if cur.vif.port_enabled {
        ops.push(Op::VifDisablePort { dev: VIF_DEV, port: VIF_PORT });
    }
    if cur.vif.dev_enabled {
        ops.push(Op::VifDisableDev { dev: VIF_DEV });
    }
    if cur.vif.group {
        ops.push(Op::VifDestroyGroup { group: VIF_GROUP });
    }
    // sensor
    if cur.sensor.is_some() {
        ops.push(Op::SensorDisable { pad: PAD });
    }
}

/// Everything from the sensor to the SCL channel, in the SDK's order.
fn build_capture(want: &Desired, modes: &ModeTable, ops: &mut Vec<Op>) -> Result<(), PlanError> {
    use topo::*;
    let index = modes.driver_index(&want.sensor.mode)?;
    let max = want.sensor.mode_max_fps;
    ops.push(Op::SensorSetRes { pad: PAD, index, mode: want.sensor.mode.clone() });
    ops.push(Op::SensorEnable { pad: PAD });
    ops.push(Op::VifCreateGroup { group: VIF_GROUP });
    ops.push(Op::VifConfigureDev { dev: VIF_DEV });
    ops.push(Op::VifEnableDev { dev: VIF_DEV });
    ops.push(Op::VifConfigurePort { dev: VIF_DEV, port: VIF_PORT });
    ops.push(Op::SetDepth { port: VIF_OUT, user: 0, buffers: 4 });
    ops.push(Op::VifEnablePort { dev: VIF_DEV, port: VIF_PORT });
    ops.push(Op::IspCreateDevice { dev: ISP_DEV });
    ops.push(Op::IspCreateChannel { dev: ISP_DEV, chn: ISP_CHN, pad: PAD });
    ops.push(Op::IspSetChnParam { dev: ISP_DEV, chn: ISP_CHN, nr3d: want.image.nr3d });
    ops.push(Op::IspStartChannel { dev: ISP_DEV, chn: ISP_CHN });
    ops.push(Op::IspConfigurePort { dev: ISP_DEV, chn: ISP_CHN, port: ISP_PORT });
    ops.push(Op::SetDepth { port: ISP_OUT, user: 0, buffers: 5 });
    ops.push(Op::IspEnablePort { dev: ISP_DEV, chn: ISP_CHN, port: ISP_PORT });
    ops.push(Op::Bind { src: VIF_OUT, dst: ISP_IN, kind: BindKind::Realtime, src_fps: max, dst_fps: max });
    ops.push(Op::SclCreateDevice { dev: SCL_DEV, hw_mask: SCL_HW_MASK });
    ops.push(Op::SclCreateChannel { dev: SCL_DEV, chn: SCL_CHN });
    ops.push(Op::SclStartChannel { dev: SCL_DEV, chn: SCL_CHN });
    ops.push(Op::Bind { src: ISP_OUT, dst: SCL_IN, kind: BindKind::Realtime, src_fps: max, dst_fps: max });
    ops.push(Op::IspLoadIq { dev: ISP_DEV, chn: ISP_CHN, path: want.image.iq_bin.clone() });
    for adjust in tuning::plan(&want.image.tuning, &Tuning::new()) {
        ops.push(Op::IspAdjust { dev: ISP_DEV, chn: ISP_CHN, adjust });
    }
    ops.push(Op::SensorOrientation { pad: PAD, mirror: want.image.mirror, flip: want.image.flip });
    if want.sensor.fps < max {
        ops.push(Op::SensorFpsLive { fps: want.sensor.fps });
    }
    Ok(())
}

/// HDMI and USB outputs. `fresh` = the capture chain was just rebuilt (every port must be configured again).
///
/// An encoder joining another output's frames or leaving them is planned in three parts: what it gives
/// up first — its own pool, port and upscaler when it joins (memory freed before the other pool may grow for it), the
/// other pool when it leaves (before that output's steps take the pool down or make it anew) —, then HDMI's, USB's and
/// the media encoder's steps in that order, each taking its own path or adding its encoder as a reader of the pool it
/// joins.
fn build_outputs(cur: &Current, want: &Desired, ops: &mut Vec<Op>, fresh: bool) {
    use topo::*;
    let start = ops.len();
    let fps = want.sensor.fps;
    // the rate the live windows' frames come at: the sensor's, or the player's file's
    let src_fps = src_fps(want);
    if cur.scl.clock_hz != Some(480_000_000) {
        ops.push(Op::SclClock { hz: 480_000_000 });
    }
    if !fresh {
        // frame rate: live, then bind rates follow
        let cur_fps = cur.sensor.as_ref().map(|s| effective_fps(s.fps_live, want.sensor.mode_max_fps)).unwrap_or(0);
        if cur_fps != fps {
            ops.push(Op::SensorFpsLive { fps: if fps == want.sensor.mode_max_fps { 0 } else { fps } });
            ops.push(Op::Bind { src: ISP_OUT, dst: SCL_IN, kind: BindKind::Realtime, src_fps: fps, dst_fps: fps });
        }
    }

    // the memory-to-memory scaler and the upscaler device before any output's steps: HDMI's and USB's steps are planned
    // apart and may be reordered below (each output's upscaler channel is its own)
    if stretch_wanted(want) && !cur.scl_stretch.device {
        ops.push(Op::SclCreateDevice { dev: SCL_STRETCH_DEV, hw_mask: SCL_STRETCH_HW_MASK });
    }
    if upscaler_wanted(want) && !cur.scl_up.device {
        ops.push(Op::SclCreateDevice { dev: SCL_UP_DEV, hw_mask: SCL_UP_HW_MASK });
    }
    // an encoder reading another output's frame now or before: its steps around that output's are ordered by hand, not
    // by the heap below
    let joined = usb_joins(want);
    let joining = joined || cur.usb_joined() || media_host(want).is_some() || cur.media_joined().is_some();
    if !fresh {
        let now = after(cur, &ops[start..]);
        media_release(&now, want, ops);
        let now = after(cur, &ops[start..]);
        usb_release(&now, want, ops);
    }
    // the player's decoder before any output binds its port; its thread stopped before anything of it changes
    player_up(&after(cur, &ops[start..]), want, ops);
    // what HDMI's steps are planned from
    let base = after(cur, &ops[start..]);

    // ---- HDMI
    let hdmi_start = ops.len();
    // a new timing restarts DISP/HDMI; anything else re-feeds DISP from the pool as USB's encoder is re-fed
    match &want.hdmi {
        None => {
            teardown_hdmi_feed(&base, ops);
            let base = after(&base, &ops[hdmi_start..]);
            teardown_upscaler(&base, &HDMI_FEED, ops);
            if base.scl.ports[SCL_PORT_HDMI as usize].enabled && !fresh {
                ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_HDMI });
            }
            // the scaler goes with the rest once nothing needs it ([`drop_idle_stretch`])
            if base.hdmi.is_some() {
                ops.push(Op::HdmiDispDown);
            }
        }
        Some(h) => {
            let restart = base.hdmi.as_ref().is_some_and(|s| s.timing != h.timing);
            let rate = h.rate(src_fps);
            let way = hdmi_way(h);
            let joiners = joiners(want, POOL_HDMI);
            let pool = hdmi_pool(h, src_fps, &joiners);
            let feed_same = !fresh && !restart && hdmi_feed_same(&base, h, src_fps, &joiners);
            if !feed_same {
                if restart {
                    // the pool goes while DISP, its reader, is still there
                    teardown_hdmi_feed(&base, ops);
                } else {
                    if base.feed(DISP_IN).is_some() {
                        ops.push(Op::FeedStop { dst: DISP_IN });
                    }
                    pool_down_if_moved(&base, &pool, ops);
                }
            }
            if restart {
                ops.push(Op::HdmiDispDown);
            }
            if restart || base.hdmi.is_none() {
                ops.push(Op::HdmiDispUp { timing: h.timing });
            }
            if !feed_same {
                let c = after(&base, &ops[hdmi_start..]);
                let pool_op = pool_op(&c, &pool);
                let src = if covered(&way) {
                    port_off(&c, &HDMI_FEED, fresh, ops);
                    ops.extend(pool_op);
                    pool.src
                } else {
                    let src = build_feed(&c, &HDMI_FEED, h.from, h.crop, h.scl_out, h.frame.view.size(), src_fps, rate, fresh, pool.n, pool_op, ops);
                    feed_queue(src, FEED_USER_FRAMES, pool.n, ops);
                    src
                };
                ops.push(Op::FeedStart { src, dst: DISP_IN, fps: rate, way });
            }
        }
    }

    // ---- USB, planned from the state HDMI's steps leave (they make HDMI's pool, which a joined USB reads)
    let usb_start = ops.len();
    let now = after(cur, &ops[start..]);
    // the encoder (codec, size, rate, quality) restarts only for its own changes; a new crop, zoom or picture layout
    // re-feeds the running encoder: from SCL port 2 directly, or through the upscaler when the crop is smaller than the
    // live window; with pictures beside the live one the feed composes the frame
    let encoder_same = !fresh && usb_encoder_same(&now, want);
    match &want.usb {
        None => {
            teardown_usb_encoder(&now, ops);
            let now = after(cur, &ops[start..]);
            teardown_upscaler(&now, &USB_FEED, ops);
            if now.scl.ports[SCL_PORT_USB as usize].enabled && !fresh {
                ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB });
            }
        }
        Some(u) if u.frame_of == FrameOf::Ring => {
            // no pool and no feed: SCL port 2 at the stream's size bound into the encoder in a ring, built in this
            // order: the port configured, the channel made in ring mode, bound and started, then the port enabled —
            // mi_sys BUGs (a kernel oops) on a ring bind to a port already started. A new crop is set on the running port; anything else builds the encoder again
            let dev = venc_dev(u.codec);
            let port = &now.scl.ports[SCL_PORT_USB as usize];
            if !encoder_same {
                if !fresh {
                    teardown_usb_encoder(&now, ops);
                }
                let c = after(&now, &ops[usb_start..]);
                teardown_upscaler(&c, &USB_FEED, ops);
                let c = after(&now, &ops[usb_start..]);
                if c.scl.ports[SCL_PORT_USB as usize].enabled && !fresh {
                    ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB });
                }
                ops.push(Op::SclConfigurePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB, crop: u.crop, size: u.size });
                ops.push(Op::VencUp { dev, chn: VENC_CHN_USB, codec: u.codec, size: u.size, fps: u.fps, mjpeg_quality: want.image.mjpeg_quality, bitrate: 0, gop: 0, bound: usb_bound(u, want) });
                ops.push(Op::SclEnablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB });
            } else if port.configured != Some((u.crop, u.size)) {
                ops.push(Op::SclConfigurePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB, crop: u.crop, size: u.size });
            }
            if !encoder_same || now.pump != Some(u.epoch) {
                if encoder_same && now.pump.is_some() {
                    ops.push(Op::PumpStop);
                }
                ops.push(Op::PumpStart { epoch: u.epoch, venc_dev: dev, venc_chn: VENC_CHN_USB });
            }
        }
        Some(u) if joined => {
            // its own way is gone already ([`usb_release`]); the encoder reads HDMI's canvases
            let dev = venc_dev(u.codec);
            let venc = venc_in(dev, VENC_CHN_USB);
            if !encoder_same {
                ops.push(Op::VencUp { dev, chn: VENC_CHN_USB, codec: u.codec, size: u.size, fps: u.fps, mjpeg_quality: want.image.mjpeg_quality, bitrate: 0, gop: 0, bound: None });
            }
            let c = after(&now, &ops[usb_start..]);
            if !c.pool(POOL_HDMI).is_some_and(|p| p.readers.contains(&venc)) {
                // a new encoder device is served once a port's depth changes (as [`feed_queue`] does for an own feed):
                // HDMI's port, as it is, before the encoder's first canvas
                if !encoder_same {
                    if let (Some(f), Some(p)) = (c.feed(DISP_IN).filter(|f| !covered(&f.way)), c.pool(POOL_HDMI)) {
                        feed_queue(f.src, FEED_USER_FRAMES, p.n, ops);
                    }
                }
                ops.push(Op::PoolReader { id: POOL_HDMI, reader: venc, fps: u.fps });
            }
            if !encoder_same || now.pump != Some(u.epoch) {
                if encoder_same && now.pump.is_some() {
                    ops.push(Op::PumpStop);
                }
                ops.push(Op::PumpStart { epoch: u.epoch, venc_dev: dev, venc_chn: VENC_CHN_USB });
            }
        }
        Some(u) => {
            let dev = venc_dev(u.codec);
            let venc = venc_in(dev, VENC_CHN_USB);
            let way = usb_way(u);
            let joiners = joiners(want, POOL_USB);
            let pool = usb_pool(u, src_fps, &joiners);
            let feed_same = encoder_same && usb_feed_same(&now, u, src_fps, false, &joiners);
            if !encoder_same {
                if !fresh {
                    teardown_usb_encoder(&now, ops);
                }
            } else if !feed_same {
                if let Some(f) = now.feed_into(VENC_CHN_USB) {
                    ops.push(Op::FeedStop { dst: f.dst });
                }
                pool_down_if_moved(&now, &pool, ops);
            }
            if !encoder_same {
                ops.push(Op::VencUp { dev, chn: VENC_CHN_USB, codec: u.codec, size: u.size, fps: u.fps, mjpeg_quality: want.image.mjpeg_quality, bitrate: 0, gop: 0, bound: None });
            }
            if !feed_same {
                let c = after(&now, &ops[usb_start..]);
                let pool_op = pool_op(&c, &pool);
                let src = if covered(&way) {
                    port_off(&c, &USB_FEED, fresh, ops);
                    ops.extend(pool_op);
                    pool.src
                } else {
                    let src = build_feed(&c, &USB_FEED, u.from, u.crop, u.scl_out, u.frame.view.size(), src_fps, u.fps, fresh, pool.n, pool_op, ops);
                    feed_queue(src, FEED_USER_FRAMES, pool.n, ops);
                    src
                };
                ops.push(Op::FeedStart { src, dst: venc, fps: u.fps, way });
            }
            if !encoder_same || now.pump != Some(u.epoch) {
                if encoder_same && now.pump.is_some() {
                    ops.push(Op::PumpStop);
                }
                ops.push(Op::PumpStart { epoch: u.epoch, venc_dev: dev, venc_chn: VENC_CHN_USB });
            }
        }
    }

    // ---- media, planned from the state the steps above leave (they may have made or dropped the shared upscaler device)
    let mid = after(cur, &ops[start..]);
    let media_start = ops.len();
    build_media(&mid, want, fresh, ops);
    let mid = after(&mid, &ops[media_start..]);
    // memory is freed before it is taken across the outputs: HDMI's steps go after USB's and the media's when HDMI takes
    // media heap and they give some back (a covering picture gone: HDMI's port writes again, USB's canvases go). Not
    // while an encoder reads another output's frame, now or before: its steps are ordered around that output's above
    let price = |c: &Current| crate::heap::video(c).total();
    let (before, hdmi, rest) = (price(cur), price(&after(cur, &ops[hdmi_start..usb_start])), price(&after(cur, &ops[usb_start..])));
    if !joining && hdmi > before && rest < before {
        ops[hdmi_start..].rotate_left(usb_start - hdmi_start);
    }
    // `mid` is the state every step above leaves (reordered or not)
    let end = ops.len();
    player_start(&mid, want, ops);
    let mid = after(&mid, &ops[end..]);
    let end = ops.len();
    drop_idle_stretch(&mid, want, ops);
    drop_idle_upscaler(&after(&mid, &ops[end..]), ops);
}

/// The rate the live windows' frames come at: the sensor's, or the player's file's.
fn src_fps(want: &Desired) -> u32 {
    want.player.as_ref().map_or(want.sensor.fps, |p| p.fps)
}

/// The player's decoder as `want` has it, before the outputs' steps (they bind its port): the thread stopped first when
/// it sends another file or its decoder changes; a decoder of another codec, size or ring made anew (its binds off
/// first); its port's size set live. No file: the decoder goes here, its binds off first, before the outputs' steps
/// bring the live path back — so a stop never holds the decoder and the live path at once.
fn player_up(cur: &Current, want: &Desired, ops: &mut Vec<Op>) {
    let wanted = want.player.as_ref().map(|p| (p.codec, p.size, p.ring));
    let same = cur.vdec.map(|v| (v.codec, v.size, v.ring)) == wanted;
    if cur.playing.is_some() && (!same || cur.playing.as_deref() != want.player.as_ref().map(|p| p.path.as_str())) {
        ops.push(Op::PlayStop);
    }
    let Some(p) = want.player.as_ref() else {
        for b in cur.binds.iter().filter(|b| b.src == topo::VDEC_OUT) {
            ops.push(Op::Unbind { src: b.src, dst: b.dst });
        }
        if cur.vdec.is_some() {
            ops.push(Op::VdecDown);
        }
        return;
    };
    match cur.vdec {
        Some(v) if same => {
            if v.port != p.port {
                ops.push(Op::VdecPort { port: p.port });
            }
        }
        v => {
            if v.is_some() {
                for b in cur.binds.iter().filter(|b| b.src == topo::VDEC_OUT) {
                    ops.push(Op::Unbind { src: b.src, dst: b.dst });
                }
                ops.push(Op::VdecDown);
            }
            ops.push(Op::VdecUp { codec: p.codec, size: p.size, ring: p.ring, port: p.port });
        }
    }
}

/// After the outputs' steps: the player thread started once its decoder's port is bound where `want` has it.
fn player_start(cur: &Current, want: &Desired, ops: &mut Vec<Op>) {
    if let Some(p) = want.player.as_ref().filter(|_| cur.playing.is_none()) {
        ops.push(Op::PlayStart { path: p.path.clone(), fps: p.fps });
    }
}

/// What USB gives up before HDMI's steps when its frame becomes HDMI's or stops being it. Joining: its own feed, pool,
/// upscaler and port (an encoder that changes goes too) — the memory they hold freed before HDMI's pool may take more.
/// Leaving: its encoder taken off HDMI's pool (or down, when it changes) before HDMI's steps take that pool down or make
/// it anew; the own path is built after them. Nothing when HDMI's frames are no part of it.
fn usb_release(c: &Current, want: &Desired, ops: &mut Vec<Op>) {
    use topo::*;
    let joined = usb_joins(want);
    if !joined && !c.usb_joined() {
        return;
    }
    let start = ops.len();
    if want.usb.is_none() || !usb_encoder_same(c, want) {
        // feed, pump, own pool, HDMI's reader, encoder
        teardown_usb_encoder(c, ops);
    } else if joined {
        if let Some(f) = c.feed_into(VENC_CHN_USB) {
            ops.push(Op::FeedStop { dst: f.dst });
        }
        pool_down(c, POOL_USB, ops);
    } else if let Some(r) = c.usb_reader() {
        ops.push(Op::PoolUnreader { id: POOL_HDMI, reader: r });
    }
    if joined {
        let c = &after(c, &ops[start..]);
        teardown_upscaler(c, &USB_FEED, ops);
        if c.scl.ports[SCL_PORT_USB as usize].enabled {
            ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB });
        }
    }
}

/// USB's frame is HDMI's: its encoder reads HDMI's pool (the policy's `FrameOf::Hdmi`; HDMI wanted with it).
fn usb_joins(want: &Desired) -> bool {
    want.hdmi.is_some() && want.usb.as_ref().is_some_and(|u| u.frame_of == FrameOf::Hdmi)
}

/// The encoder input that reads HDMI's pool for `want`, if USB's frame is HDMI's.
fn usb_joining(want: &Desired) -> Option<ChnPort> {
    want.usb.as_ref().filter(|_| usb_joins(want)).map(|u| topo::venc_in(topo::venc_dev(u.codec), topo::VENC_CHN_USB))
}

/// The pool of another output the media encoder reads for `want` (its frame is that output's).
fn media_host(want: &Desired) -> Option<u32> {
    match want.media.as_ref()?.frame_of {
        FrameOf::Own | FrameOf::Ring => None,
        FrameOf::Hdmi => want.hdmi.as_ref().map(|_| topo::POOL_HDMI),
        FrameOf::Usb => want.usb.as_ref().filter(|u| u.frame_of == FrameOf::Own).map(|_| topo::POOL_USB),
    }
}

/// The media encoder of `m`, as a reader.
fn media_venc(m: &MediaPlan) -> ChnPort {
    topo::venc_in(topo::venc_dev(m.codec), topo::VENC_CHN_MEDIA)
}

/// The encoders of other outputs that read pool `id` for `want`, beside its own reader, each with its rate.
fn joiners(want: &Desired, id: u32) -> Vec<(ChnPort, u32)> {
    let mut v = Vec::new();
    if id == topo::POOL_HDMI {
        v.extend(usb_joining(want).zip(want.usb.as_ref().map(|u| u.fps)));
    }
    if media_host(want) == Some(id) {
        v.extend(want.media.as_ref().map(|m| (media_venc(m), m.fps)));
    }
    v
}

/// What the media encoder gives up before HDMI's and USB's steps when its frame becomes another output's or stops being
/// it, as USB's in [`usb_release`]: joining — its own feed, pool, upscaler and port; moving to another output's pool, or
/// leaving one those steps take down or change — its encoder taken off the pool it reads (an encoder that changes goes
/// whole), the photo's encoder with it. Leaving a pool that stays, it reads it on until its own path is built (the
/// media steps switch it: the file loses at most the frame the switch spans).
fn media_release(c: &Current, want: &Desired, ops: &mut Vec<Op>) {
    use topo::*;
    let host = media_host(want);
    let now = c.media_joined();
    if host.is_none() && now.is_none() {
        return;
    }
    let start = ops.len();
    if want.media.is_none() || !media_encoder_same(c, want.media.as_ref()) {
        teardown_media(c, ops);
    } else {
        if host != now && (host.is_some() || now.is_some_and(|id| pool_changes(c, want, id))) {
            if let (Some(r), Some(id)) = (c.snap_reader(), c.snap_pool()) {
                ops.push(Op::PoolUnreader { id, reader: r });
            }
            if let (Some(r), Some(id)) = (c.media_reader(), now) {
                ops.push(Op::PoolUnreader { id, reader: r });
            }
        }
        if host.is_some() {
            if let Some(f) = c.feed_into(VENC_CHN_MEDIA) {
                ops.push(Op::FeedStop { dst: f.dst });
            }
            pool_down(c, POOL_MEDIA, ops);
        }
    }
    if host.is_some() {
        let c = after(c, &ops[start..]);
        teardown_upscaler(&c, &MEDIA_FEED, ops);
        if c.scl.ports[SCL_PORT_MEDIA as usize].enabled {
            ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_MEDIA });
        }
    }
}

/// HDMI's or USB's pool `id` goes or changes in its output's steps for `want`: that output going, its pool made anew
/// or of another count, USB's encoder changing or USB reading HDMI's frame.
fn pool_changes(c: &Current, want: &Desired, id: u32) -> bool {
    let Some(p) = c.pool(id) else { return true };
    match id {
        topo::POOL_HDMI => want.hdmi.as_ref().is_none_or(|h| c.hdmi.as_ref().map(|s| s.timing) != Some(h.timing) || !pool_same(p, &hdmi_pool(h, src_fps(want), &joiners(want, id)))),
        topo::POOL_USB => want.usb.as_ref().is_none_or(|u| usb_joins(want) || !usb_encoder_same(c, want) || !pool_same(p, &usb_pool(u, src_fps(want), &joiners(want, id)))),
        _ => true,
    }
}

/// An output's live picture is scaled up to its window by the frame-mode upscaler (SCL dev 1): its own port's, not one
/// whose frame another output makes.
fn upscaler_wanted(want: &Desired) -> bool {
    let up = |f: &OutputFrame, scl_out: Size| f.way != Way::Covered && scl_out != f.view.size();
    let player = |f: &OutputFrame, from: Input| f.way != Way::Covered && from == Input::Player;
    want.hdmi.as_ref().is_some_and(|h| up(&h.frame, h.scl_out) || player(&h.frame, h.from))
        || want.usb.as_ref().is_some_and(|u| !usb_joins(want) && (up(&u.frame, u.scl_out) || player(&u.frame, u.from)))
        || want.media.as_ref().is_some_and(|m| media_host(want).is_none() && up(&m.frame, m.scl_out))
}

/// The memory-to-memory scaler device goes once no output needs it ([`stretch_wanted`]) and no compositor uses it.
fn drop_idle_stretch(cur: &Current, want: &Desired, ops: &mut Vec<Op>) {
    if !stretch_wanted(want) && cur.scl_stretch.device && !cur.composing() {
        ops.push(Op::SclDestroyDevice { dev: topo::SCL_STRETCH_DEV });
    }
}

/// How the HDMI feed hands DISP its frames: the canvases of its pool, as an encoder's.
pub fn hdmi_way(h: &HdmiPlan) -> FeedWay {
    encoder_way(topo::POOL_HDMI, h.timing.size(), &h.frame)
}

/// HDMI's pool, read by DISP's input port 0 and by the encoders `joiners` whose frame is HDMI's: canvases of the
/// screen's size. Covered, its views go nowhere: it stays on SCL port 0.
fn hdmi_pool(h: &HdmiPlan, src_fps: u32, joiners: &[(ChnPort, u32)]) -> PoolState {
    use topo::*;
    let way = hdmi_way(h);
    let src = if covered(&way) { HDMI_FEED.out } else { feed_src(&HDMI_FEED, h.from, h.scl_out, h.frame.view.size()) };
    let size = h.timing.size();
    let rated: Vec<(ChnPort, u32)> = std::iter::once((DISP_IN, h.rate(src_fps))).chain(joiners.iter().copied()).collect();
    let n = canvases(&rated, src_fps, size, &way);
    PoolState { id: POOL_HDMI, src, readers: rated.iter().map(|r| r.0).collect(), size, rows: pool_rows(size.h), n }
}

/// HDMI's port path is built for `h` with its pool, DISP fed at the timing's rate (`src_fps` the sensor's), its canvas
/// count the one for its readers (`joiners` beside DISP). The joiners themselves are their outputs' steps.
fn hdmi_feed_same(cur: &Current, h: &HdmiPlan, src_fps: u32, joiners: &[(ChnPort, u32)]) -> bool {
    use topo::*;
    let pool = hdmi_pool(h, src_fps, joiners);
    let way = hdmi_way(h);
    let path = if covered(&way) {
        !cur.scl.ports[HDMI_FEED.port as usize].enabled && !cur.scl_of(SCL_UP_DEV, HDMI_FEED.up_chn).channel
    } else {
        feed_path_same(cur, &HDMI_FEED, h.from, h.crop, h.scl_out, h.frame.view.size(), src_fps)
    };
    let want = FeedState { src: pool.src, dst: DISP_IN, fps: h.rate(src_fps), way };
    path && cur.feed(DISP_IN) == Some(&want) && cur.pool(POOL_HDMI).is_some_and(|p| pool_same(p, &pool))
}

/// HDMI's feed and pool down, the pool while DISP still exists and after the port writing it (the rest of the path —
/// the other port, the upscaler — is the caller's).
fn teardown_hdmi_feed(cur: &Current, ops: &mut Vec<Op>) {
    if cur.feed(topo::DISP_IN).is_some() {
        ops.push(Op::FeedStop { dst: topo::DISP_IN });
    }
    pool_down(cur, topo::POOL_HDMI, ops);
}

/// The media chain: SCL port 1 (through its upscaler when the zoom asks for it) → the media encoder → the recorder; or,
/// when the recording's frame is another output's, the encoder a reader of that output's pool (its own way gone in
/// [`media_release`]). A new crop re-feeds the running encoder, so zooming does not cut a recording. A photo's encoder
/// gets no feed: the daemon hands it one frame of the port itself (`Hal::shoot`). The photo during a recording: its
/// JPEG encoder a reader of the pool the recording reads, which the compositor hands one canvas.
fn build_media(cur: &Current, want: &Desired, fresh: bool, ops: &mut Vec<Op>) {
    use topo::*;
    let encoder_same = !fresh && media_encoder_same(cur, want.media.as_ref());
    let Some(m) = want.media.as_ref() else {
        media_down(cur, fresh, ops);
        return;
    };
    let dev = venc_dev(m.codec);
    let venc = venc_in(dev, VENC_CHN_MEDIA);
    let host = media_host(want);
    let start = ops.len();
    // a new encoder device is served once a port's depth changes (as [`feed_queue`] does for an own feed): the port that
    // writes the pool it joins, as it is, before its first canvas
    let serve = |c: &Current, id: u32, ops: &mut Vec<Op>| {
        if let Some(p) = c.pool(id).filter(|p| c.port_size(p.src).is_some()) {
            feed_queue(p.src, FEED_USER_FRAMES, p.n, ops);
        }
    };
    match host {
        Some(id) => {
            if !encoder_same {
                ops.push(Op::VencUp { dev, chn: VENC_CHN_MEDIA, codec: m.codec, size: m.size, fps: m.fps, mjpeg_quality: m.quality, bitrate: m.bitrate, gop: m.gop, bound: None });
            }
            let c = after(cur, &ops[start..]);
            if !c.pool(id).is_some_and(|p| p.readers.contains(&venc)) {
                if !encoder_same {
                    serve(&c, id, ops);
                }
                ops.push(Op::PoolReader { id, reader: venc, fps: m.fps });
            }
        }
        None => {
            let pool = media_pool(m);
            let fed = !fresh && media_feed_same(cur, m);
            if !encoder_same {
                if !fresh {
                    teardown_media(cur, ops);
                }
            } else if !fed {
                if let Some(f) = cur.feed_into(VENC_CHN_MEDIA) {
                    ops.push(Op::FeedStop { dst: f.dst });
                }
                pool_down_if_moved(cur, &pool, ops);
            }
            if !encoder_same {
                ops.push(Op::VencUp { dev, chn: VENC_CHN_MEDIA, codec: m.codec, size: m.size, fps: m.fps, mjpeg_quality: m.quality, bitrate: m.bitrate, gop: m.gop, bound: None });
            }
            if !(encoder_same && fed) {
                let c = after(cur, &ops[start..]);
                let pool_op = pool_op(&c, &pool);
                let way = media_way(m);
                let src = if covered(&way) {
                    port_off(&c, &MEDIA_FEED, fresh, ops);
                    ops.extend(pool_op);
                    pool.src
                } else {
                    build_feed(&c, &MEDIA_FEED, Input::Sensor, m.crop, m.scl_out, m.frame.view.size(), m.src_fps, m.fps, fresh, pool.n, pool_op, ops)
                };
                // leaving another output's pool that stays: its own pool made beside it, the encoder switched to it
                // now that its port writes (the photo's encoder follows in the steps below)
                let c = after(cur, &ops[start..]);
                if let Some(id) = c.media_joined() {
                    if let Some(id) = c.snap_pool().filter(|p| *p == id) {
                        ops.push(Op::PoolUnreader { id, reader: venc_in(VENC_DEV_JPEG, VENC_CHN_SNAP) });
                    }
                    ops.push(Op::PoolUnreader { id, reader: venc });
                    ops.push(Op::PoolReader { id: POOL_MEDIA, reader: venc, fps: m.fps });
                }
                if m.shot() {
                    // room for the frame the daemon takes beside the port's own quota
                    if !covered(&way) {
                        feed_queue(src, 1, pool.n, ops);
                    }
                } else {
                    if !covered(&way) {
                        feed_queue(src, FEED_USER_FRAMES, pool.n, ops);
                    }
                    feed_queue(src, FEED_USER_FRAMES, pool.n, ops);
                    ops.push(Op::FeedStart { src, dst: venc, fps: m.fps, way });
                }
            }
        }
    }
    // a photo during the recording: its JPEG encoder a reader of the pool the recording reads
    let c = after(cur, &ops[start..]);
    let read = host.unwrap_or(POOL_MEDIA);
    let snap = venc_in(VENC_DEV_JPEG, VENC_CHN_SNAP);
    match (&c.snap, m.snapshot) {
        (None, true) => {
            ops.push(Op::VencUp { dev: VENC_DEV_JPEG, chn: VENC_CHN_SNAP, codec: Codec::Mjpeg, size: m.size, fps: m.fps, mjpeg_quality: crate::policy::PHOTO_QUALITY, bitrate: 0, gop: 0, bound: None });
            serve(&c, read, ops);
            ops.push(Op::PoolReader { id: read, reader: snap, fps: 0 });
        }
        (Some(_), true) if c.snap_pool() != Some(read) => {
            if let Some(id) = c.snap_pool() {
                ops.push(Op::PoolUnreader { id, reader: snap });
            }
            ops.push(Op::PoolReader { id: read, reader: snap, fps: 0 });
        }
        (Some(v), false) => {
            if let Some(id) = c.snap_pool() {
                ops.push(Op::PoolUnreader { id, reader: snap });
            }
            ops.push(Op::VencDown { dev: v.dev, chn: v.chn });
        }
        _ => {}
    }
    let recording = if encoder_same { cur.recording.as_ref() } else { None };
    if recording != m.file.as_ref() {
        if recording.is_some() {
            ops.push(Op::RecordStop);
        }
        if let Some(path) = &m.file {
            ops.push(Op::RecordStart { venc_dev: dev, venc_chn: VENC_CHN_MEDIA, codec: m.codec, path: path.clone(), size: m.size, fps: m.fps });
        }
    }
}

/// The media encoder runs with the wanted settings (or none is wanted and none runs).
fn media_encoder_same(cur: &Current, want: Option<&MediaPlan>) -> bool {
    cur.media.as_ref().map(|v| (v.dev, v.codec, v.size, v.fps, v.mjpeg_quality, v.bitrate, v.gop))
        == want.map(|m| (topo::venc_dev(m.codec), m.codec, m.size, m.fps, m.quality, m.bitrate, m.gop))
}

/// The media chain runs as wanted (or none is wanted and nothing of it is built): fed from its own port, or reading
/// the pool of the output whose frame it is with nothing of its own; the photo's encoder reading the same pool.
fn media_same(cur: &Current, want: &Desired) -> bool {
    use topo::*;
    if !media_encoder_same(cur, want.media.as_ref()) {
        return false;
    }
    match want.media.as_ref() {
        None => !cur.scl.ports[SCL_PORT_MEDIA as usize].enabled && !cur.scl_media_up.channel && cur.recording.is_none() && cur.snap.is_none(),
        Some(m) => {
            let host = media_host(want);
            let fed = match host {
                Some(id) => {
                    cur.media_joined() == Some(id)
                        && cur.feed_into(VENC_CHN_MEDIA).is_none()
                        && cur.pool(POOL_MEDIA).is_none()
                        && !cur.scl.ports[SCL_PORT_MEDIA as usize].enabled
                        && !cur.scl_media_up.channel
                }
                None => cur.media_joined().is_none() && media_feed_same(cur, m),
            };
            let snap = if m.snapshot { cur.snap_pool() == Some(host.unwrap_or(POOL_MEDIA)) } else { cur.snap.is_none() };
            fed && cur.recording == m.file && snap
        }
    }
}

/// How the feed hands frames to the media encoder (a photo's one canvas too): as USB's.
pub fn media_way(m: &MediaPlan) -> FeedWay {
    encoder_way(topo::POOL_MEDIA, m.size, &m.frame)
}

/// The media encoder's pool: a recording's as any feed's; a photo's one canvas — it takes one frame, the port may
/// skip every other one meanwhile (a pool of three beside USB 4K and HDMI 4K found no room in the media heap [R
/// 2026-10-30]).
fn media_pool(m: &MediaPlan) -> PoolState {
    use topo::*;
    let way = media_way(m);
    let src = if covered(&way) { MEDIA_FEED.out } else { feed_src(&MEDIA_FEED, Input::Sensor, m.scl_out, m.frame.view.size()) };
    let n = if m.shot() { 1 } else { canvases(&[(media_venc(m), m.fps)], m.src_fps, m.size, &way) };
    PoolState { id: POOL_MEDIA, src, readers: vec![media_venc(m)], size: m.size, rows: pool_rows(m.size.h), n }
}

/// The media port path is built for `m` with its pool, and the encoder gets its frames as `m` wants: fed for a
/// recording, not for a photo.
fn media_feed_same(cur: &Current, m: &MediaPlan) -> bool {
    use topo::*;
    let pool = media_pool(m);
    let way = media_way(m);
    let path = if covered(&way) {
        !cur.scl.ports[MEDIA_FEED.port as usize].enabled && !cur.scl_of(SCL_UP_DEV, MEDIA_FEED.up_chn).channel
    } else {
        feed_path_same(cur, &MEDIA_FEED, Input::Sensor, m.crop, m.scl_out, m.frame.view.size(), m.src_fps)
    };
    let want = (!m.shot()).then(|| FeedState { src: pool.src, dst: pool.readers[0], fps: m.fps, way });
    path && cur.feed_into(VENC_CHN_MEDIA) == want.as_ref() && cur.pool(POOL_MEDIA).is_some_and(|p| pool_same(p, &pool))
}

/// Recorder, the feed, the pool and the media encoder down, and a photo's encoder beside it; an encoder reading another
/// output's pool taken off it before it goes (SCL port 1 and its upscaler are the caller's).
/// The media chain down, nothing wanting it: its encoders, feed and pool ([`teardown_media`]), its upscaler and port.
fn media_down(cur: &Current, fresh: bool, ops: &mut Vec<Op>) {
    use topo::*;
    let start = ops.len();
    teardown_media(cur, ops);
    let cur = &after(cur, &ops[start..]);
    teardown_upscaler(cur, &MEDIA_FEED, ops);
    if cur.scl.ports[SCL_PORT_MEDIA as usize].enabled && !fresh {
        ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_MEDIA });
    }
}

fn teardown_media(cur: &Current, ops: &mut Vec<Op>) {
    if let Some(v) = &cur.snap {
        if let (Some(r), Some(id)) = (cur.snap_reader(), cur.snap_pool()) {
            ops.push(Op::PoolUnreader { id, reader: r });
        }
        ops.push(Op::VencDown { dev: v.dev, chn: v.chn });
    }
    if cur.recording.is_some() {
        ops.push(Op::RecordStop);
    }
    if let Some(f) = cur.feed_into(topo::VENC_CHN_MEDIA) {
        ops.push(Op::FeedStop { dst: f.dst });
    }
    pool_down(cur, topo::POOL_MEDIA, ops);
    if let (Some(r), Some(id)) = (cur.media_reader(), cur.media_joined()) {
        ops.push(Op::PoolUnreader { id, reader: r });
    }
    if let Some(v) = &cur.media {
        ops.push(Op::VencDown { dev: v.dev, chn: v.chn });
    }
}

/// The encoder already runs with the wanted codec, size, rate and quality (or none is wanted and none runs).
fn usb_encoder_same(cur: &Current, want: &Desired) -> bool {
    cur.venc.as_ref().map(|v| (v.codec, v.size, v.fps, v.mjpeg_quality, v.bound))
        == want.usb.as_ref().map(|u| (u.codec, u.size, u.fps, want.image.mjpeg_quality, usb_bound(u, want)))
}

/// The port bound into USB's encoder in a ring, when its frame is one (`FrameOf::Ring`).
fn usb_bound(u: &UsbPlan, want: &Desired) -> Option<Ring> {
    (u.frame_of == FrameOf::Ring).then_some(Ring { port: topo::USB_FEED.out, src_fps: want.sensor.mode_max_fps })
}

/// How the USB feed hands frames to the encoder for `u`: always the canvases of its pool.
fn usb_way(u: &UsbPlan) -> FeedWay {
    encoder_way(topo::POOL_USB, u.size, &u.frame)
}

/// The canvases of pool `pool` for an output's frames of `size` as `f` assembles them: the port writes its view, or
/// nothing when covered; every picture lies in the canvases, put back where the port wrote over it.
fn encoder_way(pool: u32, size: Size, f: &OutputFrame) -> FeedWay {
    let view = (f.way != Way::Covered).then_some(f.view);
    let visible = if view.is_some() { f.visible.clone() } else { Vec::new() };
    FeedWay::Canvas { pool, size, live: f.window, view, visible, windows: f.windows.clone() }
}

/// The USB encoder's pool for `u`, read by the encoders `joiners` too (the recording's, when its frame is USB's).
/// Covered, its views go nowhere: it stays on SCL port 2.
fn usb_pool(u: &UsbPlan, src_fps: u32, joiners: &[(ChnPort, u32)]) -> PoolState {
    use topo::*;
    let way = usb_way(u);
    let src = if covered(&way) { USB_FEED.out } else { feed_src(&USB_FEED, u.from, u.scl_out, u.frame.view.size()) };
    let rated: Vec<(ChnPort, u32)> = std::iter::once((venc_in(venc_dev(u.codec), VENC_CHN_USB), u.fps)).chain(joiners.iter().copied()).collect();
    let n = canvases(&rated, src_fps, u.size, &way);
    PoolState { id: POOL_USB, src, readers: rated.iter().map(|r| r.0).collect(), size: u.size, rows: pool_rows(u.size.h), n }
}

fn covered(way: &FeedWay) -> bool {
    matches!(way, FeedWay::Canvas { view: None, .. })
}

/// The canvases a pool of `size` frames needs: what its port holds ([`PORT_HOLDS`]), and what its readers hold — the port
/// writes `write_fps` canvases a second, each reader `(port, fps)` takes `fps` of them ([`crate::pace`]), and a canvas stays
/// with the readers that took it for the longest of their holds ([`hold_us`]); so the canvases held on average are the write rate times the
/// mean of that longest hold, each reader's paces taken as independent [I] — and one more while the feed puts pictures
/// back into each canvas the port wrote (the port must not wait for it). Covered: a front and a back; with DISP a third —
/// DISP keeps the front before the current one about two frame periods (a reader beside DISP takes the front again and
/// again).
fn canvases(readers: &[(ChnPort, u32)], write_fps: u32, size: Size, way: &FeedWay) -> u32 {
    let disp = readers.iter().any(|(r, _)| r.module == Module::Disp);
    if covered(way) {
        return 2 + disp as u32;
    }
    let w = write_fps.max(1) as f64;
    let mut by_hold: Vec<(f64, f64)> = readers.iter().map(|&(r, fps)| ((fps as f64 / w).min(1.0), hold_us(r, fps, size) as f64 / 1e6)).collect();
    by_hold.sort_by(|a, b| b.1.total_cmp(&a.1));
    // a canvas is held for a reader's hold when that reader took it and no reader holding longer did
    let (mut untaken, mut mean_s) = (1.0, 0.0);
    for (share, hold_s) in by_hold {
        mean_s += untaken * share * hold_s;
        untaken *= 1.0 - share;
    }
    PORT_HOLDS + (w * mean_s - 1e-6).ceil().max(0.0) as u32 + way.puts_back() as u32
}

/// The canvases the port holds: the one it writes and the next it is lent ahead.
const PORT_HOLDS: u32 = 2;

/// How long reader `r`, taking `fps` canvases a second, holds each of `size`: the 90th percentile of the holds measured on
/// the camera (`/proc/microcam_canvas_holds`, logs/s6-13). An encoder's from the row of the smallest measured size holding
/// the frame, else the largest's.
/// - DISP keeps a canvas until its next one has been scanned: three of its own periods, the most seen — 101 ms at 30 a
///   second beside USB MJPEG 4K45 (3.0 periods), 94 ms at 25 beside a 4K25 recording (2.3), 31 ms at 60 on 1440p60 (1.9)
///   [R 2026-11-14].
/// - JPEG ([`JPEG_HOLDS`]), H.26x ([`H26X_HOLDS`]).
fn hold_us(r: ChnPort, fps: u32, size: Size) -> u64 {
    let row = |rows: &[(Size, u64)]| rows.iter().find(|(s, _)| s.w >= size.w && s.h >= size.h).or(rows.last()).map_or(0, |r| r.1);
    match (r.module, r.dev) {
        (Module::Disp, _) => 3 * 1_000_000 / fps.max(1) as u64,
        (Module::Venc, topo::VENC_DEV_JPEG) => row(JPEG_HOLDS),
        _ => row(H26X_HOLDS),
    }
}

/// A JPEG encoder's hold of a canvas (µs) by frame size: 1080 5.2 ms (USB MJPEG 1080@120), 4K 20 ms (USB MJPEG 4K45,
/// 19.4–19.8) [R 2026-11-14].
const JPEG_HOLDS: &[(Size, u64)] = &[(Size { w: 1920, h: 1080 }, 5_200), (Size { w: 3840, h: 2160 }, 20_000)];

/// An H.26x encoder's hold of a canvas (µs) by frame size: 1080 8 ms (a 1080@100 recording, 7.9), 4K 57 ms (a 4K25
/// recording: 49 on its own pool, 57 beside USB's H.264 on the Wave5) [R 2026-11-14]; a 4K recording's first
/// frames 51–58 [R 2026-11-14, after the first GOP's QP floor].
const H26X_HOLDS: &[(Size, u64)] = &[(Size { w: 1920, h: 1080 }, 8_000), (Size { w: 3840, h: 2160 }, 57_000)];

/// The rows a pool allocates per plane for frames `h` high: the encoders read whole blocks — H.264 32 rows, JPEG 16 —
/// and drop (H.264) or never finish (JPEG) a frame whose buffer is shorter. DISP needs none of it; its pool is allocated alike (a 4K canvas 16 rows more).
fn pool_rows(h: u32) -> u32 {
    h.div_ceil(32) * 32
}

/// `src_fps` = the sensor's rate (above the host's while a recording sets it). `joined`: USB's frame is HDMI's — its
/// encoder reads HDMI's pool, nothing of its own path is built. `joiners`: the other encoders that read USB's pool.
fn usb_feed_same(cur: &Current, u: &UsbPlan, src_fps: u32, joined: bool, joiners: &[(ChnPort, u32)]) -> bool {
    use topo::*;
    if joined {
        return cur.usb_joined() && cur.feed_into(VENC_CHN_USB).is_none() && cur.pool(POOL_USB).is_none() && !cur.scl.ports[USB_FEED.port as usize].enabled && !cur.scl_up.channel;
    }
    if u.frame_of == FrameOf::Ring {
        let port = &cur.scl.ports[USB_FEED.port as usize];
        return !cur.usb_joined() && cur.feed_into(VENC_CHN_USB).is_none() && cur.pool(POOL_USB).is_none() && port.enabled && port.configured == Some((u.crop, u.size)) && !cur.scl_of(SCL_UP_DEV, USB_FEED.up_chn).channel;
    }
    let pool = usb_pool(u, src_fps, joiners);
    let way = usb_way(u);
    let path = if covered(&way) {
        !cur.scl.ports[USB_FEED.port as usize].enabled && !cur.scl_of(SCL_UP_DEV, USB_FEED.up_chn).channel
    } else {
        feed_path_same(cur, &USB_FEED, u.from, u.crop, u.scl_out, u.frame.view.size(), src_fps)
    };
    let want = FeedState { src: pool.src, dst: pool.readers[0], fps: u.fps, way };
    path && cur.feed_into(VENC_CHN_USB) == Some(&want) && cur.pool(POOL_USB).is_some_and(|p| pool_same(p, &pool))
}

/// The pool's step: made when there is none — handing to its own reader when that exists and reads no pool yet; the
/// readers joining it, each at its own rate, are added by their outputs' steps —, its count changed when that is all
/// that differs.
fn pool_op(cur: &Current, want: &PoolState) -> Option<Op> {
    match cur.pool(want.id) {
        None => {
            let readers = want.readers.first().copied().filter(|r| cur.reader_exists(*r) && cur.pool_read_by(*r).is_none()).into_iter().collect();
            Some(Op::PoolUp { id: want.id, src: want.src, readers, size: want.size, rows: want.rows, n: want.n })
        }
        Some(p) if p.n != want.n => Some(Op::PoolCount { id: want.id, n: want.n }),
        Some(_) => None,
    }
}

/// A pool as made is the one wanted: the port it lends to, its own reader (the first; one beside it — an encoder whose
/// frame this pool's output makes — is that output's step), its canvases' size and count.
fn pool_same(p: &PoolState, want: &PoolState) -> bool {
    (p.id, p.src, p.readers.first(), p.size, p.rows, p.n) == (want.id, want.src, want.readers.first(), want.size, want.rows, want.n)
}

/// The pool goes when it lends to another port, hands to another own reader or its canvases are of another size —
/// before that port's path changes (mi_sys checks the channel when an allocator is taken off).
fn pool_down_if_moved(cur: &Current, want: &PoolState, ops: &mut Vec<Op>) {
    if cur.pool(want.id).is_some_and(|p| (p.src, p.readers.first(), p.size, p.rows) != (want.src, want.readers.first(), want.size, want.rows)) {
        ops.push(Op::PoolDown { id: want.id });
    }
}

/// Covered: the feed's port and its upscaler off (nothing reads them).
fn port_off(cur: &Current, f: &topo::Feed, fresh: bool, ops: &mut Vec<Op>) {
    use topo::*;
    teardown_upscaler(cur, f, ops);
    if cur.scl.ports[f.port as usize].enabled && !fresh {
        ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: f.port });
    }
}

/// The port whose frames the feed hands on: the SCL port, or its upscaler when `scl_out` is smaller than `size` or the
/// player's decoder feeds it.
fn feed_src(f: &topo::Feed, from: Input, scl_out: Size, size: Size) -> ChnPort {
    if from == Input::Player || scl_out != size {
        f.up
    } else {
        f.out
    }
}

/// The feed's ports give the wanted crop: the SCL port at `scl_out`, and the upscaler behind it bound at `src_fps` when
/// `scl_out` is smaller than `size`; from the player, the SCL port idle and its upscaler channel bound to the decoder.
#[allow(clippy::too_many_arguments)]
fn feed_path_same(cur: &Current, f: &topo::Feed, from: Input, crop: Rect, scl_out: Size, size: Size, src_fps: u32) -> bool {
    use topo::*;
    if from == Input::Player {
        let up = cur.scl_of(SCL_UP_DEV, f.up_chn);
        let p = &up.ports[SCL_UP_PORT as usize];
        return !cur.scl.ports[f.port as usize].enabled
            && up.started
            && p.enabled
            && p.configured == Some((crop, size))
            && cur.bind(VDEC_OUT, f.up).is_some_and(|b| b.src_fps == src_fps)
            && cur.bind(f.out, f.up).is_none();
    }
    if cur.bind(VDEC_OUT, f.up).is_some() {
        return false;
    }
    let port = &cur.scl.ports[f.port as usize];
    let port_ok = port.enabled && port.configured == Some((crop, scl_out));
    let up_state = cur.scl_of(SCL_UP_DEV, f.up_chn);
    let path_ok = if scl_out != size {
        let up = &up_state.ports[SCL_UP_PORT as usize];
        up_state.started && up.enabled && up.configured == Some((Rect::full(scl_out), size)) && cur.bind(f.out, f.up).is_some_and(|b| b.src_fps == src_fps)
    } else {
        !up_state.channel
    };
    port_ok && path_ok
}

/// The feed's SCL port and, when the crop needs scaling up, the upscaler behind it; returns the port the feed takes
/// frames from (a quota of `buffers`). `pool` — the pool's step — goes after that port's channel exists and before its
/// first frame: right before it is enabled, or at the end when it stays enabled. The feed itself is already stopped.
#[allow(clippy::too_many_arguments)]
fn build_feed(cur: &Current, f: &topo::Feed, from: Input, crop: Rect, scl_out: Size, size: Size, src_fps: u32, fps: u32, fresh: bool, buffers: u32, mut pool: Option<Op>, ops: &mut Vec<Op>) -> ChnPort {
    use topo::*;
    if from == Input::Player {
        return player_feed(cur, f, crop, size, src_fps, fresh, buffers, pool, ops);
    }
    if cur.bind(VDEC_OUT, f.up).is_some() {
        ops.push(Op::Unbind { src: VDEC_OUT, dst: f.up });
    }
    let upscale = scl_out != size;
    let port = &cur.scl.ports[f.port as usize];
    let port_same = !fresh && port.enabled && port.configured == Some((crop, scl_out));
    let up_bound = cur.bind(f.out, f.up).is_some_and(|b| b.src_fps == src_fps);
    if (!port_same || !upscale || !up_bound) && cur.bind(f.out, f.up).is_some() {
        ops.push(Op::Unbind { src: f.out, dst: f.up });
    }
    if !upscale {
        teardown_upscaler_after_unbind(cur, f, ops);
    }
    if !port_same {
        if port.enabled && !fresh {
            ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: f.port });
        }
        ops.push(Op::SclConfigurePort { dev: SCL_DEV, chn: SCL_CHN, port: f.port, crop, size: scl_out });
        if upscale {
            ops.push(Op::SetDepth { port: f.out, user: 0, buffers: f.buffers });
            ops.push(Op::SclEnablePort { dev: SCL_DEV, chn: SCL_CHN, port: f.port });
        } else {
            ops.extend(pool.take());
            feed_port_up(f.out, buffers, ops);
        }
    }
    if !upscale {
        ops.extend(pool);
        return f.out;
    }
    let up = cur.scl_of(SCL_UP_DEV, f.up_chn);
    if !up.device {
        ops.push(Op::SclCreateDevice { dev: SCL_UP_DEV, hw_mask: SCL_UP_HW_MASK });
    }
    if !up.channel {
        ops.push(Op::SclCreateChannel { dev: SCL_UP_DEV, chn: f.up_chn });
        ops.push(Op::SclStartChannel { dev: SCL_UP_DEV, chn: f.up_chn });
    }
    let up_port = &up.ports[SCL_UP_PORT as usize];
    let want_port = (Rect::full(scl_out), size);
    if !(up.channel && up_port.enabled && up_port.configured == Some(want_port)) {
        if up.channel && up_port.enabled {
            ops.push(Op::SclDisablePort { dev: SCL_UP_DEV, chn: f.up_chn, port: SCL_UP_PORT });
        }
        ops.push(Op::SclConfigurePort { dev: SCL_UP_DEV, chn: f.up_chn, port: SCL_UP_PORT, crop: want_port.0, size: want_port.1 });
        ops.extend(pool.take());
        feed_port_up(f.up, buffers, ops);
    }
    ops.extend(pool);
    if !(port_same && up_bound) {
        ops.push(Op::Bind { src: f.out, dst: f.up, kind: BindKind::FrameBase, src_fps, dst_fps: fps });
    }
    f.up
}

/// The player's way into an output's frame: SCL 0's port of the output idle, the output's channel of SCL dev 1
/// bound to the decoder's port, its port writing `crop` of the decoded picture scaled to `size` into the pool's views.
#[allow(clippy::too_many_arguments)]
fn player_feed(cur: &Current, f: &topo::Feed, crop: Rect, size: Size, src_fps: u32, fresh: bool, buffers: u32, mut pool: Option<Op>, ops: &mut Vec<Op>) -> ChnPort {
    use topo::*;
    if cur.bind(f.out, f.up).is_some() {
        ops.push(Op::Unbind { src: f.out, dst: f.up });
    }
    if cur.scl.ports[f.port as usize].enabled && !fresh {
        ops.push(Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: f.port });
    }
    let up = cur.scl_of(SCL_UP_DEV, f.up_chn);
    if !up.device {
        ops.push(Op::SclCreateDevice { dev: SCL_UP_DEV, hw_mask: SCL_UP_HW_MASK });
    }
    if !up.channel {
        ops.push(Op::SclCreateChannel { dev: SCL_UP_DEV, chn: f.up_chn });
        ops.push(Op::SclStartChannel { dev: SCL_UP_DEV, chn: f.up_chn });
    }
    let p = &up.ports[SCL_UP_PORT as usize];
    if !(up.channel && p.enabled && p.configured == Some((crop, size))) {
        if up.channel && p.enabled {
            ops.push(Op::SclDisablePort { dev: SCL_UP_DEV, chn: f.up_chn, port: SCL_UP_PORT });
        }
        ops.push(Op::SclConfigurePort { dev: SCL_UP_DEV, chn: f.up_chn, port: SCL_UP_PORT, crop, size });
        ops.extend(pool.take());
        feed_port_up(f.up, buffers, ops);
    }
    ops.extend(pool);
    let bound = cur.bind(VDEC_OUT, f.up);
    if bound.is_none_or(|b| b.src_fps != src_fps) {
        if bound.is_some() {
            ops.push(Op::Unbind { src: VDEC_OUT, dst: f.up });
        }
        ops.push(Op::Bind { src: VDEC_OUT, dst: f.up, kind: BindKind::FrameBase, src_fps, dst_fps: src_fps });
    }
    f.up
}

/// A configured port the feed takes from: its quota (the views its pool lends), enabled.
fn feed_port_up(port: ChnPort, buffers: u32, ops: &mut Vec<Op>) {
    ops.push(Op::SetDepth { port, user: 0, buffers });
    ops.push(Op::SclEnablePort { dev: port.dev, chn: port.chn, port: port.port });
}

/// `user` frames of the feed's port queued for microcamd, set once the encoder exists and before its first frame: the
/// depth change is what makes mi_sys work out again which device passes its workers serve, and an encoder device
/// registered since it last did (its first channel created) leaves every frame handed in by hand queued, unread
///.
fn feed_queue(port: ChnPort, user: u32, buffers: u32, ops: &mut Vec<Op>) {
    ops.push(Op::SetDepth { port, user, buffers });
}

/// The encoder on channel `chn` taken down with what feeds and drains it, nothing else: the next plan builds it again as
/// a step of its own output. The way out of an encoder that lost a frame handed to it, or was lost itself.
pub fn teardown_encoder(cur: &Current, chn: u32) -> Vec<Op> {
    let mut ops = Vec::new();
    if chn == topo::VENC_CHN_MEDIA {
        teardown_media(cur, &mut ops);
    } else {
        teardown_usb_encoder(cur, &mut ops);
    }
    ops
}

/// Feed, pump, pool and encoder down; an encoder reading HDMI's pool taken off it before it goes (mi_sys BUGs on an
/// allocator of a destroyed channel).
fn teardown_usb_encoder(cur: &Current, ops: &mut Vec<Op>) {
    if let Some(f) = cur.feed_into(topo::VENC_CHN_USB) {
        ops.push(Op::FeedStop { dst: f.dst });
    }
    if cur.pump.is_some() {
        ops.push(Op::PumpStop);
    }
    pool_down(cur, topo::POOL_USB, ops);
    if let Some(r) = cur.usb_reader().filter(|_| cur.usb_joined()) {
        ops.push(Op::PoolUnreader { id: topo::POOL_HDMI, reader: r });
    }
    if let Some(v) = &cur.venc {
        // a ring's port goes down inside it (the HAL: encoder stopped, port off, unbound, destroyed — the port left
        // writing unbound makes SCL fail the whole channel's frame); its next user brings it up afresh
        ops.push(Op::VencDown { dev: v.dev, chn: v.chn });
    }
}

/// The feed's upscaler channel gone, with its input bind (the feed is the caller's; the device goes in
/// [`drop_idle_upscaler`] once no channel is left).
fn teardown_upscaler(cur: &Current, f: &topo::Feed, ops: &mut Vec<Op>) {
    for b in cur.binds.iter().filter(|b| b.dst == f.up) {
        ops.push(Op::Unbind { src: b.src, dst: b.dst });
    }
    teardown_upscaler_after_unbind(cur, f, ops);
}

fn teardown_upscaler_after_unbind(cur: &Current, f: &topo::Feed, ops: &mut Vec<Op>) {
    use topo::*;
    let up = cur.scl_of(SCL_UP_DEV, f.up_chn);
    if up.ports[SCL_UP_PORT as usize].enabled {
        ops.push(Op::SclDisablePort { dev: SCL_UP_DEV, chn: f.up_chn, port: SCL_UP_PORT });
    }
    if up.started {
        ops.push(Op::SclStopChannel { dev: SCL_UP_DEV, chn: f.up_chn });
    }
    if up.channel {
        ops.push(Op::SclDestroyChannel { dev: SCL_UP_DEV, chn: f.up_chn });
    }
}

/// `cur` with `ops` executed (bookkeeping only).
/// Pool `id` down, the port writing it disabled first: an enabled port whose pool is gone takes buffers of its own, up
/// to its quota (5 at 4K: more than an encoder's pool of 4), which the guard prices at that step.
fn pool_down(c: &Current, id: u32, ops: &mut Vec<Op>) {
    if let Some(p) = c.pool(id) {
        if c.port_size(p.src).is_some() {
            ops.push(Op::SclDisablePort { dev: p.src.dev, chn: p.src.chn, port: p.src.port });
        }
        ops.push(Op::PoolDown { id });
    }
}

fn after(cur: &Current, ops: &[Op]) -> Current {
    let mut c = cur.clone();
    for op in ops {
        c.apply(op);
    }
    c
}

/// The upscaler device goes when the planned operations leave it without a channel.
fn drop_idle_upscaler(cur: &Current, ops: &mut Vec<Op>) {
    if cur.scl_up.device && !cur.scl_any_channel(topo::SCL_UP_DEV) {
        ops.push(Op::SclDestroyDevice { dev: topo::SCL_UP_DEV });
    }
}

// ---- execution ------------------------------------------------------------------------------------------------

/// The hardware boundary the executor talks to. The daemon's implementation calls MI; the tests record.
pub trait Hal {
    fn exec(&mut self, op: &Op) -> Result<(), HalError>;
    /// Frames the capture has received so far (any counter that moves while the sensor delivers), None if unknown.
    fn frames(&mut self) -> Option<u64> {
        None
    }
    /// The encoder on channel `chn` stopped taking or handing out the frames its feed gives it: only building it anew
    /// brings its output back.
    fn encoder_lost(&mut self, _chn: u32) -> bool {
        false
    }
    /// For `reader` of pool `pool`, as the canvas module counts them since the reader was added: the hand-overs' times
    /// after the reader's last release summed in µs, their count, and the last release (ns, the kernel's monotonic
    /// clock) — DISP's: the display's phase and its pass ([`crate::lock`]).
    fn after_release(&mut self, _pool: u32, _reader: ChnPort) -> Option<(u64, u64, u64)> {
        None
    }
    /// Whether the feed into `disp` hands DISP each canvas when the port is lent it, before the write
    /// ([`crate::lock::early`]); a new feed starts without.
    fn early(&mut self, _disp: ChnPort, _on: bool) {}
    /// The sensor's frame length in lines in effect, None if unknown.
    fn frame_lines(&mut self) -> Option<u32> {
        None
    }
    /// The sensor's frame length from the next frame on (the display lock's trim; a new rate's `fps_live` replaces it).
    fn set_frame_lines(&mut self, _lines: u32) -> Result<(), HalError> {
        Ok(())
    }
    /// What the auto exposure runs right now (for locking it), None if unknown.
    fn exposure_now(&mut self) -> Option<tuning::ExposureNow> {
        None
    }
    /// What the auto white balance runs right now (for locking it, or switching it off), None if unknown.
    fn white_balance_now(&mut self) -> Option<tuning::WhiteBalanceNow> {
        None
    }
    /// A photo: the media encoder `dev`/`chn` (fed by nothing) gets one canvas of its pool assembled as `way` says — the
    /// menu as the `ui` baked it in patch set `seq` or later, and the live picture of the first frame of `src` from the
    /// port's overlay frame `from` on, which carries the photo's menu, or (None) the newest — and the encoded frame is
    /// taken out, waited for up to `timeout_ms`.
    #[allow(clippy::too_many_arguments)]
    fn shoot(&mut self, src: ChnPort, dev: u32, chn: u32, codec: Codec, fps: u32, way: &FeedWay, from: Option<u64>, seq: u64, timeout_ms: u32) -> Result<Vec<u8>, HalError> {
        let _ = (src, codec, fps, way, from, seq, timeout_ms);
        Err(HalError { op: format!("shoot {dev}/{chn}"), detail: "not supported".into() })
    }
    /// A photo during a recording: the compositor of the feed into `feed` — the one handing the canvases of the pool the
    /// recording reads — hands the JPEG encoder `snap`, a reader of that pool, the first finished canvas programmed at
    /// microcam_rgn's frame `from` of `src` or later (None: the next) and holding patch set `seq` or later; its JPEG, waited
    /// for up to `timeout_ms`.
    #[allow(clippy::too_many_arguments)]
    fn snapshot(&mut self, src: ChnPort, feed: ChnPort, snap: ChnPort, from: Option<u64>, seq: u64, timeout_ms: u32) -> Result<Vec<u8>, HalError> {
        let _ = (src, feed, from, seq, timeout_ms);
        Err(HalError { op: format!("snapshot {}/{}", snap.dev, snap.chn), detail: "not supported".into() })
    }
    /// The newest frame of `src` (bound to nothing, `size`) copied by the memory-to-memory scaler into the picture
    /// `into` (a camera resource), waited for up to `timeout_ms`; the scaler's device is made for it and taken down
    /// again unless it is up already (`stretch_up`, [`Current::scl_stretch`]).
    fn grab(&mut self, src: ChnPort, size: Size, into: PictureId, stretch_up: bool, timeout_ms: u32) -> Result<(), HalError> {
        let _ = (src, size, stretch_up, timeout_ms);
        Err(HalError { op: format!("grab {}", into.0), detail: "not supported".into() })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{op}: {detail}")]
pub struct HalError {
    pub op: String,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
#[error("step {index} ({op:?}) failed: {error}")]
pub struct ApplyError {
    pub index: usize,
    pub op: Op,
    pub error: HalError,
}

/// Run the operations in order, recording each success into `cur`. Stops at the first failure — but a picture knob's,
/// which is recorded refused and the steps go on —; `cur` then describes exactly the partial state, and
/// `teardown_capture(cur)` is the rollback.
pub fn apply(hal: &mut dyn Hal, cur: &mut Current, want: &Desired, ops: &[Op]) -> Result<(), ApplyError> {
    apply_with(hal, cur, want, ops, &mut |_, _| Ok(()))
}

/// [`apply`], with `before` called right before each step runs; its error stops the transition there, the step not run.
pub fn apply_with(hal: &mut dyn Hal, cur: &mut Current, want: &Desired, ops: &[Op], before: &mut dyn FnMut(&mut dyn Hal, &Op) -> Result<(), HalError>) -> Result<(), ApplyError> {
    cur.desired = None;
    for (i, op) in ops.iter().enumerate() {
        if let Err(why) = cur.precondition(op) {
            return Err(ApplyError { index: i, op: op.clone(), error: HalError { op: format!("{op:?}"), detail: format!("precondition: {why}") } });
        }
        before(hal, op).map_err(|e| ApplyError { index: i, op: op.clone(), error: e })?;
        match (hal.exec(op), op) {
            (Ok(()), _) => cur.apply(op),
            (Err(e), Op::IspAdjust { adjust, .. }) => cur.isp.refused.extend(adjust.knobs().into_iter().map(|(k, v)| (k, (v, e.detail.clone())))),
            (Err(e), _) => return Err(ApplyError { index: i, op: op.clone(), error: e }),
        }
    }
    cur.desired = Some(want.clone());
    Ok(())
}

/// A `Hal` that records every operation and can be told to fail at a given step (tests, dry runs).
#[derive(Default)]
pub struct TraceHal {
    pub calls: Vec<Op>,
    pub fail_at: Option<usize>,
}

impl Hal for TraceHal {
    fn exec(&mut self, op: &Op) -> Result<(), HalError> {
        if self.fail_at == Some(self.calls.len()) {
            return Err(HalError { op: format!("{op:?}"), detail: "injected failure".into() });
        }
        self.calls.push(op.clone());
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::topo::*;
    use super::*;
    use crate::desired::{Desired, Epoch, Revision};
    use crate::formats::tests::{table as formats, H264, MJPEG};
    use crate::hdmi::{Monitor, MonitorCaps};
    use crate::modes::tests::table as modes;
    use crate::policy::tests::{hdmi_first, policy};
    use crate::policy::{BootPreset, Frames, Inputs, Layout, Prefs, ScreenAspect, View};

    /// The encoder gets the whole live frame through its pool, nothing put back.
    fn live_way(w: &Option<FeedWay>) -> bool {
        matches!(w, Some(FeedWay::Canvas { windows, view: Some(v), .. }) if windows.is_empty() && v.x == 0 && v.y == 0)
    }

    /// A transition failing at any of its steps is undone by planning back to the state it started from: the way back
    /// leaves exactly that state (the H.26x device aside, which stays once made). Here an idle 4K HDMI to a USB H.264
    /// 1080p stream beside a 1080p recording — sensor, both encoders, pools, feeds.
    #[test]
    fn a_failure_at_any_step_goes_back_to_the_state_before_the_transition() {
        let mut start = Current::default();
        let idle = want(&philips(), None, 1);
        run(&mut start, &idle);
        let f = formats();
        let h = f.resolve(H264, 4, 333_333).unwrap();
        let prefs = Prefs { recording: Some(crate::policy::Recording { size: Size::new(1920, 1080), path: "v".into() }), ..hdmi_first() };
        let wish = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(2)).unwrap();
        let ops = plan(&start, &wish, &modes()).unwrap().ops;
        assert!(ops.len() > 10);
        for k in 0..ops.len() {
            let mut cur = start.clone();
            let e = apply(&mut TraceHal { fail_at: Some(k), ..Default::default() }, &mut cur, &wish, &ops).unwrap_err();
            assert_eq!(e.index, k);
            let back = plan(&cur, &idle, &modes()).unwrap();
            apply(&mut TraceHal::default(), &mut cur, &idle, &back.ops).unwrap();
            cur.codec_device = start.codec_device;
            // a port the way back turned off keeps its last setting, which nothing reads until it is configured again
            for (p, s) in cur.scl.ports.iter_mut().zip(&start.scl.ports) {
                if !p.enabled && !s.enabled {
                    p.configured = s.configured;
                }
            }
            assert_eq!(cur, start, "failed at step {k} ({:?}); back: {:?}", ops[k], back.ops);
        }
    }

    pub(crate) fn philips() -> Monitor {
        Monitor::Present(MonitorCaps { modes: vec![(Size::new(3840, 2160), 30), (Size::new(1920, 1080), 60)] })
    }
    fn fhd() -> Monitor {
        Monitor::Present(MonitorCaps { modes: vec![(Size::new(1920, 1080), 60)] })
    }

    fn want(monitor: &Monitor, host: Option<(u8, u8, u32, u64)>, rev: u64) -> Desired {
        let f = formats();
        let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
        policy()
            .plan(&Inputs { monitor, host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &hdmi_first(), tuning: &Default::default() }, &modes(), Revision(rev))
            .unwrap()
    }

    fn run(cur: &mut Current, d: &Desired) -> Plan {
        let p = plan(cur, d, &modes()).unwrap();
        let mut hal = TraceHal::default();
        apply(&mut hal, cur, d, &p.ops).unwrap();
        p
    }

    fn pos(ops: &[Op], pred: impl Fn(&Op) -> bool) -> usize {
        ops.iter().position(pred).unwrap_or_else(|| panic!("op not found in {ops:?}"))
    }

    #[test]
    fn cold_start_builds_in_the_sdk_order() {
        let mut cur = Current::default();
        let d = want(&philips(), None, 1);
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::Rebuild);
        let o = &p.ops;
        let sensor = pos(o, |x| matches!(x, Op::SensorEnable { .. }));
        let vif = pos(o, |x| matches!(x, Op::VifEnablePort { .. }));
        let isp = pos(o, |x| matches!(x, Op::IspStartChannel { .. }));
        let b1 = pos(o, |x| matches!(x, Op::Bind { src, dst, .. } if *src == VIF_OUT && *dst == ISP_IN));
        let scl = pos(o, |x| matches!(x, Op::SclStartChannel { .. }));
        let b2 = pos(o, |x| matches!(x, Op::Bind { src, dst, .. } if *src == ISP_OUT && *dst == SCL_IN));
        let iq = pos(o, |x| matches!(x, Op::IspLoadIq { .. }));
        let orient = pos(o, |x| matches!(x, Op::SensorOrientation { .. }));
        let clk = pos(o, |x| matches!(x, Op::SclClock { hz: 480_000_000 }));
        let p0 = pos(o, |x| matches!(x, Op::SclEnablePort { port: 0, .. }));
        let up = pos(o, |x| matches!(x, Op::HdmiDispUp { .. }));
        let pool = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. }));
        let b3 = pos(o, |x| matches!(x, Op::FeedStart { dst, .. } if *dst == DISP_IN));
        assert!(sensor < vif && vif < isp && isp < b1 && b1 < scl && scl < b2 && b2 < iq && iq < orient);
        // DISP first (the pool's reader), the pool before the port's first frame, the feed last
        assert!(orient < clk && clk < up && up < pool && pool < p0 && p0 < b3, "{o:?}");
        assert_eq!(o[pool], Op::PoolUp { id: POOL_HDMI, src: SCL_OUT_HDMI, readers: vec![DISP_IN], size: Size::new(3840, 2160), rows: 2176, n: 5 });
        assert!(matches!(&o[b3], Op::FeedStart { src: SCL_OUT_HDMI, fps: 30, way: FeedWay::Canvas { pool: POOL_HDMI, view: Some(v), windows, .. }, .. } if *v == Rect::full(Size::new(3840, 2160)) && windows.is_empty()));
        assert!(!o.iter().any(|x| matches!(x, Op::Bind { dst, .. } if dst.module == Module::Disp)), "nothing is bound into DISP");
        assert!(!o.iter().any(|x| matches!(x, Op::VencUp { .. } | Op::PumpStart { .. })), "no USB wanted");
        assert_eq!(cur.hdmi.as_ref().unwrap().timing, Timing::P2160_30);
        assert_eq!(cur.desired.as_ref(), Some(&d));
    }

    #[test]
    fn a_mode_switch_restarts_the_sensor_on_the_running_chain_and_keeps_hdmi_up() {
        let mut cur = Current::default();
        run(&mut cur, &want(&fhd(), None, 1)); // 1080p60 monitor, idle: 4K45 preset
        let d = want(&fhd(), Some((MJPEG, 4, 83_333, 7)), 2); // USB 1080p120 → mode 24: a switch
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::Switch);
        let o = &p.ops;
        let feed_off = pos(o, |x| matches!(x, Op::FeedStop { dst } if *dst == DISP_IN));
        let p0_off = pos(o, |x| matches!(x, Op::SclDisablePort { port: 0, .. }));
        let vif_off = pos(o, |x| matches!(x, Op::VifDisableDev { .. }));
        let snr_off = pos(o, |x| matches!(x, Op::SensorDisable { .. }));
        let snr_on = pos(o, |x| matches!(x, Op::SensorSetRes { .. }));
        let vif_on = pos(o, |x| matches!(x, Op::VifEnablePort { .. }));
        // the feed before its port, every port before the sensor stops; the VIF→ISP bind kept (a realtime bind's rate
        // does not limit the frames)
        assert!(feed_off < p0_off && p0_off < vif_off && vif_off < snr_off && snr_off < snr_on && snr_on < vif_on, "{o:?}");
        assert!(!o.iter().any(|x| matches!(x, Op::Unbind { src, .. } | Op::Bind { src, .. } if *src == VIF_OUT)), "{o:?}");
        // the ISP, the scalers, the VIF group, the IQ and DISP/HDMI are kept
        assert!(!o.iter().any(|x| matches!(x, Op::IspDestroyDevice { .. } | Op::IspStopChannel { .. } | Op::IspLoadIq { .. } | Op::VifDestroyGroup { .. }
            | Op::SclStopChannel { .. } | Op::SclDestroyDevice { .. } | Op::HdmiDispDown)), "{o:?}");
        // HDMI's port cut anew in the new mode's coordinates, its pool kept (its canvases' size has not changed)
        assert!(!o.iter().any(|x| matches!(x, Op::PoolDown { id: POOL_HDMI } | Op::PoolUp { id: POOL_HDMI, .. })), "{o:?}");
        let hdmi_cut = pos(o, |x| matches!(x, Op::SclConfigurePort { port: 0, .. }));
        assert!(snr_on < hdmi_cut && cur.pool(POOL_HDMI).is_some_and(|p| p.readers.first() == Some(&DISP_IN)) && cur.feed(DISP_IN).is_some());
        // after the switch: USB's 1080p120 shows HDMI's 1080p60 picture, so its encoder reads HDMI's pool at 120 a
        // second while DISP takes 60 — no port, pool or feed of its own
        let jpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        let venc = pos(o, |x| matches!(x, Op::VencUp { fps: 120, .. }));
        // a depth set on HDMI's port once the encoder exists: that is what makes mi_sys serve the new encoder device
        let reader = pos(o, |x| *x == Op::PoolReader { id: POOL_HDMI, reader: jpeg, fps: 120 });
        let pump = pos(o, |x| matches!(x, Op::PumpStart { epoch: Epoch(7), .. }));
        assert!(snr_on < venc && venc < reader && reader < pump, "{o:?}");
        assert!(o[venc..reader].iter().any(|x| matches!(x, Op::SetDepth { port, user: FEED_USER_FRAMES, .. } if *port == SCL_OUT_HDMI)), "{o:?}");
        assert!(!o.iter().any(|x| matches!(x, Op::SclEnablePort { port: 2, .. } | Op::PoolUp { id: POOL_USB, .. })), "{o:?}");
        assert!(!o.iter().any(|x| matches!(x, Op::Bind { dst, .. } if dst.module == Module::Venc)), "nothing is bound into an encoder");
        assert_eq!(cur.sensor.as_ref().unwrap().mode, "1920x1080@121 rk40 bin hts278");
        assert_eq!(cur.sensor.as_ref().unwrap().fps_live, 120);
        assert_eq!(cur.pump, Some(Epoch(7)));
        // one pool, DISP its own reader at the timing's rate
        assert!(cur.pool(POOL_USB).is_none());
        assert_eq!(cur.pool(POOL_HDMI).map(|p| p.readers.clone()), Some(vec![DISP_IN, jpeg]));
        assert_eq!(cur.feed(DISP_IN).map(|f| f.fps), Some(60));
    }

    #[test]
    fn fps_only_change_touches_no_resource() {
        let mut cur = Current::default();
        run(&mut cur, &want(&fhd(), Some((MJPEG, 4, 83_333, 1)), 1)); // 1080p @120
        let d = want(&fhd(), Some((MJPEG, 4, 166_666, 1)), 2); // same stream, 60 fps
        // the policy gives a new VENC fps too, so this is Outputs (fps + usb encoder rate); check nothing capture-side moves
        let p = plan(&cur, &d, &modes()).unwrap();
        assert!(matches!(p.rung, Rung::Outputs | Rung::FpsOnly));
        assert!(p.ops.iter().any(|x| matches!(x, Op::SensorFpsLive { fps: 60 })));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::SensorDisable { .. } | Op::SclDestroyDevice { .. } | Op::IspDestroyDevice { .. } | Op::HdmiDispDown)));
    }

    #[test]
    fn stream_stop_and_restart_is_a_pump_restart_only() {
        let mut cur = Current::default();
        run(&mut cur, &want(&fhd(), Some((MJPEG, 4, 83_333, 1)), 1));
        // host stops: policy goes to the preset (a rebuild, mode 29) — but a pure epoch change with the same stream:
        let d = want(&fhd(), Some((MJPEG, 4, 83_333, 2)), 2);
        let p = plan(&cur, &d, &modes()).unwrap();
        assert_eq!(p.rung, Rung::PumpOnly);
        assert_eq!(p.ops, vec![Op::PumpStop, Op::PumpStart { epoch: Epoch(2), venc_dev: VENC_DEV_JPEG, venc_chn: 0 }]);
    }

    #[test]
    fn usb_size_change_in_the_same_mode_rebuilds_only_the_encoder() {
        let mut cur = Current::default();
        run(&mut cur, &want(&philips(), Some((MJPEG, 6, 333_333, 1)), 1)); // 4K monitor: mode 22 for everything; 3840x1440
        let d = want(&philips(), Some((MJPEG, 2, 333_333, 2)), 2); // 3840x1600 at the same rate → still mode 22, centre crop
        let p = plan(&cur, &d, &modes()).unwrap();
        assert_eq!(p.rung, Rung::UsbOnly);
        let o = &p.ops;
        assert_eq!(o[..2], [Op::FeedStop { dst: venc_in(VENC_DEV_JPEG, VENC_CHN_USB) }, Op::PumpStop], "feed and pump quiet before the encoder is touched");
        let down = pos(o, |x| matches!(x, Op::VencDown { .. }));
        let off = pos(o, |x| matches!(x, Op::SclDisablePort { port: 2, .. }));
        let cfg = pos(o, |x| matches!(x, Op::SclConfigurePort { port: 2, crop, .. } if *crop == Rect { x: 0, y: 280, w: 3840, h: 1600 }));
        let up = pos(o, |x| matches!(x, Op::VencUp { .. }));
        let pool = pos(o, |x| matches!(x, Op::PoolUp { .. }));
        let on = pos(o, |x| matches!(x, Op::SclEnablePort { port: 2, .. }));
        // the port before its pool goes; the encoder before the new pool: the pool puts an allocator on its input, then
        // on the port before its first frame
        assert!(off < down && down < up && up < cfg && cfg < pool && pool < on, "{o:?}");
        assert!(!o.iter().any(|x| matches!(x, Op::SensorSetRes { .. } | Op::IspStopChannel { .. } | Op::HdmiDispDown)));
    }

    #[test]
    fn a_photo_beside_a_usb_stream_touches_only_scl_port_1_and_the_media_encoder() {
        let at = |photo, zoom, rev| {
            let prefs = Prefs { photo, view: View { aspect: ScreenAspect::Four3, zoom }, ..hdmi_first() };
            let f = formats();
            let h = f.resolve(MJPEG, 1, 222_222).unwrap();
            policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        run(&mut cur, &at(None, 0, 1));
        let d = at(Some(Size::new(3840, 2160)), 0, 2);
        let m = d.media.clone().unwrap();
        assert_eq!(m.crop, Rect::full(Size::new(3840, 2160)), "the display aspect stays off the photo, as off USB");
        assert_eq!(m.size, Size::new(3840, 2160));
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::MediaOnly);
        assert!(p.ops.iter().all(|o| match o {
            Op::SclConfigurePort { port, .. } | Op::SclEnablePort { port, .. } => *port == topo::SCL_PORT_MEDIA,
            Op::VencUp { chn, .. } => *chn == topo::VENC_CHN_MEDIA,
            // the daemon hands the frame in itself, the menu laid in by the port's overlay: no feed
            Op::SetDepth { port, .. } => *port == topo::SCL_OUT_MEDIA,
            // the photo's frame goes to the encoder through the media pool
            Op::PoolUp { id, src, .. } => *id == topo::POOL_MEDIA && *src == topo::SCL_OUT_MEDIA,
            _ => false,
        }), "{:?}", p.ops);
        assert!(cur.feed_into(topo::VENC_CHN_MEDIA).is_none() && cur.pool(topo::POOL_MEDIA).is_some_and(|p| p.src == topo::SCL_OUT_MEDIA));
        // zoomed ×2 and asked for 1080p: scaled down into the box
        let z = at(Some(Size::new(1920, 1080)), 10, 3);
        assert_eq!(z.media.as_ref().unwrap().size, Size::new(1920, 1080));
        // a rebuild or rollback while the photo chain exists tears it down before the SCL goes
        let mut o = Vec::new();
        teardown_capture(&cur, &mut o);
        let down = pos(&o, |x| matches!(x, Op::VencDown { chn: topo::VENC_CHN_MEDIA, .. }));
        let off = pos(&o, |x| matches!(x, Op::SclDisablePort { port: topo::SCL_PORT_MEDIA, .. }));
        let destroy = pos(&o, |x| matches!(x, Op::SclDestroyChannel { dev: topo::SCL_DEV, .. }));
        let pool = pos(&o, |x| matches!(x, Op::PoolDown { id: topo::POOL_MEDIA }));
        assert!(off < pool && pool < down && down < destroy, "the port before its pool: {o:?}");
        // done: back to no media, nothing else touched
        let p = run(&mut cur, &at(None, 0, 5));
        assert_eq!(p.rung, Rung::MediaOnly);
        assert!(cur.media.is_none() && !cur.scl.ports[topo::SCL_PORT_MEDIA as usize].enabled);
        assert!(cur.venc.is_some(), "the USB encoder keeps running");
        assert!(cur.scl_stretch.device, "USB's frame keeps the memory-to-memory scaler (its menu is baked with it)");
    }

    /// A 4K recording on a 1080p monitor, its frame apart from the outputs' (a scene shown at three levels: HDMI 3, USB
    /// 2, the recording 0); USB above HDMI in the precedence, so a host stream runs and HDMI pauses (three distinct frames).
    fn recording(zoom: u8, host: Option<(u8, u8, u32, u64)>, rev: u64) -> Desired {
        recording_with(crate::precedence::Precedence::default(), zoom, host, rev)
    }

    fn recording_with(precedence: crate::precedence::Precedence, zoom: u8, host: Option<(u8, u8, u32, u64)>, rev: u64) -> Desired {
        let prefs = Prefs {
            recording: Some(crate::policy::Recording { size: Size::new(3840, 2160), path: "/mnt/VIDEO/V00001.mp4".into() }),
            view: View { aspect: ScreenAspect::Wide, zoom },
            precedence,
            frames: Frames { scene: true, ..Frames::default() },
            levels: crate::ui::Levels { hdmi: 3, usb: 2, media: 0 },
            ..hdmi_first()
        };
        let f = formats();
        let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
        policy().plan(&Inputs { monitor: &fhd(), host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
    }

    const REC_4K: &str = "3840x2160@30 stock";

    /// Beside a recording, three distinct frames: the lower output pauses; one whose floor forbids the pause is no plan
    /// (the transition that would make it is refused), whichever of the two it is.
    #[test]
    fn beside_a_recording_a_floor_that_forbids_the_pause_is_no_plan() {
        use crate::precedence::Precedence;
        let f = formats();
        let h = f.resolve(MJPEG, 4, 83_333).unwrap();
        for (q, lower) in [("usb > hdmi:lower", "hdmi"), ("hdmi > usb:lower", "usb")] {
            let precedence = Precedence::parse(&format!("recording > player > {q} > photo > drawing > picture")).unwrap();
            let prefs = Prefs {
                recording: Some(crate::policy::Recording { size: Size::new(3840, 2160), path: "v".into() }),
                frames: Frames { scene: true, ..Frames::default() },
                levels: crate::ui::Levels { hdmi: 3, usb: 2, media: 0 },
                precedence,
                ..hdmi_first()
            };
            let r = policy().plan(&Inputs { monitor: &fhd(), host: Some((&h, Epoch(7))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1));
            assert!(matches!(r, Err(crate::policy::PolicyError::NoPause(o)) if o == lower), "{q}: {r:?}");
        }
    }

    /// A mode of the same frame size (a 4K recording's 4K30 after the 4K45 preset): the ports' cuts hold, so nothing
    /// behind them stops for the sensor's restart.
    #[test]
    fn a_switch_to_a_mode_of_the_same_size_leaves_the_ports_and_feeds_running() {
        let mut cur = Current::default();
        run(&mut cur, &want(&fhd(), None, 1)); // idle 4K45
        let d = recording_with(crate::policy::tests::hdmi_over_usb(), 1, None, 2);
        assert_eq!(d.sensor.mode, REC_4K);
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::Switch);
        let off = pos(&p.ops, |x| matches!(x, Op::SensorDisable { .. }));
        assert!(p.ops[..off].iter().all(|x| matches!(x, Op::VifDisablePort { .. } | Op::VifDisableDev { .. })), "{:?}", p.ops);
        assert_eq!(cur.sensor.as_ref().unwrap().mode, REC_4K);
        assert!(cur.recording.is_some());
    }

    #[test]
    fn zooming_while_recording_re_feeds_the_encoder_without_cutting_the_file() {
        let mut cur = Current::default();
        let idle = want(&fhd(), None, 1);
        run(&mut cur, &idle);
        let d = recording(0, None, 2);
        let m = d.media.clone().unwrap();
        assert_eq!((d.sensor.mode.as_str(), d.sensor.fps, m.src_fps, m.fps), (REC_4K, 25, 25, 25), "the recording's mode at its rate");
        let p = run(&mut cur, &d);
        let up = pos(&p.ops, |x| matches!(x, Op::VencUp { chn: topo::VENC_CHN_MEDIA, codec: Codec::H264, .. }));
        let rec = pos(&p.ops, |x| matches!(x, Op::RecordStart { .. }));
        assert!(up < rec);
        // ×2: the window is 1920×1080, the media upscaler brings it back to the recording's 3840×2160
        let p = run(&mut cur, &recording(10, None, 3));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::VencUp { .. } | Op::RecordStop | Op::RecordStart { .. })), "{:?}", p.ops);
        assert!(p.ops.contains(&Op::SclCreateChannel { dev: topo::SCL_UP_DEV, chn: topo::SCL_MEDIA_UP_CHN }));
        assert!(cur.scl_media_up.started && cur.recording.is_some());
        // back to ×1: the upscaler goes, the file goes on
        let p = run(&mut cur, &recording(0, None, 4));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::RecordStop)), "{:?}", p.ops);
        assert!(!cur.scl_media_up.channel && !cur.scl_up.device);
        // stop: the recorder closes the file before the encoder goes
        let p = run(&mut cur, &want(&fhd(), None, 5));
        let stop = pos(&p.ops, |x| matches!(x, Op::RecordStop));
        let down = pos(&p.ops, |x| matches!(x, Op::VencDown { chn: topo::VENC_CHN_MEDIA, .. }));
        assert!(stop < down);
        assert!(cur.media.is_none() && cur.recording.is_none());
    }

    #[test]
    fn usb_and_recording_share_the_upscaler_device_until_the_last_channel_goes() {
        let mut cur = Current::default();
        run(&mut cur, &recording(0, None, 1));
        // zoomed: a 4K USB stream and a 4K recording both need the upscaler
        let p = run(&mut cur, &recording(20, Some((MJPEG, 1, 222_222, 3)), 2));
        assert_eq!(p.ops.iter().filter(|x| matches!(x, Op::SclCreateDevice { dev: topo::SCL_UP_DEV, .. })).count(), 1, "{:?}", p.ops);
        assert!(cur.scl_up.channel && cur.scl_media_up.channel);
        // stream off: the USB channel goes, the device stays for the recording
        let p = run(&mut cur, &recording(20, None, 3));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::SclDestroyDevice { .. })), "{:?}", p.ops);
        assert!(cur.scl_up.device && cur.scl_media_up.started);
        // recording stops: the device goes with its last channel
        let p = run(&mut cur, &recording(0, None, 4));
        let chn = pos(&p.ops, |x| matches!(x, Op::SclDestroyChannel { dev: topo::SCL_UP_DEV, chn: topo::SCL_MEDIA_UP_CHN }));
        let dev = pos(&p.ops, |x| matches!(x, Op::SclDestroyDevice { dev: topo::SCL_UP_DEV }));
        assert!(chn < dev);
        assert!(!cur.scl_up.device);
    }

    #[test]
    fn a_pool_s_canvases_follow_its_readers_holds_and_rates() {
        let uhd = Size::new(3840, 2160);
        let live = FeedWay::Canvas { pool: POOL_HDMI, size: uhd, live: Rect::full(uhd), view: Some(Rect::full(uhd)), visible: vec![Rect::full(uhd)], windows: vec![] };
        let (jpeg, h264, rec) = (venc_in(VENC_DEV_JPEG, VENC_CHN_USB), venc_in(VENC_DEV_H26X, VENC_CHN_USB), venc_in(VENC_DEV_H26X, VENC_CHN_MEDIA));
        // DISP alone at the port's 30: three periods held, two canvases with the port
        assert_eq!(canvases(&[(DISP_IN, 30)], 30, uhd, &live), 5);
        // 2160p30 beside USB MJPEG 4K45 on one pool written at 45: DISP takes 30 of them
        assert_eq!(canvases(&[(DISP_IN, 30), (jpeg, 45)], 45, uhd, &live), 6);
        // a 4K25 recording and USB H.264 at 7 beside DISP at 25: DISP's hold covers theirs
        assert_eq!(canvases(&[(DISP_IN, 25), (rec, 25), (h264, 7)], 25, uhd, &live), 5);
        // an encoder's own pool: JPEG 4K45 3, H.26x 4K25 4, a slow one fewer; 1080 frames held for less
        assert_eq!(canvases(&[(jpeg, 45)], 45, uhd, &live), 3);
        assert_eq!(canvases(&[(rec, 25)], 25, uhd, &live), 4);
        assert_eq!(canvases(&[(h264, 7)], 25, uhd, &live), 3);
        assert_eq!(canvases(&[(rec, 100)], 100, Size::new(1920, 1080), &live), 3);
    }

    #[test]
    fn readers_of_one_picture_at_different_rates_read_one_pool_each_at_its_own() {
        // a 4K25 recording and a host's H.264 4K stream the H.26x encoder's budget slows to 7 beside 2160p30: HDMI's
        // pool alone, written by its port at the sensor's 25; DISP fed at 25, the recording and USB joining at theirs
        let prefs = Prefs { recording: Some(crate::policy::Recording { size: Size::new(3840, 2160), path: "v".into() }), ..hdmi_first() };
        let f = formats();
        let h = f.resolve(H264, 6, 333_333).unwrap();
        let d = policy().plan(&Inputs { monitor: &philips(), host: Some((&h, Epoch(1))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert_eq!((d.sensor.fps, d.usb.as_ref().map(|u| u.fps), d.media.as_ref().map(|m| m.fps)), (25, Some(7), Some(25)));
        let mut cur = Current::default();
        let p = run(&mut cur, &d);
        let (h264_usb, h264_media) = (venc_in(VENC_DEV_H26X, VENC_CHN_USB), venc_in(VENC_DEV_H26X, VENC_CHN_MEDIA));
        assert!(p.ops.contains(&Op::PoolReader { id: POOL_HDMI, reader: h264_usb, fps: 7 }), "{:?}", p.ops);
        assert!(p.ops.contains(&Op::PoolReader { id: POOL_HDMI, reader: h264_media, fps: 25 }), "{:?}", p.ops);
        assert_eq!(cur.feed(DISP_IN).map(|f| f.fps), Some(25));
        assert_eq!(cur.pools.iter().map(|p| p.id).collect::<Vec<_>>(), vec![POOL_HDMI]);
        let hdmi = cur.pool(POOL_HDMI).unwrap();
        assert_eq!((hdmi.readers.len(), hdmi.n), (3, 5));
        assert!(!cur.scl.ports[SCL_PORT_USB as usize].enabled && !cur.scl.ports[SCL_PORT_MEDIA as usize].enabled);
    }

    #[test]
    fn beside_a_recording_three_distinct_frames_pause_the_output_the_priority_does_not_name() {
        use crate::desired::Pause;
        use crate::policy::tests::hdmi_over_usb;
        use crate::precedence::Precedence;
        let host = Some((MJPEG, 4, 83_333, 7));
        // USB's 1080p frame HDMI's (no scene): two frames written, both outputs run beside the recording
        let prefs = Prefs { recording: Some(crate::policy::Recording { size: Size::new(3840, 2160), path: "v".into() }), ..hdmi_first() };
        let f = formats();
        let h = f.resolve(MJPEG, 4, 83_333).unwrap();
        let two = policy().plan(&Inputs { monitor: &fhd(), host: Some((&h, Epoch(7))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(1)).unwrap();
        assert!(two.hdmi.is_some() && two.usb.as_ref().is_some_and(|u| u.frame_of == FrameOf::Hdmi), "{two:?}");
        assert!(!two.pauses());
        assert_eq!(two.media.as_ref().map(|m| m.frame_of), Some(FrameOf::Own), "the 4K recording beside 1080p outputs");
        // three distinct frames: the higher output keeps its live picture, the other keeps a frame without it
        let usb_wins = recording_with(Precedence::default(), 0, host, 1);
        let paused = usb_wins.hdmi.as_ref().unwrap();
        assert_eq!(paused.paused, Some(Pause::Recording));
        assert_eq!((paused.frame.way, paused.frame.window), (Way::Covered, Rect::default()));
        assert!(usb_wins.usb.as_ref().is_some_and(|u| u.paused.is_none() && u.frame.way != Way::Covered));
        let hdmi_wins = recording_with(hdmi_over_usb(), 0, host, 2);
        assert!(hdmi_wins.hdmi.as_ref().is_some_and(|h| h.paused.is_none()));
        assert!(hdmi_wins.usb.as_ref().is_some_and(|u| u.paused == Some(Pause::Recording) && u.frame.way == Way::Covered && u.frame_of == FrameOf::Own));
        // the frame left out when it does not fit: the output goes, as before the pause kept one
        let out = Desired { hdmi: None, ..usb_wins.clone() };
        assert!(out.hdmi.is_none() && out.usb.is_some() && !out.pauses());
        // only one output wanted: it runs, whatever the precedence
        let alone = recording_with(Precedence::default(), 0, None, 3);
        assert!(alone.hdmi.as_ref().is_some_and(|h| h.paused.is_none()));
        // the recording itself is the same in all of them
        assert_eq!(usb_wins.media, hdmi_wins.media);
        assert_eq!(hdmi_wins.media, alone.media);
        // HDMI paused: its pool covered — no port writes it, DISP still fed (three canvases)
        let mut cur = Current::default();
        run(&mut cur, &usb_wins);
        assert!(!cur.scl.ports[topo::SCL_PORT_HDMI as usize].enabled && cur.feed(topo::DISP_IN).is_some());
        assert_eq!(cur.pool(topo::POOL_HDMI).map(|p| p.n), Some(3));
        assert!(cur.recording.is_some() && cur.pump.is_some());
        // USB paused: the host gets frames (covered, its port off); its live picture comes back when the recording stops
        let mut cur = Current::default();
        run(&mut cur, &hdmi_wins);
        assert_eq!(cur.pump, Some(Epoch(7)));
        assert!(!cur.scl.ports[topo::USB_FEED.port as usize].enabled && cur.recording.is_some());
        let prefs = Prefs { precedence: hdmi_over_usb(), ..hdmi_first() };
        let after = policy().plan(&Inputs { monitor: &fhd(), host: Some((&h, Epoch(7))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(4)).unwrap();
        run(&mut cur, &after);
        assert_eq!(cur.pump, Some(Epoch(7)));
        assert!(cur.recording.is_none() && after.usb.as_ref().is_some_and(|u| u.paused.is_none()));
    }

    #[test]
    fn a_host_stream_during_a_recording_is_served_from_the_recordings_mode_and_rate() {
        let mut cur = Current::default();
        run(&mut cur, &recording(0, None, 1));
        // USB 1080p120 would switch to the binned mode; the recording keeps its mode, USB gets its rate
        let d = recording(0, Some((MJPEG, 4, 83_333, 7)), 2);
        assert_eq!((d.sensor.mode.as_str(), d.usb.as_ref().unwrap().fps), (REC_4K, 25));
        let p = run(&mut cur, &d);
        assert_ne!(p.rung, Rung::Rebuild);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::RecordStop | Op::SensorSetRes { .. })));
        assert!(cur.recording.is_some() && cur.venc.is_some());
    }

    #[test]
    fn display_aspect_back_to_wide_restores_the_full_frame_without_an_hdmi_restart() {
        let at = |aspect, rev| {
            let prefs = Prefs { view: View { aspect, zoom: 0 }, ..hdmi_first() };
            policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        let wide = at(ScreenAspect::Wide, 1);
        run(&mut cur, &wide);
        let full = wide.hdmi.as_ref().unwrap().crop;
        for (aspect, rev) in [(ScreenAspect::Four3, 2), (ScreenAspect::Sixteen10, 3), (ScreenAspect::Wide, 4)] {
            let d = at(aspect, rev);
            let p = run(&mut cur, &d);
            assert_eq!(p.rung, Rung::HdmiOnly, "{aspect:?}");
            assert!(!p.ops.contains(&Op::HdmiDispDown), "{aspect:?}");
            assert_eq!(cur.scl.ports[SCL_PORT_HDMI as usize].configured.map(|(c, _)| c), Some(d.hdmi.as_ref().unwrap().crop));
            assert!(cur.scl.ports[SCL_PORT_HDMI as usize].enabled && cur.feed(DISP_IN).is_some());
        }
        assert_eq!(cur.scl.ports[SCL_PORT_HDMI as usize].configured.map(|(c, _)| c), Some(full));
        assert_eq!(plan(&cur, &at(ScreenAspect::Wide, 5), &modes()).unwrap().rung, Rung::Nothing);
    }

    #[test]
    fn hdmi_zoom_goes_through_its_upscaler_never_up_on_the_scl_and_keeps_hdmi_up() {
        let at = |monitor: &Monitor, aspect, zoom, rev| {
            let prefs = Prefs { view: View { aspect, zoom }, ..hdmi_first() };
            policy().plan(&Inputs { monitor, host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        run(&mut cur, &at(&philips(), ScreenAspect::Wide, 0, 1));
        let mut rev = 1;
        for aspect in [ScreenAspect::Wide, ScreenAspect::Sixteen10, ScreenAspect::Four3] {
            for zoom in [0, 3, 10, 40, 0] {
                rev += 1;
                let d = at(&philips(), aspect, zoom, rev);
                let h = d.hdmi.clone().unwrap();
                let p = run(&mut cur, &d);
                assert!(!p.ops.contains(&Op::HdmiDispDown), "{aspect:?} ×{zoom}");
                let (crop, out) = cur.scl.ports[SCL_PORT_HDMI as usize].configured.unwrap();
                assert_eq!(crop, h.crop);
                assert!(out.w <= crop.w && out.h <= crop.h, "{aspect:?} ×{zoom}: SCL {crop:?} → {out:?}");
                // DISP gets whole canvases of the screen's size, the live picture in them at the window's size
                let f = cur.feed(DISP_IN).unwrap();
                let upscaled = out != h.frame.view.size();
                assert_eq!(f.src, if upscaled { SCL_HDMI_UP } else { SCL_OUT_HDMI }, "{aspect:?} ×{zoom}");
                assert_eq!(cur.port_size(f.src), Some(h.frame.view.size()));
                assert_eq!(cur.pool(POOL_HDMI).map(|p| (p.src, p.size)), Some((f.src, h.timing.size())));
                assert_eq!(cur.scl_hdmi_up.channel, upscaled);
            }
        }
        // ×5 on a 4K screen: SCL port 0 hands over the crop as is, the upscaler (dev 1 chn 2, bound frame by frame)
        // makes the screen of it
        run(&mut cur, &at(&philips(), ScreenAspect::Wide, 0, 98));
        let d = at(&philips(), ScreenAspect::Wide, 40, 99);
        let p = run(&mut cur, &d);
        let h = d.hdmi.unwrap();
        assert_eq!(h.scl_out, h.crop.size());
        let bind = pos(&p.ops, |x| matches!(x, Op::Bind { src: SCL_OUT_HDMI, dst: SCL_HDMI_UP, kind: BindKind::FrameBase, .. }));
        let pool = pos(&p.ops, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, src: SCL_HDMI_UP, .. }));
        let start = pos(&p.ops, |x| matches!(x, Op::FeedStart { src: SCL_HDMI_UP, dst: DISP_IN, .. }));
        assert!(pool < bind && bind < start, "{:?}", p.ops);
        assert_eq!(cur.scl_hdmi_up.ports[SCL_UP_PORT as usize].configured, Some((Rect::full(h.scl_out), h.timing.size())));
        assert!(!cur.scl_up.channel && !cur.scl_media_up.channel, "HDMI's channel of its own on dev 1");
        // back to ×1: the upscaler's channel and device go, the pool back on port 0
        let p = run(&mut cur, &at(&philips(), ScreenAspect::Wide, 0, 100));
        assert!(p.ops.contains(&Op::SclDestroyDevice { dev: SCL_UP_DEV }) && cur.scl_hdmi_up == SclState::default());
        assert_eq!(cur.pool(POOL_HDMI).map(|p| p.src), Some(SCL_OUT_HDMI));
        // a 1080p screen: the full 4K frame is scaled down on the SCL
        let mut cur = Current::default();
        let d = at(&fhd(), ScreenAspect::Wide, 0, 1);
        run(&mut cur, &d);
        assert_eq!(d.hdmi.unwrap().scl_out, Size::new(1920, 1080));
    }

    /// A file played: the decoder made before any output binds its port, HDMI's live window written by its
    /// channel of SCL dev 1 bound to the decoder (SCL 0's port idle), USB joined to HDMI reading the same canvases, the
    /// player thread last; the decoder's port scaled down to a 1080p screen; a sensor rebuild during it keeps the
    /// decoder and the thread; the stop puts the sensor back and takes the decoder down after the thread and its binds.
    #[test]
    fn a_file_played_is_bound_into_the_outputs_upscaler_channels_and_stopped_back_to_the_sensor() {
        let playing = |monitor: &Monitor, host: Option<(u8, u8, u32, u64)>, file: Option<&str>, rev: u64| {
            let player = file.map(|f| crate::policy::Playback { path: f.into(), codec: Codec::H264, size: Size::new(3840, 2160), fps: 25, largest: 720_000 });
            let prefs = Prefs { player, ..hdmi_first() };
            let f = formats();
            let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
            policy().plan(&Inputs { monitor, host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        run(&mut cur, &playing(&philips(), Some((MJPEG, 1, 333_333, 1)), None, 1));
        assert!(cur.usb_joined());
        let d = playing(&philips(), Some((MJPEG, 1, 333_333, 1)), Some("/mnt/VIDEO/V00001.mp4"), 2);
        let pl = d.player.clone().unwrap();
        assert_eq!((pl.port, pl.ring, pl.fps), (Size::new(3840, 2160), 2 << 20, 25));
        let p = run(&mut cur, &d);
        let up = pos(&p.ops, |x| matches!(x, Op::VdecUp { .. }));
        let bind = pos(&p.ops, |x| matches!(x, Op::Bind { src: VDEC_OUT, dst: SCL_HDMI_UP, src_fps: 25, .. }));
        let off = pos(&p.ops, |x| matches!(x, Op::SclDisablePort { dev: SCL_DEV, port: SCL_PORT_HDMI, .. }));
        let start = pos(&p.ops, |x| matches!(x, Op::PlayStart { .. }));
        assert!(up < bind && bind < start && off < start && start == p.ops.len() - 1, "{:?}", p.ops);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::HdmiDispDown | Op::VencDown { .. })), "{:?}", p.ops);
        let h = d.hdmi.as_ref().unwrap();
        assert_eq!((h.from, h.crop), (Input::Player, Rect::full(Size::new(3840, 2160))));
        assert_eq!(cur.feed(DISP_IN).map(|f| f.src), Some(SCL_HDMI_UP));
        assert_eq!(cur.scl_hdmi_up.ports[SCL_UP_PORT as usize].configured, Some((h.crop, h.timing.size())));
        assert!(cur.usb_joined(), "{:?} {:?}", d.usb, cur.pools);
        assert!(!cur.scl.ports[SCL_PORT_HDMI as usize].enabled, "{:?}", p.ops);
        assert_eq!(cur.playing.as_deref(), Some("/mnt/VIDEO/V00001.mp4"));
        // nothing to do once built
        assert_eq!(plan(&cur, &d, &modes()).unwrap().ops, Vec::<Op>::new());
        // a 1080p screen (USB gone): the decoder's port scaled down to it, set live; the screen's restart rebinds
        let d = playing(&fhd(), None, Some("/mnt/VIDEO/V00001.mp4"), 3);
        assert_eq!(d.player.as_ref().unwrap().port, Size::new(1920, 1080));
        let p = run(&mut cur, &d);
        assert!(p.ops.contains(&Op::VdecPort { port: Size::new(1920, 1080) }) && !p.ops.iter().any(|x| matches!(x, Op::VdecDown | Op::PlayStop)), "{:?}", p.ops);
        assert_eq!(cur.scl_hdmi_up.ports[SCL_UP_PORT as usize].configured, Some((Rect::full(Size::new(1920, 1080)), Size::new(1920, 1080))));
        // a host stream that needs another sensor mode: the capture rebuilt, the decoder and the thread kept
        let d = playing(&fhd(), Some((MJPEG, 4, 83_333, 2)), Some("/mnt/VIDEO/V00001.mp4"), 4);
        let p = run(&mut cur, &d);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VdecDown | Op::VdecUp { .. } | Op::PlayStop | Op::PlayStart { .. })), "{:?}", p.ops);
        assert!(cur.bind(VDEC_OUT, SCL_HDMI_UP).is_some() && cur.playing.is_some());
        // stopped: the thread first, the sensor back on SCL 0's port, the decoder down after its binds
        let d = playing(&fhd(), Some((MJPEG, 4, 83_333, 2)), None, 5);
        let p = run(&mut cur, &d);
        let stop = pos(&p.ops, |x| matches!(x, Op::PlayStop));
        let unbind = pos(&p.ops, |x| matches!(x, Op::Unbind { src: VDEC_OUT, .. }));
        let down = pos(&p.ops, |x| matches!(x, Op::VdecDown));
        assert!(stop < unbind && unbind < down, "{:?}", p.ops);
        assert!(cur.vdec.is_none() && cur.playing.is_none() && !cur.binds.iter().any(|b| b.src == VDEC_OUT));
        assert_eq!(cur.feed(DISP_IN).map(|f| f.src), Some(SCL_OUT_HDMI));
        // and everything down from a playing state leaves nothing of the player
        run(&mut cur, &playing(&philips(), None, Some("/mnt/VIDEO/V00001.mp4"), 6));
        let ops = teardown_all(&cur);
        apply(&mut TraceHal::default(), &mut cur, &d, &ops).unwrap();
        assert!(cur.vdec.is_none() && cur.playing.is_none() && cur.binds.is_empty(), "{ops:?}");
    }

    #[test]
    fn usb_zoom_goes_through_the_upscaler_without_restarting_the_encoder() {
        let f = formats();
        let host = f.resolve(MJPEG, 4, 83_333).unwrap(); // MJPEG 1920x1080
        let at = |zoom, ep: u64, rev| {
            let prefs = Prefs { view: View { aspect: ScreenAspect::Wide, zoom }, ..hdmi_first() };
            policy().plan(&Inputs { monitor: &philips(), host: Some((&host, Epoch(ep))), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let src = |cur: &Current| cur.feed_into(VENC_CHN_USB).map(|f| f.src);
        let mut cur = Current::default();
        run(&mut cur, &at(0, 1, 1));
        assert!(!cur.scl_up.device && src(&cur) == Some(SCL_OUT_USB));
        // ×5: the 768×432 window is smaller than 1080p → SCL port 2 hands it over as is, the upscaler makes 1080p
        let d = at(40, 1, 2);
        let u = d.usb.clone().unwrap();
        assert_eq!(u.scl_out, u.crop.size());
        let p = run(&mut cur, &d);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::VencUp { .. } | Op::PumpStop | Op::HdmiDispDown)), "{:?}", p.ops);
        assert!(cur.scl_up.started && cur.bind(SCL_OUT_USB, SCL_UP).is_some() && src(&cur) == Some(SCL_UP));
        assert_eq!(cur.scl_up.ports[0].configured, Some((Rect::full(u.scl_out), u.size)));
        assert!(cur.pool(POOL_USB).is_some_and(|p| p.src == SCL_UP), "the pool moved to the upscaler's port");
        assert_eq!(plan(&cur, &at(40, 1, 3), &modes()).unwrap().rung, Rung::Nothing);
        // ×1.5: the window is still larger than 1080p → straight again, the upscaler gone
        let p = run(&mut cur, &at(5, 1, 4));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::VencUp { .. })));
        // (HDMI, zoomed alike, keeps the device for its own channel)
        assert!(!cur.scl_up.channel && src(&cur) == Some(SCL_OUT_USB) && cur.bind(SCL_OUT_USB, SCL_UP).is_none());
        assert!(cur.pool(POOL_USB).is_some_and(|p| p.src == SCL_OUT_USB), "the pool back on port 2");
        // a stream stop while zoomed takes the upscaler down with the encoder
        run(&mut cur, &at(40, 1, 5));
        assert!(cur.scl_up.device);
        let prefs = Prefs { view: View { aspect: ScreenAspect::Wide, zoom: 40 }, ..hdmi_first() };
        let off = policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(6)).unwrap();
        run(&mut cur, &off);
        assert!(cur.venc.is_none() && cur.pump.is_none() && !cur.scl_up.channel && !cur.scl_up.started && cur.scl_up.ports == SclState::default().ports);
        assert!(!cur.binds.iter().any(|b| b.src == SCL_UP || b.dst == SCL_UP));
    }

    #[test]
    fn hdmi_unplug_and_replug_moves_the_hdmi_side_and_the_sensor_s_rate_only() {
        let mut cur = Current::default();
        // USB 720p30: a frame of its own beside HDMI's 1080p60; the sensor at the faster output's 60
        run(&mut cur, &want(&fhd(), Some((MJPEG, 3, 333_333, 1)), 1));
        assert_eq!(cur.sensor.as_ref().map(|s| s.fps_live), Some(60));
        let usb_side = |x: &Op| matches!(x, Op::VencUp { .. } | Op::VencDown { .. } | Op::PumpStop | Op::PumpStart { .. } | Op::PoolUp { id: POOL_USB, .. } | Op::PoolDown { id: POOL_USB });
        let d = want(&Monitor::Absent, Some((MJPEG, 3, 333_333, 1)), 2); // the same mode without a monitor
        let p = run(&mut cur, &d);
        assert!(p.ops.contains(&Op::HdmiDispDown) && p.ops.contains(&Op::SensorFpsLive { fps: 30 }), "{:?}", p.ops);
        assert!(!p.ops.iter().any(usb_side), "{:?}", p.ops);
        assert!(cur.hdmi.is_none() && !cur.scl.ports[0].enabled && cur.pump.is_some());
        let d = want(&fhd(), Some((MJPEG, 3, 333_333, 1)), 3);
        let p = run(&mut cur, &d);
        assert!(p.ops.iter().any(|x| matches!(x, Op::HdmiDispUp { timing: Timing::P1080_60, .. })) && p.ops.contains(&Op::SensorFpsLive { fps: 60 }), "{:?}", p.ops);
        assert!(!p.ops.iter().any(usb_side), "{:?}", p.ops);
    }

    #[test]
    fn stream_off_after_a_mode_change_does_not_touch_the_removed_encoder_twice() {
        let mut cur = Current::default();
        run(&mut cur, &want(&fhd(), None, 1)); // idle 4K45 with HDMI 1080p60
        run(&mut cur, &want(&fhd(), Some((MJPEG, 1, 333_333, 1)), 2)); // 4K30 → the 4K43 mode + encoder, its own feed
        let d = want(&fhd(), None, 3); // host stops → back to the preset: a switch
        let p = plan(&cur, &d, &modes()).unwrap();
        assert_eq!(p.rung, Rung::Switch);
        let stops = p.ops.iter().filter(|o| matches!(o, Op::FeedStop { dst } if dst.module == Module::Venc && dst.chn == VENC_CHN_USB)).count();
        let downs = p.ops.iter().filter(|o| matches!(o, Op::VencDown { .. })).count();
        assert_eq!((stops, downs), (1, 1), "{:?}", p.ops);
        run(&mut cur, &d); // apply() enforces every precondition
        assert!(cur.venc.is_none() && cur.pump.is_none() && cur.hdmi.is_some());
    }

    #[test]
    fn nothing_to_do_when_desired_is_already_built() {
        let mut cur = Current::default();
        let d = want(&philips(), None, 1);
        run(&mut cur, &d);
        let p = plan(&cur, &d.clone().with_revision(Revision(2)), &modes()).unwrap();
        assert_eq!(p.rung, Rung::Nothing);
        assert!(p.ops.is_empty());
    }

    #[test]
    fn failure_leaves_the_truth_and_rollback_covers_only_what_exists() {
        let mut cur = Current::default();
        let d = want(&philips(), None, 1);
        let p = plan(&cur, &d, &modes()).unwrap();
        let isp_start = pos(&p.ops, |x| matches!(x, Op::IspStartChannel { .. }));
        let mut hal = TraceHal { fail_at: Some(isp_start), ..Default::default() };
        let err = apply(&mut hal, &mut cur, &d, &p.ops).unwrap_err();
        assert_eq!(err.index, isp_start);
        assert!(cur.sensor.as_ref().unwrap().enabled && cur.vif.port_enabled && cur.isp.channel && !cur.isp.started);
        assert!(cur.desired.is_none());
        let mut rb = Vec::new();
        teardown_capture(&cur, &mut rb);
        assert!(!rb.iter().any(|x| matches!(x, Op::IspStopChannel { .. } | Op::SclDestroyDevice { .. } | Op::Unbind { .. })));
        let chn = pos(&rb, |x| matches!(x, Op::IspDestroyChannel { .. }));
        let dev = pos(&rb, |x| matches!(x, Op::IspDestroyDevice { .. }));
        let vif = pos(&rb, |x| matches!(x, Op::VifDestroyGroup { .. }));
        let snr = pos(&rb, |x| matches!(x, Op::SensorDisable { .. }));
        assert!(chn < dev && dev < vif && vif < snr);
        // and replanning from the partial state is a rebuild that first finishes the teardown
        let p2 = plan(&cur, &d, &modes()).unwrap();
        assert_eq!(p2.rung, Rung::Rebuild);
        assert_eq!(&p2.ops[..rb.len()], &rb[..]);
    }

    fn tuned(mut d: Desired, knobs: &[(crate::tuning::Knob, i32)]) -> Desired {
        d.image.tuning.extend(knobs.iter().copied());
        d
    }

    #[test]
    fn a_picture_knob_on_a_running_pipeline_is_one_isp_call() {
        use crate::tuning::Knob;
        let mut cur = Current::default();
        run(&mut cur, &want(&philips(), Some((MJPEG, 1, 222_222, 1)), 1));
        let d = tuned(want(&philips(), Some((MJPEG, 1, 222_222, 1)), 2), &[(Knob::Brightness, 70), (Knob::Saturation, 120)]);
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::ImageOnly);
        assert_eq!(p.ops.len(), 2);
        assert!(p.ops.iter().all(|o| matches!(o, Op::IspAdjust { .. })));
        assert_eq!(plan(&cur, &d, &modes()).unwrap().rung, Rung::Nothing, "applied knobs are remembered");
    }

    #[test]
    fn a_mode_switch_keeps_the_iq_and_the_knobs() {
        use crate::tuning::Knob;
        let mut cur = Current::default();
        let d = tuned(want(&fhd(), None, 1), &[(Knob::Contrast, 60)]);
        run(&mut cur, &d);
        let d = tuned(want(&fhd(), Some((MJPEG, 4, 83_333, 7)), 2), &[(Knob::Contrast, 60)]);
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::Switch);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::IspLoadIq { .. } | Op::IspAdjust { .. })), "the ISP channel runs on: {:?}", p.ops);
        assert_eq!(cur.isp.tuning.get(&Knob::Contrast), Some(&60));
    }

    /// A picture knob the ISP refuses after the IQ load does not stop the build: the video comes up, the knob is
    /// recorded refused, not asked again on the next plan at the same value, and asked again after the next IQ load.
    #[test]
    fn a_refused_knob_does_not_stop_the_build_and_waits_for_the_next_load() {
        use crate::tuning::Knob;
        /// Refuses every brightness step.
        #[derive(Default)]
        struct Dim(Vec<Op>);
        impl Hal for Dim {
            fn exec(&mut self, op: &Op) -> Result<(), HalError> {
                self.0.push(op.clone());
                match op {
                    Op::IspAdjust { adjust: Adjust::Iq { knob: Knob::Brightness, .. }, .. } => Err(HalError { op: format!("{op:?}"), detail: "no IQ baseline".into() }),
                    _ => Ok(()),
                }
            }
        }
        let mut cur = Current::default();
        let d = tuned(want(&philips(), None, 1), &[(Knob::Brightness, 70), (Knob::Contrast, 60)]);
        let ops = plan(&cur, &d, &modes()).unwrap().ops;
        let mut hal = Dim::default();
        apply(&mut hal, &mut cur, &d, &ops).unwrap();
        assert_eq!(hal.0.len(), ops.len(), "every step ran");
        assert!(cur.hdmi.is_some(), "the video is up");
        assert_eq!(cur.isp.refused.get(&Knob::Brightness), Some(&(70, "no IQ baseline".to_string())));
        assert_eq!((cur.isp.tuning.get(&Knob::Contrast), cur.isp.tuning.get(&Knob::Brightness)), (Some(&60), None));
        assert_eq!(plan(&cur, &d, &modes()).unwrap().rung, Rung::Nothing, "not asked again at the same value");
        let other = tuned(want(&philips(), None, 2), &[(Knob::Brightness, 71), (Knob::Contrast, 60)]);
        assert_eq!(plan(&cur, &other, &modes()).unwrap().ops.len(), 1, "asked again at another value");
        let mut reload = d.clone();
        reload.image.iq_bin = "/other.bin".into();
        let ops = plan(&cur, &reload, &modes()).unwrap().ops;
        assert!(ops.iter().any(|o| matches!(o, Op::IspAdjust { adjust: Adjust::Iq { knob: Knob::Brightness, value: 70 }, .. })), "asked again after the load");
        apply(&mut TraceHal::default(), &mut cur, &reload, &ops).unwrap();
        assert!(cur.isp.refused.is_empty() && cur.isp.tuning.get(&Knob::Brightness) == Some(&70));
    }

    /// The vendor's compare view as a scene would lay it out on `screen`: one picture in the left half beside the live
    /// right half; two in the left quarters; three top-left, bottom-left, top-right with the live bottom-right quarter.
    fn compare(ids: &[u64], screen: Size) -> Option<Layout> {
        let (w, h) = ((screen.w / 2) & !1, (screen.h / 2) & !1);
        let r = |x, y, w, h| Rect { x, y, w, h };
        let (live, windows) = match ids.len() {
            0 => return None,
            1 => (r(w, 0, w, screen.h), vec![r(0, 0, w, screen.h)]),
            2 => (r(w, 0, w, screen.h), vec![r(0, 0, w, h), r(0, h, w, h)]),
            _ => (r(w, h, w, h), vec![r(0, 0, w, h), r(0, h, w, h), r(w, 0, w, h)]),
        };
        Some(Layout { raster: screen, live, windows })
    }

    /// The plan with pictures `ids` beside the live picture on every output, as the compare view lays them out.
    fn frozen_at(monitor: &Monitor, ids: &[u64], zoom: u8, host: Option<(u8, u8, u32, u64)>, rev: u64) -> Desired {
        frozen_with(hdmi_first(), monitor, ids, zoom, host, rev)
    }

    /// `frozen_at` on other preferences (the 1080 boot preset: idle beside a UHD monitor on a mode no stream takes).
    fn frozen_with(base: Prefs, monitor: &Monitor, ids: &[u64], zoom: u8, host: Option<(u8, u8, u32, u64)>, rev: u64) -> Desired {
        let prefs = Prefs { view: View { aspect: ScreenAspect::Wide, zoom }, ..base };
        let f = formats();
        let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
        let at = |prefs: &Prefs| policy().plan(&Inputs { monitor, host: h.as_ref().map(|(s, e)| (s, *e)), prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap();
        let bare = at(&prefs);
        let frames = Frames { hdmi: bare.hdmi.and_then(|x| compare(ids, x.timing.size())), usb: bare.usb.and_then(|x| compare(ids, x.size)), media: None, scene: true };
        at(&Prefs { frames, ..prefs })
    }

    /// The pictures' windows in HDMI's canvases.
    fn on_hdmi(cur: &Current) -> usize {
        match cur.feed(DISP_IN).map(|f| &f.way) {
            Some(FeedWay::Canvas { windows, .. }) => windows.len(),
            None => 0,
        }
    }

    #[test]
    fn covered_usb_runs_its_pool_on_the_feed_clock_with_the_port_off() {
        use crate::policy::{Frames, Layout};
        let host = Some((MJPEG, 1, 333_333, 1)); // MJPEG 4K30
        let uhd = Size::new(3840, 2160);
        let at = |windows: Vec<Rect>, rev| {
            let f = formats();
            let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
            let usb = (!windows.is_empty()).then(|| Layout { raster: uhd, live: Rect::full(uhd), windows });
            let prefs = Prefs { frames: Frames { hdmi: None, usb, media: None, scene: true }, ..hdmi_first() };
            policy().plan(&Inputs { monitor: &Monitor::Absent, host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let full = Rect::full(uhd);
        let mut cur = Current::default();
        run(&mut cur, &at(Vec::new(), 1));
        assert_eq!(cur.pool(POOL_USB).map(|p| (p.src, p.n)), Some((SCL_OUT_USB, 3)));
        // a picture over the whole frame: the feed stops, the port goes off, the pool keeps two canvases, no encoder change
        let p = run(&mut cur, &at(vec![full], 2));
        let stop = pos(&p.ops, |x| matches!(x, Op::FeedStop { .. }));
        let off = pos(&p.ops, |x| matches!(x, Op::SclDisablePort { port: SCL_PORT_USB, .. }));
        let start = pos(&p.ops, |x| matches!(x, Op::FeedStart { way: FeedWay::Canvas { view: None, .. }, .. }));
        assert!(stop < off && off < start, "{:?}", p.ops);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::PoolDown { .. })), "{:?}", p.ops);
        assert_eq!(cur.pool(POOL_USB).map(|p| p.n), Some(2));
        assert!(!cur.scl.ports[SCL_PORT_USB as usize].enabled);
        assert_eq!(plan(&cur, &at(vec![full], 3), &modes()).unwrap().rung, Rung::Nothing);
        // gone: the port back on, the pool at three before its first frame
        let p = run(&mut cur, &at(Vec::new(), 4));
        let count = pos(&p.ops, |x| matches!(x, Op::PoolCount { n: 3, .. }));
        let on = pos(&p.ops, |x| matches!(x, Op::SclEnablePort { port: SCL_PORT_USB, .. }));
        assert!(count < on, "{:?}", p.ops);
        assert!(live_way(&cur.feed_into(VENC_CHN_USB).map(|f| f.way.clone())));
    }

    #[test]
    fn frames_on_hdmi_re_feed_disp_from_its_pool_and_never_restart_it() {
        let m = philips();
        let mut cur = Current::default();
        run(&mut cur, &frozen_at(&m, &[], 0, None, 1));
        assert!(cur.scl_stretch.device, "the scaler is up with HDMI: its menu over pictures is baked with it");
        // one frame: the feed stops, port 0 writes the right half of the canvases, the feed lays the picture beside it
        let p = run(&mut cur, &frozen_at(&m, &[1], 0, None, 2));
        assert_eq!(p.rung, Rung::HdmiOnly);
        let o = &p.ops;
        let stop = pos(o, |x| matches!(x, Op::FeedStop { dst } if *dst == DISP_IN));
        let port = pos(o, |x| matches!(x, Op::SclConfigurePort { port: SCL_PORT_HDMI, size, .. } if *size == Size::new(1920, 2160)));
        let start = pos(o, |x| matches!(x, Op::FeedStart { dst, .. } if *dst == DISP_IN));
        assert!(stop < port && port < start, "{o:?}");
        assert!(!o.iter().any(|x| matches!(x, Op::HdmiDispDown | Op::HdmiDispUp { .. } | Op::PoolDown { .. } | Op::PoolUp { .. } | Op::SensorSetRes { .. } | Op::SclCreateDevice { .. })), "{o:?}");
        let Some(FeedWay::Canvas { live, view, visible, .. }) = cur.feed(DISP_IN).map(|f| f.way.clone()) else { panic!() };
        assert_eq!((live, view, visible), (Rect { x: 1920, y: 0, w: 1920, h: 2160 }, Some(Rect { x: 1920, y: 0, w: 1920, h: 2160 }), vec![live]));
        assert_eq!(on_hdmi(&cur), 1);
        let d = cur.desired.clone().unwrap();
        assert_eq!(plan(&cur, &d.with_revision(Revision(3)), &modes()).unwrap().rung, Rung::Nothing);
        // two, three: the feed re-laid each time, the pool and DISP as they are
        for (ids, rev) in [(&[1, 2][..], 4), (&[1, 2, 3], 5)] {
            let p = run(&mut cur, &frozen_at(&m, ids, 0, None, rev));
            assert!(!p.ops.iter().any(|x| matches!(x, Op::HdmiDispDown | Op::PoolDown { .. } | Op::PoolUp { .. })), "{:?}", p.ops);
            assert_eq!(on_hdmi(&cur), ids.len());
        }
        // a fourth picture in the oldest's window: the same geometry, no step (what lies in the window is the ui's)
        assert_eq!(plan(&cur, &frozen_at(&m, &[4, 2, 3], 0, None, 6), &modes()).unwrap().rung, Rung::Nothing);
        // clear: the live picture takes the whole canvas again
        let p = run(&mut cur, &frozen_at(&m, &[], 0, None, 7));
        assert!(pos(&p.ops, |x| matches!(x, Op::FeedStop { .. })) < pos(&p.ops, |x| matches!(x, Op::SclConfigurePort { port: SCL_PORT_HDMI, .. })));
        assert!(on_hdmi(&cur) == 0 && cur.scl_stretch.device);
        assert_eq!(cur.feed(DISP_IN).map(|f| f.src), Some(SCL_OUT_HDMI));
    }

    #[test]
    fn frames_beside_a_zoomed_usb_stream_leave_its_upscaler_alone_and_survive_a_mode_switch() {
        let m = philips();
        let host = Some((MJPEG, 4, 333_333, 1)); // MJPEG 1080p30
        let mut cur = Current::default();
        run(&mut cur, &frozen_at(&m, &[], 40, host, 1));
        assert!(cur.scl_up.channel && cur.scl_hdmi_up.channel, "×5 goes through the USB and the HDMI upscalers");
        let p = run(&mut cur, &frozen_at(&m, &[1, 2], 40, host, 2));
        let touches_usb = |x: &Op| match x {
            Op::SclCreateDevice { dev, .. } | Op::SclDestroyDevice { dev } => *dev == SCL_UP_DEV,
            Op::SclCreateChannel { dev, chn } | Op::SclDestroyChannel { dev, chn } => (*dev, *chn) == (SCL_UP_DEV, SCL_UP_CHN),
            _ => false,
        };
        assert!(!p.ops.iter().any(touches_usb), "{:?}", p.ops);
        assert!(cur.scl_up.started && cur.pump.is_some() && cur.scl_stretch.device);
        // the stream stops while zoomed into a sensor switch (the 1080 boot preset): USB's upscaler goes with the stream;
        // DISP stays up with the pictures
        let fhd_preset = Prefs { boot_preset: BootPreset::Fhd, ..hdmi_first() };
        let p = run(&mut cur, &frozen_with(fhd_preset.clone(), &m, &[1, 2], 40, None, 3));
        assert_eq!(p.rung, Rung::Switch);
        assert!(p.ops.contains(&Op::SclDestroyChannel { dev: SCL_UP_DEV, chn: SCL_UP_CHN }) && !p.ops.contains(&Op::HdmiDispDown), "{:?}", p.ops);
        assert!(!cur.scl_up.channel && cur.scl_hdmi_up.channel && on_hdmi(&cur) == 2);
        // the monitor goes: the feed, then the pool (DISP still there), then DISP; the scaler and the upscaler with them
        let p = run(&mut cur, &frozen_with(fhd_preset.clone(), &Monitor::Absent, &[1, 2], 40, None, 4));
        let (stop, pool, down) = (pos(&p.ops, |x| *x == Op::FeedStop { dst: DISP_IN }), pos(&p.ops, |x| *x == Op::PoolDown { id: POOL_HDMI }), pos(&p.ops, |x| *x == Op::HdmiDispDown));
        assert!(stop < pool && pool < down, "{:?}", p.ops);
        assert!(cur.hdmi.is_none() && cur.pools.is_empty() && !cur.scl_stretch.device && !cur.scl_up.device);
        // and come back with it: DISP first, its pool, the feed
        let p = run(&mut cur, &frozen_with(fhd_preset, &m, &[1, 2], 40, None, 5));
        let (up, pool, start) = (pos(&p.ops, |x| matches!(x, Op::HdmiDispUp { .. })), pos(&p.ops, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. })), pos(&p.ops, |x| matches!(x, Op::FeedStart { dst: DISP_IN, .. })));
        assert!(up < pool && pool < start, "{:?}", p.ops);
        assert_eq!(on_hdmi(&cur), 2);
    }

    #[test]
    fn the_monitor_switched_away_and_back_beside_a_zoomed_usb_stream_brings_the_frames_back() {
        // a monitor input switch is an unplug and replug; beside this stream the sensor mode stays (4K45 with or
        // without a monitor), only the outputs change: USB 4K30 joins HDMI's frame and makes its own when HDMI goes
        let m = philips();
        let host = Some((MJPEG, 1, 333_333, 1));
        let mut cur = Current::default();
        run(&mut cur, &frozen_at(&m, &[], 4, host, 1));
        run(&mut cur, &frozen_at(&m, &[1, 2, 3], 4, host, 2));
        let p = run(&mut cur, &frozen_at(&Monitor::Absent, &[1, 2, 3], 4, host, 3));
        assert_eq!(p.rung, Rung::Outputs);
        // the screen goes; the frames stay on USB, the scaler with the compositor
        assert!(cur.hdmi.is_none() && cur.pool(POOL_HDMI).is_none() && cur.scl_stretch.device);
        assert!(matches!(&cur.feed_into(VENC_CHN_USB).unwrap().way, FeedWay::Canvas { windows, .. } if windows.len() == 3));
        run(&mut cur, &frozen_at(&m, &[1, 2, 3], 4, host, 4));
        assert_eq!(on_hdmi(&cur), 3);
        // and a sensor switch with HDMI up keeps the scaler: the stream stops into the 1080 preset
        let p = run(&mut cur, &frozen_with(Prefs { boot_preset: BootPreset::Fhd, ..hdmi_first() }, &m, &[1, 2, 3], 4, None, 5));
        assert_eq!(p.rung, Rung::Switch);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::SclCreateDevice { dev: SCL_STRETCH_DEV, .. } | Op::SclDestroyDevice { dev: SCL_STRETCH_DEV })));
        assert!(cur.scl_stretch.device && on_hdmi(&cur) == 3);
    }

    #[test]
    fn hdmi_s_pool_goes_while_disp_is_there_and_a_new_timing_restarts_disp_between_pools() {
        let mut cur = Current::default();
        let at_1440 = Prefs { hdmi_override: Some(Timing::P1440_60), ..hdmi_first() };
        run(&mut cur, &policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &at_1440, tuning: &Default::default() }, &modes(), Revision(1)).unwrap());
        // another timing (same sensor mode): the feed, the pool, DISP down; DISP up, a pool of the new size, the feed
        let p = run(&mut cur, &want(&fhd(), None, 2));
        assert_eq!(p.rung, Rung::HdmiOnly);
        let at = |pred: &dyn Fn(&Op) -> bool| pos(&p.ops, pred);
        let order = [
            at(&|x| *x == Op::FeedStop { dst: DISP_IN }),
            at(&|x| *x == Op::PoolDown { id: POOL_HDMI }),
            at(&|x| *x == Op::HdmiDispDown),
            at(&|x| *x == Op::HdmiDispUp { timing: Timing::P1080_60 }),
            at(&|x| matches!(x, Op::PoolUp { id: POOL_HDMI, size, .. } if *size == Size::new(1920, 1080))),
            at(&|x| matches!(x, Op::SclEnablePort { port: SCL_PORT_HDMI, .. })),
            at(&|x| matches!(x, Op::FeedStart { dst: DISP_IN, .. })),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{:?}", p.ops);
        // canvases at the sensor's 45 fps: the timing's 60 would hand the same frames again
        assert_eq!((cur.sensor.as_ref().unwrap().fps_live, cur.feed(DISP_IN).map(|f| f.fps)), (0, Some(45)));
        // a rollback from there takes everything down in an order every step's precondition allows
        let mut down = cur.clone();
        for op in teardown_all(&cur) {
            down.precondition(&op).unwrap();
            down.apply(&op);
        }
        assert!(down.hdmi.is_none() && down.pools.is_empty() && down.feeds.is_empty() && !down.scl_stretch.device);
        // DISP refuses to go while its pool or its feed is there
        assert!(cur.precondition(&Op::HdmiDispDown).is_err());
        let mut c = cur.clone();
        c.apply(&Op::FeedStop { dst: DISP_IN });
        assert!(c.precondition(&Op::HdmiDispDown).is_err() && c.precondition(&Op::PoolDown { id: POOL_HDMI }).is_ok());
    }

    #[test]
    fn a_usb_encoder_taken_down_alone_comes_back_as_a_usb_step() {
        let host = Some((MJPEG, 2, 166_666, 1)); // MJPEG 3840x1600: a frame of its own beside HDMI's 4K
        let m = philips();
        let mut cur = Current::default();
        run(&mut cur, &frozen_at(&m, &[1], 0, host, 1));
        let ops = teardown_encoder(&cur, VENC_CHN_USB);
        assert_eq!(ops, vec![
            Op::FeedStop { dst: venc_in(VENC_DEV_JPEG, VENC_CHN_USB) },
            Op::PumpStop,
            Op::SclDisablePort { dev: SCL_DEV, chn: SCL_CHN, port: SCL_PORT_USB },
            Op::PoolDown { id: POOL_USB },
            Op::VencDown { dev: VENC_DEV_JPEG, chn: VENC_CHN_USB },
        ]);
        // a pump that does not stop keeps the encoder: nothing after it runs
        let (mut stuck, mut hal) = (cur.clone(), TraceHal { fail_at: Some(1), ..TraceHal::default() });
        let want = stuck.desired.clone().unwrap();
        assert!(apply(&mut hal, &mut stuck, &want, &ops).is_err());
        assert!(stuck.pump.is_some() && stuck.venc.is_some() && stuck.feed_into(VENC_CHN_USB).is_none() && stuck.desired.is_none());
        let mut hal = TraceHal::default();
        apply(&mut hal, &mut cur, &want, &ops).unwrap();
        let p = plan(&cur, &want.with_revision(Revision(2)), &modes()).unwrap();
        assert_eq!(p.rung, Rung::UsbOnly);
        let start = pos(&p.ops, |x| matches!(x, Op::FeedStart { way: FeedWay::Canvas { windows, .. }, .. } if !windows.is_empty()));
        assert!(pos(&p.ops, |x| matches!(x, Op::VencUp { .. })) < start && start < pos(&p.ops, |x| matches!(x, Op::PumpStart { .. })), "{:?}", p.ops);
    }

    /// The plan with the live picture on the whole 4K screen and pictures at `windows` (bottom to top) over it.
    fn over_live(windows: &[Rect], rev: u64) -> Desired {
        let m = philips();
        let screen = Size::new(3840, 2160);
        let layout = (!windows.is_empty()).then(|| Layout { raster: screen, live: Rect::full(screen), windows: windows.to_vec() });
        let prefs = Prefs { frames: Frames { hdmi: layout, scene: true, ..Frames::default() }, ..hdmi_first() };
        policy().plan(&Inputs { monitor: &m, host: None, prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
    }

    #[test]
    fn hdmi_pictures_over_the_live_one_lie_in_its_canvases_and_covered_runs_on_the_feed_clock() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let mut cur = Current::default();
        run(&mut cur, &over_live(&[], 1));
        // one picture over the live one: put back after each write, one canvas more; the port as it was
        let p = run(&mut cur, &over_live(&[r(400, 400, 1280, 720)], 2));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::SclDisablePort { .. } | Op::SclConfigurePort { .. } | Op::HdmiDispDown | Op::PoolDown { .. })), "{:?}", p.ops);
        assert!(p.ops.contains(&Op::PoolCount { id: POOL_HDMI, n: 6 }));
        assert_eq!(on_hdmi(&cur), 1);
        // a picture over the whole screen: the feed stops, the port goes off, the pool runs on three canvases on the
        // compositor's clock
        let p = run(&mut cur, &over_live(&[Rect::full(Size::new(3840, 2160))], 3));
        let stop = pos(&p.ops, |x| matches!(x, Op::FeedStop { dst } if *dst == DISP_IN));
        let off = pos(&p.ops, |x| matches!(x, Op::SclDisablePort { port: SCL_PORT_HDMI, .. }));
        let start = pos(&p.ops, |x| matches!(x, Op::FeedStart { dst: DISP_IN, way: FeedWay::Canvas { view: None, .. }, .. }));
        assert!(stop < off && off < start, "{:?}", p.ops);
        assert!(!cur.scl.ports[SCL_PORT_HDMI as usize].enabled && cur.pool(POOL_HDMI).map(|p| (p.src, p.n)) == Some((SCL_OUT_HDMI, 3)));
        assert_eq!(plan(&cur, &cur.desired.clone().unwrap().with_revision(Revision(4)), &modes()).unwrap().rung, Rung::Nothing);
        // gone: the pool at five before the port's first frame, then the feed
        let p = run(&mut cur, &over_live(&[], 5));
        let count = pos(&p.ops, |x| *x == Op::PoolCount { id: POOL_HDMI, n: 5 });
        let on = pos(&p.ops, |x| matches!(x, Op::SclEnablePort { port: SCL_PORT_HDMI, .. }));
        let start = pos(&p.ops, |x| matches!(x, Op::FeedStart { dst: DISP_IN, .. }));
        assert!(count < on && on < start, "{:?}", p.ops);
        assert!(on_hdmi(&cur) == 0 && cur.scl_stretch.device);
    }

    #[test]
    fn a_picture_over_the_usb_live_picture_is_pasted_into_its_frames_by_the_feed() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let host = Some((MJPEG, 1, 222_222, 1)); // MJPEG 4K45
        let uhd = Size::new(3840, 2160);
        let at = |windows: &[Rect], rev| {
            let f = formats();
            let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
            let usb = (!windows.is_empty()).then(|| Layout { raster: uhd, live: Rect::full(uhd), windows: windows.to_vec() });
            let prefs = Prefs { frames: Frames { hdmi: None, usb, media: None, scene: true }, ..hdmi_first() };
            policy().plan(&Inputs { monitor: &Monitor::Absent, host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        run(&mut cur, &at(&[], 1));
        let p = run(&mut cur, &at(&[r(400, 200, 1920, 1080)], 2));
        assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::SclConfigurePort { .. })), "{:?}", p.ops);
        let FeedWay::Canvas { windows, .. } = &cur.feed_into(VENC_CHN_USB).unwrap().way;
        assert_eq!(windows, &vec![r(400, 200, 1920, 1080)]);
        assert!(cur.scl_stretch.device);
        // gone: the frames handed on as they are; the scaler stays for the menu's bake
        run(&mut cur, &at(&[], 3));
        assert!(live_way(&cur.feed_into(VENC_CHN_USB).map(|f| f.way.clone())) && cur.scl_stretch.device);
    }

    #[test]
    fn an_output_giving_media_heap_back_goes_before_one_taking_it() {
        let host = Some((MJPEG, 1, 222_222, 1)); // MJPEG 4K45 beside HDMI 2160p
        let uhd = Size::new(3840, 2160);
        let at = |hdmi: &[Rect], usb: &[Rect], rev| {
            let f = formats();
            let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
            let layout = |windows: &[Rect]| {
                (!windows.is_empty()).then(|| Layout { raster: uhd, live: Rect::full(uhd), windows: windows.to_vec() })
            };
            // USB at another level than HDMI's: each output laid out on its own, USB on its own pool
            let levels = crate::ui::Levels { usb: 2, ..Default::default() };
            let prefs = Prefs { frames: Frames { hdmi: layout(hdmi), usb: layout(usb), media: None, scene: true }, levels, ..hdmi_first() };
            policy().plan(&Inputs { monitor: &philips(), host: h.as_ref().map(|(s, e)| (s, *e)), prefs: &prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        // HDMI covered (three canvases), USB with a picture over its live picture (one canvas more)
        run(&mut cur, &at(&[Rect::full(uhd)], &[Rect { x: 400, y: 200, w: 1920, h: 1080 }], 1));
        assert_eq!((cur.pool(POOL_HDMI).map(|p| p.n), cur.pool(POOL_USB).map(|p| p.n)), (Some(3), Some(4)));
        // both cleared: USB gives a canvas back before HDMI takes two
        let p = run(&mut cur, &at(&[], &[], 2));
        let usb = pos(&p.ops, |x| *x == Op::PoolCount { id: POOL_USB, n: 3 });
        let hdmi = pos(&p.ops, |x| *x == Op::PoolCount { id: POOL_HDMI, n: 5 });
        assert!(usb < hdmi, "{:?}", p.ops);
        assert!(live_way(&cur.feed_into(VENC_CHN_USB).map(|f| f.way.clone())) && on_hdmi(&cur) == 0);
    }

    #[test]
    fn frames_on_usb_change_how_the_feed_hands_frames_on_and_never_rebuild_the_encoder() {
        let host = Some((MJPEG, 1, 222_222, 1)); // MJPEG 4K45
        let way = |cur: &Current| cur.feed_into(VENC_CHN_USB).map(|f| f.way.clone());
        // beside a 1080p monitor (a 4K one's frame would be USB's: it reads HDMI's pool) and none
        for m in [fhd(), Monitor::Absent] {
            let mut cur = Current::default();
            run(&mut cur, &frozen_at(&m, &[], 0, host, 1));
            assert!(live_way(&way(&cur)));
            // a frame: port 2 shrinks to the live half and the feed composes; the encoder and the sensor run on
            let p = run(&mut cur, &frozen_at(&m, &[1], 0, host, 2));
            let o = &p.ops;
            assert!(!o.iter().any(|x| matches!(x, Op::SensorSetRes { .. } | Op::VencDown { .. } | Op::VencUp { .. } | Op::PumpStop)), "{o:?}");
            let stop = pos(o, |x| matches!(x, Op::FeedStop { dst } if dst.module == Module::Venc && dst.chn == VENC_CHN_USB));
            let port = pos(o, |x| matches!(x, Op::SclConfigurePort { dev: SCL_DEV, port: SCL_PORT_USB, size, .. } if *size == Size::new(1920, 2160)));
            let start = pos(o, |x| matches!(x, Op::FeedStart { dst, way: FeedWay::Canvas { windows, .. }, .. } if dst.module == Module::Venc && !windows.is_empty()));
            assert!(stop < port && port < start, "{o:?}");
            let f = cur.feed_into(VENC_CHN_USB).unwrap().clone();
            let FeedWay::Canvas { size, live, windows, .. } = f.way;
            assert_eq!((f.src, size, live, windows.len()), (SCL_OUT_USB, Size::new(3840, 2160), Rect { x: 1920, y: 0, w: 1920, h: 2160 }, 1));
            assert!(cur.scl_stretch.device);
            assert_eq!(plan(&cur, &cur.desired.clone().unwrap().with_revision(Revision(3)), &modes()).unwrap().rung, Rung::Nothing);
            // a second frame restarts the feed with the new layout
            let p = run(&mut cur, &frozen_at(&m, &[1, 2], 0, host, 4));
            assert!(pos(&p.ops, |x| matches!(x, Op::FeedStop { .. })) < pos(&p.ops, |x| matches!(x, Op::FeedStart { .. })));
            assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::PumpStop)), "{:?}", p.ops);
            assert!(matches!(way(&cur), Some(FeedWay::Canvas { windows, .. }) if windows.len() == 2));
            // cleared: the feed stops before port 2 changes and comes back handing frames on as they are
            let p = run(&mut cur, &frozen_at(&m, &[], 0, host, 5));
            let stop = pos(&p.ops, |x| matches!(x, Op::FeedStop { .. }));
            assert!(stop < pos(&p.ops, |x| matches!(x, Op::SclDisablePort { port: SCL_PORT_USB, .. })));
            assert!(!p.ops.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::VencUp { .. })), "{:?}", p.ops);
            assert!(live_way(&way(&cur)) && cur.scl_stretch.device);
            // frames again, then the stream stops: feed, pump, encoder in that order
            run(&mut cur, &frozen_at(&m, &[1], 0, host, 6));
            let p = run(&mut cur, &frozen_at(&m, &[1], 0, None, 7));
            let (stop, pump, venc) = (pos(&p.ops, |x| matches!(x, Op::FeedStop { .. })), pos(&p.ops, |x| *x == Op::PumpStop), pos(&p.ops, |x| matches!(x, Op::VencDown { .. })));
            assert!(stop < pump && pump < venc, "{:?}", p.ops);
            assert!(cur.feed_into(VENC_CHN_USB).is_none() && cur.scl_stretch.device == (m != Monitor::Absent));
            // and everything down from a composing state
            run(&mut cur, &frozen_at(&m, &[1], 0, host, 8));
            let ops = teardown_all(&cur);
            let mut hal = TraceHal::default();
            let mut down = cur.clone();
            for op in &ops {
                down.precondition(op).unwrap();
                hal.exec(op).unwrap();
                down.apply(op);
            }
            assert!(down.feeds.is_empty() && !down.scl_stretch.device && down.venc.is_none());
        }
    }

    #[test]
    fn a_failed_hdmi_pool_rolls_back_in_order_and_a_replan_feeds_disp() {
        let m = philips();
        let mut cur = Current::default();
        let d = frozen_at(&m, &[1], 0, None, 1);
        let p = plan(&cur, &d, &modes()).unwrap();
        let pool = pos(&p.ops, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. }));
        let mut hal = TraceHal { fail_at: Some(pool), ..Default::default() };
        assert_eq!(apply(&mut hal, &mut cur, &d, &p.ops).unwrap_err().index, pool);
        assert!(cur.hdmi.is_some() && cur.pool(POOL_HDMI).is_none() && cur.scl_stretch.device);
        // the daemon's rollback: the capture side, then the screen and the scaler
        let rb = teardown_all(&cur);
        let mut c = cur.clone();
        apply(&mut TraceHal::default(), &mut c, &d, &rb).unwrap();
        assert!(c.pools.is_empty() && !c.scl_stretch.device && c.hdmi.is_none());
        // or a replan from the partial state: DISP as it is, its pool and feed made
        let p = run(&mut cur, &d);
        assert!(!p.ops.iter().any(|x| matches!(x, Op::HdmiDispUp { .. } | Op::HdmiDispDown)), "{:?}", p.ops);
        assert_eq!(on_hdmi(&cur), 1);
    }

    // ---- USB's frame HDMI's

    const UHD40: &str = "3840x2160@40 rk fastpll";

    /// HDMI 2160p30 on the 4K monitor beside a host stream, every output in one sensor mode (a stream coming or going
    /// changes no mode, as when the preset's mode is the stream's).
    fn one_mode(host: Option<(u8, u8, u32, u64)>, prefs: &Prefs, rev: u64) -> Desired {
        let f = formats();
        let h = host.map(|(fi, fr, iv, ep)| (f.resolve(fi, fr, iv).unwrap(), Epoch(ep)));
        policy().single_mode(UHD40).plan(&Inputs { monitor: &philips(), host: h.as_ref().map(|(s, e)| (s, *e)), prefs, tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
    }

    /// MJPEG 4K30 and H.264 4K30: HDMI's frame on the 4K monitor at the same level.
    const MJPEG_4K30: (u8, u8, u32) = (MJPEG, 1, 333_333);
    const H264_4K30: (u8, u8, u32) = (H264, 6, 333_333);

    fn stream((f, fr, iv): (u8, u8, u32), epoch: u64) -> Option<(u8, u8, u32, u64)> {
        Some((f, fr, iv, epoch))
    }

    /// A step of HDMI's own: its feed, pool, port, upscaler channel, DISP. (USB's encoder taken onto or off HDMI's pool
    /// is USB's step.)
    fn hdmi_step(o: &Op) -> bool {
        let up = |dev: u32, chn: u32| (dev, chn) == (SCL_UP_DEV, SCL_HDMI_UP_CHN);
        match o {
            Op::FeedStart { dst, .. } | Op::FeedStop { dst } => *dst == DISP_IN,
            Op::PoolUp { id, .. } | Op::PoolDown { id } | Op::PoolCount { id, .. } => *id == POOL_HDMI,
            Op::SclConfigurePort { dev, chn, port, .. } | Op::SclEnablePort { dev, chn, port } | Op::SclDisablePort { dev, chn, port } => (*dev, *chn, *port) == (SCL_DEV, SCL_CHN, SCL_PORT_HDMI) || up(*dev, *chn),
            Op::SclCreateChannel { dev, chn } | Op::SclDestroyChannel { dev, chn } | Op::SclStartChannel { dev, chn } | Op::SclStopChannel { dev, chn } => up(*dev, *chn),
            Op::HdmiDispUp { .. } | Op::HdmiDispDown => true,
            _ => false,
        }
    }

    fn usb_own_pool(o: &Op) -> bool {
        matches!(o, Op::PoolUp { id: POOL_USB, .. } | Op::PoolDown { id: POOL_USB } | Op::PoolCount { id: POOL_USB, .. })
    }

    #[test]
    fn usb_joins_hdmi_s_frame_only_at_its_size_level_and_view_whatever_its_rate() {
        let prefs = hdmi_first();
        let joined = |d: &Desired| d.usb.as_ref().map(|u| u.frame_of);
        // 4K30 beside 2160p30, both at the full menu: joined, whatever the codec
        let d = one_mode(stream(MJPEG_4K30, 1), &prefs, 1);
        assert_eq!(joined(&d), Some(FrameOf::Hdmi));
        // H.264 of the live picture alone: a ring from its own port instead
        assert_eq!(joined(&one_mode(stream(H264_4K30, 1), &prefs, 1)), Some(FrameOf::Ring));
        // no menu with a scene: every output shows the live picture alone, whatever its level
        let bare = Prefs { levels: crate::ui::Levels { usb: 0, ..Default::default() }, ..hdmi_first() };
        assert_eq!(joined(&one_mode(stream(MJPEG_4K30, 1), &bare, 1)), Some(FrameOf::Hdmi));
        // the same zoom on both: still one frame
        let zoomed = one_mode(stream(MJPEG_4K30, 1), &Prefs { view: View { aspect: ScreenAspect::Wide, zoom: 10 }, ..hdmi_first() }, 1);
        assert_eq!(joined(&zoomed), Some(FrameOf::Hdmi));
        // the panel's aspect is HDMI's alone: another crop
        let four3 = one_mode(stream(MJPEG_4K30, 1), &Prefs { view: View { aspect: ScreenAspect::Four3, zoom: 0 }, ..hdmi_first() }, 1);
        assert_eq!(joined(&four3), Some(FrameOf::Own));
        // another rate (4K45 asked of the one 4K40 mode, beside 2160p30): the same picture, read at its own rate
        let d = one_mode(Some((MJPEG, 1, 222_222, 1)), &prefs, 1);
        assert_eq!((joined(&d), d.usb.as_ref().map(|u| u.fps), d.hdmi.as_ref().map(|h| h.rate(d.sensor.fps))), (Some(FrameOf::Hdmi), Some(40), Some(30)));
        // another level, another size (1080p)
        let scene = Frames { scene: true, ..Frames::default() };
        let level = one_mode(stream(MJPEG_4K30, 1), &Prefs { levels: crate::ui::Levels { usb: 1, ..Default::default() }, frames: scene, ..hdmi_first() }, 1);
        assert_eq!(joined(&level), Some(FrameOf::Own));
        assert_eq!(joined(&one_mode(Some((MJPEG, 4, 333_333, 1)), &prefs, 1)), Some(FrameOf::Own));
        // no HDMI: its own
        assert_eq!(joined(&one_mode(stream(MJPEG_4K30, 1), &Prefs { hdmi_enabled: false, ..hdmi_first() }, 1)), Some(FrameOf::Own));
        // a stream starting before the `ui` laid USB out: HDMI's layout is USB's at HDMI's raster and level, so it joins
        // at once (no own pool built first)
        let uhd = Size::new(3840, 2160);
        let half = Rect { x: 0, y: 0, w: 1920, h: 2160 };
        let windows = vec![Rect { x: 1920, y: 0, w: 1920, h: 2160 }];
        let frames = Frames { hdmi: Some(Layout { raster: uhd, live: half, windows }), usb: None, media: None, scene: true };
        let d = one_mode(stream(MJPEG_4K30, 1), &Prefs { frames, ..hdmi_first() }, 1);
        let (u, h) = (d.usb.unwrap(), d.hdmi.unwrap());
        assert_eq!((u.frame_of, &u.frame, u.crop), (FrameOf::Hdmi, &h.frame, h.crop));
        assert_eq!(u.frame.window, half);
    }

    #[test]
    fn usb_comes_and_goes_as_a_reader_of_hdmi_s_pool_without_an_hdmi_step() {
        let prefs = hdmi_first();
        let mut cur = Current::default();
        run(&mut cur, &one_mode(None, &prefs, 1));
        let hdmi_pool = cur.pool(POOL_HDMI).cloned().unwrap();
        assert_eq!((hdmi_pool.readers.as_slice(), hdmi_pool.n), (&[DISP_IN][..], 5));
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        // stream on: the encoder, the depth set on HDMI's port (a new encoder device is served from then on), the reader,
        // the pump — no pool, port, upscaler or compositor of USB's own, nothing of HDMI's
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &prefs, 2));
        let o = &p.ops;
        assert!(!o.iter().any(hdmi_step), "{o:?}");
        assert!(!o.iter().any(|x| usb_own_pool(x) || matches!(x, Op::SclEnablePort { port: SCL_PORT_USB, .. } | Op::FeedStart { .. } | Op::SclCreateChannel { .. })), "{o:?}");
        let up = pos(o, |x| matches!(x, Op::VencUp { dev: VENC_DEV_JPEG, chn: VENC_CHN_USB, .. }));
        let depth = pos(o, |x| *x == Op::SetDepth { port: SCL_OUT_HDMI, user: FEED_USER_FRAMES, buffers: 5 });
        let reader = pos(o, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == mjpeg));
        let pump = pos(o, |x| matches!(x, Op::PumpStart { epoch: Epoch(1), .. }));
        assert!(up < depth && depth < reader && reader < pump, "{o:?}");
        assert!(cur.usb_joined() && cur.feed_into(VENC_CHN_USB).is_none() && cur.pool(POOL_USB).is_none());
        // the same pool: DISP's 5 canvases serve the encoder too
        assert_eq!(cur.pool(POOL_HDMI).map(|p| (p.readers.clone(), p.n)), Some((vec![DISP_IN, mjpeg], 5)));
        assert_eq!(plan(&cur, &one_mode(stream(MJPEG_4K30, 1), &prefs, 3), &modes()).unwrap().rung, Rung::Nothing);
        // stream off: pump, reader, encoder — HDMI goes on as it was
        let p = run(&mut cur, &one_mode(None, &prefs, 5));
        let o = &p.ops;
        assert!(!o.iter().any(hdmi_step), "{o:?}");
        let stop = pos(o, |x| *x == Op::PumpStop);
        let off = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let down = pos(o, |x| matches!(x, Op::VencDown { chn: VENC_CHN_USB, .. }));
        assert!(stop < off && off < down, "{o:?}");
        assert_eq!(cur.pool(POOL_HDMI), Some(&hdmi_pool));
        assert!(cur.venc.is_none() && !cur.usb_joined());
    }

    #[test]
    fn usb_h26x_of_the_live_picture_alone_is_a_ring_from_its_port() {
        let prefs = hdmi_first();
        let mut cur = Current::default();
        run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &prefs, 1));
        let hdmi_pool = cur.pool(POOL_HDMI).cloned().unwrap();
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        assert!(cur.usb_joined());
        // H.264, HDMI's frame too: the JPEG encoder off HDMI's pool and down, SCL port 2 at the stream's size, the
        // H.26x channel bound to it in a ring, the pump — no pool or feed of USB's, nothing of HDMI's
        let d = one_mode(stream(H264_4K30, 3), &prefs, 2);
        assert_eq!(d.usb.as_ref().map(|u| u.frame_of), Some(FrameOf::Ring));
        let o = &run(&mut cur, &d).ops;
        assert!(!o.iter().any(|x| hdmi_step(x) || usb_own_pool(x) || matches!(x, Op::FeedStart { .. } | Op::PoolReader { .. })), "{o:?}");
        let off = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let down = pos(o, |x| matches!(x, Op::VencDown { dev: VENC_DEV_JPEG, .. }));
        let conf = pos(o, |x| matches!(x, Op::SclConfigurePort { port: SCL_PORT_USB, size, .. } if *size == Size::new(3840, 2160)));
        let on = pos(o, |x| matches!(x, Op::SclEnablePort { port: SCL_PORT_USB, .. }));
        // the ring declares the sensor mode's rate as its source, so the encoder is handed only the stream's rate
        let up = pos(o, |x| matches!(x, Op::VencUp { dev: VENC_DEV_H26X, bound: Some(r), .. } if r.port == USB_FEED.out && r.src_fps == d.sensor.mode_max_fps));
        let pump = pos(o, |x| matches!(x, Op::PumpStart { .. }));
        assert!(off < down && down < conf && conf < up && up < on && on < pump, "the ring bound before its port starts: {o:?}");
        assert!(!cur.usb_joined() && cur.pool(POOL_USB).is_none() && cur.feed_into(VENC_CHN_USB).is_none());
        assert_eq!(cur.pool(POOL_HDMI).map(|p| p.readers.clone()), Some(vec![DISP_IN]));
        let again = plan(&cur, &one_mode(stream(H264_4K30, 3), &prefs, 3), &modes()).unwrap(); assert_eq!(again.rung, Rung::Nothing, "{:?}", again.ops);
        // zoomed past the port's own size (4K from the 4K sensor): an upscaler writes the frame, so no ring — here
        // HDMI's frame, the same zoom; unzoomed, the ring again
        let zoomed = one_mode(stream(H264_4K30, 3), &Prefs { view: View { aspect: ScreenAspect::Wide, zoom: 10 }, ..hdmi_first() }, 4);
        assert_eq!(zoomed.usb.as_ref().map(|u| u.frame_of), Some(FrameOf::Hdmi));
        run(&mut cur, &zoomed);
        assert!(cur.usb_joined() && cur.venc.as_ref().is_some_and(|v| v.bound.is_none()) && !cur.scl.ports[SCL_PORT_USB as usize].enabled);
        run(&mut cur, &one_mode(stream(H264_4K30, 3), &prefs, 4));
        assert!(cur.pool(POOL_USB).is_none() && cur.venc.as_ref().is_some_and(|v| v.bound.map(|r| r.port) == Some(USB_FEED.out)));
        // back to MJPEG: the ring's encoder down before its port, then HDMI's reader as before
        let o = &run(&mut cur, &one_mode(stream(MJPEG_4K30, 4), &prefs, 5)).ops;
        // back to MJPEG: the ring's encoder down with its port (one step), then HDMI's reader as before
        let down = pos(o, |x| matches!(x, Op::VencDown { dev: VENC_DEV_H26X, .. }));
        let reader = pos(o, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == mjpeg));
        assert!(down < reader && !o.iter().any(|x| matches!(x, Op::SclDisablePort { port: SCL_PORT_USB, .. })), "{o:?}");
        // stream off from a ring: encoder, then its port
        run(&mut cur, &one_mode(stream(H264_4K30, 5), &prefs, 6));
        let o = &run(&mut cur, &one_mode(None, &prefs, 7)).ops;
        pos(o, |x| matches!(x, Op::VencDown { chn: VENC_CHN_USB, .. }));
        assert!(!o.iter().any(|x| matches!(x, Op::SclDisablePort { port: SCL_PORT_USB, .. })), "{o:?}");
        assert_eq!(cur.pool(POOL_HDMI).map(|p| p.n), Some(hdmi_pool.n));
        assert!(cur.venc.is_none() && !cur.scl.ports[SCL_PORT_USB as usize].enabled);
        // H.264 4K at its fastest offered rate (35, the encoder's ceiling): a ring
        let fast = one_mode(stream((H264, 6, 285_714), 9), &prefs, 9);
        assert!(fast.usb.as_ref().is_some_and(|u| u.fps == 35 && u.frame_of == FrameOf::Ring), "{:?}", fast.usb);
        // pictures in USB's frame: its own pool, fed — no ring
        let scene = Prefs { frames: Frames { scene: true, ..Frames::default() }, ..hdmi_first() };
        let d = one_mode(stream(H264_4K30, 8), &scene, 8);
        let u = d.usb.as_ref().unwrap();
        assert!(u.frame_of != FrameOf::Ring || u.frame.way == Way::Live, "{u:?}");
    }

    #[test]
    fn usb_moves_between_its_own_pool_and_hdmi_s_giving_memory_before_taking_it() {
        let level = |usb: u8| Prefs { levels: crate::ui::Levels { usb, ..Default::default() }, frames: Frames { scene: true, ..Frames::default() }, ..hdmi_first() };
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        let mut cur = Current::default();
        run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &level(3), 1));
        assert!(cur.usb_joined());
        // the menu's level on USB changes: its own frame again — off HDMI's pool, then its own pool, port and feed; the
        // encoder and the pump run on
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &level(1), 2));
        let o = &p.ops;
        assert!(!o.iter().any(|x| hdmi_step(x) || matches!(x, Op::VencDown { .. } | Op::VencUp { .. } | Op::PumpStop | Op::PumpStart { .. })), "{o:?}");
        let off = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let pool = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_USB, .. }));
        let port = pos(o, |x| matches!(x, Op::SclEnablePort { port: SCL_PORT_USB, .. }));
        let feed = pos(o, |x| matches!(x, Op::FeedStart { dst, .. } if *dst == mjpeg));
        assert!(off < pool && pool < port && port < feed, "{o:?}");
        assert!(!cur.usb_joined() && cur.pool(POOL_HDMI).is_some_and(|p| p.readers == [DISP_IN]));
        // and back: its own feed, pool and port go before it reads HDMI's canvases (no new encoder: no depth to set)
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &level(3), 3));
        let o = &p.ops;
        assert!(!o.iter().any(|x| hdmi_step(x) || matches!(x, Op::VencDown { .. } | Op::VencUp { .. } | Op::PumpStop | Op::PumpStart { .. } | Op::SetDepth { .. })), "{o:?}");
        let stop = pos(o, |x| matches!(x, Op::FeedStop { dst } if *dst == mjpeg));
        let pool = pos(o, |x| *x == Op::PoolDown { id: POOL_USB });
        let port = pos(o, |x| matches!(x, Op::SclDisablePort { port: SCL_PORT_USB, .. }));
        let reader = pos(o, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == mjpeg));
        assert!(stop < port && port < pool && pool < reader, "{o:?}");
        assert!(cur.usb_joined() && !cur.scl.ports[SCL_PORT_USB as usize].enabled);

        // HDMI's timing coming to USB's size (a 1080p monitor swapped for the 4K one): USB's own 4K pool goes before
        // HDMI's 4K pool is made for DISP, which the encoder then joins at its rate in the same transition
        let on = |monitor: &Monitor, rev| {
            let f = formats();
            let (h, e) = (f.resolve(MJPEG, 1, 333_333).unwrap(), Epoch(1));
            policy().single_mode(UHD40).plan(&Inputs { monitor, host: Some((&h, e)), prefs: &hdmi_first(), tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let mut cur = Current::default();
        run(&mut cur, &on(&fhd(), 1));
        assert_eq!((cur.hdmi.as_ref().map(|h| h.timing.size()), cur.usb_joined()), (Some(Size::new(1920, 1080)), false));
        let p = run(&mut cur, &on(&philips(), 2));
        let o = &p.ops;
        let usb_gone = pos(o, |x| *x == Op::PoolDown { id: POOL_USB });
        let hdmi_old = pos(o, |x| *x == Op::PoolDown { id: POOL_HDMI });
        let hdmi_new = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. }));
        assert!(usb_gone < hdmi_new && hdmi_old < hdmi_new, "{o:?}");
        assert_eq!(o[hdmi_new], Op::PoolUp { id: POOL_HDMI, src: SCL_OUT_HDMI, readers: vec![DISP_IN], size: Size::new(3840, 2160), rows: 2176, n: 5 });
        let joins = pos(o, |x| *x == Op::PoolReader { id: POOL_HDMI, reader: mjpeg, fps: 30 });
        assert!(hdmi_new < joins && !o.iter().any(|x| matches!(x, Op::VencDown { .. })), "the encoder stays and joins the new pool: {o:?}");
        assert!(cur.usb_joined());
        // and away again: USB off HDMI's pool, HDMI's 4K pool down, then the 1080p one and USB's own 4K one
        let p = run(&mut cur, &on(&fhd(), 3));
        let o = &p.ops;
        let off = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let hdmi_old = pos(o, |x| *x == Op::PoolDown { id: POOL_HDMI });
        let hdmi_new = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. }));
        let usb_own = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_USB, .. }));
        assert!(off < hdmi_old && hdmi_old < hdmi_new && hdmi_old < usb_own, "{o:?}");
        assert!(!cur.usb_joined() && cur.feed_into(VENC_CHN_USB).is_some());
    }

    #[test]
    fn hdmi_going_away_while_usb_reads_its_pool_moves_usb_to_its_own_in_the_same_transition() {
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        let mut cur = Current::default();
        run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 1));
        assert!(cur.usb_joined());
        // HDMI off: USB off its pool first (its encoder never without a channel nor on a closed pool's allocator), HDMI
        // gives its canvases back, then USB's own path; the encoder and the pump stay
        let off = Prefs { hdmi_enabled: false, ..hdmi_first() };
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &off, 2));
        let o = &p.ops;
        assert!(!o.iter().any(|x| matches!(x, Op::VencDown { .. } | Op::VencUp { .. } | Op::PumpStop | Op::PumpStart { .. })), "{o:?}");
        let unreader = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let disp_feed = pos(o, |x| *x == Op::FeedStop { dst: DISP_IN });
        let hdmi_pool = pos(o, |x| *x == Op::PoolDown { id: POOL_HDMI });
        let disp = pos(o, |x| *x == Op::HdmiDispDown);
        let usb_pool = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_USB, .. }));
        let usb_feed = pos(o, |x| matches!(x, Op::FeedStart { dst, .. } if *dst == mjpeg));
        assert!(unreader < disp_feed && disp_feed < hdmi_pool && hdmi_pool < disp, "{o:?}");
        assert!(hdmi_pool < usb_pool && usb_pool < usb_feed, "give before take: {o:?}");
        assert!(cur.hdmi.is_none() && cur.pool(POOL_HDMI).is_none() && cur.pool(POOL_USB).is_some() && cur.pump.is_some());
        // HDMI back: USB's own pool goes before HDMI's comes, with the encoder as its reader
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 3));
        let o = &p.ops;
        let usb_pool = pos(o, |x| *x == Op::PoolDown { id: POOL_USB });
        let hdmi_pool = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. }));
        assert!(usb_pool < hdmi_pool, "{o:?}");
        assert!(cur.usb_joined() && cur.pool(POOL_USB).is_none());
        // with the monitor gone instead (another class, the same mode here): the same order
        let absent = |rev| {
            let f = formats();
            let h = f.resolve(MJPEG, 1, 333_333).unwrap();
            policy().single_mode(UHD40).plan(&Inputs { monitor: &Monitor::Absent, host: Some((&h, Epoch(1))), prefs: &hdmi_first(), tuning: &Default::default() }, &modes(), Revision(rev)).unwrap()
        };
        let p = run(&mut cur, &absent(4));
        let o = &p.ops;
        let unreader = pos(o, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let hdmi_pool = pos(o, |x| *x == Op::PoolDown { id: POOL_HDMI });
        let usb_pool = pos(o, |x| matches!(x, Op::PoolUp { id: POOL_USB, .. }));
        assert!(unreader < hdmi_pool && hdmi_pool < usb_pool, "{o:?}");
    }

    #[test]
    fn a_rebuild_a_rollback_or_a_lost_encoder_takes_usb_off_hdmi_s_pool_before_its_encoder_goes() {
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        let mut cur = Current::default();
        run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 1));
        // the encoder may not go while a pool still hands it canvases
        assert!(cur.precondition(&Op::VencDown { dev: VENC_DEV_JPEG, chn: VENC_CHN_USB }).is_err());
        assert!(cur.precondition(&Op::PoolUnreader { id: POOL_HDMI, reader: DISP_IN }).is_err(), "DISP is the pool's own reader");
        assert!(cur.precondition(&Op::PoolReader { id: POOL_HDMI, reader: mjpeg, fps: 30 }).is_err(), "a reader once");
        // everything down (a rollback): consumers first, every step valid on the state before it
        let ops = teardown_all(&cur);
        let mut c = cur.clone();
        apply(&mut TraceHal::default(), &mut c, cur.desired.as_ref().unwrap(), &ops).unwrap();
        let off = pos(&ops, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg });
        let down = pos(&ops, |x| matches!(x, Op::VencDown { chn: VENC_CHN_USB, .. }));
        let pool = pos(&ops, |x| *x == Op::PoolDown { id: POOL_HDMI });
        assert!(off < down && down < pool, "{ops:?}");
        // a lost encoder: it alone goes, off the pool first, and comes back as a reader — HDMI untouched
        let ops = teardown_encoder(&cur, VENC_CHN_USB);
        assert_eq!(ops, vec![Op::PumpStop, Op::PoolUnreader { id: POOL_HDMI, reader: mjpeg }, Op::VencDown { dev: VENC_DEV_JPEG, chn: VENC_CHN_USB }]);
        apply(&mut TraceHal::default(), &mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 1), &ops).unwrap();
        cur.desired = None;
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 2));
        assert!(!p.ops.iter().any(hdmi_step), "{:?}", p.ops);
        assert!(p.ops.iter().any(|x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == mjpeg)) && cur.usb_joined());
        // a sensor mode switch: HDMI's pool comes back from the port that now writes it, and the encoder reads it once
        // it is made
        let d = want(&philips(), stream(MJPEG_4K30, 1), 3);
        assert_eq!(d.usb.as_ref().map(|u| u.frame_of), Some(FrameOf::Hdmi));
        let mut cur = Current::default();
        let fhd_preset = Prefs { boot_preset: BootPreset::Fhd, ..hdmi_first() };
        run(&mut cur, &policy().plan(&Inputs { monitor: &philips(), host: None, prefs: &fhd_preset, tuning: &Default::default() }, &modes(), Revision(1)).unwrap());
        let p = run(&mut cur, &d);
        assert_eq!(p.rung, Rung::Switch);
        let o = &p.ops;
        let pool = o.iter().rposition(|x| matches!(x, Op::PoolUp { id: POOL_HDMI, .. })).unwrap();
        let up = pos(o, |x| matches!(x, Op::VencUp { chn: VENC_CHN_USB, .. }));
        let reader = pos(o, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == mjpeg));
        assert!(pool < reader && up < reader, "{o:?}");
        assert!(cur.usb_joined());
    }

    // ---- one pool per distinct frame ----

    const MEDIA_H264: ChnPort = venc_in(VENC_DEV_H26X, VENC_CHN_MEDIA);
    const SNAP: ChnPort = venc_in(VENC_DEV_JPEG, VENC_CHN_SNAP);

    /// A 4K recording beside HDMI 2160p and a host stream on the 4K monitor, in one sensor mode; `edit` changes the prefs.
    fn rec_beside(host: Option<(u8, u8, u32, u64)>, edit: impl Fn(&mut Prefs), rev: u64) -> Desired {
        let mut prefs = Prefs { recording: Some(crate::policy::Recording { size: Size::new(3840, 2160), path: "/mnt/VIDEO/V00002.mp4".into() }), ..hdmi_first() };
        edit(&mut prefs);
        one_mode(host, &prefs, rev)
    }

    fn file_untouched(o: &[Op]) -> bool {
        !o.iter().any(|x| matches!(x, Op::RecordStop | Op::RecordStart { .. } | Op::VencDown { chn: VENC_CHN_MEDIA, .. } | Op::VencUp { chn: VENC_CHN_MEDIA, .. }))
    }

    #[test]
    fn a_recording_whose_frame_is_hdmi_s_reads_hdmi_s_pool_beside_usb() {
        let mjpeg = venc_in(VENC_DEV_JPEG, VENC_CHN_USB);
        let mut cur = Current::default();
        run(&mut cur, &one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 1));
        assert!(cur.usb_joined());
        // the recording starts at 25: HDMI hands at 25 too, USB takes 25 — one frame for the three
        let d = rec_beside(stream(MJPEG_4K30, 1), |_| {}, 2);
        assert_eq!((d.sensor.fps, d.usb.as_ref().map(|u| (u.fps, u.frame_of)), d.media.as_ref().map(|m| (m.fps, m.frame_of))), (25, Some((25, FrameOf::Hdmi)), Some((25, FrameOf::Hdmi))));
        run(&mut cur, &d);
        let pool = cur.pool(POOL_HDMI).unwrap();
        assert_eq!((pool.readers.as_slice(), pool.n), ([DISP_IN, mjpeg, MEDIA_H264].as_slice(), 5));
        assert!(cur.pool(POOL_USB).is_none() && cur.pool(POOL_MEDIA).is_none());
        assert!(!cur.scl.ports[SCL_PORT_USB as usize].enabled && !cur.scl.ports[SCL_PORT_MEDIA as usize].enabled);
        assert!(cur.recording.is_some() && cur.feed_into(VENC_CHN_MEDIA).is_none() && cur.media_joined() == Some(POOL_HDMI));
        assert_eq!(plan(&cur, &d.clone().with_revision(Revision(3)), &modes()).unwrap().rung, Rung::Nothing);

        // the host stream stops and starts again: USB's encoder alone goes and comes, the file and HDMI untouched
        let p = run(&mut cur, &rec_beside(None, |_| {}, 4));
        assert!(file_untouched(&p.ops) && !p.ops.iter().any(hdmi_step), "{:?}", p.ops);
        assert_eq!(cur.pool(POOL_HDMI).map(|p| p.readers.clone()), Some(vec![DISP_IN, MEDIA_H264]));
        let p = run(&mut cur, &rec_beside(stream(MJPEG_4K30, 2), |_| {}, 5));
        assert!(file_untouched(&p.ops) && !p.ops.iter().any(hdmi_step), "{:?}", p.ops);
        assert!(cur.usb_joined() && cur.media_joined() == Some(POOL_HDMI));

        // the recording stops: its encoder off HDMI's pool before it goes (the rate back to 30 re-feeds DISP and USB)
        let p = run(&mut cur, &one_mode(stream(MJPEG_4K30, 2), &hdmi_first(), 6));
        let off = pos(&p.ops, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: MEDIA_H264 });
        let down = pos(&p.ops, |x| matches!(x, Op::VencDown { chn: VENC_CHN_MEDIA, .. }));
        assert!(pos(&p.ops, |x| *x == Op::RecordStop) < off && off < down, "{:?}", p.ops);
        assert!(cur.usb_joined() && cur.media.is_none() && cur.pool(POOL_HDMI).is_some_and(|p| p.readers.len() == 2));
        // and everything down from the joined state keeps the hardware's order
        run(&mut cur, &rec_beside(stream(MJPEG_4K30, 2), |_| {}, 7));
        let ops = teardown_all(&cur);
        apply(&mut TraceHal::default(), &mut cur, &one_mode(None, &hdmi_first(), 8), &ops).unwrap();
        assert!(cur.pools.is_empty() && cur.media.is_none() && cur.venc.is_none());
    }

    #[test]
    fn a_recording_leaves_a_frame_that_stops_being_its_own_without_cutting_the_file() {
        let mut cur = Current::default();
        run(&mut cur, &rec_beside(stream(MJPEG_4K30, 1), |_| {}, 1));
        assert_eq!(cur.media_joined(), Some(POOL_HDMI));
        // a menu with a scene comes, the recording at level 0 and the outputs at 3: its frame is its own — its port and
        // pool, the encoder and the file going on
        let scene = |p: &mut Prefs| p.frames.scene = true;
        let d = rec_beside(stream(MJPEG_4K30, 1), scene, 2);
        assert_eq!(d.media.as_ref().map(|m| m.frame_of), Some(FrameOf::Own));
        let p = run(&mut cur, &d);
        assert!(file_untouched(&p.ops) && !p.ops.iter().any(hdmi_step), "{:?}", p.ops);
        // its own pool and port first, then the switch: the encoder reads HDMI's canvases until its port writes
        let own = pos(&p.ops, |x| matches!(x, Op::PoolUp { id: POOL_MEDIA, .. }));
        let port = pos(&p.ops, |x| matches!(x, Op::SclEnablePort { port: SCL_PORT_MEDIA, .. }));
        let off = pos(&p.ops, |x| *x == Op::PoolUnreader { id: POOL_HDMI, reader: MEDIA_H264 });
        let on = pos(&p.ops, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_MEDIA && *r == MEDIA_H264));
        let feed = pos(&p.ops, |x| matches!(x, Op::FeedStart { dst, .. } if *dst == MEDIA_H264));
        assert!(own < port && port < off && off < on && on < feed, "{:?}", p.ops);
        assert!(cur.media_joined().is_none() && cur.usb_joined() && cur.pool(POOL_MEDIA).is_some_and(|p| p.n == 4));
        // the scene gone: back onto HDMI's pool, its own port and pool gone first
        let p = run(&mut cur, &rec_beside(stream(MJPEG_4K30, 1), |_| {}, 3));
        assert!(file_untouched(&p.ops) && !p.ops.iter().any(hdmi_step), "{:?}", p.ops);
        let pool = pos(&p.ops, |x| *x == Op::PoolDown { id: POOL_MEDIA });
        let port = pos(&p.ops, |x| matches!(x, Op::SclDisablePort { port: SCL_PORT_MEDIA, .. }));
        let on = pos(&p.ops, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == POOL_HDMI && *r == MEDIA_H264));
        assert!(port < pool && pool < on, "the port stops writing before its pool goes: {:?}", p.ops);
        let before = rec_beside(stream(MJPEG_4K30, 1), scene, 9);
        let mut c = Current::default();
        run(&mut c, &before);
        let ops = plan(&c, &rec_beside(stream(MJPEG_4K30, 1), |_| {}, 10), &modes()).unwrap().ops;
        assert!(crate::heap::peak(&c, &ops) <= crate::heap::video(&c).total(), "joining takes no memory at any step");
        assert_eq!(cur.media_joined(), Some(POOL_HDMI));
        // with the scene, the recording at the outputs' level: one frame again
        let same = rec_beside(stream(MJPEG_4K30, 1), |p| { p.frames.scene = true; p.levels.media = 3 }, 4);
        assert_eq!(same.media.as_ref().map(|m| m.frame_of), Some(FrameOf::Hdmi));
    }

    #[test]
    fn a_photo_during_a_recording_is_one_more_reader_of_the_recording_s_pool() {
        let photo = |p: &mut Prefs| p.photo = Some(Size::new(3840, 2160));
        for scene in [false, true] {
            let edit = |p: &mut Prefs| { p.frames.scene = scene; };
            let mut cur = Current::default();
            run(&mut cur, &rec_beside(stream(MJPEG_4K30, 1), edit, 1));
            let read = if scene { POOL_MEDIA } else { POOL_HDMI };
            let p = run(&mut cur, &rec_beside(stream(MJPEG_4K30, 1), |p| { edit(p); photo(p) }, 2));
            assert!(file_untouched(&p.ops), "{:?}", p.ops);
            let up = pos(&p.ops, |x| matches!(x, Op::VencUp { chn: VENC_CHN_SNAP, .. }));
            let on = pos(&p.ops, |x| matches!(x, Op::PoolReader { id: i, reader: r, .. } if *i == read && *r == SNAP));
            assert!(up < on && cur.snap_pool() == Some(read), "{:?}", p.ops);
            assert!(!p.ops.iter().any(|x| matches!(x, Op::FeedStop { .. } | Op::FeedStart { .. } | Op::PoolCount { .. })), "nothing re-fed: {:?}", p.ops);
            // taken: off the pool, then down
            let p = run(&mut cur, &rec_beside(stream(MJPEG_4K30, 1), edit, 3));
            let off = pos(&p.ops, |x| *x == Op::PoolUnreader { id: read, reader: SNAP });
            let down = pos(&p.ops, |x| matches!(x, Op::VencDown { chn: VENC_CHN_SNAP, .. }));
            assert!(off < down && file_untouched(&p.ops) && cur.snap.is_none(), "{:?}", p.ops);
        }
    }

    #[test]
    fn with_no_monitor_a_recording_reads_usb_s_pool_and_keeps_its_file_when_usb_stops() {
        let no_hdmi = |p: &mut Prefs| p.hdmi_enabled = false;
        let mut cur = Current::default();
        let d = rec_beside(stream(MJPEG_4K30, 1), no_hdmi, 1);
        assert_eq!(d.media.as_ref().map(|m| m.frame_of), Some(FrameOf::Usb));
        run(&mut cur, &d);
        // USB's pool for both readers: H.264's four canvases
        assert_eq!(cur.pool(POOL_USB).map(|p| (p.readers.len(), p.n)), Some((2, 4)));
        assert_eq!(cur.media_joined(), Some(POOL_USB));
        // the stream stops: the recording takes its own port and pool, the file goes on
        let p = run(&mut cur, &rec_beside(None, no_hdmi, 2));
        assert!(file_untouched(&p.ops), "{:?}", p.ops);
        let off = pos(&p.ops, |x| *x == Op::PoolUnreader { id: POOL_USB, reader: MEDIA_H264 });
        let own = pos(&p.ops, |x| matches!(x, Op::PoolUp { id: POOL_MEDIA, .. }));
        assert!(off < own, "{:?}", p.ops);
        assert!(cur.media_joined().is_none() && cur.pool(POOL_USB).is_none() && cur.venc.is_none());
    }

    #[test]
    fn a_failure_at_any_step_of_a_join_or_a_leave_goes_back_to_the_state_before() {
        let scene = |p: &mut Prefs| p.frames.scene = true;
        let cases: Vec<(Desired, Desired)> = vec![
            (one_mode(stream(MJPEG_4K30, 1), &hdmi_first(), 1), rec_beside(stream(MJPEG_4K30, 1), |_| {}, 2)),
            (rec_beside(stream(MJPEG_4K30, 1), |_| {}, 1), rec_beside(stream(MJPEG_4K30, 1), scene, 2)),
            (rec_beside(stream(MJPEG_4K30, 1), scene, 1), rec_beside(stream(MJPEG_4K30, 1), |_| {}, 2)),
            (rec_beside(stream(MJPEG_4K30, 1), |_| {}, 1), rec_beside(stream(MJPEG_4K30, 1), |p| p.photo = Some(Size::new(3840, 2160)), 2)),
            (rec_beside(stream(MJPEG_4K30, 1), |p| p.hdmi_enabled = false, 1), rec_beside(None, |p| p.hdmi_enabled = false, 2)),
        ];
        for (from, to) in cases {
            let mut start = Current::default();
            run(&mut start, &from);
            let ops = plan(&start, &to, &modes()).unwrap().ops;
            for k in 0..ops.len() {
                let mut cur = start.clone();
                let e = apply(&mut TraceHal { fail_at: Some(k), ..Default::default() }, &mut cur, &to, &ops).unwrap_err();
                assert_eq!(e.index, k);
                let back = plan(&cur, &from, &modes()).unwrap();
                apply(&mut TraceHal::default(), &mut cur, &from, &back.ops).unwrap_or_else(|e| panic!("failed at step {k} ({:?}); back {:?}: {e:?}", ops[k], back.ops));
                cur.codec_device = start.codec_device;
                assert_eq!(cur.pools, start.pools, "failed at step {k} ({:?}); back: {:?}", ops[k], back.ops);
                assert_eq!((cur.venc.is_some(), cur.media.is_some(), cur.snap.is_some(), cur.recording.is_some()), (start.venc.is_some(), start.media.is_some(), start.snap.is_some(), start.recording.is_some()));
            }
        }
    }
}
