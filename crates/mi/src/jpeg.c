/* JPEG → NV12 for the picture windows: libjpeg-turbo (mozjpeg-sys) reading the stored planes as they are (raw data, no
 * upsampling, no colour conversion), its errors caught with setjmp — the library's default error handler exits the
 * process, and microcamd is built with panic = abort, so they cannot be caught on the Rust side. */
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "jpeglib.h"

struct err { struct jpeg_error_mgr pub; jmp_buf jb; };

static void on_error(j_common_ptr c) { longjmp(((struct err *)c->err)->jb, 1); }
static void on_message(j_common_ptr c) { (void)c; }

/* rows of the smallest component block after scaling (the field's name follows the API version of the headers) */
#if JPEG_LIB_VERSION >= 70
#define MIN_V_SCALED(c) ((c)->min_DCT_v_scaled_size)
#define COMP_V_SCALED(k) ((k)->DCT_v_scaled_size)
#else
#define MIN_V_SCALED(c) ((c)->min_DCT_scaled_size)
#define COMP_V_SCALED(k) ((k)->DCT_scaled_size)
#endif

/* the picture's size, and its size decoded at `eighths`/8; 0 = a readable baseline/progressive JPEG, -1 = not one */
int microcam_jpeg_size(const unsigned char *data, unsigned long len, unsigned eighths, unsigned *w, unsigned *h, unsigned *ow, unsigned *oh)
{
    struct jpeg_decompress_struct c;
    struct err e;
    c.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.output_message = on_message;
    if (setjmp(e.jb)) { jpeg_destroy_decompress(&c); return -1; }
    jpeg_create_decompress(&c);
    jpeg_mem_src(&c, data, len);
    jpeg_read_header(&c, TRUE);
    *w = c.image_width;
    *h = c.image_height;
    c.scale_num = eighths;
    c.scale_denom = 8;
    jpeg_calc_output_dimensions(&c);
    *ow = c.output_width;
    *oh = c.output_height;
    jpeg_destroy_decompress(&c);
    return 0;
}

/* Decode, scaled by `eighths`/8 (the decoder's own scaled IDCT), into an NV12 frame of that size: Y rows at `y` (stride
 * `ys`), interleaved Cb/Cr rows at `uv` (stride `uvs`). Only 3-component 4:2:0 pictures (what our encoder writes);
 * 0 = done, -1 = a decode error, -2 = another sampling (not handled yet). */
int microcam_jpeg_nv12(const unsigned char *data, unsigned long len, unsigned eighths, unsigned char *y, unsigned ys, unsigned char *uv, unsigned uvs)
{
    struct jpeg_decompress_struct c;
    struct err e;
    static unsigned char cb[16][4096], cr[16][4096];
    c.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.output_message = on_message;
    if (setjmp(e.jb)) { jpeg_destroy_decompress(&c); return -1; }
    jpeg_create_decompress(&c);
    jpeg_mem_src(&c, data, len);
    jpeg_read_header(&c, TRUE);
    jpeg_component_info *k = c.comp_info;
    if (c.num_components != 3 || k[0].h_samp_factor != 2 || k[0].v_samp_factor != 2 || k[1].h_samp_factor != 1 ||
        k[1].v_samp_factor != 1 || k[2].h_samp_factor != 1 || k[2].v_samp_factor != 1) {
        jpeg_destroy_decompress(&c);
        return -2;
    }
    c.raw_data_out = TRUE;
    c.do_fancy_upsampling = FALSE;
    c.scale_num = eighths;
    c.scale_denom = 8;
    jpeg_start_decompress(&c);
    unsigned W = c.output_width, H = c.output_height, rows = c.max_v_samp_factor * MIN_V_SCALED(&c);
    /* a scaled decode hands out the chroma at the luma's resolution (the library scales it up by its IDCT instead of
     * upsampling, jdmaster.c), unscaled at half: rows per call and samples per row as the library writes them */
    unsigned crows = COMP_V_SCALED(&k[1]), cwidth = k[1].width_in_blocks * crows, step = crows == rows ? 2 : 1;
    if (rows > 16 || (crows != rows && crows * 2 != rows) || cwidth > 4096) {
        jpeg_abort_decompress(&c);
        jpeg_destroy_decompress(&c);
        return -2;
    }
    JSAMPROW yr[16], cbr[16], crr[16];
    JSAMPARRAY planes[3] = { yr, cbr, crr };
    for (unsigned i = 0; i < crows; i++) { cbr[i] = cb[i]; crr[i] = cr[i]; }
    while (c.output_scanline < H) {
        unsigned l = c.output_scanline;
        /* rows past the picture's bottom (a last partial MCU row) all land on its last row */
        for (unsigned i = 0; i < rows; i++) yr[i] = y + (size_t)(l + i < H ? l + i : H - 1) * ys;
        jpeg_read_raw_data(&c, planes, rows);
        for (unsigned i = 0; i < rows / 2 && l / 2 + i < (H + 1) / 2; i++) {
            unsigned char *d = uv + (size_t)(l / 2 + i) * uvs;
            const unsigned char *b = cb[i * step], *r = cr[i * step];
            for (unsigned x = 0; x < (W + 1) / 2; x++) { d[2 * x] = b[x * step]; d[2 * x + 1] = r[x * step]; }
        }
    }
    jpeg_finish_decompress(&c);
    jpeg_destroy_decompress(&c);
    return 0;
}

