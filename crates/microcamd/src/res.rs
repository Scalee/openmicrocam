//! The menu's resources: pictures by the menu's own ids, from pixels it hands over, a JPEG
//! file on the camera, or the camera's picture taken by command. A resource is held in the form its use needs
//! an image the scene shows in the menu layer reads RGBA (premultiplied, what the 2D engine and
//! the CPU draw from), a full-colour image in the frame reads NV12 (what the scalers and DISP read) — each made only
//! while something uses it, one copy shared by every output, both out of microcamd's share of the media heap with the tiles. A JPEG
//! is decoded off the `ui` thread straight into the media heap, by the hardware decoder (the CPU for what it refuses):
//! RGBA at the n/8 scale nearest below the one the scene shows it at (the 2D engine stretches it the rest of the way,
//! filtered), NV12 at the size its largest window copies 1:1 (`microcam_core::frame::form_size`; the CPU at the n/8 of
//! `microcam_core::frame::eighths`); again when shown larger. An NV12 form short of room is never decoded smaller: it waits
//! for the room (`mi::layer::wake_at`); a picture prepared ahead is decoded after everything shown, into `mi::layer`'s
//! spare, which the share gives up first. Only the file's head is read until a form is wanted, and a form
//! its embedded thumbnail serves is made from that alone; our photos are written with one ([`with_thumbnail`]). A
//! camera picture exists in NV12 only, at the size the menu named. An MP4 file is its cover (`microcam_core::mp4`: our
//! recordings carry one), decoded as a JPEG of that size; one without a cover fails with «no picture». A released
//! resource goes once the current scene names it no more. They belong to the menu that declared them: a new menu, or
//! none, clears them.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use microcam_core::desired::PictureId;
use microcam_core::controls::Holder;
use microcam_core::protocol::{ResCode, ResState};
use microcam_core::{Rect, Size};
use microcam_draw::render::{Picture, Pixels};
use microcam_draw::tiny_skia as sk;
use mi::layer::Image;
use mi::picture::Frame;

/// A side of an uploaded picture.
pub const MAX_SIDE: u32 = 4096;

/// A change the store reports: a resource's new state for the menu, or only new pixels to draw.
pub enum Change {
    State(String, ResState),
    Pixels,
}

pub struct Resources {
    inner: Mutex<Inner>,
    wake: Condvar,
    tell: Box<dyn Fn(Change) + Send + Sync>,
}

/// Which of a resource's forms a job makes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Rgba,
    /// the NV12 form a frame shows
    Nv12,
    /// the NV12 form of a picture prepared ahead, held as `mi::layer`'s spare
    Ahead,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Entry>,
    /// pictures waiting for their decoder
    jobs: VecDeque<(String, Form)>,
    /// bumped by every new set of pixels: their keys (an NV12 form's key is its [`PictureId`])
    keys: u64,
    /// bumped by every clear, so a decode finished for an earlier menu is dropped
    session: u64,
}

enum Src {
    File(PathBuf),
    Pixels,
    /// the camera picture's key while it is being taken
    Camera(u64),
}

/// One form of a resource: its pixels with their key, the scale they hold and the one wanted — the RGBA form's in
/// eighths (8: the source's own size), the NV12 form's its size (none: nothing).
struct Held<P: ?Sized, S> {
    pix: Option<(Arc<P>, u64)>,
    have: S,
    want: S,
}

impl<P: ?Sized, S: Default> Default for Held<P, S> {
    fn default() -> Self {
        Self { pix: None, have: S::default(), want: S::default() }
    }
}

struct Entry {
    src: Src,
    /// the source picture's size, once known
    full: Option<(u32, u32)>,
    rgba: Held<dyn Pixels, u32>,
    /// the scale the scene last showed the RGBA form at
    shown: f32,
    /// the NV12 form a frame shows
    nv12: Held<Frame, Option<Size>>,
    /// keys of NV12 decodes a newer one replaced: the `ui` may still name one (taken before the newer came), and the
    /// newer decode serves it — a part is given in the source picture's pixels, whatever the scale
    nv12_was: Vec<u64>,
    /// the shown form found no room in microcamd's share: its size, the mark what microcamd holds firm must fall to for
    /// it to fit, and why — asked again only then
    short: Option<(Size, u64, String)>,
    /// the NV12 form cannot be made from the file (its decode failed): the picture is left out of every frame
    nv12_failed: bool,
    /// the NV12 form prepared ahead: its size and its key in `mi::layer`'s spare (None: it did not fit beside what was
    /// held, not made); not asked again while the picture is prepared at that size
    ahead: Option<(Size, Option<u64>)>,
    /// the size a decode ahead is queued or running for
    ahead_want: Option<Size>,
    /// an RGBA job (the file's head included) queued or running
    busy: bool,
    released: bool,
    failed: bool,
    /// shown no more: the precedence gave its room to the video; its window stays, black, until the menu
    /// drops it
    yielded: bool,
}

impl Entry {
    fn new(src: Src) -> Self {
        Self { src, full: None, rgba: Held::default(), shown: 0.0, nv12: Held::default(), nv12_was: Vec::new(), short: None, nv12_failed: false, ahead: None, ahead_want: None, busy: false, released: false, failed: false, yielded: false }
    }

    /// The form prepared ahead goes out of the spare (dropped), and nothing is decoded ahead for it.
    fn drop_ahead(&mut self) {
        if let Some((_, Some(key))) = self.ahead.take() {
            drop(mi::layer::spare_take(key));
        }
        self.ahead_want = None;
    }
}

impl Resources {
    /// The store and its decoder thread, its pictures out of microcamd's share; `tell` hears every change (without the store's
    /// lock held).
    pub fn spawn(tell: impl Fn(Change) + Send + Sync + 'static) -> Arc<Resources> {
        let r = Arc::new(Resources { inner: Mutex::default(), wake: Condvar::new(), tell: Box::new(tell) });
        let me = r.clone();
        thread::Builder::new().name("res".into()).spawn(move || me.decoder()).expect("the resource thread");
        r
    }

