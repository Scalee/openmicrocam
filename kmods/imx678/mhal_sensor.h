/* SPDX-License-Identifier: MIT-0 OR GPL-2.0 */
/*
 * The sensor-driver interface of the YIZHAN 4K camera's mhal.ko (SigmaStar SSC8836 / Mercury6; DrvSensorHandleVer 2.2,
 * DrvSensorIFVer 2.1, DrvSensorI2CVer 1.1), as far as this driver uses it: docs/firmware/components/mhal-csi-snr.md §2.
 *
 * What is the interface: the ten imported functions' names and signatures, the handle's size, the offsets, sizes and
 * value encodings of the fields mhal reads or this driver fills, the order of the function slots. The type and field
 * names are ours; the gaps nobody here touches are reserved bytes. Every offset is pinned below.
 */
#ifndef MHAL_SENSOR_H
#define MHAL_SENSOR_H

#include <linux/types.h>
#include <linux/stddef.h>

struct snr_version { u16 major, minor; };

/* ---- I2C (the bus mhal gives the driver) ---- */
struct reg { u16 reg, val; };			/* a register write; also the I2C array element */

enum { I2C_A8D8, I2C_A16D8, I2C_A8D16, I2C_A16D16 };	/* address and data widths */
enum { I2C_LEGACY, I2C_NORMAL };

struct snr_i2c_cfg {
	u32 mode, format, speed;
	u16 addr, reserved;
};

struct snr_i2c {
	struct snr_version version;
	u8 reserved0[0x14 - 0x04];
	int (*write)(struct snr_i2c *i2c, struct snr_i2c_cfg *cfg, u16 reg, u16 val);			/* 0x14 */
	void *reserved1;
	int (*write_array)(struct snr_i2c *i2c, struct snr_i2c_cfg *cfg, struct reg *w, u32 n);	/* 0x1c */
	void *reserved2[2];
	int (*read_array)(struct snr_i2c *i2c, struct snr_i2c_cfg *cfg, struct reg *r, u32 n);	/* 0x28 */
};

/* ---- the sensor pads: power, clocks, the CSI receiver ---- */
enum { MIPI_BUS = 1 };
enum { CSI_CLK_OFF = -1, CSI_CLK_216M = 4 };
enum { MCLK_24M = 10 };

struct snr_if {
	struct snr_version version;
	void *reserved0;
	int (*power_down)(u32 pad, int asserted);	/* 0x08: the power-down pin, 1 = asserted (sensor off) */
	int (*reset)(u32 pad, int asserted);		/* 0x0c: the reset pin, 1 = asserted (sensor held in reset) */
	int (*mclk)(u32 pad, u8 on, u32 freq);		/* 0x10 */
	void *reserved1[2];
	int (*set_pads)(u32 pad, u32 bus, u32 lanes);	/* 0x1c */
	int (*csi_clock)(u32 pad, int clk);		/* 0x20 */
	int (*csi_lanes)(u32 pad, u16 lanes, u8 on);	/* 0x24 */
	int (*csi_packets)(u32 pad, u16 w0, u16 w1, u16 w2);	/* 0x28: the long-packet data types accepted */
};

/* ---- the handle ---- */
enum { ORIENT_NORMAL, ORIENT_MIRROR, ORIENT_FLIP, ORIENT_MIRROR_FLIP };
enum { FRAME_ACTIVE = 1 };			/* the frame notify's status: a frame has started */

struct snr_res {				/* one entry of the resolution list, 72 bytes */
	u32 width, height, max_fps, min_fps;
	u32 crop_x, crop_y, out_width, out_height;
	char name[32];
	u32 reserved[2];
};

struct snr_handle;
typedef int (*snr_fn)(struct snr_handle *h);

