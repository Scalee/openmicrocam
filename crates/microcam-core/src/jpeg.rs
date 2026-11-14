//! A JPEG's head without its picture data — the picture's size and its EXIF thumbnail's place, read a segment at a
//! time — and the EXIF segment that puts a thumbnail into a photo of ours.

use std::io::{Read, Seek, SeekFrom};

use crate::Size;

/// What a JPEG's head says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub size: Size,
    /// the embedded thumbnail (EXIF IFD1, a JPEG): its offset and length in the file
    pub thumb: Option<(u64, u32)>,
}

impl Head {
    /// The segments from SOI up to the first frame header, the others skipped; None when it is not a JPEG with one.
    pub fn read(r: &mut (impl Read + Seek)) -> Option<Head> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b[..2]).ok()?;
        if b[..2] != [0xff, 0xd8] {
            return None;
        }
        let mut at = 2u64;
        let mut thumb = None;
        loop {
            r.read_exact(&mut b).ok()?;
            let (m, len) = (b[1], u16::from_be_bytes([b[2], b[3]]) as u64);
            if b[0] != 0xff || len < 2 {
                return None;
            }
            let body = at + 4;
            match m {
                // a frame header (not DHT, JPG, DAC)
                0xc0..=0xcf if !matches!(m, 0xc4 | 0xc8 | 0xcc) => {
                    let mut f = [0u8; 5];
                    r.read_exact(&mut f).ok()?;
                    let (h, w) = (u16::from_be_bytes([f[1], f[2]]) as u32, u16::from_be_bytes([f[3], f[4]]) as u32);
                    return (w > 0 && h > 0).then_some(Head { size: Size::new(w, h), thumb });
                }
                0xe1 if thumb.is_none() => {
                    let mut seg = vec![0u8; len as usize - 2];
                    r.read_exact(&mut seg).ok()?;
                    // the TIFF structure starts after "Exif\0\0"
                    thumb = exif_thumb(&seg).map(|(o, l)| (body + 6 + o as u64, l));
                }
                // the picture's data, or a marker without a length, before any frame header
                0xd8..=0xda | 0x01 => return None,
                _ => {}
            }
            at = body + len - 2;
            r.seek(SeekFrom::Start(at)).ok()?;
        }
    }
}

/// An APP1 segment's body: the thumbnail's offset in its TIFF structure and its length, when IFD1 names a JPEG one.
fn exif_thumb(seg: &[u8]) -> Option<(u32, u32)> {
    let tiff = seg.strip_prefix(b"Exif\0\0")?;
    let le = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| tiff.get(i..i + 2).map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
    let u32_at = |i: usize| tiff.get(i..i + 4).map(|b| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) });
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd0 = u32_at(4)? as usize;
    let ifd1 = u32_at(ifd0 + 2 + 12 * u16_at(ifd0)? as usize)? as usize;
    if ifd1 == 0 {
        return None;
    }
    let (mut off, mut len) = (None, None);
    for e in (0..u16_at(ifd1)? as usize).map(|k| ifd1 + 2 + 12 * k) {
        match u16_at(e)? {
            0x0201 => off = u32_at(e + 8),
            0x0202 => len = u32_at(e + 8),
            _ => {}
        }
    }
    let (off, len) = (off?, len?);
    (len > 0 && off as usize + len as usize <= tiff.len()).then_some((off, len))
}

