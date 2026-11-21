//! microcam — CLI client of microcamd's control socket.
//!
//!   microcam [--sock PATH | --tcp HOST:PORT] status [--brief]      microcam mem [reset] [--brief]   (--brief: one line)
//!   microcam set <control> <value>    microcam adjust <control> <steps>   (a camera setting: `microcam get` lists them;
//!                                     a value is a number, true/false or one of the control's values)
//!   microcam get [control]            (the controls as they are, with the outputs, player, card and the menu's record)
//!   microcam watch                    (the snapshot, then the camera's events as they come, until stopped)
//!   microcam photo                    (a full-size photo into the card's next PHOTO/P#####.jpg; `watch` shows its outcome)
//!   microcam format                   (the SD card formatted FAT32, PHOTO/ and VIDEO/ made)
//!   microcam files [FROM [COUNT]]     (the card's photos and videos, newest first: COUNT (50) from place FROM (0))
//!   microcam reset                    (every setting back to its default, the menu's record gone, the scene taken)
//!   microcam key <key>                (a remote's key pressed and released: up, down, left, right, menu, zoom-in, …)
//!   microcam key <key> press|release  (press: held while this command runs, released when it is stopped)
//!   microcam record start | record stop   (at the `record-size` control's size; answered at once, what runs in `get`)
//!   microcam play <file> | play stop   (answered at once: the player starting or stopping in `get`, `watch` shows its outcome)
//!   microcam sim <format> <frame> <interval> | microcam sim off          (pretend a host stream)
//!   microcam scene <scene.json> [seconds] | microcam scene clear   (be the menu: show the scene while this command holds its
//!                                         connection — the given seconds, or until stopped — printing microcamd's events;
//!                                         `clear` takes the scene from whoever holds it and lets it go)
//!   microcam menu <script.jsonl>   (be the menu, Unix socket only: send the script's requests in order — `{"sleep": S}`
//!                               waits, a `resource` with `"data": "<file>"` passes the file's RGBA bytes in a memfd —
//!                               printing every reply and event with its time, until the script ends)
//!   microcam account        (the video priced beside the heap's listing, microcamd's share and its use)
//!   microcam price <request> [+ <request>…]   (what the requests would cost, nothing built: `microcam price set hdmi off
//!                                         + record start + sim 1 1 333333`)
//!   microcam rebuild        microcam log [N]        microcam quit        microcam raw '<json>'
//!   microcam log follow     (new log lines as they come, until stopped)
//!   microcam log-level [debug|info|off|…,ui=debug,kernel=off]   (live; without one, what it is)

use std::io::{BufRead, BufReader, Write};
use std::process::ExitCode;

use microcam_core::controls::{Control, Value};
use microcam_core::edit::Op;
use microcam_core::keys::{Key, Push};
use microcam_core::protocol::{OutputInfo, Request, Response, Snapshot};
use microcam_core::scene::Scene;
use microcam_core::tuning::{Knob, AS_TUNED};

