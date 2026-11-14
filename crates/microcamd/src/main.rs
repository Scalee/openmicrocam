//! microcamd — the camera daemon. Owns the media pipeline (sensor → ISP → scaler → HDMI and USB), speaks to the UVC
//! gadget, serves the control socket. See docs/ARCHITECTURE.md.

mod card;
mod config;
mod control;
mod controls;
mod files;
mod followers;
mod keys;
mod logger;
mod mem;
mod monitor;
mod pipeline;
mod player;
mod recorder;
mod res;
mod running;
mod ui;
mod uvc;
mod watchdog;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use microcam_core::formats::{FormatTable, STOCK_GADGET};
use microcam_core::modes::{ModeTable, ParsedModes};
use microcam_core::policy::Policy;
use microcam_core::protocol::ListInUse;

use crate::config::Config;

/// «microcam-build <commit>[-dirty] <time>» (build.rs); `microcam status` shows what follows the prefix.
pub const BUILD: &str = concat!("microcam-build ", env!("MICROCAM_BUILD"));

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("abi-check") {
        return abi_check();
    }
    let cfg_path = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "/customer/microcam/config.toml".into()));
    let (cfg, tables) = match Config::load(&cfg_path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("microcamd: {e}");
            return ExitCode::from(2);
        }
    };
    logger::init(&cfg.log.level);
    log::info!("microcamd {} build {} starting, config {}", env!("CARGO_PKG_VERSION"), &BUILD["microcam-build ".len()..], cfg_path.display());
    match run(cfg, tables) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("microcamd: {e}");
            ExitCode::from(1)
        }
    }
}