struct snr_handle {
	struct snr_version version;
	char model[32];				/* 0x004 */
	void *reserved0;
	void *priv;				/* 0x028: the pointer given at registration */
	u32 reserved1;
	struct snr_i2c_cfg i2c_cfg;		/* 0x030 */
	struct snr_i2c *i2c;			/* 0x040 */
	struct snr_if *pads;			/* 0x044 */
	int gain_delay, shutter_delay;		/* 0x048: frames from a write to its effect */
	int gain_ctrls, shutter_ctrls;		/* 0x050 */
	u32 reset_pol, pwdn_pol, vsync_pol, hsync_pol, pclk_pol;	/* 0x058 */
	u32 isp_type;				/* 0x06c: 1 = external sensor */
	u32 bayer;				/* 0x070: 0 = RGGB */
	u32 rgbir;				/* 0x074 */
	u32 bus;				/* 0x078 */
	u32 precision;				/* 0x07c: 1 = 10 bits */
	u32 conversion;				/* 0x080: 5 = 10 to 12 bits, 9000 */
	u32 orient;				/* 0x084 */
	u32 reserved2[2];
	u32 n_res, cur_res;			/* 0x090 */
	struct snr_res res[31];			/* 0x098 */
	u32 mipi_lanes;				/* 0x950 */
	u32 mipi_format;			/* 0x954: 1 = raw */
	u32 mipi_yuv_order, mipi_hsync_mode, mipi_sampling_delay, mipi_hdr_mode, mipi_hdr_vc;
	u32 reserved3[4];
	u32 mclk;				/* 0x97c */
	u32 min_gain;				/* 0x980 */
	u32 reserved4[2];
	int (*power_on)(struct snr_handle *h, u32 pad);			/* 0x98c */
	int (*power_off)(struct snr_handle *h, u32 pad);		/* 0x990 */
	snr_fn init;							/* 0x994 */
	void *post_init;						/* 0x998 */
	snr_fn release;							/* 0x99c */
	void *suspend, *resume;						/* 0x9a0 */
	int (*test_pattern)(struct snr_handle *h, u32 mode);		/* 0x9a8 */
	int (*get_id)(struct snr_handle *h, u32 *id);			/* 0x9ac */
	int (*get_res)(struct snr_handle *h, u32 i, struct snr_res **res);	/* 0x9b0 */
	int (*get_cur_res)(struct snr_handle *h, u32 *i, struct snr_res **res);	/* 0x9b4 */
	int (*set_res)(struct snr_handle *h, u32 i);			/* 0x9b8 */
	int (*get_orient)(struct snr_handle *h, u32 *orient);		/* 0x9bc */
	int (*set_orient)(struct snr_handle *h, u32 orient);		/* 0x9c0 */
	void *get_caps;							/* 0x9c4 */
	int (*frame_notify)(struct snr_handle *h, u32 status);		/* 0x9c8 */
	int (*get_shutter)(struct snr_handle *h, u32 *us);		/* 0x9cc */
	int (*set_shutter)(struct snr_handle *h, u32 us);		/* 0x9d0 */
	int (*get_gain)(struct snr_handle *h, u32 *gain);		/* 0x9d4 */
	int (*set_gain)(struct snr_handle *h, u32 gain);		/* 0x9d8 */
	int (*shutter_range)(struct snr_handle *h, u32 *min, u32 *max);	/* 0x9dc */
	int (*gain_range)(struct snr_handle *h, u32 *min, u32 *max);	/* 0x9e0 */
	int (*get_fps)(struct snr_handle *h);				/* 0x9e4 */
	int (*set_fps)(struct snr_handle *h, u32 fps);			/* 0x9e8 */
	void *set_gain_cal;						/* 0x9ec */
	int (*gain_linearity)(struct snr_handle *h, void *gaps, u32 n);	/* 0x9f0 */
	int (*shutter_info)(struct snr_handle *h, u32 *info);		/* 0x9f4: {length, max, min, step} */
	int (*res_count)(struct snr_handle *h, u32 *n);			/* 0x9f8 */
	void *reserved5[8];
};

#define AT(field, off) _Static_assert(offsetof(struct snr_handle, field) == (off), #field)
_Static_assert(sizeof(struct snr_handle) == 2588, "the handle mhal allocates");
_Static_assert(sizeof(struct snr_res) == 72 && sizeof(struct snr_i2c_cfg) == 16, "res, i2c cfg");
AT(model, 0x004); AT(priv, 0x028); AT(i2c_cfg, 0x030); AT(i2c, 0x040); AT(pads, 0x044); AT(gain_delay, 0x048);
AT(shutter_ctrls, 0x054); AT(reset_pol, 0x058); AT(pclk_pol, 0x068); AT(isp_type, 0x06c); AT(bus, 0x078);
AT(conversion, 0x080); AT(orient, 0x084); AT(n_res, 0x090); AT(res, 0x098); AT(mipi_lanes, 0x950); AT(mipi_hdr_vc, 0x968);
AT(mclk, 0x97c); AT(min_gain, 0x980); AT(power_on, 0x98c); AT(release, 0x99c); AT(test_pattern, 0x9a8);
AT(frame_notify, 0x9c8); AT(get_fps, 0x9e4); AT(shutter_info, 0x9f4); AT(res_count, 0x9f8);
_Static_assert(offsetof(struct snr_i2c, write) == 0x14 && offsetof(struct snr_i2c, write_array) == 0x1c &&
	       offsetof(struct snr_i2c, read_array) == 0x28, "snr_i2c");
_Static_assert(offsetof(struct snr_if, power_down) == 0x08 && offsetof(struct snr_if, set_pads) == 0x1c &&
	       offsetof(struct snr_if, csi_packets) == 0x28, "snr_if");

/* ---- mhal's and cam_os's exports ---- */
void *CamOsMemAlloc(u32 size);
void CamOsMemRelease(void *p);
void CamOsPrintf(const char *fmt, ...);
void CamOsUsSleep(u32 us);
int DrvRegisterSensorDriverEx(u32 pad, int (*init_handle)(struct snr_handle *h), void *priv);
int DrvRegisterSensorI2CSlaveID(u32 pad, u8 id);
int DrvSensorHandleVer(u32 major, u32 minor);
int DrvSensorIFVer(u32 major, u32 minor);
int DrvSensorI2CVer(u32 major, u32 minor);
void DrvSensorRelease(u32 pad);

#endif