enum Target {
    Unix(String),
    Tcp(String),
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut target = Target::Unix("/tmp/microcam.sock".into());
    while args.len() >= 2 && args[0].starts_with("--") {
        match args[0].as_str() {
            "--sock" => target = Target::Unix(args[1].clone()),
            "--tcp" => target = Target::Tcp(args[1].clone()),
            _ => break,
        }
        args.drain(..2);
    }
    let brief = args.iter().any(|a| a == "--brief");
    args.retain(|a| a != "--brief");
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("usage: microcam [--sock P|--tcp H:P] status|set NAME VALUE|adjust NAME STEPS|get [NAME]|watch|photo|format|files [FROM [COUNT]]|reset|key NAME [press|release]|record start|record stop|play FILE|play pause|resume|stop|seek MS|scene FILE [S]|scene clear|ui-windows|mem [reset]|account|price REQ [+ REQ…]|sim F FR IV|sim off|rebuild|log [N]|log follow|log-level [SPEC]|quit|raw JSON");
        return ExitCode::from(2);
    };
    let req = match (cmd, &args[1..]) {
        ("scene", [c]) if c == "clear" => return menu(&target, None, 0),
        ("menu", [file]) => return script(&target, file),
        ("log", [f]) if f == "follow" => return follow(&target),
        ("get", rest) if rest.len() <= 1 => return watch(&target, Some(rest.first().map(String::as_str))),
        ("watch", []) => return watch(&target, None),
        ("key", [k]) => return key(&target, k, &[Push::Press, Push::Release], false),
        ("key", [k, p]) if p == "press" => return key(&target, k, &[Push::Press], true),
        ("scene", [file, rest @ ..]) => {
            let scene: Scene = match std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("scene: {file}: {e}");
                    return ExitCode::from(2);
                }
            };
            return menu(&target, Some(scene), rest.first().and_then(|s| s.parse().ok()).unwrap_or(0));
        }
        ("price", rest) => match rest.split(|a| a == "+").map(|words| request(words, brief)).collect() {
            Ok(with) => Request::Price { with },
            Err(e) => {
                eprintln!("price: {e}");
                return ExitCode::from(2);
            }
        },
        _ => match request(&args, brief) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        },
    };
    match send(&target, &req) {
        Ok(resp) => {
            if let Some(s) = resp.status.as_ref().filter(|_| brief) {
                println!("{}", brief_status(s));
            } else if let Some(s) = &resp.status {
                println!("state: {}  uptime {} s  transitions {}", word(&s.state), s.uptime_s, s.transitions);
                println!("build: {}", if s.build.is_empty() { "unknown (built before build ids)" } else { &s.build });
                for l in &s.lists {
                    match &l.anomaly {
                        Some(why) => println!("⚠ {}: {why}; in use: {}", l.table, l.used),
                        None => println!("{}: {}", l.table, l.used),
                    }
                }
                println!("monitor: {}", s.monitor);
                println!("host stream: {}", s.host_stream.as_deref().unwrap_or("none"));
                if let Some(d) = &s.desired {
                    println!("sensor: {} @{} fps", d.sensor.mode, d.sensor.fps);
                    println!("hdmi: {}", d.hdmi.as_ref().map(|h| format!("{} window {}x{}+{}+{}", h.timing.name(), h.frame.window.w, h.frame.window.h, h.frame.window.x, h.frame.window.y)).unwrap_or("off".into()));
                    let usb = |u: &microcam_core::desired::UsbPlan| {
                        let of = match u.frame_of {
                            microcam_core::desired::FrameOf::Hdmi => ", joined HDMI's frame",
                            microcam_core::desired::FrameOf::Ring => ", a ring from its port",
                            _ => "",
                        };
                        format!("{:?} {} @{} crop {}x{}+{}+{}{of}", u.codec, u.size, u.fps, u.crop.w, u.crop.h, u.crop.x, u.crop.y)
                    };
                    println!("usb: {}", d.usb.as_ref().map(usb).unwrap_or("none".into()));
                } else {
                    println!("desired: none (not built)");
                }
                // what the ISP holds: every knob since its IQ load, and what it refused
                let isp = &s.current.isp;
                if isp.iq.is_some() {
                    let held = |k: &Knob, v: &i32| if *k == Knob::Backlight && *v == AS_TUNED { format!("{} as-tuned", k.name()) } else { format!("{} {v}", k.name()) };
                    let knobs: Vec<String> = isp.tuning.iter().map(|(k, v)| held(k, v)).collect();
                    println!("picture: {}", if knobs.is_empty() { "as the IQ file left it".into() } else { knobs.join(", ") });
                    for (k, (v, why)) in &isp.refused {
                        println!("⚠ picture: {} {v} refused: {why}", k.name());
                    }
                }
                if let Some(t) = &s.last_transition {
                    println!("last transition: {t}");
                }
                if let Some(e) = &s.last_error {
                    println!("last error: {e}");
                }
                if let Some((f, d, o)) = s.pump {
                    println!("usb pump: {f} frames queued, {d} dropped, {o} oversize");
                }
                if let Some((avg, max)) = s.usb_delay {
                    println!("usb delay: capture → USB queue avg {avg:.1} ms, max {max:.1} ms (last 10 s of the stream)");
                }
                if let Some(u) = &s.usb_ui {
                    println!("usb ui: {u}");
                }
                if let Some(u) = &s.media_ui {
                    println!("media ui: {u}");
                }
                let [h, u, m] = s.ui_levels;
                println!("ui levels: hdmi {h}, usb {u}, media {m}");
                if std::env::var("MICROCAM_JSON").is_ok() {
                    println!("{}", serde_json::to_string_pretty(&s.current).unwrap_or_default());
                }
            }
            if let Some(lines) = &resp.lines {
                for l in lines {
                    println!("{l}");
                }
            }
            if let Some(m) = &resp.message {
                println!("{m}");
            }
            if let Some(v) = &resp.value {
                println!("{v}");
            }
            if let Some(f) = &resp.files {
                println!("files {}–{} of {} (version {})", f.from + 1, f.from + f.list.len(), f.total, f.version);
                for e in &f.list {
                    println!("{:>6}  {:<16} {:>10} B", e.number, e.name, e.bytes);
                }
            }
            if let Some(a) = &resp.account {
                println!("video priced {}", owners(&a.priced));
                println!("heap held {}", owners(&a.held));
                println!("heap {} MiB: free {}; microcamd's share {}, held {}", mib(a.heap), mib(a.free), mib(a.share), mib(a.used));
            }
            if let Some(p) = &resp.price {
                println!("video after {}", owners(&p.priced));
                println!("peak {} MiB; microcamd's share after {}", mib(p.peak), mib(p.share));
                match &p.short {
                    Some(short) => println!("refused: {short}"),
                    None => println!("fits"),
                }
                for g in &p.gives {
                    println!("gives way: {} {}", g.holder.name(), match g.step {
                        microcam_core::precedence::Step::Timing(t) => t.name().to_string(),
                        s => s.word().to_string(),
                    });
                }
            }
            if resp.ok {
                if resp.status.is_none() && resp.lines.is_none() && resp.message.is_none() && resp.account.is_none() && resp.price.is_none() && resp.value.is_none() && resp.files.is_none() {
                    println!("ok");
                }
                ExitCode::SUCCESS
            } else {
                let code = resp.code.map(|c| format!(" ({})", serde_json::to_value(c).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default())).unwrap_or_default();
                eprintln!("error{code}: {}", resp.error.as_deref().unwrap_or("?"));
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("microcam: {e}");
            ExitCode::from(1)
        }
    }
}