fn read(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The list a loaded kernel module shows microcamd as a read-only parameter (`/sys/module/<module>/parameters/<list>`).
enum ModuleList<T> {
    Taken(T),
    /// the module gives none: the stock modules
    None,
    /// one microcamd cannot use, and why, in plain words with the file
    Unusable(String),
}

fn module_list<T>(file: &std::path::Path, parse: impl FnOnce(&str) -> Result<T, String>) -> ModuleList<T> {
    match std::fs::read_to_string(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ModuleList::None,
        Err(e) => ModuleList::Unusable(format!("{}: unreadable ({e})", file.display())),
        Ok(text) => match parse(&text) {
            Ok(t) => {
                log::info!("{}: the module's list taken", file.display());
                ModuleList::Taken(t)
            }
            Err(e) => ModuleList::Unusable(format!("{}: a list microcamd cannot use ({e})", file.display())),
        },
    }
}

/// The vendor MI libraries carry no NEEDED entries: every libc/libm/librt symbol they use must be in the process's
/// global scope. The linker drops libraries our own code does not reference, so load them here, globally, before the
/// first MI call (libispalgo died on `pow` otherwise, 2026-10-24).
fn preload_system_libs() {
    for name in [c"libm.so.6", c"librt.so.1", c"libpthread.so.0", c"libdl.so.2"] {
        // SAFETY: dlopen of a system library by name; the handle is intentionally kept for the process lifetime.
        let h = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
        if h.is_null() {
            log::error!("preload {}: dlopen failed", name.to_string_lossy());
        }
    }
}

fn run(cfg: Config, tables: PathBuf) -> Result<(), String> {
    // armed before the first MI call: a hang anywhere from here on ends in a reset
    let health = watchdog::Health::new();
    health.ok();
    let wd = watchdog::start(&cfg.watchdog.device, health.clone());
    preload_system_libs();
    let mut prefs = cfg.prefs();
    let mut policy = Policy::parse(&read(&tables.join("policy.toml"))?).map_err(|e| e.to_string())?;

    // hardware
    mi::sys::init().map_err(|e| e.to_string())?;
    log::info!("MI_SYS {}", mi::sys::version().unwrap_or_default());
    let live = mi::sensor::modes(microcam_core::reconcile::topo::PAD).map_err(|e| format!("sensor mode list: {e}"))?;
    log::info!("driver lists {} sensor modes", live.len());
    // the boot loads the sensor driver and the gadget before microcamd; neither is reloaded under it
    let mut lists = Vec::new();
    let modes = match module_list(&cfg.paths.modes, |t| ModeTable::parse(t).and_then(|p| p.bind(&live)).map_err(|e| e.to_string())) {
        ModuleList::Taken(t) => t,
        list => {
            let t = ParsedModes::conservative(&live).ok_or("driver lists no mode 0")?;
            let m = t.iter().next().expect("one mode");
            let size = format!("{}x{}@{}", m.width, m.height, m.max_fps);
            let mode0 = format!("the driver's mode 0 only, {size}, at the stock driver's timing");
            let (used, anomaly) = match list {
                ModuleList::Unusable(why) => (mode0, Some(why)),
                _ if ParsedModes::is_stock_driver(&live) => (format!("the stock driver's one mode, {size}"), None),
                _ => (mode0, Some(format!("the loaded driver gives no list ({} absent) and is not the stock one: it lists {} modes", cfg.paths.modes.display(), live.len()))),
            };
            lists.push(ListInUse { table: "sensor mode list".into(), used, anomaly });
            policy = policy.single_mode(&m.name);
            t
        }
    };
    let formats = Arc::new(match module_list(&cfg.paths.formats, |t| FormatTable::parse(t).map_err(|e| e.to_string())) {
        ModuleList::Taken(f) => f,
        ModuleList::None => {
            // the gadget's own sysfs directory: /sys/module/g_webcam/parameters/formats → /sys/module/g_webcam/srcversion
            let src = cfg.paths.formats.parent().and_then(|p| p.parent()).map(|m| m.join("srcversion"));
            let srcversion = src.and_then(|p| std::fs::read_to_string(p).ok()).map(|v| v.trim().to_string()).unwrap_or_default();
            let anomaly = (srcversion != STOCK_GADGET.srcversion).then(|| {
                format!("the loaded gadget gives no list ({} absent) and is not the tested stock one (srcversion «{srcversion}», the tested {}): its formats may not be these", cfg.paths.formats.display(), STOCK_GADGET.srcversion)
            });
            let stock = FormatTable::stock();
            let name = |c| match c {
                microcam_core::Codec::Mjpeg => "MJPEG",
                microcam_core::Codec::H264 => "H.264",
                microcam_core::Codec::H265 => "H.265",
            };
            let codecs: Vec<String> = stock.iter().map(|f| format!("{} ×{}", name(f.codec), f.frames.len())).collect();
            lists.push(ListInUse { table: "USB format list".into(), used: format!("the stock gadget's, carried in microcamd: {}", codecs.join(", ")), anomaly });
            stock
        }
        ModuleList::Unusable(why) => {
            lists.push(ListInUse { table: "USB format list".into(), used: "MJPEG 3840x2160@30 only".into(), anomaly: Some(why) });
            FormatTable::conservative()
        }
    });
    for l in &lists {
        match &l.anomaly {
            Some(why) => log::error!("{}: {}; in use: {}", l.table, why, l.used),
            None => log::info!("{}: {}", l.table, l.used),
        }
    }
    if let Err(e) = policy.validate(&modes) {
        log::error!("policy does not fit the mode table: {e}");
        return Err(e.to_string());
    }

    let (tx, rx) = mpsc::channel::<pipeline::Msg>();
    // the `ui`'s baked patches for each output's compositor, by pool (USB, media, HDMI)
    let patches: [Arc<mi::compose::PatchBox>; 3] = Default::default();
    let ui_tx = ui::spawn(tx.clone(), patches.clone())?;
    // what microcamd holds firm fell to a mark armed: the pipeline builds a layout it kept and wakes the `ui`. Only a
    // send: the hook runs on whichever thread freed, under whatever lock it holds (the `ui`'s state among them)
    let freed = tx.clone();
    mi::layer::on_freed(Box::new(move || {
        let _ = freed.send(pipeline::Msg::Freed);
    }));
    let rgn = match mi::rgn::init() {
        Ok(()) => true,
        Err(e) => {
            log::error!("RGN init: {e}; no ui on any output");
            false
        }
    };
    monitor::start(tx.clone()).map_err(|e| format!("HDMI init: {e}"))?;
    if let Err(e) = mi::hdmi::open() {
        log::warn!("HDMI open: {e}");
    }
    let monitor_now = monitor::probe();
    log::info!("monitor: {}", pipeline::describe_monitor(&monitor_now));

    let shared = uvc::pump::Shared::new();
    let pump_for_ctl = shared.clone();
    let counts = shared.clone();
    logger::health(move || {
        let st = counts.st.lock().unwrap();
        (st.frames, st.dropped)
    });
    // the saved settings over config.toml's defaults
    let followers = ui_tx.followers.clone();
    let files = files::Files::new(followers.clone());
    let covers = Arc::new(recorder::Covers::default());
    let running = running::Running::new(followers.clone(), files.clone());
    let controls = controls::Controls::start(cfg.paths.settings.clone(), &mut prefs, &policy, followers.clone(), running.clone(), tx.clone());
    let ctl_hw = controls.clone();
    {
        let c = Arc::downgrade(&controls);
        running.on_record(Box::new(move |on| c.upgrade().map_or_else(Vec::new, |c| c.record_runs(on))));
    }
    let player = player::Player::new(followers.clone());
    {
        let (followers, files) = (followers.clone(), files.clone());
        let moved = Box::new(move |at| files.card(at));
        let tell = Box::new(move |card| followers.tell(&microcam_core::protocol::Event::Card { card }));
        if let Err(e) = card::start(tx.clone(), moved, tell) {
            log::error!("card: not followed ({e}); photos and recordings only while the boot's card stays");
        }
    }
    let hal = mi::hal::MiHal::new(
        Box::new(uvc::pump::Handle(shared.clone())),
        {
            let (running, tx) = (running.clone(), tx.clone());
            // a writer that failed ends its recording by the road a pulled card does
            Box::new(recorder::Recorder::new(
                Arc::new(move |r| {
                    if running.record_closed(r) {
                        let _ = tx.send(pipeline::Msg::Wish(microcam_core::wish::Wish::RecordStop, None));
                    }
                }),
                covers.clone(),
            ))
        },
        player.control(),
        Box::new(move |v| ctl_hw.hardware(v)),
        Box::new(res::Source(ui_tx.res.clone())),
        {
            let tx = tx.clone();
            Arc::new(move |chn| {
                let _ = tx.send(pipeline::Msg::EncoderLost(chn));
            })
        },
        patches,
    );
    let pl = pipeline::Pipeline::new(Box::new(hal), modes, policy, prefs, monitor_now, health, ui_tx.clone(), controls.clone(), running.clone(), player.clone(), covers, rgn);
    let pipeline_thread = thread::Builder::new().name("pipeline".into()).spawn(move || pl.run(rx)).map_err(|e| e.to_string())?;

    if cfg.usb.enabled {
        uvc::pump::spawn(shared.clone());
        match uvc::Uvc::open(cfg.usb, formats.clone(), tx.clone(), shared, controls.clone()) {
            Ok(g) => g.spawn(),
            Err(e) => log::error!("UVC gadget not available ({e}); running without USB"),
        }
    }

    let quit = Arc::new(AtomicBool::new(false));
    let keys = keys::Keys::start(followers.clone());
    let ctl = Arc::new(control::Control { tx: tx.clone(), ui: ui_tx, formats, quit: quit.clone(), pump: pump_for_ctl, player, controls: controls.clone(), keys, running, files, lists });
    ctl.listen(&cfg.control.unix_socket, &cfg.control.tcp).map_err(|e| format!("control socket: {e}"))?;
    log::info!("control on {} and {}", cfg.control.unix_socket.display(), cfg.control.tcp);

    // signals → quit
    for sig in [libc::SIGINT, libc::SIGTERM] {
        let q = quit.clone();
        // SAFETY: the handler only touches an atomic.
        unsafe {
            QUIT.store(Arc::into_raw(q) as usize, Ordering::SeqCst);
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }

    while !quit.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }
    log::info!("stopping: tearing the pipeline down");
    let _ = pipeline::ask(&tx, pipeline::Msg::Shutdown, Duration::from_secs(20));
    drop(tx);
    let _ = pipeline_thread.join();
    controls.save();
    if rgn {
        let _ = mi::rgn::deinit();
    }
    let _ = mi::sys::exit();
    if let Some(w) = wd {
        w.disarm();
    }
    log::info!("bye");
    Ok(())
}

static QUIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

extern "C" fn on_signal(_: libc::c_int) {
    let p = QUIT.load(Ordering::SeqCst);
    if p != 0 {
        // SAFETY: the pointer was produced by Arc::into_raw in `run` and is never freed.
        let q = unsafe { &*(p as *const AtomicBool) };
        q.store(true, Ordering::SeqCst);
    }
}

/// Print the sizes of the MI structs this binary was compiled with, next to the values read from the vendor binary. Run on the camera after every header change.
fn abi_check() -> ExitCode {
    use std::mem::size_of;
    let rows: &[(&str, usize, usize)] = &[
        ("MI_SYS_ChnPort_t", size_of::<mi_sys::MI_SYS_ChnPort_t>(), 0x10),
        ("MI_SYS_BufInfo_t", size_of::<mi_sys::MI_SYS_BufInfo_t>(), 0x108),
        ("MI_SCL_OutPortParam_t", size_of::<mi_sys::MI_SCL_OutPortParam_t>(), 0x18),
        ("MI_VENC_ChnAttr_t", size_of::<mi_sys::MI_VENC_ChnAttr_t>(), 0x4c),
        ("MI_VENC_Stream_t", size_of::<mi_sys::MI_VENC_Stream_t>(), 0x48),
        ("MI_VENC_Pack_t", size_of::<mi_sys::MI_VENC_Pack_t>(), 0xb8),
        ("MI_VIF_GroupAttr_t", size_of::<mi_sys::MI_VIF_GroupAttr_t>(), 0x1c),
        ("MI_VIF_DevAttr_t", size_of::<mi_sys::MI_VIF_DevAttr_t>(), 0x14),
        ("MI_VIF_OutputPortAttr_t", size_of::<mi_sys::MI_VIF_OutputPortAttr_t>(), 0x14),
        ("MI_SNR_PADInfo_t", size_of::<mi_sys::MI_SNR_PADInfo_t>(), 0x34),
        ("MI_SNR_PlaneInfo_t", size_of::<mi_sys::MI_SNR_PlaneInfo_t>(), 0x48),
        ("MI_SNR_Res_t", size_of::<mi_sys::MI_SNR_Res_t>(), 0x34),
        ("MI_HDMI_Attr_t", size_of::<mi_sys::MI_HDMI_Attr_t>(), 0x30),
        ("MI_DISP_PubAttr_t", size_of::<mi_sys::MI_DISP_PubAttr_t>(), 52),
        ("MI_DISP_VideoLayerAttr_t", size_of::<mi_sys::MI_DISP_VideoLayerAttr_t>(), 20),
        ("MI_DISP_InputPortAttr_t", size_of::<mi_sys::MI_DISP_InputPortAttr_t>(), 12),
        ("MI_ISP_ChannelAttr_t", size_of::<mi_sys::MI_ISP_ChannelAttr_t>(), 0x50),
        ("MI_ISP_ChnParam_t", size_of::<mi_sys::MI_ISP_ChnParam_t>(), 0x10),
        ("MI_ISP_OutPortParam_t", size_of::<mi_sys::MI_ISP_OutPortParam_t>(), 0xc),
        ("MI_ISP_IQ_BRIGHTNESS_TYPE_t", size_of::<mi_sys::MI_ISP_IQ_BRIGHTNESS_TYPE_t>(), 0x4c),
        ("MI_ISP_IQ_SHARPNESS_TYPE_t", size_of::<mi_sys::MI_ISP_IQ_SHARPNESS_TYPE_t>(), 0x1328),
        ("MI_ISP_AWB_ATTR_TYPE_t", size_of::<mi_sys::MI_ISP_AWB_ATTR_TYPE_t>(), 0x5b8),
        ("MI_ISP_AE_EXPO_INFO_TYPE_t", size_of::<mi_sys::MI_ISP_AE_EXPO_INFO_TYPE_t>(), 0x23c),
        ("uvc_streaming_control", size_of::<mi_sys::uapi::uvc_streaming_control>(), 34),
        ("uvc_request_data", size_of::<mi_sys::uapi::uvc_request_data>(), 64),
        ("v4l2_event", size_of::<mi_sys::uapi::v4l2_event>(), 128), // 32-bit ARM: timespec is 8 bytes
    ];
    let mut bad = 0;
    for (name, got, want) in rows {
        let ok = got == want;
        if !ok {
            bad += 1;
        }
        println!("{name:32} {got:#06x} expect {want:#06x} {}", if ok { "ok" } else { "MISMATCH" });
    }
    println!("ioctl VIDIOC_DQEVENT {:#010x} UVCIOC_SEND_RESPONSE {:#010x}", uvc::ioctl::VIDIOC_DQEVENT, uvc::ioctl::UVCIOC_SEND_RESPONSE);
    if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
