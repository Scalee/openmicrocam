// SPDX-License-Identifier: MIT-0 OR GPL-2.0
/*
 * IMX678 sensor driver of the black YIZHAN 4K camera (SigmaStar SSC8836 / Mercury6, the XHU4006 firmware), in place of the
 * camera's vendor imx678_MIPI.ko: the sensor modes microcamd runs (modes.py), the AE's shutter, gain and frame rate, the
 * orientation, behind mhal's sensor-driver interface (OpenMicroCam's mhal_sensor.h). Made from OpenMicroCam's GC8613
 * driver; what is the IMX678's comes from the vendor driver (its tables, power-up, handle) and the sensor's public Linux
 * driver (register names).
 *
 * The sensor, Sony IMX678: I2C at 0x34 (8-bit form) with 16-bit register addresses, MIPI CSI-2 RAW10 on 4 lanes, 24 MHz INCK.
 * - Frame length VMAX (0x3028, 20 bits LE) in lines; a line is HMAX counts of 74.25 MHz (the mode's line time).
 * - Exposure: SHR (0x3050, 20 bits LE) counts from the frame's end: exposure = VMAX - SHR lines, SHR at least SHR_MIN.
 * - Gain: 0x3070/0x3071, 11 bits, in 0.3 dB (0 .. 240; above 100 = 30 dB the sensor adds digital gain).
 * - Mirror and flip: 0x3020 and 0x3021.
 * The run-time settings are written at the frame-start notify inside the register hold (REGHOLD 0x3001), so they take
 * effect together on one frame.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/delay.h>
#include "mhal_sensor.h"

struct mode {
	const char *name;
	const struct reg *regs;
	u16 n_regs;
	u16 width, height;		/* what the ISP gives out */
	u16 out_width, out_height;	/* what the sensor sends (the ISP crops the margin) */
	u8 max_fps, lanes;
	u32 line_ns;
	u32 frame_lines;		/* VMAX at max_fps */
	u32 expo;			/* the exposure the table leaves, lines */
};

#define PAUSE 0xffff		/* a table entry that waits its value in ms */
#include "modes.h"

#define MIN_FPS		3
#define SHR_MIN		8	/* the sensor's: SHR below it is not taken */
#define LINES_MAX	0xfffff	/* VMAX and SHR are 20-bit */
#define REGHOLD		0x3001
/* the AE range of the vendor driver, which the camera's ISP tuning (its IQ file) was made with */
#define SHUTTER_MIN_US	8
#define SHUTTER_MAX_US	333333
#define GAIN_MIN	1024
#define GAIN_MAX	0x3e3400	/* 72 dB */

/* the sensor's settings that change at run time */
struct ae {
	u32 frame;		/* VMAX, lines */
	u32 expo;		/* exposure, lines */
	u16 code;		/* gain code, 0.3 dB; NO_GAIN: the sensor's own until the AE sets one */
};
#define NO_GAIN 0xffff

struct imx678 {
	const struct mode *mode;
	u32 fps;		/* as set_fps got it (x1000 when it came so); 0 = the mode's */
	u32 rate_frame;		/* the frame length the rate asks for */
	u32 shutter_us;		/* the AE's shutter; 0: the table's exposure until the AE sets one */
	struct ae want;		/* what the next frame is to run with */
	struct ae sent;		/* what the sensor has */
	u8 orient;		/* bit 0 mirror, bit 1 flip */
	bool orient_due;
};

static int chmap;
module_param(chmap, int, 0444);
MODULE_PARM_DESC(chmap, "sensor pads to drive, a bit each");
static int i2c_slave_id;
module_param(i2c_slave_id, int, 0444);
MODULE_PARM_DESC(i2c_slave_id, "per pad, a byte each: the I2C slave id mhal uses");
static int lane_num;
module_param(lane_num, int, 0444);
MODULE_PARM_DESC(lane_num, "accepted and unused: each mode has its own lane count");

/* live: a rate in fps or a frame length in lines for the running mode from the next frame (never shorter than the
 * mode's frame); 0 once taken */
static int fps_live, vts_live;
module_param(fps_live, int, 0644);
module_param(vts_live, int, 0644);
/* what the sensor runs with: frame length, exposure lines, total gain (1024 = 1x) */
static uint ae_vts, ae_expo_lines, ae_gain;
module_param(ae_vts, uint, 0444);
module_param(ae_expo_lines, uint, 0444);
module_param(ae_gain, uint, 0444);
/* mains frequency of the lighting: an exposure the frame cannot hold is cut to a whole number of its half-cycles (10 ms
 * at 50 Hz), the ISP's anti-flicker steps; 0 = cut to the frame alone */