/// A value as typed: true or false, a whole number, else a word (a choice).
fn value(word: &str) -> Value {
    match word {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        w => w.parse().map(Value::Int).unwrap_or_else(|_| Value::Text(w.to_string())),
    }
}

/// One request from its words: the command and its arguments.
fn request(words: &[String], brief: bool) -> Result<Request, String> {
    let Some(cmd) = words.first() else { return Err("an empty request".into()) };
    Ok(match (cmd.as_str(), &words[1..]) {
        ("status", _) => Request::Status,
        ("set", [name, v]) => Request::Set { name: name.clone(), value: value(v) },
        ("adjust", [name, n]) => match n.parse() {
            Ok(steps) => Request::Adjust { name: name.clone(), steps },
            Err(_) => return Err(format!("adjust: '{n}' is not a number of steps")),
        },
        ("photo", []) => Request::Photo,
        ("format", []) => Request::Format,
        ("files", rest) if rest.len() <= 2 => {
            let n = |i: usize, d: usize| rest.get(i).map_or(Ok(d), |w| w.parse().map_err(|_| format!("files: '{w}' is not a number")));
            Request::Files { from: n(0, 0)?, count: n(1, 50)? }
        }
        ("reset", []) => Request::Reset,
        ("key", [k, p]) => Request::Key { key: of_word(k).ok_or(format!("key: no key '{k}'"))?, phase: of_word(p).ok_or(format!("key: '{p}' is not press or release"))? },
        ("record", [a]) if a == "start" => Request::Record,
        ("record", [a]) if a == "stop" => Request::RecordStop,
        ("play", [a]) if a == "pause" => Request::PlayPause,
        ("play", [a]) if a == "resume" => Request::PlayResume,
        ("play", [a]) if a == "stop" => Request::PlayStop,
        ("play", [a, ms]) if a == "seek" => match ms.parse() {
            Ok(ms) => Request::PlaySeek { ms },
            Err(_) => return Err(format!("seek: '{ms}' is not milliseconds")),
        },
        ("play", [file]) => Request::Play { file: file.clone() },
        ("ui-windows", _) => Request::UiWindows,
        ("mem", []) => Request::Mem { reset: false, brief },
        ("mem", [a]) if a == "reset" => Request::Mem { reset: true, brief },
        ("sim", [off]) if off == "off" => Request::SimStream { off: true, format: 0, frame: 0, interval: 0 },
        ("sim", [f, fr, iv]) => Request::SimStream {
            off: false,
            format: f.parse().unwrap_or(1),
            frame: fr.parse().unwrap_or(1),
            interval: iv.parse().unwrap_or(0),
        },
        ("rebuild", _) => Request::Rebuild,
        ("log-level", rest) => Request::LogLevel { spec: rest.first().cloned() },
        ("log", rest) => Request::Log { lines: rest.first().and_then(|n| n.parse().ok()).unwrap_or(40) },
        ("quit", _) => Request::Quit,
        ("raw", [j]) => serde_json::from_str(j).map_err(|e| format!("bad json: {e}"))?,
        ("account", []) => Request::Account,
        _ => return Err(format!("unknown command '{}'", words.join(" "))),
    })
}