    /// A resource from straight RGBA pixels (rows without padding), ready at once (the caller tells the menu, after its
    /// reply); refused when the id is in use or the share full.
    pub fn pixels(&self, id: &str, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(format!("{w}x{h}: each side 1..{MAX_SIDE}"));
        }
        if rgba.len() < w as usize * h as usize * 4 {
            return Err(format!("{} bytes for {w}x{h} RGBA", rgba.len()));
        }
        check_new(&self.inner.lock().unwrap(), id)?;
        let mut pix = Image::new(w, h).map_err(|e| format!("{w}x{h}: {e}"))?;
        for (d, s) in pix.bytes().chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            let p = sk::ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
            d.copy_from_slice(&[p.red(), p.green(), p.blue(), p.alpha()]);
        }
        pix.sync().map_err(|e| e.to_string())?;
        let mut i = self.inner.lock().unwrap();
        check_new(&i, id)?;
        i.keys += 1;
        let key = i.keys;
        let mut e = Entry::new(Src::Pixels);
        e.full = Some((w, h));
        e.rgba = Held { pix: Some((Arc::new(pix), key)), have: 8, want: 8 };
        i.map.insert(id.into(), e);
        Ok(())
    }

    /// A resource from a JPEG file: loading now (the caller tells the menu, after its reply), ready once its header is
    /// read, its pixels when the scene shows it.
    pub fn file(&self, id: &str, path: &str) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        check_new(&i, id)?;
        let mut e = Entry::new(Src::File(path.into()));
        e.busy = true;
        i.map.insert(id.into(), e);
        Ok(())
    }

    /// The decoder starts on a file declared by [`Self::file`] (once the menu heard it is loading, so `ready` comes after).
    pub fn load(&self, id: &str) {
        self.inner.lock().unwrap().jobs.push_back((id.into(), Form::Rgba));
        self.wake.notify_one();
    }

    /// A resource the camera's picture will fill: loading now; the pipeline takes it into the picture named by the key
    /// returned ([`Self::grabbed`], [`Self::grab_failed`]).
    pub fn camera(&self, id: &str) -> Result<PictureId, String> {
        let mut i = self.inner.lock().unwrap();
        check_new(&i, id)?;
        i.keys += 1;
        let key = i.keys;
        let mut e = Entry::new(Src::Camera(key));
        e.busy = true;
        i.map.insert(id.into(), e);
        Ok(PictureId(key))
    }

    /// The camera picture `key` was taken: the resource is ready.
    pub fn grabbed(&self, key: PictureId, frame: Frame) {
        let told = {
            let mut i = self.inner.lock().unwrap();
            let Some((id, e)) = i.map.iter_mut().find(|(_, e)| matches!(e.src, Src::Camera(k) if k == key.0)) else { return };
            let (w, h) = (frame.size.w, frame.size.h);
            e.full = Some((w, h));
            let size = Some(frame.size);
            e.nv12 = Held { pix: Some((Arc::new(frame), key.0)), have: size, want: size };
            e.busy = false;
            Change::State(id.clone(), ResState::Ready { w, h })
        };
        (self.tell)(told);
    }

    /// The camera picture `key` could not be taken: the resource failed.
    pub fn grab_failed(&self, key: PictureId, code: ResCode, error: String, holders: Vec<Holder>) {
        let told = {
            let mut i = self.inner.lock().unwrap();
            let Some((id, e)) = i.map.iter_mut().find(|(_, e)| matches!(e.src, Src::Camera(k) if k == key.0)) else { return };
            e.busy = false;
            e.failed = true;
            Change::State(id.clone(), ResState::Failed { code, error, holders })
        };
        (self.tell)(told);
    }

    /// The bytes of microcamd's share each picture the menu holds takes (its RGBA and NV12 forms; a form ahead is spare,
    /// dropped when room is wanted), by id; released ones are going and not named.
    pub fn held(&self) -> Vec<(String, u64)> {
        let i = self.inner.lock().unwrap();
        i.map
            .iter()
            .filter(|(_, e)| !e.released)
            .map(|(k, e)| (k.clone(), e.rgba.pix.as_ref().map_or(0, |(p, _)| p.rgba().len() as u64) + e.nv12.pix.as_ref().map_or(0, |(f, _)| f.held())))
            .collect()
    }

    /// The pictures give their room to the video : each the menu holds is shown no more — its forms dropped,
    /// its windows black in the frames made from now on — and the menu told it failed for lack of resources, `holders`
    /// naming what took the room.
    pub fn give_way(&self, holders: Vec<Holder>) {
        let ids: Vec<String> = {
            let mut i = self.inner.lock().unwrap();
            let mut ids = Vec::new();
            for (k, e) in i.map.iter_mut().filter(|(_, e)| !e.released && !e.failed && !e.yielded) {
                e.yielded = true;
                e.rgba = Held::default();
                e.nv12 = Held::default();
                e.short = None;
                e.drop_ahead();
                ids.push(k.clone());
            }
            ids
        };
        for id in ids {
            let error = "the video took the room of the menu's pictures".to_string();
            (self.tell)(Change::State(id, ResState::Failed { code: ResCode::Resources, error, holders: holders.clone() }));
        }
    }

    /// The menu releases it: it goes at the next [`Self::sweep`] that finds it unused.
    pub fn release(&self, id: &str) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        match i.map.get_mut(id) {
            Some(e) if !e.released => {
                e.released = true;
                Ok(())
            }
            _ => Err(format!("no resource '{id}'")),
        }
    }

    /// Every resource gone at once (the menu changed or left); no events.
    pub fn clear(&self) {
        let mut i = self.inner.lock().unwrap();
        for e in i.map.values_mut() {
            e.drop_ahead();
        }
        i.map.clear();
        i.jobs.clear();
        i.session += 1;
    }

    /// Whether a scene may name `id`: declared, not released.
    pub fn known(&self, id: &str) -> bool {
        self.inner.lock().unwrap().map.get(id).is_some_and(|e| !e.released)
    }

    /// Whether what `id` lacks of its size and NV12 form may still come: declared, not failed. A form short of
    /// microcamd's share is still coming: it waits for the room ([`Self::short`]).
    pub fn coming(&self, id: &str) -> bool {
        self.inner.lock().unwrap().map.get(id).is_some_and(|e| !e.released && !e.failed && !e.yielded)
    }

    /// Whether `id` failed, or its NV12 form cannot be made: no frame lays it out.
    pub fn failed(&self, id: &str) -> bool {
        self.inner.lock().unwrap().map.get(id).is_some_and(|e| e.failed || e.nv12_failed)
    }

    /// Whether `id`'s NV12 form waits for room in microcamd's share, or gave its room to the video.
    pub fn short(&self, id: &str) -> bool {
        self.inner.lock().unwrap().map.get(id).is_some_and(|e| e.short.is_some() || e.yielded)
    }

    /// The source picture's size of `id`, once known.
    pub fn size(&self, id: &str) -> Option<Size> {
        self.inner.lock().unwrap().map.get(id)?.full.map(|(w, h)| Size::new(w, h))
    }

    /// What the renderer gets of `id` for the menu layer: its size and RGBA pixels once known.
    pub fn picture(&self, id: &str) -> Option<Picture> {
        let i = self.inner.lock().unwrap();
        let e = i.map.get(id)?;
        Some(Picture { full: e.full?, pix: e.rgba.pix.clone().filter(|_| !e.yielded) })
    }

    /// The scene shows `id` in the menu layer at `scale` output pixels per source pixel at most: a JPEG decoded at an n/8
    /// below the one nearest under it is decoded again (the engine only stretches it up the rest of the way: its
    /// shrinking is not filtered). Whether what it holds now is enough.
    pub fn want(&self, id: &str, scale: f32) -> bool {
        // a hair above a step counts as on it: the engine then shrinks by less than a pixel in a hundred
        let need = ((scale * 8.0 + 0.05).floor() as u32).clamp(1, 8);
        let mut i = self.inner.lock().unwrap();
        let Some(e) = i.map.get_mut(id) else {
            return false;
        };
        if e.yielded {
            return false;
        }
        match &e.src {
            Src::File(_) if !e.failed => {}
            Src::Camera(_) => return false,
            _ => return e.rgba.pix.is_some(),
        }
        if e.rgba.pix.is_some() && e.rgba.have >= need {
            return true;
        }
        e.shown = scale;
        if e.rgba.want < need {
            e.rgba.want = need;
            if !e.busy {
                e.busy = true;
                i.jobs.push_back((id.into(), Form::Rgba));
                self.wake.notify_one();
            }
        }
        false
    }

    /// `id`'s NV12 form, the key of the one held once there is one: for `shown` — a frame shows `crop` (source pixels)
    /// in a window of that many pixels at most — and for `ahead` — a picture prepared ahead, laid out larger than anything
    /// shown of it. The shown form is made at the size [`microcam_core::frame::form_size`] gives (again when it holds
    /// another), or waits when microcamd's share has no room for it until what is held firm falls to its mark. The form
    /// ahead is decoded after every shown one, only beside what is held, into `mi::layer`'s spare; a frame showing the
    /// picture then takes it out of the spare, or decodes it as a shown one when the book dropped it.
    pub fn frame(&self, id: &str, shown: Option<(Rect, Size)>, ahead: Option<(Rect, Size)>) -> Option<PictureId> {
        let mut i = self.inner.lock().unwrap();
        let Inner { map, jobs, .. } = &mut *i;
        let e = map.get_mut(id).filter(|e| !e.yielded)?;
        let full = e.full.map(|(w, h)| Size::new(w, h))?;
        let size = |(crop, window): (Rect, Size)| match &e.src {
            Src::File(_) if !e.failed => Some(microcam_core::frame::form_size(full, crop.size(), window)),
            // the menu's pixels at their own size; a camera picture is as it was taken
            Src::Pixels if !e.failed => Some(Size::new(full.w & !1, full.h & !1)),
            _ => None,
        };
        let (shown, ahead) = (shown.and_then(size), ahead.and_then(size));
        let spare = e.ahead.map(|(a, key)| (a, key.is_some()));
        let (mut next, drop_ahead) = next(e.nv12.have, spare, shown, ahead);
        if drop_ahead {
            e.drop_ahead();
        }
        if let Next::Take(s) = next {
            let (a, key) = e.ahead.take().expect("spare");
            match mi::layer::spare_take(key.expect("made")) {
                Some(f) => {
                    let key = key.expect("made");
                    if let Some((_, was)) = e.nv12.pix.replace((f, key)) {
                        e.nv12_was.push(was);
                    }
                    e.nv12.have = Some(a);
                    e.nv12.want = Some(a);
                    e.short = None;
                    return Some(PictureId(key));
                }
                // the book dropped it: decoded as a shown one
                None => next = Next::Shown(s),
            }
        }
        let mut queue = |form: Form| {
            if !jobs.iter().any(|(j, f)| j == id && *f == form) {
                jobs.push_back((id.into(), form));
                self.wake.notify_one();
            }
        };
        match next {
            Next::Shown(s) => {
                // short of room: asked again once what is held firm fell to its mark
                if let Some((was, mark, _)) = &e.short {
                    if *was == s && mi::layer::firm() > *mark {
                        mi::layer::wake_at(*mark);
                        return e.nv12.pix.as_ref().map(|(_, k)| PictureId(*k));
                    }
                    e.short = None;
                    e.nv12.want = e.nv12.have;
                }
                if e.nv12.want != Some(s) {
                    e.nv12.want = Some(s);
                    queue(Form::Nv12);
                }
            }
            Next::Ahead(a) if e.ahead_want != Some(a) => {
                e.ahead_want = Some(a);
                queue(Form::Ahead);
            }
            _ => {}
        }
        e.nv12.pix.as_ref().map(|(_, k)| PictureId(*k))
    }

    /// Released resources `used` no longer names go, each told as gone; a form nothing showed in the last drawing
    /// (`rgba`, `nv12`: the ids each form was used for) is dropped. An NV12 decode a still set holds (in a box or a
    /// compositor) lives on by that reference until the set goes.
    pub fn sweep(&self, used: &std::collections::HashSet<&str>, rgba: &std::collections::HashSet<&str>, nv12: &std::collections::HashSet<&str>) {
        let gone: Vec<String> = {
            let mut i = self.inner.lock().unwrap();
            let gone: Vec<String> = i.map.iter().filter(|(k, e)| e.released && !used.contains(k.as_str())).map(|(k, _)| k.clone()).collect();
            for k in &gone {
                if let Some(mut e) = i.map.remove(k) {
                    e.drop_ahead();
                }
            }
            for (k, e) in i.map.iter_mut() {
                // a JPEG's form is decoded again when needed; the menu's pixels and a camera picture have no other copy
                if let Src::File(_) = e.src {
                    if !rgba.contains(k.as_str()) {
                        e.rgba = Held::default();
                    }
                    if !nv12.contains(k.as_str()) {
                        e.nv12 = Held::default();
                        e.nv12_was.clear();
                        e.short = None;
                        e.drop_ahead();
                    }
                }
            }
            gone
        };
        for k in gone {
            (self.tell)(Change::State(k, ResState::Gone));
        }
    }

    /// The NV12 picture named `key` (or the decode that replaced it).
    pub fn decoded(&self, key: PictureId) -> Option<Arc<Frame>> {
        let i = self.inner.lock().unwrap();
        let e = i.map.values().find(|e| e.nv12.pix.as_ref().is_some_and(|(_, k)| *k == key.0) || e.nv12_was.contains(&key.0))?;
        e.nv12.pix.as_ref().map(|(f, _)| f.clone())
    }

    /// The decoder: a file's header first (ready), then a form's pixels at the scale wanted, within microcamd's share;
    /// the forms ahead after every other job.
    fn decoder(&self) {
        loop {
            let (id, job, src, session) = {
                let mut i = self.inner.lock().unwrap();
                let (id, form) = loop {
                    let pick = i.jobs.iter().position(|(_, f)| *f != Form::Ahead).or((!i.jobs.is_empty()).then_some(0));
                    match pick.and_then(|k| i.jobs.remove(k)) {
                        Some(j) => break j,
                        None => i = self.wake.wait(i).unwrap(),
                    }
                };
                let Some(e) = i.map.get(&id) else { continue };
                let enough = |w: Size| e.nv12.have.is_some_and(|h| h.w >= w.w && h.h >= w.h);
                let job = match form {
                    Form::Rgba => Job::Rgba { want: e.rgba.want, have: e.rgba.pix.as_ref().map_or(0, |_| e.rgba.have), shown: e.shown },
                    // wanted no more, or made meanwhile
                    Form::Nv12 if e.nv12.want.is_none() || e.nv12.want == e.nv12.have => continue,
                    Form::Nv12 => Job::Nv12 { want: e.nv12.want, ahead: false },
                    Form::Ahead => match e.ahead_want {
                        Some(a) if !enough(a) && e.ahead.is_none_or(|(s, _)| s != a) => Job::Nv12 { want: Some(a), ahead: true },
                        _ => continue,
                    },
                };
                let src = match &e.src {
                    Src::File(p) => Some(p.clone()),
                    Src::Pixels => None,
                    Src::Camera(_) => continue,
                };
                (id.clone(), job, src, i.session)
            };
            let t0 = std::time::Instant::now();
            let r = match src {
                Some(path) => self.decode(&path, job),
                // the menu's pixels in NV12: converted from their RGBA copy
                None => self.convert(&id),
            };
            let mut i = self.inner.lock().unwrap();
            if i.session != session {
                continue;
            }
            i.keys += 1;
            let key = i.keys;
            let Inner { map, jobs, .. } = &mut *i;
            let Some(e) = map.get_mut(&id) else { continue };
            if let Job::Rgba { .. } = job {
                e.busy = false;
            }
            // decoded for a picture that gave its room away meanwhile: dropped
            if e.yielded {
                continue;
            }
            let told = match r {
                Ok((full, got)) => {
                    let first = e.full.is_none();
                    let new = matches!((&got, job), (Some(Got::Rgba(..)), _) | (Some(Got::Nv12(_)), Job::Nv12 { ahead: false, .. }));
                    e.full = Some((full.w, full.h));
                    match (got, job) {
                        (Some(Got::Rgba(n, pix)), _) => {
                            log::debug!("res: {id} {full} decoded RGBA at {n}/8 in {} ms", t0.elapsed().as_millis());
                            e.rgba.have = n;
                            e.rgba.pix = Some((Arc::new(pix), key));
                        }
                        (Some(Got::Nv12(f)), Job::Nv12 { want, ahead: true }) => {
                            // still wanted ahead: into the spare, in place of the one before; else dropped
                            if e.ahead_want == want {
                                log::debug!("res: {id} {full} decoded NV12 at {} ahead in {} ms", f.size, t0.elapsed().as_millis());
                                e.drop_ahead();
                                mi::layer::spare_put(key, Arc::new(f));
                                e.ahead = want.map(|a| (a, Some(key)));
                            }
                        }
                        (Some(Got::Nv12(f)), _) => {
                            log::debug!("res: {id} {full} decoded NV12 at {} in {} ms", f.size, t0.elapsed().as_millis());
                            e.nv12.have = Some(f.size);
                            if let Some((_, was)) = e.nv12.pix.replace((Arc::new(f), key)) {
                                e.nv12_was.push(was);
                            }
                        }
                        (Some(Got::Short { mark, why }), Job::Nv12 { want: Some(s), .. }) => {
                            log::info!("res: {id}: {why}; it waits");
                            e.nv12.want = e.nv12.have;
                            e.short = Some((s, mark, why));
                        }
                        // a form ahead with no room beside what is held: not made, not asked again at that size
                        (None, Job::Nv12 { want: Some(a), ahead: true }) => {
                            log::debug!("res: {id}: no room for its form ahead at {a}");
                            if e.ahead_want == Some(a) {
                                e.drop_ahead();
                                e.ahead = Some((a, None));
                            }
                        }
                        _ => {}
                    }
                    // the RGBA form wanted otherwise meanwhile: once more
                    if e.rgba.want > e.rgba.have && !matches!(job, Job::Rgba { want, .. } if e.rgba.want == want) {
                        e.busy = true;
                        jobs.push_back((id.clone(), Form::Rgba));
                        self.wake.notify_one();
                    }
                    if first {
                        Some(Change::State(id, ResState::Ready { w: full.w, h: full.h }))
                    } else {
                        // a form waiting for room: the menu hears it is not shown
                        (new || e.short.is_some()).then_some(Change::Pixels)
                    }
                }
                Err((code, error)) => {
                    log::warn!("res: {id}: {error}");
                    if e.full.is_some() {
                        // the picture is known: a form it held stays, and is not asked again at that scale
                        match job {
                            Job::Rgba { .. } => {
                                e.rgba.want = e.rgba.have;
                                None
                            }
                            // not asked again at that size
                            Job::Nv12 { ahead: true, want } => {
                                if e.ahead_want == want {
                                    e.drop_ahead();
                                    e.ahead = want.map(|a| (a, None));
                                }
                                None
                            }
                            // no frame waits for it: it is left out of them
                            Job::Nv12 { .. } => {
                                e.nv12.want = e.nv12.have;
                                e.nv12_failed = true;
                                Some(Change::Pixels)
                            }
                        }
                    } else {
                        e.failed = true;
                        // the holders of a refusal for lack of resources are named by the `ui`, which knows them all
                        Some(Change::State(id, ResState::Failed { code, error, holders: Vec::new() }))
                    }
                }
            };
            drop(i);
            if let Some(c) = told {
                (self.tell)(c);
            }
        }
    }

    /// A JPEG's size, and (with a scale wanted) its pixels: the RGBA form at that n/8 or the largest below it that the
    /// share has room for beside what it holds — never smaller than what it holds; the NV12 form at that size when the
    /// share has room for it ([`Got::Short`] when not; a form ahead only beside what is held, nothing made when not).
    fn decode(&self, path: &std::path::Path, job: Job) -> Result<(Size, Option<Got>), Fail> {
        use std::io::{Read, Seek};
        let fail = |e: std::io::Error| (ResCode::Unreadable, format!("{}: {e}", path.display()));
        let mut file = std::fs::File::open(path).map_err(fail)?;
        // a video: its cover, a JPEG of its own size
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) {
            let moov = microcam_core::mp4::moov(&mut file).map_err(fail)?;
            let no_picture = |why: &str| (ResCode::NoPicture, format!("{}{why}", path.display()));
            let jpeg = moov.as_deref().and_then(microcam_core::mp4::cover).ok_or_else(|| no_picture(" carries no cover"))?.to_vec();
            let full = microcam_core::jpeg::Head::read(&mut std::io::Cursor::new(&jpeg)).ok_or_else(|| no_picture("'s cover is not a JPEG"))?.size;
            if matches!(job, Job::Rgba { want: 0, .. } | Job::Nv12 { want: None, .. }) {
                return Ok((full, None));
            }
            return self.decode_jpeg(&jpeg, full, job, path);
        }
        // the head alone says the size
        let head = microcam_core::jpeg::Head::read(&mut std::io::BufReader::new(&mut file)).ok_or_else(|| (ResCode::Unreadable, format!("{}: not a JPEG", path.display())))?;
        let full = head.size;
        if matches!(job, Job::Rgba { want: 0, .. } | Job::Nv12 { want: None, .. }) {
            return Ok((full, None));
        }
        if let Some(got) = self.from_thumbnail(&mut file, head, job) {
            return Ok((full, Some(got)));
        }
        let mut jpeg = Vec::new();
        file.rewind().and_then(|_| file.read_to_end(&mut jpeg)).map_err(fail)?;
        self.decode_jpeg(&jpeg, full, job, path)
    }

    /// The form `job` asks for made from the whole `jpeg` (`full`, of the file `path`), as [`Self::decode`] says.
    fn decode_jpeg(&self, jpeg: &[u8], full: Size, job: Job, path: &std::path::Path) -> Result<(Size, Option<Got>), Fail> {
        let at = |e: u32| mi::picture::jpeg_size(jpeg, e).map(|(_, s)| (s, e));
        let bytes = |s: Size| match job {
            Job::Rgba { .. } => s.w as u64 * s.h as u64 * 4,
            Job::Nv12 { .. } => ((s.w + 15) & !15) as u64 * ((s.h + 1) & !1) as u64 * 3 / 2,
        };
        // the larger of what either decoder makes, with the hardware decoder's file and picture for the decode's time
        let need = |&(s, e): &(Size, u32)| bytes(s).max(at(e).map_or(0, |(c, _)| bytes(c))) + jpd_bytes(jpeg, full, s);
        let left = || mi::layer::share().saturating_sub(mi::layer::held());
        let (size, e) = match job {
            Job::Rgba { want: 0, .. } | Job::Nv12 { want: None, .. } => return Ok((full, None)),
            Job::Rgba { want, have, .. } => {
                // finest first; the first asks the share for room
                let tries: Vec<(Size, u32)> = ((have + 1)..=want).rev().filter_map(at).collect();
                if let Some(t) = tries.first() {
                    mi::layer::room(need(t));
                }
                let left = left();
                let Some(&t) = tries.iter().find(|t| need(t) <= left) else {
                    if have > 0 {
                        return Ok((full, None));
                    }
                    return Err((ResCode::Resources, format!("{} KiB of microcamd's share of {} KiB held, not even an eighth of {full} fits", mi::layer::held() >> 10, mi::layer::share() >> 10)));
                };
                t
            }
            // one size: the window's; a form ahead takes only what is left beside what is held
            Job::Nv12 { want: Some(size), ahead } => {
                let t = (size, microcam_core::frame::eighths(full, size));
                let n = need(&t);
                if ahead {
                    if n > left() {
                        return Ok((full, None));
                    }
                } else if !mi::layer::room(n) {
                    let mark = mi::layer::share().saturating_sub(n);
                    mi::layer::wake_at(mark);
                    let why = format!("{size} wants {} KiB, {} KiB of microcamd's share of {} KiB held", n >> 10, mi::layer::held() >> 10, mi::layer::share() >> 10);
                    return Ok((full, Some(Got::Short { mark, why })));
                }
                t
            }
        };
        let rgba = matches!(job, Job::Rgba { .. });
        // the hardware decoder first; what it does not read, the CPU
        match hardware(jpeg, full, size, full, rgba, e) {
            Ok(got) => return Ok((full, Some(got))),
            Err(why) => log::debug!("res: {}: {why}; the CPU decodes it", path.display()),
        }
        let unreadable = |e: String| (ResCode::Unreadable, format!("{}: {e}", path.display()));
        Ok((full, Some(if rgba {
            let (size, _) = at(e).ok_or_else(|| unreadable("not a JPEG the decoder reads".into()))?;
            let mut pix = Image::new(size.w, size.h).map_err(|e| (ResCode::Resources, e.to_string()))?;
            mi::picture::jpeg_rgba(jpeg, e, pix.bytes()).map_err(unreadable)?;
            pix.sync().map_err(|e| unreadable(e.to_string()))?;
            Got::Rgba(e, pix)
        } else {
            Got::Nv12(Frame::decode(jpeg, e).map_err(unreadable)?)
        })))
    }

    /// The form `job` asks for made from the file's embedded thumbnail, when it serves:
    /// its aspect the picture's; NV12 — the size wanted no larger than the thumbnail and made from it 1:1 or by a
    /// filtered shrink; RGBA — the thumbnail between the shown size and the n/8 wanted. By the hardware decoder only,
    /// within the share; otherwise the whole file serves.
    fn from_thumbnail(&self, file: &mut std::fs::File, head: microcam_core::jpeg::Head, job: Job) -> Option<Got> {
        use std::io::{Read, Seek, SeekFrom};
        let (at, len) = head.thumb?;
        let full = head.size;
        let mut jpeg = vec![0u8; len as usize];
        file.seek(SeekFrom::Start(at)).and_then(|_| file.read_exact(&mut jpeg)).ok()?;
        let thumb = microcam_core::jpeg::Head::read(&mut std::io::Cursor::new(&jpeg))?.size;
        if (thumb.w as u64 * full.h as u64).abs_diff(thumb.h as u64 * full.w as u64) * 100 > full.w as u64 * thumb.h as u64 {
            return None;
        }
        let (size, rgba, eighths) = match job {
            Job::Nv12 { want: Some(s), .. } => {
                let clean = |t: u32, s: u32| t.abs_diff(s) <= 1 || t * 5 >= s * 6;
                (s.w <= thumb.w && s.h <= thumb.h && clean(thumb.w, s.w) && clean(thumb.h, s.h)).then_some((s, false, 0))?
            }
            Job::Rgba { want, have, shown } if want > have => {
                let (t, n) = (thumb.w as f32 / full.w as f32, want as f32 / 8.0);
                // half a pixel either way
                let px = 0.5 / full.w as f32;
                (t >= shown.min(n) - px && t <= shown.max(n) + px).then_some((thumb, true, want))?
            }
            _ => return None,
        };
        let bytes = if rgba { size.w as u64 * size.h as u64 * 4 } else { ((size.w + 15) & !15) as u64 * size.h as u64 * 3 / 2 };
        // a form a frame shows asks the share (the spare goes first); one prepared ahead takes what is left
        let need = bytes + jpd_bytes(&jpeg, thumb, size);
        let fits = if matches!(job, Job::Nv12 { ahead: true, .. }) { need <= mi::layer::share().saturating_sub(mi::layer::held()) } else { mi::layer::room(need) };
        if !fits {
            return None;
        }
        hardware(&jpeg, thumb, size, full, rgba, eighths).inspect_err(|why| log::debug!("res: the thumbnail: {why}; the whole file serves")).ok()
    }

    /// The menu's pixels of `id` as NV12, from their RGBA copy.
    fn convert(&self, id: &str) -> Result<(Size, Option<Got>), Fail> {
        let rgba = {
            let i = self.inner.lock().unwrap();
            i.map.get(id).and_then(|e| e.rgba.pix.as_ref().map(|(p, _)| p.clone())).ok_or((ResCode::Unreadable, "no pixels".to_string()))?
        };
        let (w, h) = rgba.size();
        Ok((Size::new(w, h), Some(Got::Nv12(Frame::from_rgba(w, h, rgba.rgba()).map_err(|e| (ResCode::Resources, e))?))))
    }
}

