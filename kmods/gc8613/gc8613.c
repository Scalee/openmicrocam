// SPDX-License-Identifier: MIT-0 OR GPL-2.0
/*
 * GC8613 sensor driver of the YIZHAN 4K camera (SigmaStar SSC8836 / Mercury6), in place of the camera's stock
 * gc8613_MIPI.ko (docs/firmware/components/sensor-gc8613.md): the sensor modes microcamd runs (modes.py), the AE's
 * shutter, gain and frame rate, the orientation, behind mhal's sensor-driver interface (mhal_sensor.h).
 *
 * The sensor, GalaxyCore GC8613: I2C at 0x20 with 16-bit register addresses, MIPI CSI-2 RAW10, 24 MHz MCLK.
 * - Frame length (VTS, 0x0340/0x0341) and exposure (0x0202/0x0203) count lines, 14 bits each; a line is the mode's
 *   line time. The exposure stays EXPO_MARGIN lines inside the frame.
 * - Gain = an analog step (the eight registers of a row of tables/stock.txt [gain]) × a digital factor in 1/64
 *   (0x0064 integer part, 0x0065 fraction in bits 5:0; the write of 0x0065 takes the pair). The PGA pair 0x0614/0x0615
 *   is written under the gain hold, 0x031d = 0x2d … 0x28: the sensor maker's procedure, as the public drivers write it.
 * - Mirror and flip: 0x0063 and 0x022c, then the sensor maker's OTP autoload run again for the new readout direction.
 * What changes at run time is written at the frame-start notify, so a frame never starts half-programmed.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/delay.h>
#include "mhal_sensor.h"

struct mode {
	const char *name;
	const struct reg *regs;
	u16 n_regs;
	u16 width, height;
	u8 max_fps, lanes;
	u32 line_ns;
	u16 frame_lines;	/* VTS at max_fps */
	u16 expo;		/* the exposure the table leaves, lines */
	u8 pinned;		/* runs at max_fps whatever the ISP asks; its rate is set through fps_live */
};

struct gain_step {
	u32 gain;		/* 1024 = 1× */
	u8 val[8];		/* 0614 0615 0225 1467 1468 00b8 00b9 1447 */
};

#define PAUSE 0xffff		/* a table entry that waits its value in ms */
#include "modes.h"

#define MIN_FPS		5
#define EXPO_MARGIN	8	/* lines of a frame the exposure leaves free: the sensor's own (datasheet §7.9) */
#define LINES_MAX	0x3fff	/* the frame length and exposure fields */
/* the AE range the camera's ISP tuning (its IQ file) was made with: the stock driver's */
#define SHUTTER_MIN_US	30
#define SHUTTER_MAX_US	200000
#define GAIN_MIN	1024
#define GAIN_MAX	(776 * 1024)

/* the sensor's settings that change at run time */
struct ae {
	u16 frame;		/* frame length, lines */
	u16 expo;		/* exposure, lines */
	u8 step;		/* analog gain step; NO_GAIN: the sensor's own until the AE sets one */
	u16 dgain;		/* digital factor, 64 = 1× */
};
#define NO_GAIN 0xff