/// A closed set's member from its word on the wire (`zoom-in`, `release`).
fn of_word<T: serde::de::DeserializeOwned>(w: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(w.into())).ok()
}

/// A closed set's member as its word on the wire.
fn word(t: &impl serde::Serialize) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn mib(b: u64) -> String {
    format!("{:.1}", b as f64 / (1 << 20) as f64)
}

/// Owners and their MiB in one line, their sum first.
fn owners(o: &[(String, u64)]) -> String {
    let list: Vec<String> = o.iter().map(|(n, b)| format!("{n} {}", mib(*b))).collect();
    format!("{} MiB: {}", mib(o.iter().map(|x| x.1).sum()), list.join(", "))
}

/// `microcam status --brief`: the state, build, sensor and outputs, pump, unused module lists and error in one line.
fn brief_status(s: &microcam_core::protocol::Status) -> String {
    let mut out = format!("{} · build {} · up {} s · {} transitions", word(&s.state), s.build.split(' ').next().filter(|b| !b.is_empty()).unwrap_or("?"), s.uptime_s, s.transitions);
    if let Some(d) = &s.desired {
        out += &format!(" · sensor {} @{} · hdmi {}", d.sensor.mode, d.sensor.fps, d.hdmi.as_ref().map_or("off".into(), |h| h.timing.name()));
        out += &match &d.usb {
            Some(u) => format!(" · usb {:?} {} @{}{}", u.codec, u.size, u.fps, if u.frame_of == microcam_core::desired::FrameOf::Hdmi { " joined" } else { "" }),
            None => " · usb none".into(),
        };
    }
    if let Some((f, d, _)) = s.pump {
        out += &format!(" · pump {f} queued {d} dropped");
    }
    for l in &s.lists {
        out += &format!(" · {}{}: {}", if l.anomaly.is_some() { "⚠ " } else { "" }, l.table, if l.anomaly.is_some() { "not the module's" } else { "stock" });
    }
    out + &format!(" · error {}", s.last_error.as_deref().unwrap_or("—"))
}

/// One control in a line: its value, kind, range or values, default, and why it cannot be changed now.
fn control_line(c: &Control) -> String {
    let range = match (&c.min, &c.max) {
        (Some(min), Some(max)) => format!(" {min}…{max} by {}", c.step.unwrap_or(1)),
        _ if !c.values.is_empty() => format!(" {}", c.values.join(" · ")),
        _ => String::new(),
    };
    let kind = serde_json::to_value(c.kind).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let kept = if c.kept { "" } else { ", not kept" };
    let why = c.why.as_ref().map(|w| format!("  — unavailable: {w}")).unwrap_or_default();
    format!("{} = {}   ({kind}{range}, default {}{kept}){why}", c.name, c.value, c.default)
}