/// The engine's scale for a `full`-size picture wanted at `size`: the coarsest of 1/1, 1/2, 1/4, 1/8 that SCL dev 3
/// then brings to `size` 1:1 (to a pixel) or by a shrink it filters (≥ 1.2:1).
fn jpd_ratio(full: Size, size: Size) -> u32 {
    let fits = |r: u32| {
        let (w, h) = (full.w / r, full.h / r);
        (w.abs_diff(size.w) <= 1 && h.abs_diff(size.h) <= 1) || (w * 5 >= size.w * 6 && h * 5 >= size.h * 6)
    };
    [8, 4, 2, 1].into_iter().find(|r| fits(*r)).unwrap_or(1)
}

/// What a hardware decode holds beside the form for its time: the file and the engine's picture.
fn jpd_bytes(jpeg: &[u8], full: Size, size: Size) -> u64 {
    jpeg.len().div_ceil(16) as u64 * 16 + mi::jpd::bytes(full, jpd_ratio(full, size))
}

/// `jpeg` (`full`) decoded by the hardware decoder and brought to `size` by SCL dev 3, as RGBA (`eighths` its scale)
/// or NV12 (of the picture `source`: `full` itself, or the picture `jpeg` is the thumbnail of).
fn hardware(jpeg: &[u8], full: Size, size: Size, source: Size, rgba: bool, eighths: u32) -> Result<Got, String> {
    let picture = {
        let input = mi::jpd::Input::read(jpeg)?;
        mi::jpd::decode(&input, full, jpd_ratio(full, size))?
    };
    Ok(if rgba { Got::Rgba(eighths, Image::from_yuyv(&picture, size.w, size.h)?) } else { Got::Nv12(Frame::from_yuyv(&picture, size, source)?) })
}

