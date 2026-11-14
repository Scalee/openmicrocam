//! ISO BMFF (MP4) boxes read within bounds, for every reader of our MP4 files (`demux` for the player, `mp4::cover` for
//! the gallery): a box's size is its 32-bit field, or the 64-bit `largesize` after the type when that field is 1, or
//! the rest of its container when it is 0. A box that does not fit in what is left of its container — cut by the end
//! of a file, or naming a size smaller than its own header — ends the walk: the boxes before it are kept.

use std::io::{self, Read, Seek, SeekFrom};

/// A box's header: where the box starts, its type, the header's length and the whole box's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub at: u64,
    pub kind: [u8; 4],
    pub head: u64,
    pub size: u64,
}

impl Head {
    /// The header of the box at `at` whose bytes start `b`, with `room` bytes from `at` to its container's end: None
    /// when it does not fit there.
    fn parse(b: &[u8], at: u64, room: u64) -> Option<Head> {
        let kind: [u8; 4] = b.get(4..8)?.try_into().ok()?;
        let (head, size) = match u32::from_be_bytes(b[..4].try_into().ok()?) {
            0 => (8, room),
            1 => (16, u64::from_be_bytes(b.get(8..16)?.try_into().ok()?)),
            s => (8, s as u64),
        };
        (head <= size && size <= room).then_some(Head { at, kind, head, size })
    }

    /// Where the box's body starts.
    pub fn body_at(&self) -> u64 {
        self.at + self.head
    }

    pub fn body_len(&self) -> u64 {
        self.size - self.head
    }

    /// Where the next box starts.
    pub fn end(&self) -> u64 {
        self.at + self.size
    }
}

/// The boxes of `b` one level down: each one's type and body.
pub fn boxes(b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let h = Head::parse(&b[at..], at as u64, (b.len() - at) as u64)?;
        let body = &b[h.body_at() as usize..h.end() as usize];
        at = h.end() as usize;
        Some((h.kind, body))
    })
}

/// The body of the first box of type `kind` among those of `b`.
pub fn child<'a>(b: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(b).find(|(k, _)| k == kind).map(|(_, v)| v)
}

/// The body of the box the types `kinds` lead to, one level each.
pub fn path<'a>(b: &'a [u8], kinds: &[&[u8; 4]]) -> Option<&'a [u8]> {
    kinds.iter().try_fold(b, |b, k| child(b, k))
}

/// The header of the top-level box at `at` of a file of `end` bytes: None when the file has no whole box there.
pub fn head_at<R: Read + Seek>(r: &mut R, at: u64, end: u64) -> io::Result<Option<Head>> {
    let room = end.saturating_sub(at);
    let mut h = [0u8; 16];
    let n = room.min(16) as usize;
    if n < 8 {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut h[..n])?;
    Ok(Head::parse(&h[..n], at, room))
}

/// The body of the top-level box `h`: None when it is longer than `max` bytes.
pub fn body<R: Read + Seek>(r: &mut R, h: &Head, max: u64) -> io::Result<Option<Vec<u8>>> {
    if h.body_len() > max {
        return Ok(None);
    }
    let mut b = vec![0u8; h.body_len() as usize];
    r.seek(SeekFrom::Start(h.body_at()))?;
    r.read_exact(&mut b)?;
    Ok(Some(b))
}

/// The top-level boxes' headers of a file of `end` bytes, in order, up to the first that is not whole.
pub fn heads<R: Read + Seek>(r: &mut R, end: u64) -> io::Result<Vec<Head>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(h) = head_at(r, at, end)? {
        at = h.end();
        out.push(h);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat()
    }

    fn large(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&1u32.to_be_bytes()[..], kind, &((body.len() + 16) as u64).to_be_bytes(), body].concat()
    }

    fn kinds(b: &[u8]) -> Vec<(String, usize)> {
        boxes(b).map(|(k, v)| (String::from_utf8_lossy(&k).into_owned(), v.len())).collect()
    }

    #[test]
    fn a_32_bit_size_a_largesize_and_a_size_0_each_bound_their_box() {
        let b = [bx(b"ftyp", &[1; 4]), large(b"mdat", &[2; 5]), bx(b"free", &[]), [&0u32.to_be_bytes()[..], b"mdat", &[3; 6]].concat()].concat();
        assert_eq!(kinds(&b), [("ftyp".into(), 4), ("mdat".into(), 5), ("free".into(), 0), ("mdat".into(), 6)]);
        assert_eq!(child(&b, b"mdat"), Some(&[2u8; 5][..]), "the first of its type");
        let nested = bx(b"moov", &bx(b"udta", &large(b"meta", b"abc")));
        assert_eq!(path(&nested, &[b"moov", b"udta", b"meta"]), Some(&b"abc"[..]));
    }

    #[test]
    fn a_box_that_does_not_fit_ends_the_walk_and_keeps_the_ones_before() {
        let whole = bx(b"moof", &[1; 4]);
        // cut: the size runs past the buffer
        let cut = bx(b"mdat", &[2; 10]);
        assert_eq!(kinds(&[&whole[..], &cut[..12]].concat()), [("moof".into(), 4)]);
        // a size under the header's own length
        assert_eq!(kinds(&[&whole[..], &4u32.to_be_bytes()[..], b"free"].concat()), [("moof".into(), 4)]);
        assert_eq!(kinds(&[&whole[..], &[&1u32.to_be_bytes()[..], b"mdat", &8u64.to_be_bytes()].concat()].concat()), [("moof".into(), 4)]);
        // a largesize cut inside its 64 bits, a largesize past the buffer, a header cut short
        let l = large(b"mdat", &[2; 3]);
        assert_eq!(kinds(&[&whole[..], &l[..12]].concat()), [("moof".into(), 4)]);
        assert_eq!(kinds(&[&whole[..], &l[..l.len() - 1]].concat()), [("moof".into(), 4)]);
        assert_eq!(kinds(&[&whole[..], &[0, 0, 0]].concat()), [("moof".into(), 4)]);
        assert_eq!(kinds(&[]), []);
    }

    #[test]
    fn a_files_top_level_boxes_are_read_up_to_the_first_cut_one() {
        let file = [bx(b"ftyp", &[1; 4]), large(b"mdat", &[2; 40]), bx(b"moov", b"xyz"), bx(b"moof", &[3; 20])].concat();
        let end = file.len() as u64 - 5;
        let mut r = io::Cursor::new(&file[..end as usize]);
        let hs = heads(&mut r, end).unwrap();
        assert_eq!(hs.iter().map(|h| &h.kind).collect::<Vec<_>>(), [b"ftyp", b"mdat", b"moov"]);
        assert_eq!((hs[1].at, hs[1].head, hs[1].size), (12, 16, 56));
        assert_eq!(body(&mut r, &hs[2], 3).unwrap().as_deref(), Some(&b"xyz"[..]));
        assert_eq!(body(&mut r, &hs[2], 2).unwrap(), None, "longer than allowed");
        // size 0 at the top: the rest of the file
        let file = [bx(b"ftyp", &[]), [&0u32.to_be_bytes()[..], b"mdat", &[9; 7]].concat()].concat();
        let hs = heads(&mut io::Cursor::new(&file), file.len() as u64).unwrap();
        assert_eq!((hs[1].kind, hs[1].size), (*b"mdat", 15));
    }
}