/// An output in a line: its frame, HDMI's timing or the USB stream, its level and pause.
fn output_line(o: &OutputInfo) -> String {
    let timing = o.timing.map(|t| format!(" {}", t.name())).unwrap_or_default();
    let stream = o.codec.map(|c| format!(" {} @{}", word(&c), o.fps.unwrap_or(0))).unwrap_or_default();
    let paused = o.paused.map(|p| format!(" paused ({p:?})")).unwrap_or_default();
    format!("{} {}x{}{timing}{stream} level {}{paused}", word(&o.name), o.w, o.h, o.level)
}

fn print_snapshot(s: &Snapshot) {
    for c in &s.controls {
        println!("{}", control_line(c));
    }
    let outs: Vec<String> = s.outputs.iter().map(output_line).collect();
    println!("outputs: {}", if outs.is_empty() { "none".into() } else { outs.join(", ") });
    println!("player: {}", s.player.as_ref().map_or("none".into(), |p| format!("{} {} at {} of {} ms", p.file, word(&p.state), p.ms, p.duration_ms)));
    let space = match (s.card.free, s.card.total) {
        (Some(free), Some(total)) => format!(", {:.1} of {:.1} GB free", free as f64 / 1e9, total as f64 / 1e9),
        _ => String::new(),
    };
    println!("card: {}{space}, {} files (version {})", word(&s.card.at), s.files.total, s.files.version);
    let r = &s.running;
    let rec = r.record.as_ref().map(|x| format!("recording {:?} {} for {} ms", x.state, x.file, x.ms));
    let runs: Vec<String> = r.photo.iter().map(|f| format!("photo {f}")).chain(rec).chain(r.format.then(|| "formatting".to_string())).chain(r.building.iter().map(|n| format!("building {n}"))).collect();
    println!("running: {}", if runs.is_empty() { "nothing".into() } else { runs.join(", ") });
    println!("menu's record: {}", s.record.as_ref().map_or("none".into(), |r| r.to_string()));
}

/// `get` (Some: the snapshot, one control of it when named) or `watch` (None: the snapshot, then the events until
/// stopped).
fn watch(target: &Target, get: Option<Option<&str>>) -> ExitCode {
    let run = || -> std::io::Result<ExitCode> {
        let (mut stream, mut reader) = connect(target)?;
        stream.write_all((serde_json::to_string(&Request::Watch)? + "\n").as_bytes())?;
        let r = reply(&mut reader)?;
        let Some(s) = r.snapshot else {
            eprintln!("error: {}", r.error.as_deref().unwrap_or("no snapshot"));
            return Ok(ExitCode::from(1));
        };
        match get {
            Some(Some(name)) => match s.controls.iter().find(|c| c.name == name) {
                Some(c) => println!("{}", control_line(c)),
                None => {
                    eprintln!("error: no control '{name}'");
                    return Ok(ExitCode::from(1));
                }
            },
            _ => print_snapshot(&s),
        }
        if get.is_some() {
            return Ok(ExitCode::SUCCESS);
        }
        loop {
            let mut l = String::new();
            if reader.read_line(&mut l)? == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "microcamd closed the connection"));
            }
            print!("event {l}");
            let _ = std::io::stdout().flush();
        }
    };
    run().unwrap_or_else(|e| {
        eprintln!("microcam: {e}");
        ExitCode::from(1)
    })
}