static int flicker_hz = 50;
module_param(flicker_hz, int, 0644);
/* the shutter the AE last asked for, us (before the frame's cap) */
static uint ae_shutter_us;
module_param(ae_shutter_us, uint, 0444);
/* live register access at the next frame: write 0xRRRRVV to reg_poke (register RRRR := VV), 0xRRRR to reg_peek (its
 * value then in reg_peek_val, -2 when the read failed) */
static int reg_poke = -1, reg_peek = -1, reg_peek_val = -1;
module_param(reg_poke, int, 0644);
module_param(reg_peek, int, 0644);
module_param(reg_peek_val, int, 0444);

static int mode_list_get(char *buf, const struct kernel_param *kp)
{
	return scnprintf(buf, PAGE_SIZE, "%s", mode_list);
}

static int mode_list_set(const char *val, const struct kernel_param *kp)
{
	return -EPERM;
}

static const struct kernel_param_ops mode_list_ops = { .set = mode_list_set, .get = mode_list_get };
module_param_cb(modes, &mode_list_ops, NULL, 0444);
MODULE_PARM_DESC(modes, "the driver's modes as microcamd reads them");

static struct imx678 *pads[4];

static struct imx678 *of(struct snr_handle *h)
{
	return h->priv;
}

static int wr(struct snr_handle *h, u16 reg, u8 val)
{
	return h->i2c->write(h->i2c, &h->i2c_cfg, reg, val);
}

static int wr_all(struct snr_handle *h, const struct reg *w, u32 n)
{
	return h->i2c->write_array(h->i2c, &h->i2c_cfg, (struct reg *)w, n);
}

/* ---- power ---- */

/* the vendor driver's power-up: both lines asserted, the CSI receiver set for 4 lanes and the raw long packets, the
 * power-down line let go, 40 ms, the reset line let go, then the clock */
static int power_on(struct snr_handle *h, u32 pad)
{
	struct snr_if *p = h->pads;

	p->power_down(pad, 1);
	p->reset(pad, 1);
	p->set_pads(pad, MIPI_BUS, h->mipi_lanes);
	p->csi_clock(pad, CSI_CLK_216M);
	p->csi_lanes(pad, h->mipi_lanes, 1);
	p->csi_packets(pad, 0, 0x1c00, 0);
	p->power_down(pad, 0);
	msleep(40);
	p->reset(pad, 0);
	CamOsUsSleep(1);
	p->mclk(pad, 1, MCLK_24M);
	CamOsUsSleep(20);
	return 0;
}

static int power_off(struct snr_handle *h, u32 pad)
{
	struct snr_if *p = h->pads;

	p->reset(pad, 1);
	p->power_down(pad, 1);
	p->mclk(pad, 0, MCLK_24M);
	p->csi_clock(pad, CSI_CLK_OFF);
	return 0;
}

/* the vendor driver's check: the sensor answers a read of 0x3f12 / 0x3f13 */
static int get_id(struct snr_handle *h, u32 *id)
{
	struct reg r[2] = { { 0x3f12 }, { 0x3f13 } };
	int tries;

	*id = 0;
	for (tries = 4; h->i2c->read_array(h->i2c, &h->i2c_cfg, r, 2); msleep(1))
		if (!--tries)
			return -1;
	*id = (r[0].val & 0xff) << 8 | (r[1].val & 0xff);
	return 0;
}

/* ---- modes ---- */

static u32 lines_per_second(const struct mode *m)
{
	return 1000000000u / m->line_ns;
}

/* the frame the rate asks for: the rate comes first, the exposure stays inside it (a longer one would cost frames; the
 * AE makes up with gain) */
static void frame_for_expo(struct imx678 *s)
{
	s->want.frame = min_t(u32, s->rate_frame, LINES_MAX);
}