/// Why a resource's decode failed: its code for the menu, the text for people.
type Fail = (ResCode, String);

/// What a picture's NV12 forms need next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Nothing,
    /// the form held ahead serves what is shown (of this size): taken out of the spare
    Take(Size),
    /// the shown form at this size
    Shown(Size),
    /// a form ahead at this size
    Ahead(Size),
}

/// What comes next holding the shown form `have` and the form ahead `spare` (its size, and whether it was made), for
/// what a frame shows (`shown`) and the window ahead (`ahead`): what is shown first — from the spare when it serves —,
/// then the form ahead unless the shown one serves it too, then the shown form brought down to what is shown. And
/// whether the form ahead goes: when nothing is prepared ahead any more and what is shown does not take it — a picture
/// prepared ahead is no longer prepared the moment it is shown, so the spare is offered to what is shown first.
fn next(have: Option<Size>, spare: Option<(Size, bool)>, shown: Option<Size>, ahead: Option<Size>) -> (Next, bool) {
    let n = next_form(have, spare, shown, ahead);
    (n, ahead.is_none() && spare.is_some() && !matches!(n, Next::Take(_)))
}

fn next_form(have: Option<Size>, spare: Option<(Size, bool)>, shown: Option<Size>, ahead: Option<Size>) -> Next {
    let enough = |h: Option<Size>, w: Size| h.is_some_and(|h| h.w >= w.w && h.h >= w.h);
    match (shown, ahead) {
        (Some(s), _) if !enough(have, s) => match spare {
            Some((a, true)) if enough(Some(a), s) => Next::Take(s),
            _ => Next::Shown(s),
        },
        (_, Some(a)) if enough(have, a) || spare.is_some_and(|(x, _)| x == a) => Next::Nothing,
        (_, Some(a)) => Next::Ahead(a),
        (Some(s), None) if have != Some(s) => Next::Shown(s),
        _ => Next::Nothing,
    }
}