/// A key's phases sent on one connection in order; `hold`: the connection kept open until stopped, so the key stays
/// pressed (microcamd releases what a connection leaves pressed when it closes).
fn key(target: &Target, name: &str, phases: &[Push], hold: bool) -> ExitCode {
    let Some(key) = of_word::<Key>(name) else {
        eprintln!("key: no key '{name}'");
        return ExitCode::from(2);
    };
    let run = || -> std::io::Result<ExitCode> {
        let (mut stream, mut reader) = connect(target)?;
        for phase in phases {
            let req = Request::Key { key, phase: *phase };
            stream.write_all((serde_json::to_string(&req)? + "\n").as_bytes())?;
            let r = reply(&mut reader)?;
            if !r.ok {
                eprintln!("error: {}", r.error.as_deref().unwrap_or("?"));
                return Ok(ExitCode::from(1));
            }
        }
        if hold {
            println!("{name} held until stopped");
            loop {
                std::thread::park();
            }
        }
        println!("ok");
        Ok(ExitCode::SUCCESS)
    };
    run().unwrap_or_else(|e| {
        eprintln!("microcam: {e}");
        ExitCode::from(1)
    })
}

/// New log lines as they come, until stopped.
fn follow(target: &Target) -> ExitCode {
    let mut from = u64::MAX;
    loop {
        match send(target, &Request::LogSince { from }) {
            Ok(r) => {
                for l in r.lines.unwrap_or_default() {
                    println!("{l}");
                }
                if let Some(n) = r.message.and_then(|m| m.parse().ok()) {
                    from = n;
                }
            }
            Err(e) => eprintln!("microcam: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// One request and its reply.
fn send(target: &Target, req: &Request) -> std::io::Result<Response> {
    let (mut stream, mut reader) = connect(target)?;
    stream.write_all((serde_json::to_string(req)? + "\n").as_bytes())?;
    reply(&mut reader)
}

fn connect(target: &Target) -> std::io::Result<(Box<dyn ReadWrite>, BufReader<Box<dyn ReadWrite>>)> {
    Ok(match target {
        Target::Unix(p) => {
            let s = std::os::unix::net::UnixStream::connect(p)?;
            (Box::new(s.try_clone()?), BufReader::new(Box::new(s)))
        }
        Target::Tcp(a) => {
            let s = std::net::TcpStream::connect(a)?;
            (Box::new(s.try_clone()?), BufReader::new(Box::new(s)))
        }
    })
}

/// The next reply, printing the events before it.
fn reply(reader: &mut impl BufRead) -> std::io::Result<Response> {
    loop {
        let mut l = String::new();
        if reader.read_line(&mut l)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "microcamd closed the connection"));
        }
        if l.starts_with("{\"event\"") {
            print!("event {l}");
            continue;
        }
        return serde_json::from_str(&l).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}: {l}")));
    }
}

/// Be the menu: take the scene, show `scene` (none: an empty one, let go at once), and hold it `secs` seconds (0: until
/// stopped) printing microcamd's events.
fn menu(target: &Target, scene: Option<Scene>, secs: u64) -> ExitCode {
    let run = || -> std::io::Result<Response> {
        let (mut stream, mut reader) = connect(target)?;
        stream.write_all(b"{\"cmd\":\"hello\",\"take\":true}\n")?;
        let r = reply(&mut reader)?;
        if !r.ok {
            return Ok(r);
        }
        if let (Some(h), Some(s)) = (&r.hello, &r.snapshot) {
            let outs: Vec<String> = s.outputs.iter().map(output_line).collect();
            println!("menu: protocol {}, outputs: {}", h.protocol, if outs.is_empty() { "none".into() } else { outs.join(", ") });
        }
        let Some(scene) = scene else {
            return Ok(r);
        };
        let commit = Request::Commit { base: 0, ops: vec![Op::Replace { nodes: scene.nodes }] };
        stream.write_all((serde_json::to_string(&commit)? + "\n").as_bytes())?;
        let r = reply(&mut reader)?;
        if !r.ok {
            return Ok(r);
        }
        println!("revision {}, holding the scene{}", r.revision.unwrap_or(0), if secs > 0 { format!(" for {secs} s") } else { " until stopped".into() });
        let end = (secs > 0).then(|| std::time::Instant::now() + std::time::Duration::from_secs(secs));
        loop {
            let mut l = String::new();
            if let Some(end) = end {
                let left = end.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Ok(r);
                }
                reader.get_ref().timeout(left);
            }
            match reader.read_line(&mut l) {
                Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "microcamd closed the connection")),
                Ok(_) => print!("event {l}"),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(r),
                Err(e) => return Err(e),
            }
        }
    };
    match run() {
        Ok(r) if r.ok => ExitCode::SUCCESS,
        Ok(r) => {
            eprintln!("error: {}", r.error.as_deref().unwrap_or("?"));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("microcam: {e}");
            ExitCode::from(1)
        }
    }
}