static void expo_from_shutter(struct imx678 *s)
{
	const struct mode *m = s->mode;
	/* no exposure is 4 s long: the clamp keeps the product in 32 bits */
	u32 lines = s->shutter_us ? max(min(s->shutter_us, 4000000u) * 1000 / m->line_ns, 1u) : m->expo;
	u32 most = s->rate_frame - SHR_MIN;

	/* longer than the frame: the longest flicker-free exposure that fits (the AE asked a multiple of the half-cycle) */
	if (lines > most && flicker_hz > 0) {
		u32 half_us = 500000 / flicker_hz;
		u32 fit_us = most * m->line_ns / 1000 / half_us * half_us;

		if (fit_us)
			most = fit_us * 1000 / m->line_ns;
	}
	s->want.expo = min_t(u32, lines, most);
	frame_for_expo(s);
}

/* the mode's table: every write tried three times, the table given up at a write that fails thrice */
static int init(struct snr_handle *h)
{
	struct imx678 *s = of(h);
	const struct mode *m = s->mode;
	int i, tries;

	for (i = 0; i < m->n_regs; i++) {
		if (m->regs[i].reg == PAUSE) {
			msleep(m->regs[i].val);
			continue;
		}
		for (tries = 3; wr(h, m->regs[i].reg, m->regs[i].val); )
			if (!--tries) {
				pr_err("imx678: %s: write %d of %d failed\n", m->name, i, m->n_regs);
				return -1;
			}
	}
	/* what the table left; the AE's settings go out at the next frame */
	s->sent = (struct ae){ m->frame_lines, m->expo, NO_GAIN };
	ae_vts = m->frame_lines;
	ae_expo_lines = m->expo;
	return 0;
}

static void use_mode(struct snr_handle *h, u32 i)
{
	struct imx678 *s = of(h);

	s->mode = &modes[i];
	s->fps = 0;
	s->rate_frame = modes[i].frame_lines;
	expo_from_shutter(s);
	h->cur_res = i;
	h->mipi_lanes = modes[i].lanes;
}

static int set_res(struct snr_handle *h, u32 i)
{
	if (i >= ARRAY_SIZE(modes))
		return -1;
	use_mode(h, i);
	pr_info("imx678: mode %u %s\n", i, modes[i].name);
	return 0;
}

static int res_count(struct snr_handle *h, u32 *n)
{
	*n = h->n_res;
	return 0;
}

static int get_res(struct snr_handle *h, u32 i, struct snr_res **res)
{
	if (i >= h->n_res)
		return -1;
	*res = &h->res[i];
	return 0;
}

static int get_cur_res(struct snr_handle *h, u32 *i, struct snr_res **res)
{
	*i = h->cur_res;
	return get_res(h, h->cur_res, res);
}

/* ---- AE: frame length, exposure, gain ---- */

static int get_fps(struct snr_handle *h)
{
	struct imx678 *s = of(h);

	return s->fps ? s->fps : s->mode->max_fps;
}

/* the rate the ISP asks, in fps or in thousandths of one */
static int set_fps(struct snr_handle *h, u32 fps)
{
	struct imx678 *s = of(h);
	const struct mode *m = s->mode;
	u32 milli = fps >= 1000 ? fps : fps * 1000;

	if (milli < MIN_FPS * 1000 || milli > m->max_fps * 1000)
		return -1;
	s->fps = fps;
	s->rate_frame = clamp_t(u32, (u64)lines_per_second(m) * 1000 / milli, m->frame_lines, LINES_MAX);
	expo_from_shutter(s);
	return 0;
}

static int get_shutter(struct snr_handle *h, u32 *us)
{
	struct imx678 *s = of(h);

	*us = s->want.expo * s->mode->line_ns / 1000;
	return 0;
}

static int set_shutter(struct snr_handle *h, u32 us)
{
	struct imx678 *s = of(h);

	s->shutter_us = max(us, 1u);
	ae_shutter_us = s->shutter_us;
	expo_from_shutter(s);
	return 0;
}

/* up to the frame of the rate asked: the AE does not stretch the frame (frame rate first) */
static int shutter_range(struct snr_handle *h, u32 *min, u32 *max)
{
	struct imx678 *s = of(h);

	*min = SHUTTER_MIN_US;
	*max = min_t(u32, SHUTTER_MAX_US, (s->rate_frame - SHR_MIN) * s->mode->line_ns / 1000);
	return 0;
}

static int shutter_info(struct snr_handle *h, u32 *info)
{
	struct imx678 *s = of(h);

	/* {length (untouched), max, min, step} in ns */
	info[1] = SHUTTER_MAX_US * 1000u;
	info[2] = s->mode->line_ns;
	info[3] = s->mode->line_ns;
	return 0;
}