/// Our photos' thumbnails: this wide, the strip's 320×180 of the 1080-line screen on a 2160-line output (photos taken
/// before carry 480).
const THUMB_W: u32 = 640;
/// A video's cover: no wider than this (its box is written over a 48-KiB reserve).
pub const COVER_W: u32 = 480;
/// The thumbnail's quality, and the lower ones tried while it is too long for its EXIF segment.
const THUMB_QUALITY: [i32; 4] = [80, 70, 60, 50];

/// The photo `jpeg` with its thumbnail ([`thumbnail`], [`THUMB_W`] wide) in an EXIF segment: at a lower quality when it
/// would not fit the segment.
pub fn with_thumbnail(jpeg: &[u8]) -> Result<Vec<u8>, String> {
    let picture = thumbnail_picture(jpeg, THUMB_W)?;
    let mut longest = 0;
    for q in THUMB_QUALITY {
        let thumb = picture.jpeg(q)?;
        if let Some(app1) = microcam_core::jpeg::exif_app1(&thumb) {
            if q != THUMB_QUALITY[0] {
                log::info!("photo: thumbnail at quality {q}: {} KB at {} too long for EXIF", longest >> 10, THUMB_QUALITY[0]);
            }
            return microcam_core::jpeg::with_segment(jpeg, &app1).ok_or_else(|| "not a JPEG".into());
        }
        longest = longest.max(thumb.len());
    }
    Err(format!("a thumbnail of {} KB is too long for EXIF at quality {}", longest >> 10, THUMB_QUALITY[THUMB_QUALITY.len() - 1]))
}