/// An APP1 EXIF segment (marker included) with `thumb`, a JPEG, as the picture's thumbnail: IFD0 holds Orientation 1
/// (an IFD has at least one entry), IFD1 Compression 6 and the thumbnail's place. None when it is too long for a segment.
pub fn exif_app1(thumb: &[u8]) -> Option<Vec<u8>> {
    // little-endian TIFF: its header (8), IFD0 at 8 (2 + 12 + 4), IFD1 at 26 (2 + 3 × 12 + 4), the thumbnail at 68
    const IFD1: u32 = 26;
    const THUMB: u32 = 68;
    let len = 2 + 6 + THUMB as usize + thumb.len();
    if len > 0xffff {
        return None;
    }
    let mut v = vec![0xff, 0xe1];
    v.extend((len as u16).to_be_bytes());
    v.extend(b"Exif\0\0II");
    v.extend(42u16.to_le_bytes());
    v.extend(8u32.to_le_bytes());
    let entry = |v: &mut Vec<u8>, tag: u16, kind: u16, value: u32| {
        v.extend(tag.to_le_bytes());
        v.extend(kind.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        v.extend(value.to_le_bytes());
    };
    // IFD0: Orientation (SHORT) 1
    v.extend(1u16.to_le_bytes());
    entry(&mut v, 0x0112, 3, 1);
    v.extend(IFD1.to_le_bytes());
    // IFD1: Compression (SHORT) 6 = JPEG, the thumbnail's offset and length (LONG)
    v.extend(3u16.to_le_bytes());
    entry(&mut v, 0x0103, 3, 6);
    entry(&mut v, 0x0201, 4, THUMB);
    entry(&mut v, 0x0202, 4, thumb.len() as u32);
    v.extend(0u32.to_le_bytes());
    debug_assert_eq!(v.len(), 4 + 6 + THUMB as usize);
    v.extend(thumb);
    Some(v)
}

/// `jpeg` with `segment` put right after its SOI, or after an APP0 there (JFIF's place is first); None when it is not
/// a JPEG.
pub fn with_segment(jpeg: &[u8], segment: &[u8]) -> Option<Vec<u8>> {
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let at = match jpeg.get(2..6) {
        Some([0xff, 0xe0, a, b]) => 4 + u16::from_be_bytes([*a, *b]) as usize,
        _ => 2,
    };
    if at > jpeg.len() {
        return None;
    }
    Some([&jpeg[..at], segment, &jpeg[at..]].concat())
}

/// A `full`-size picture decoded at `eighths`/8, as libjpeg sizes it (rounded up).
pub fn scaled(full: Size, eighths: u32) -> Size {
    Size::new((full.w * eighths).div_ceil(8), (full.h * eighths).div_ceil(8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// SOI, APP0, DQT, SOF0 of `w` × `h`, SOS: a head as our photos have it.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
        v.extend(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        v.extend([0xff, 0xdb, 0, 4, 0, 1]);
        v.extend([0xff, 0xc0, 0, 11, 8]);
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v.extend([1, 1, 0x11, 0]);
        v.extend([0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0x12, 0x34, 0xff, 0xd9]);
        v
    }

    #[test]
    fn a_head_without_exif_has_its_size_and_no_thumbnail() {
        assert_eq!(Head::read(&mut Cursor::new(jpeg(3840, 2160))), Some(Head { size: Size::new(3840, 2160), thumb: None }));
        assert_eq!(Head::read(&mut Cursor::new(b"not a jpeg".to_vec())), None);
        // the picture's data before a frame header
        assert_eq!(Head::read(&mut Cursor::new(vec![0xff, 0xd8, 0xff, 0xda, 0, 2])), None);
    }

    #[test]
    fn a_thumbnail_put_in_is_found_where_it_lies() {
        let thumb = jpeg(480, 270);
        let file = with_segment(&jpeg(3840, 2160), &exif_app1(&thumb).unwrap()).unwrap();
        // after APP0
        assert_eq!(&file[20..22], &[0xff, 0xe1]);
        let head = Head::read(&mut Cursor::new(&file)).unwrap();
        assert_eq!(head.size, Size::new(3840, 2160));
        let (o, l) = head.thumb.unwrap();
        assert_eq!(&file[o as usize..o as usize + l as usize], &thumb[..]);
        assert_eq!(Head::read(&mut Cursor::new(&thumb)).unwrap().size, Size::new(480, 270));
    }

    #[test]
    fn a_big_endian_exif_is_read_too() {
        // MM, IFD0 at 8 with no entry, IFD1 at 14 with offset and length, the thumbnail at 44
        let thumb = [0xffu8, 0xd8, 1, 2, 0xff, 0xd9];
        let mut t = b"MM\0\x2a\0\0\0\x08\0\0\0\0\0\x0e\0\x02".to_vec();
        t.extend([0x02, 0x01, 0, 4, 0, 0, 0, 1, 0, 0, 0, 44]);
        t.extend([0x02, 0x02, 0, 4, 0, 0, 0, 1, 0, 0, 0, thumb.len() as u8]);
        t.extend([0, 0, 0, 0]);
        assert_eq!(t.len(), 44);
        t.extend(thumb);
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend(&t);
        assert_eq!(exif_thumb(&seg), Some((44, 6)));
    }

    #[test]
    fn a_segment_goes_after_soi_without_app0() {
        let mut j = jpeg(8, 8);
        j.drain(2..20);
        assert_eq!(with_segment(&j, &[0xff, 0xe1, 0, 2]).unwrap()[..6], [0xff, 0xd8, 0xff, 0xe1, 0, 2]);
    }

    #[test]
    fn scaled_rounds_up_as_libjpeg() {
        assert_eq!(scaled(Size::new(3840, 2160), 1), Size::new(480, 270));
        assert_eq!(scaled(Size::new(1001, 751), 5), Size::new(626, 470));
    }
}