static int get_gain(struct snr_handle *h, u32 *gain)
{
	struct imx678 *s = of(h);

	*gain = s->want.code == NO_GAIN ? GAIN_MIN : gain_codes[s->want.code];
	return 0;
}

/* the highest code not above the gain */
static int set_gain(struct snr_handle *h, u32 gain)
{
	struct imx678 *s = of(h);
	u16 c = 0;

	gain = clamp_t(u32, gain, GAIN_MIN, GAIN_MAX);
	while (c + 1 < ARRAY_SIZE(gain_codes) && gain_codes[c + 1] <= gain)
		c++;
	s->want.code = c;
	return 0;
}

static int gain_range(struct snr_handle *h, u32 *min, u32 *max)
{
	*min = GAIN_MIN;
	*max = GAIN_MAX;
	return 0;
}

static void send_frame(struct snr_handle *h, struct ae *a)
{
	const struct reg w[3] = { { 0x3028, a->frame & 0xff }, { 0x3029, a->frame >> 8 & 0xff }, { 0x302a, a->frame >> 16 & 0x0f } };

	wr_all(h, w, 3);
	ae_vts = a->frame;
}

static void send_expo(struct snr_handle *h, struct ae *a)
{
	u32 shr = a->frame > a->expo + SHR_MIN ? a->frame - a->expo : SHR_MIN;
	const struct reg w[3] = { { 0x3050, shr & 0xff }, { 0x3051, shr >> 8 & 0xff }, { 0x3052, shr >> 16 & 0x0f } };

	wr_all(h, w, 3);
	ae_expo_lines = a->expo;
}

static void send_gain(struct snr_handle *h, struct ae *a)
{
	const struct reg w[2] = { { 0x3070, a->code & 0xff }, { 0x3071, a->code >> 8 & 0x07 } };

	wr_all(h, w, 2);
	ae_gain = gain_codes[a->code];
}

static void send_orient(struct snr_handle *h, u8 o)
{
	const struct reg w[2] = { { 0x3020, o & 1 }, { 0x3021, o >> 1 & 1 } };

	wr_all(h, w, 2);
}

static int get_orient(struct snr_handle *h, u32 *orient)
{
	*orient = of(h)->orient;
	return 0;
}

static int set_orient(struct snr_handle *h, u32 orient)
{
	struct imx678 *s = of(h);

	if (orient > ORIENT_MIRROR_FLIP)
		return 0;
	s->orient = orient;
	s->orient_due = true;
	return 0;
}

static void live_knobs(struct snr_handle *h)
{
	struct imx678 *s = of(h);
	const struct mode *m = s->mode;
	u32 f = 0;

	if (fps_live > 0) {
		f = lines_per_second(m) / fps_live;
		fps_live = 0;
	} else if (vts_live > SHR_MIN) {
		f = vts_live;
		vts_live = 0;
	}
	if (!f)
		return;
	s->rate_frame = clamp_t(u32, f, m->frame_lines, LINES_MAX);
	expo_from_shutter(s);
}

static void peek_poke(struct snr_handle *h)
{
	int v;

	if (reg_poke >= 0) {
		v = reg_poke;
		reg_poke = -1;
		wr(h, v >> 8 & 0xffff, v & 0xff);
	}
	if (reg_peek >= 0) {
		struct reg r = { reg_peek & 0xffff };

		reg_peek = -1;
		reg_peek_val = h->i2c->read_array(h->i2c, &h->i2c_cfg, &r, 1) ? -2 : r.val;
	}
}

/* a frame has started: what changed goes to the sensor now, inside the register hold, so frame length, exposure and gain
 * take effect on the same frame */
static int frame_notify(struct snr_handle *h, u32 status)
{
	struct imx678 *s = of(h);
	struct ae *w = &s->want, *o = &s->sent;
	bool gain = w->code != NO_GAIN && w->code != o->code;

	if (status != FRAME_ACTIVE)
		return 0;
	live_knobs(h);
	if (s->orient_due || w->frame != o->frame || w->expo != o->expo || gain) {
		wr(h, REGHOLD, 1);
		if (s->orient_due) {
			send_orient(h, s->orient);
			s->orient_due = false;
		}
		if (w->frame != o->frame)
			send_frame(h, w);
		/* SHR counts from the frame's end: a new frame length needs it written again */
		if (w->expo != o->expo || w->frame != o->frame)
			send_expo(h, w);
		if (gain)
			send_gain(h, w);
		wr(h, REGHOLD, 0);
	}
	*o = *w;
	peek_poke(h);
	return 0;
}