/// A picture's thumbnail no wider than `width`, a baseline JPEG: a video's cover ([`COVER_W`]).
pub fn thumbnail(jpeg: &[u8], width: u32) -> Result<Vec<u8>, String> {
    thumbnail_picture(jpeg, width)?.jpeg(THUMB_QUALITY[0])
}

/// A thumbnail's size for a `full` picture: `width` wide (never wider than the picture), its aspect kept, even sides.
fn thumb_size(full: Size, width: u32) -> Size {
    let w = width.min(full.w) & !1;
    Size::new(w, (full.h as u64 * w as u64 / full.w.max(1) as u64) as u32 & !1)
}

/// `jpeg` brought to `width` (its aspect kept, even sides; never enlarged) for its thumbnail: the hardware decoder at
/// the coarsest 1/2ᵏ SCL dev 3 brings there 1:1 or by a filtered shrink, then SCL dev 3 — within microcamd's share, or
/// not at all.
fn thumbnail_picture(jpeg: &[u8], width: u32) -> Result<mi::jpd::Yuyv, String> {
    let full = microcam_core::jpeg::Head::read(&mut std::io::Cursor::new(jpeg)).ok_or("not a JPEG")?.size;
    let size = thumb_size(full, width);
    let ratio = jpd_ratio(full, size);
    let need = jpd_bytes(jpeg, full, size) + size.w.next_multiple_of(16) as u64 * size.h as u64 * 2;
    if !mi::layer::room(need) {
        let left = mi::layer::share().saturating_sub(mi::layer::held());
        return Err(format!("memory: {} KiB wanted, {} KiB of microcamd's share left", need >> 10, left >> 10));
    }
    let picture = {
        let input = mi::jpd::Input::read(jpeg)?;
        mi::jpd::decode(&input, full, ratio)?
    };
    if picture.size.w.abs_diff(size.w) <= 1 && picture.size.h.abs_diff(size.h) <= 1 {
        return Ok(picture);
    }
    picture.scaled(mi::scl::User::Res, size)
}

