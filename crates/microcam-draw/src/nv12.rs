//! The `ui`'s pixels blended over NV12 pictures by the CPU: a photo's `ui`, and the screen elements a composed USB
//! frame shows over frozen frames, where no overlay layer reaches. Colours go to YCbCr by BT.601 at full range, the
//! range a JPEG decoder reads the encoder's frames in.

/// Blend `src` (`w` × `h` straight RGBA pixels, rows of `src_stride` bytes) over an NV12 picture of the same size: its
/// luma rows `y` of `y_stride` and its interleaved chroma rows `uv` of `uv_stride`, one chroma row per two luma rows.
/// `w` and `h` are even. A transparent pixel leaves the picture as it is; chroma takes the 2×2 block's summed
/// alpha-weighted colour and its mean alpha.
#[allow(clippy::too_many_arguments)]
pub fn blend(src: &[u8], src_stride: usize, w: usize, h: usize, y: &mut [u8], y_stride: usize, uv: &mut [u8], uv_stride: usize) {
    debug_assert!(w % 2 == 0 && h % 2 == 0);
    // alpha and alpha-weighted Y, Cb, Cr of a pixel, each 0..=255 × 255 (BT.601 in 16.16 fixed point: the camera's CPU
    // runs this per pixel of a 4K photo's ui)
    let px = |p: &[u8]| -> (u32, u32, u32, u32) {
        let (r, g, b, a) = (p[0] as i32, p[1] as i32, p[2] as i32, p[3] as u32);
        let w = |v: i32| (((v + 32768) >> 16).clamp(0, 255) as u32) * a;
        (a, w(19595 * r + 38470 * g + 7471 * b), w((128 << 16) - 11059 * r - 21709 * g + 32768 * b), w((128 << 16) + 32768 * r - 27439 * g - 5329 * b))
    };
    for row in (0..h).step_by(2) {
        let (s0, s1) = (&src[row * src_stride..][..w * 4], &src[(row + 1) * src_stride..][..w * 4]);
        if s0.iter().skip(3).step_by(4).chain(s1.iter().skip(3).step_by(4)).all(|&a| a == 0) {
            continue;
        }
        for (r, s) in [(row, s0), (row + 1, s1)] {
            let line = &mut y[r * y_stride..][..w];
            for (o, p) in line.iter_mut().zip(s.chunks_exact(4)) {
                if p[3] != 0 {
                    let (a, ly, _, _) = px(p);
                    *o = ((*o as u32 * (255 - a) + ly + 127) / 255) as u8;
                }
            }
        }
        let line = &mut uv[row / 2 * uv_stride..][..w];
        for x in (0..w).step_by(2) {
            let quad = [&s0[x * 4..][..4], &s0[(x + 1) * 4..][..4], &s1[x * 4..][..4], &s1[(x + 1) * 4..][..4]];
            if quad.iter().all(|p| p[3] == 0) {
                continue;
            }
            let (mut a, mut cb, mut cr) = (0, 0, 0);
            for p in quad {
                let (pa, _, pb, pr) = px(p);
                (a, cb, cr) = (a + pa, cb + pb, cr + pr);
            }
            // means of the four: alpha a/4, colour cb/4 — the picture keeps 1 - a/4
            line[x] = ((line[x] as u32 * (1020 - a) + cb + 510) / 1020) as u8;
            line[x + 1] = ((line[x + 1] as u32 * (1020 - a) + cr + 510) / 1020) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_leaves_the_picture_and_an_opaque_colour_replaces_it() {
        let mut y = vec![50u8; 4 * 2];
        let mut uv = vec![90u8; 4];
        // left 2×2 block transparent, right one opaque white
        let (t, w) = ([0u8; 4], [255u8; 4]);
        let src: Vec<u8> = [t, t, w, w, t, t, w, w].concat();
        blend(&src, 16, 4, 2, &mut y, 4, &mut uv, 4);
        assert_eq!(y, vec![50, 50, 255, 255, 50, 50, 255, 255]);
        assert_eq!(&uv[..2], &[90, 90]);
        assert!(uv[2].abs_diff(128) <= 1 && uv[3].abs_diff(128) <= 1, "{uv:?}");
    }

    #[test]
    fn half_alpha_lands_half_way_and_a_quarter_covered_block_a_quarter_of_its_chroma() {
        let red = [255u8, 0, 0, 128];
        let al = 128.0 / 255.0;
        let (ry, rcr) = (0.299 * 255.0, 128.0 + 0.5 * 255.0);
        let mut y = vec![100u8; 4];
        let mut uv = vec![128u8; 2];
        let src: Vec<u8> = [red, [0; 4], [0; 4], [0; 4]].concat();
        blend(&src, 8, 2, 2, &mut y, 2, &mut uv, 2);
        assert!((y[0] as f32 - (100.0 * (1.0 - al) + ry * al)).abs() <= 1.0, "{y:?}");
        assert_eq!(&y[1..], &[100, 100, 100]);
        let want_cr = 128.0 * (1.0 - al / 4.0) + rcr * al / 4.0;
        assert!((uv[1] as f32 - want_cr).abs() <= 1.0, "{uv:?} want {want_cr}");
    }
}