static int nothing(struct snr_handle *h)
{
	return 0;
}

static int no_pattern(struct snr_handle *h, u32 mode)
{
	return 0;
}

static int no_linearity(struct snr_handle *h, void *gaps, u32 n)
{
	return 0;
}

/* ---- the handle ---- */

/* the vendor driver's handle values (cus_camsensor_init_handle_linear) */
static int fill_handle(struct snr_handle *h)
{
	struct imx678 *s = of(h);
	int i;

	if (!s)
		return -1;
	strlcpy(h->model, "IMX678_MIPI", sizeof(h->model));
	h->i2c_cfg = (struct snr_i2c_cfg){ .mode = I2C_NORMAL, .format = I2C_A16D8, .speed = 200000, .addr = 0x34 };
	h->reset_pol = 1;
	h->pwdn_pol = 1;
	h->isp_type = 1;
	h->bayer = 0;
	h->rgbir = 0;
	h->bus = MIPI_BUS;
	h->precision = 1;
	h->conversion = 4;
	h->orient = ORIENT_NORMAL;
	h->mipi_format = 1;
	h->mclk = MCLK_24M;
	h->min_gain = GAIN_MIN;
	h->gain_delay = h->shutter_delay = 2;
	h->gain_ctrls = h->shutter_ctrls = 1;

	BUILD_BUG_ON(ARRAY_SIZE(modes) > ARRAY_SIZE(h->res));
	h->n_res = ARRAY_SIZE(modes);
	for (i = 0; i < ARRAY_SIZE(modes); i++) {
		const struct mode *m = &modes[i];

		h->res[i] = (struct snr_res){ .width = m->width, .height = m->height, .max_fps = m->max_fps,
					      .min_fps = MIN_FPS, .out_width = m->out_width, .out_height = m->out_height };
		strlcpy(h->res[i].name, m->name, sizeof(h->res[i].name));
	}

	h->power_on = power_on;
	h->power_off = power_off;
	h->init = init;
	h->release = nothing;
	h->test_pattern = no_pattern;
	h->get_id = get_id;
	h->res_count = res_count;
	h->get_res = get_res;
	h->get_cur_res = get_cur_res;
	h->set_res = set_res;
	h->get_orient = get_orient;
	h->set_orient = set_orient;
	h->frame_notify = frame_notify;
	h->get_shutter = get_shutter;
	h->set_shutter = set_shutter;
	h->get_gain = get_gain;
	h->set_gain = set_gain;
	h->shutter_range = shutter_range;
	h->gain_range = gain_range;
	h->get_fps = get_fps;
	h->set_fps = set_fps;
	h->gain_linearity = no_linearity;
	h->shutter_info = shutter_info;

	*s = (struct imx678){ .want = { .code = NO_GAIN } };
	use_mode(h, 0);
	s->sent = s->want;
	return 0;
}

static int __init imx678_load(void)
{
	int pad;

	if (DrvSensorHandleVer(2, 2) == -1 || DrvSensorIFVer(2, 1) == -1 || DrvSensorI2CVer(1, 1) == -1)
		return -EINVAL;
	for (pad = 0; pad < ARRAY_SIZE(pads); pad++) {
		if (!(chmap & BIT(pad)))
			continue;
		pads[pad] = CamOsMemAlloc(sizeof(*pads[pad]));
		if (!pads[pad])
			return -ENOMEM;
		*pads[pad] = (struct imx678){};
		DrvRegisterSensorDriverEx(pad, fill_handle, pads[pad]);
		DrvRegisterSensorI2CSlaveID(pad, i2c_slave_id >> (8 * pad) & 0xff);
	}
	return 0;
}

static void __exit imx678_unload(void)
{
	int pad;

	for (pad = 0; pad < ARRAY_SIZE(pads); pad++)
		if (pads[pad]) {
			DrvSensorRelease(pad);
			CamOsMemRelease(pads[pad]);
		}
}

module_init(imx678_load);
module_exit(imx678_unload);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_DESCRIPTION("IMX678 sensor driver of the black YIZHAN 4K camera (SigmaStar SSC8836 / Mercury6)");