/// A decode the decoder thread is asked for: the RGBA form's scale wanted and held (eighths, 0: none) and the scale the
/// scene shows it at, or the NV12 form's size wanted and whether it is a form ahead.
#[derive(Clone, Copy)]
enum Job {
    Rgba { want: u32, have: u32, shown: f32 },
    Nv12 { want: Option<Size>, ahead: bool },
}

/// What a decode made: the RGBA form at its scale in eighths, or the NV12 form (its size the frame's); or nothing, the
/// shown NV12 form finding no room in microcamd's share: it waits until what is held firm falls to `mark`.
enum Got {
    Rgba(u32, Image),
    Nv12(Frame),
    Short { mark: u64, why: String },
}

/// The store as the pipeline reaches it: camera pictures put into it.
pub struct Source(pub Arc<Resources>);

impl mi::hal::PictureSource for Source {
    fn grabbed(&self, id: PictureId, frame: Frame) {
        self.0.grabbed(id, frame)
    }
}

fn check_new(i: &Inner, id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("a resource id is 1..64 bytes".into());
    }
    if i.map.contains_key(id) {
        return Err(format!("resource '{id}' exists (release it, or take another id)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A photo in the strip and prepared full screen: its strip form first; then the full screen ahead, into the spare;
    /// shown full screen, the spare taken; the spare dropped by the book (or not made), decoded as shown; no longer
    /// prepared, the shown form follows what is shown.
    #[test]
    fn what_is_shown_comes_first_and_takes_the_form_ahead() {
        let (strip, full) = (Size::new(640, 360), Size::new(3840, 2160));
        let next = |h, s, sh, a| next(h, s, sh, a).0;
        assert_eq!(next(None, None, Some(strip), Some(full)), Next::Shown(strip));
        assert_eq!(next(Some(strip), None, Some(strip), Some(full)), Next::Ahead(full));
        assert_eq!(next(Some(strip), Some((full, true)), Some(strip), Some(full)), Next::Nothing, "made ahead");
        assert_eq!(next(Some(strip), Some((full, false)), Some(strip), Some(full)), Next::Nothing, "no room ahead: not asked again");
        assert_eq!(next(None, None, None, Some(full)), Next::Ahead(full));
        assert_eq!(next(Some(strip), Some((full, true)), Some(full), None), Next::Take(full));
        assert_eq!(next(Some(strip), Some((full, false)), Some(full), None), Next::Shown(full));
        assert_eq!(next(Some(full), None, Some(strip), Some(full)), Next::Nothing, "the shown form serves ahead too");
        assert_eq!(next(Some(full), None, Some(strip), None), Next::Shown(strip));
        assert_eq!(next(Some(full), None, None, None), Next::Nothing);
    }

    /// A gallery step onto the photo prepared ahead: shown full screen, it is prepared no more — its spare is taken, not
    /// dropped for that first (it was, and the photo decoded again: the camera's log 2026-11-13, P00051–58). Prepared
    /// no more and not shown, the spare goes; a spare that does not serve what is shown goes too.
    #[test]
    fn a_photo_prepared_ahead_and_then_shown_takes_its_spare() {
        let (strip, full) = (Size::new(640, 360), Size::new(3840, 2160));
        assert_eq!(super::next(Some(strip), Some((full, true)), Some(full), None), (Next::Take(full), false));
        assert_eq!(super::next(Some(strip), Some((full, true)), Some(strip), None), (Next::Nothing, true));
        assert_eq!(super::next(Some(strip), Some((strip, true)), Some(full), None), (Next::Shown(full), true));
        assert_eq!(super::next(Some(strip), Some((full, true)), Some(strip), Some(full)), (Next::Nothing, false), "still prepared");
    }

    /// A 4K photo's thumbnail: 640×360 from the decoder's quarter (960×540, a filtered shrink of 1.5); a video's cover
    /// 480×270 straight from its eighth; a 4:3 picture keeps its aspect; a small one is never enlarged.
    #[test]
    fn a_thumbnail_takes_the_decoders_coarsest_scale_that_shrinks_cleanly() {
        let uhd = Size::new(3840, 2160);
        assert_eq!((thumb_size(uhd, THUMB_W), jpd_ratio(uhd, thumb_size(uhd, THUMB_W))), (Size::new(640, 360), 4));
        assert_eq!((thumb_size(uhd, COVER_W), jpd_ratio(uhd, thumb_size(uhd, COVER_W))), (Size::new(480, 270), 8));
        assert_eq!(thumb_size(Size::new(4000, 3000), THUMB_W), Size::new(640, 480));
        assert_eq!(thumb_size(Size::new(320, 240), THUMB_W), Size::new(320, 240));
    }
}