/// Be the menu by a script of requests (see the usage), printing what comes back with its time since the start.
fn script(target: &Target, file: &str) -> ExitCode {
    use std::os::fd::AsRawFd;
    let Target::Unix(path) = target else {
        eprintln!("menu: pixels travel as memfds, on the Unix socket only");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("menu: {file}: {e}");
            return ExitCode::from(2);
        }
    };
    let s = match std::os::unix::net::UnixStream::connect(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("microcam: {e}");
            return ExitCode::from(1);
        }
    };
    let t0 = std::time::Instant::now();
    let reader = s.try_clone().expect("a second handle of the socket");
    std::thread::spawn(move || {
        for l in BufReader::new(reader).lines().map_while(Result::ok) {
            println!("{:8.3} < {l}", t0.elapsed().as_secs_f64());
        }
        println!("{:8.3} microcamd closed the connection", t0.elapsed().as_secs_f64());
        std::process::exit(1);
    });
    for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#')) {
        let mut v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("menu: line {}: {e}", n + 1);
                return ExitCode::from(2);
            }
        };
        if let Some(secs) = v.get("sleep").and_then(|x| x.as_f64()) {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            continue;
        }
        let data = v.as_object_mut().and_then(|o| o.remove("data")).and_then(|d| d.as_str().map(String::from));
        let out = serde_json::to_string(&v).unwrap() + "\n";
        println!("{:8.3} > {}", t0.elapsed().as_secs_f64(), out.trim_end());
        let sent = match data {
            None => (&s).write_all(out.as_bytes()),
            Some(f) => std::fs::read(&f).and_then(|bytes| {
                let fd = memfd(&bytes)?;
                send_fd(s.as_raw_fd(), out.as_bytes(), fd.as_raw_fd())
            }),
        };
        if let Err(e) = sent {
            eprintln!("menu: line {}: {e}", n + 1);
            return ExitCode::from(1);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    ExitCode::SUCCESS
}

/// A memfd holding `bytes`.
fn memfd(bytes: &[u8]) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: memfd_create takes a NUL-terminated name and returns a new descriptor or -1.
    let fd = unsafe { libc::memfd_create(c"microcam-pixels".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a new descriptor, owned from here on.
    let mut f = std::fs::File::from(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
    f.write_all(bytes)?;
    Ok(f.into())
}

/// `bytes` on the socket with `fd` passed along (SCM_RIGHTS).
fn send_fd(sock: i32, bytes: &[u8], fd: i32) -> std::io::Result<()> {
    let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut _, iov_len: bytes.len() };
    let mut control = [0u64; 4];
    // SAFETY: the header points at `iov` and `control`, which outlive the call; the control message is built inside
    // `control` by the kernel's own macros.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(4) as _;
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(4) as _;
        (libc::CMSG_DATA(c) as *mut i32).write_unaligned(fd);
        let n = libc::sendmsg(sock, &msg, 0);
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // the rest of a long line, without the descriptor
        let mut at = n as usize;
        while at < bytes.len() {
            let k = libc::write(sock, bytes[at..].as_ptr().cast(), bytes.len() - at);
            if k < 0 {
                return Err(std::io::Error::last_os_error());
            }
            at += k as usize;
        }
    }
    Ok(())
}

trait ReadWrite: std::io::Read + std::io::Write {
    /// How long a read waits (the socket's, so every handle of it).
    fn timeout(&self, d: std::time::Duration);
}

impl ReadWrite for std::os::unix::net::UnixStream {
    fn timeout(&self, d: std::time::Duration) {
        let _ = self.set_read_timeout(Some(d));
    }
}

impl ReadWrite for std::net::TcpStream {
    fn timeout(&self, d: std::time::Duration) {
        let _ = self.set_read_timeout(Some(d));
    }
}