/* Decode, scaled by `eighths`/8, into RGBA rows at `out` (stride `os`, the size microcam_jpeg_size gives); alpha 255.
 * 0 = done, -1 = a decode error. */
int microcam_jpeg_rgba(const unsigned char *data, unsigned long len, unsigned eighths, unsigned char *out, unsigned os)
{
    struct jpeg_decompress_struct c;
    struct err e;
    c.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.output_message = on_message;
    if (setjmp(e.jb)) { jpeg_destroy_decompress(&c); return -1; }
    jpeg_create_decompress(&c);
    jpeg_mem_src(&c, data, len);
    jpeg_read_header(&c, TRUE);
    c.out_color_space = JCS_EXT_RGBA;
    c.scale_num = eighths;
    c.scale_denom = 8;
    jpeg_start_decompress(&c);
    while (c.output_scanline < c.output_height) {
        JSAMPROW row = out + (size_t)c.output_scanline * os;
        jpeg_read_scanlines(&c, &row, 1);
    }
    jpeg_finish_decompress(&c);
    jpeg_destroy_decompress(&c);
    return 0;
}

/* YUYV rows (`w` even, `stride` bytes apart) → a baseline 4:2:0 JPEG at `quality` in memory the library allocates
 * (*out, `len` bytes; freed with microcam_jpeg_free): a thumbnail of ours. 0 = done, -1 = failed. */
int microcam_jpeg_from_yuyv(const unsigned char *yuyv, unsigned w, unsigned h, unsigned stride, int quality, unsigned char **out, unsigned long *len)
{
    struct jpeg_compress_struct c;
    struct err e;
    unsigned char *volatile row = NULL;
    *out = NULL;
    *len = 0;
    c.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.output_message = on_message;
    if (setjmp(e.jb)) {
        jpeg_destroy_compress(&c);
        free(row);
        free(*out);
        *out = NULL;
        return -1;
    }
    jpeg_create_compress(&c);
    jpeg_mem_dest(&c, out, len);
    c.image_width = w;
    c.image_height = h;
    c.input_components = 3;
    c.in_color_space = JCS_YCbCr;
    /* libjpeg's own baseline: mozjpeg's default profile writes a progressive file, which the hardware decoder refuses */
    jpeg_c_set_int_param(&c, JINT_COMPRESS_PROFILE, JCP_FASTEST);
    jpeg_set_defaults(&c);
    jpeg_set_quality(&c, quality, TRUE);
    jpeg_start_compress(&c, TRUE);
    row = malloc(w * 3);
    if (!row)
        longjmp(e.jb, 1);
    while (c.next_scanline < h) {
        const unsigned char *s = yuyv + (size_t)c.next_scanline * stride;
        for (unsigned x = 0; x < w; x += 2, s += 4) {
            unsigned char *d = row + x * 3;
            d[0] = s[0]; d[1] = s[1]; d[2] = s[3];
            d[3] = s[2]; d[4] = s[1]; d[5] = s[3];
        }
        JSAMPROW r = row;
        jpeg_write_scanlines(&c, &r, 1);
    }
    jpeg_finish_compress(&c);
    jpeg_destroy_compress(&c);
    free(row);
    return 0;
}

void microcam_jpeg_free(unsigned char *p) { free(p); }