struct gc8613 {
	const struct mode *mode;
	u32 fps;		/* as set_fps got it (×1000 when it came so); 0 = the mode's */
	u16 rate_frame;		/* the frame length the rate asks for */
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
/* the camera's boot script loads the sensor driver with lane_num=4; each mode carries its own lane count */
static int lane_num;
module_param(lane_num, int, 0444);
MODULE_PARM_DESC(lane_num, "accepted and unused: each mode has its own lane count");

/* live: a rate in fps or a frame length in lines for the running mode from the next frame (never shorter than the
 * mode's frame); 0 once taken */
static int fps_live, vts_live;
module_param(fps_live, int, 0644);
module_param(vts_live, int, 0644);
/* what the sensor runs with: frame length, exposure lines, total gain (1024 = 1×) */
static uint ae_vts, ae_expo_lines, ae_gain;
module_param(ae_vts, uint, 0444);
module_param(ae_expo_lines, uint, 0444);
module_param(ae_gain, uint, 0444);
/* live register access at the next frame: write 0xRRRRVV to reg_poke (register RRRR := VV), 0xRRRR to reg_peek (its
 * value then in reg_peek_val, -2 when the read failed) */
static int reg_poke = -1, reg_peek = -1, reg_peek_val = -1;
module_param(reg_poke, int, 0644);
module_param(reg_peek, int, 0644);
module_param(reg_peek_val, int, 0444);

/* the mode list microcamd reads at start (modes.py writes it into modes.h with the register tables): read-only */
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

static struct gc8613 *pads[4];

static struct gc8613 *of(struct snr_handle *h)
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

/*
 * The sensor's power-up as its datasheet orders it (§7.2): supplies, then XSHUTDOWN high, and the first I2C access no
 * sooner than 50 µs after XSHUTDOWN and 1200 clock cycles after the clock runs. The board has two lines to the sensor,
 * both active low, and which of them is XSHUTDOWN is not known: the power-down line is let go as the supplies' switch,
 * the reset line as XSHUTDOWN.
 */
static int power_on(struct snr_handle *h, u32 pad)
{
	struct snr_if *p = h->pads;

	/* the CSI receiver ready for the mode's lanes before the sensor can send: RAW8, RAW10 and RAW12 long packets */
	p->set_pads(pad, MIPI_BUS, h->mipi_lanes);
	p->csi_clock(pad, CSI_CLK_216M);
	p->csi_lanes(pad, h->mipi_lanes, 1);
	p->csi_packets(pad, 0, 0x1c00, 0);
	/* from standby, whatever state the lines were left in */
	p->reset(pad, 1);
	p->power_down(pad, 1);
	CamOsUsSleep(1000);
	p->power_down(pad, 0);
	CamOsUsSleep(1000);	/* a supply behind the line settles */
	p->reset(pad, 0);
	p->mclk(pad, 1, MCLK_24M);
	CamOsUsSleep(100);
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

/* the chip id, 0x8613; the variant byte (0x03f2: 0x03 YN, 0x23 YA) is not part of it */
static int get_id(struct snr_handle *h, u32 *id)
{
	struct reg r[2] = { { 0x03f0 }, { 0x03f1 } };

	*id = 0;
	if (h->i2c->read_array(h->i2c, &h->i2c_cfg, r, 2))
		return -1;
	*id = (r[0].val & 0xff) << 8 | (r[1].val & 0xff);
	return *id == 0x8613 ? 0 : -1;
}

/* ---- modes ---- */

static u32 lines_per_second(const struct mode *m)
{
	return 1000000000u / m->line_ns;
}

/* the frame the rate asks for, stretched when the exposure needs more */
static void frame_for_expo(struct gc8613 *s)
{
	u32 f = s->rate_frame;

	if (!s->mode->pinned && s->want.expo + EXPO_MARGIN > f)
		f = s->want.expo + EXPO_MARGIN;
	s->want.frame = min_t(u32, f, LINES_MAX);
}

/* the shutter in the mode's lines: inside the frame of a pinned mode, else stretching the frame */
static void expo_from_shutter(struct gc8613 *s)
{
	const struct mode *m = s->mode;
	/* no exposure is 4 s long: the clamp keeps the product in 32 bits */
	u32 lines = s->shutter_us ? max(min(s->shutter_us, 4000000u) * 1000 / m->line_ns, 1u) : m->expo;
	u32 most = (m->pinned ? s->rate_frame : LINES_MAX) - EXPO_MARGIN;

	s->want.expo = min(lines, most);
	frame_for_expo(s);
}

/* the mode's table: every write tried three times, the table given up at a write that fails thrice */
static int init(struct snr_handle *h)
{
	struct gc8613 *s = of(h);
	const struct mode *m = s->mode;
	int i, tries;

	for (i = 0; i < m->n_regs; i++) {
		if (m->regs[i].reg == PAUSE) {
			msleep(m->regs[i].val);
			continue;
		}
		for (tries = 3; wr(h, m->regs[i].reg, m->regs[i].val); )
			if (!--tries) {
				pr_err("gc8613: %s: write %d of %d failed\n", m->name, i, m->n_regs);
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
	struct gc8613 *s = of(h);

	s->mode = &modes[i];
	s->fps = 0;
	s->rate_frame = modes[i].frame_lines;
	expo_from_shutter(s);
	h->cur_res = i;
	h->mipi_lanes = modes[i].lanes;		/* power_on and mhal read it */
}

static int set_res(struct snr_handle *h, u32 i)
{
	if (i >= ARRAY_SIZE(modes))
		return -1;
	use_mode(h, i);
	pr_info("gc8613: mode %u %s\n", i, modes[i].name);
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
	struct gc8613 *s = of(h);

	return s->fps ? s->fps : s->mode->max_fps;
}

/* a pinned mode keeps its frame; the others take the rate the ISP asks, in fps or in thousandths of one */
static int set_fps(struct snr_handle *h, u32 fps)
{
	struct gc8613 *s = of(h);
	const struct mode *m = s->mode;
	u32 milli = fps >= 1000 ? fps : fps * 1000;

	if (m->pinned)
		return 0;
	if (milli < MIN_FPS * 1000 || milli > m->max_fps * 1000)
		return -1;
	s->fps = fps;
	s->rate_frame = min_t(u32, lines_per_second(m) * 1000 / milli, LINES_MAX);
	frame_for_expo(s);
	return 0;
}

static int get_shutter(struct snr_handle *h, u32 *us)
{
	struct gc8613 *s = of(h);

	*us = s->want.expo * s->mode->line_ns / 1000;
	return 0;
}

static int set_shutter(struct snr_handle *h, u32 us)
{
	struct gc8613 *s = of(h);

	s->shutter_us = max(us, 1u);
	expo_from_shutter(s);
	return 0;
}

static int shutter_range(struct snr_handle *h, u32 *min, u32 *max)
{
	struct gc8613 *s = of(h);

	*min = SHUTTER_MIN_US;
	*max = SHUTTER_MAX_US;
	/* a pinned mode's shutter fits its frame: a longer one would cost the rate */
	if (s->mode->pinned)
		*max = (s->rate_frame - EXPO_MARGIN) * s->mode->line_ns / 1000;
	return 0;
}

static int shutter_info(struct snr_handle *h, u32 *info)
{
	struct gc8613 *s = of(h);

	/* {length (untouched), max, min, step} in ns */
	info[1] = SHUTTER_MAX_US * 1000u;
	info[2] = s->mode->line_ns;
	info[3] = s->mode->line_ns;
	return 0;
}

/* the gain the steps and the digital factor make */
static u32 total_gain(const struct ae *a)
{
	return gain_steps[a->step].gain * a->dgain / 64;
}

static int get_gain(struct snr_handle *h, u32 *gain)
{
	struct gc8613 *s = of(h);

	*gain = s->want.step == NO_GAIN ? GAIN_MIN : total_gain(&s->want);
	return 0;
}

/* the highest analog step not above the gain, the digital factor making up the rest */
static int set_gain(struct snr_handle *h, u32 gain)
{
	struct gc8613 *s = of(h);
	u8 k = 0;

	gain = clamp_t(u32, gain, GAIN_MIN, GAIN_MAX);
	while (k + 1 < ARRAY_SIZE(gain_steps) && gain_steps[k + 1].gain <= gain)
		k++;
	s->want.step = k;
	s->want.dgain = min_t(u32, (gain << 6) / gain_steps[k].gain, 0x3ff);
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
	const struct reg w[2] = { { 0x0340, a->frame >> 8 }, { 0x0341, a->frame & 0xff } };

	wr_all(h, w, 2);
	ae_vts = a->frame;
}

static void send_expo(struct snr_handle *h, struct ae *a)
{
	const struct reg w[2] = { { 0x0202, a->expo >> 8 }, { 0x0203, a->expo & 0xff } };

	wr_all(h, w, 2);
	ae_expo_lines = a->expo;
}

/* the analog step — the PGA pair under the hold, its bit 12, the column gain, the step's analog settings — then the
 * digital factor */
static void send_gain(struct snr_handle *h, struct ae *a, bool step)
{
	const u8 *v = gain_steps[a->step].val;
	const struct reg w[12] = {
		{ 0x031d, 0x2d }, { 0x0614, v[0] }, { 0x0615, v[1] }, { 0x031d, 0x28 },
		{ 0x0225, v[2] }, { 0x00b8, v[5] }, { 0x00b9, v[6] }, { 0x1447, v[7] }, { 0x1467, v[3] }, { 0x1468, v[4] },
		{ 0x0064, a->dgain >> 6 }, { 0x0065, a->dgain & 0x3f },
	};

	/* the digital factor alone when the step stays */
	if (step)
		wr_all(h, w, 12);
	else
		wr_all(h, w + 10, 2);
	ae_gain = total_gain(a);
}

/* mirror and flip (datasheet table 8; for both at once 0x0063 takes both bit sets, 0x07, as the stock driver writes it),
 * then the OTP autoload of the sensor maker's settings run again, the direction in the low bits of 0x0a73 */
static void send_orient(struct snr_handle *h, u8 o)
{
	const struct reg dir[2] = {
		{ 0x0063, (o & 1 ? 0x05 : 0) | (o & 2 ? 0x02 : 0) }, { 0x022c, o & 2 ? 1 : 0 },
	};
	const struct reg load[7] = {
		{ 0x0a67, 0x80 }, { 0x0a98, 0x04 }, { 0x05be, 0x00 }, { 0x05a9, 0x01 }, { 0x0a70, 0x03 },
		{ 0x0a73, 0x60 | o }, { 0x0a5a, 0x80 },
	};
	const struct reg done[4] = { { 0x05be, 0x01 }, { 0x0a70, 0x00 }, { 0x0080, 0x02 }, { 0x0a67, 0x00 } };

	wr_all(h, dir, 2);
	wr_all(h, load, 7);
	msleep(20);
	wr_all(h, done, 4);
}

static int get_orient(struct snr_handle *h, u32 *orient)
{
	*orient = of(h)->orient;
	return 0;
}

static int set_orient(struct snr_handle *h, u32 orient)
{
	struct gc8613 *s = of(h);

	if (orient > ORIENT_MIRROR_FLIP)
		return 0;
	s->orient = orient;
	s->orient_due = true;
	return 0;
}

static void live_knobs(struct snr_handle *h)
{
	struct gc8613 *s = of(h);
	const struct mode *m = s->mode;
	u32 f = 0;

	if (fps_live > 0) {
		f = lines_per_second(m) / fps_live;
		fps_live = 0;
	} else if (vts_live > EXPO_MARGIN) {
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

/* a frame has started: what changed goes to the sensor now. A longer frame goes before the exposure that needs it, a
 * shorter one after the exposure that fits it. */
static int frame_notify(struct snr_handle *h, u32 status)
{
	struct gc8613 *s = of(h);
	struct ae *w = &s->want, *o = &s->sent;
	bool longer;

	if (status != FRAME_ACTIVE)
		return 0;
	live_knobs(h);
	if (s->orient_due) {
		send_orient(h, s->orient);
		s->orient_due = false;
	}
	longer = w->frame > o->frame;
	if (longer && w->frame != o->frame)
		send_frame(h, w);
	if (w->expo != o->expo)
		send_expo(h, w);
	if (!longer && w->frame != o->frame)
		send_frame(h, w);
	if (w->step != NO_GAIN && (w->step != o->step || w->dgain != o->dgain))
		send_gain(h, w, w->step != o->step);
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

static int fill_handle(struct snr_handle *h)
{
	struct gc8613 *s = of(h);
	int i;

	if (!s)
		return -1;
	strlcpy(h->model, "gc8613_MIPI", sizeof(h->model));
	h->i2c_cfg = (struct snr_i2c_cfg){ .mode = I2C_NORMAL, .format = I2C_A16D8, .speed = 200000, .addr = 0x20 };
	/* the board: reset and power-down active low; the ISP outside the sensor; RGGB under the mirror and flip the
	 * camera runs with, 10-bit raw on MIPI */
	h->reset_pol = 1;
	h->pwdn_pol = 0;
	h->vsync_pol = 1;
	h->hsync_pol = 1;
	h->pclk_pol = 0;
	h->isp_type = 1;
	h->bayer = 0;
	h->rgbir = 0;
	h->bus = MIPI_BUS;
	h->precision = 1;
	h->conversion = 5;
	h->orient = ORIENT_NORMAL;
	h->mipi_format = 1;
	h->mclk = MCLK_24M;
	h->min_gain = GAIN_MIN;
	/* the AE's shutter and gain take effect two frames after their write */
	h->gain_delay = h->shutter_delay = 2;
	h->gain_ctrls = h->shutter_ctrls = 1;

	BUILD_BUG_ON(ARRAY_SIZE(modes) > ARRAY_SIZE(h->res));
	h->n_res = ARRAY_SIZE(modes);
	for (i = 0; i < ARRAY_SIZE(modes); i++) {
		const struct mode *m = &modes[i];

		h->res[i] = (struct snr_res){ .width = m->width, .height = m->height, .max_fps = m->max_fps,
					      .min_fps = MIN_FPS, .out_width = m->width, .out_height = m->height };
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

	*s = (struct gc8613){ .want = { .step = NO_GAIN } };
	use_mode(h, 0);
	s->sent = s->want;
	return 0;
}

static int __init gc8613_load(void)
{
	int pad;

	/* the interface versions this driver is built for */
	if (DrvSensorHandleVer(2, 2) == -1 || DrvSensorIFVer(2, 1) == -1 || DrvSensorI2CVer(1, 1) == -1)
		return -EINVAL;
	for (pad = 0; pad < ARRAY_SIZE(pads); pad++) {
		if (!(chmap & BIT(pad)))
			continue;
		pads[pad] = CamOsMemAlloc(sizeof(*pads[pad]));
		if (!pads[pad])
			return -ENOMEM;
		*pads[pad] = (struct gc8613){};
		DrvRegisterSensorDriverEx(pad, fill_handle, pads[pad]);
		DrvRegisterSensorI2CSlaveID(pad, i2c_slave_id >> (8 * pad) & 0xff);
	}
	return 0;
}

static void __exit gc8613_unload(void)
{
	int pad;

	for (pad = 0; pad < ARRAY_SIZE(pads); pad++)
		if (pads[pad]) {
			DrvSensorRelease(pad);
			CamOsMemRelease(pads[pad]);
		}
}

module_init(gc8613_load);
module_exit(gc8613_unload);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_DESCRIPTION("GC8613 sensor driver of the YIZHAN 4K camera (SigmaStar SSC8836 / Mercury6)");
