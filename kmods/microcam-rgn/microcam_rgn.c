// SPDX-License-Identifier: MIT-0 OR GPL-2.0
/*
 * microcam_rgn: our RGN (SCL output OSD) driver in place of the vendor mi_rgn.ko of the YIZHAN 4K camera (SigmaStar
 * SSC8836 / Mercury6). The vendor's module and the hardware it drives: docs/firmware/components/rgn-gop.md («spec»).
 *
 * Interface: the vendor one. `/dev/mi_rgn` takes the same 13 ioctls in the same 16-byte wrapper with the same payloads,
 * checks and MI error codes (spec §2.2, §2.3, §3.2, §3.4), so libmi_rgn and any program written for it work unchanged;
 * mi_scl calls the same two exports on every SCL frame (spec §3.5).
 *
 * Inside it differs from the vendor driver:
 * - every OSD region has two canvases, or three when asked (`echo canvases <handle> 3 > /proc/microcam_rgn`: a region
 *   updated every frame then never waits for one). The user writes a back one (GetCanvasInfo … UpdateCanvas,
 *   SetBitMap); the update makes it the front one, which the GOP shows from the next frame programmed on. A canvas is
 *   writable again once every frame that was programmed with it, and the frame after them, is done (NotifyFrameDone):
 *   a frame scans with the GOP registers the frames before it programmed — the GOP read a canvas freed at the Done of
 *   the last frame programmed with it, in the next frame (MMU read faults, 2026-11-14). It is brought up to the
 *   front's content by a BDMA copy before it is lent — so a canvas keeps its content across updates, as the vendor's
 *   does. The vendor's separate front buffers and their BDMA composition do not exist;
 * - a window's display attributes set while its region's canvas is lent wait for the update and take effect with it,
 *   in the same frame: a window moved and redrawn never shows the new pixels at the old place;
 * - every shown OSD window of a port is one GWIN of the port's GOP, scanned straight from its region's front canvas:
 *   the GOP fetches only the windows' area, and a port takes at most 8 OSD windows (the GWIN count; the vendor
 *   composes more into shared front buffers — not done here);
 * - the regions are torn down when the process that called Init closes the device, not when any process does.
 * GOP registers are written only inside Process, into the frame's CMDQ — but for the GWINs of a GOP whose ports get no
 * frames any more, which the reaper turns off at once: no Process would, and a canvas an enabled GWIN scans is never
 * freed.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/fs.h>
#include <linux/miscdevice.h>
#include <linux/uaccess.h>
#include <linux/slab.h>
#include <linux/list.h>
#include <linux/mutex.h>
#include <linux/wait.h>
#include <linux/jiffies.h>
#include <linux/workqueue.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/sort.h>

/* the camera's kernel exports no __memzero, which our tree's string.h makes the inline copy_from_user call */
void __memzero(void *p, __kernel_size_t n)
{
	(memset)(p, 0, n);
}

/* ---- mhal RGN (mhal.ko; the exports of spec §4.6) ---- */
struct mhal_win { u32 x, y, w, h; };
int MHAL_RGN_SetupCmdQ(void *cmdq, int type);
int MHAL_RGN_GopInitByType(u32 type);
int MHAL_RGN_CoverInit(void);
int MHAL_RGN_CoverSetColor(int cover, int layer, u32 color);
int MHAL_RGN_CoverSetWindow(int cover, int layer, u32 x, u32 y, u32 w, u32 h);
int MHAL_RGN_CoverEnable(int cover, int layer);
int MHAL_RGN_CoverDisable(int cover, int layer);
int MHAL_RGN_SetGopBindOsd(u32 mask);
int MHAL_RGN_GetGopBindOsd(u32 *mask);
int MHAL_RGN_GopSetPalette(int gop, u8 a, u8 r, u8 g, u8 b, u8 idx);
int MHAL_RGN_GopSetBaseWindow(int gop, struct mhal_win *src, struct mhal_win *dst);
int MHAL_RGN_GopGwinSetPixelFormat(int gop, int gwin, int fmt);
int MHAL_RGN_GopGwinSetWindow(int gop, int gwin, u32 w, u32 h, u32 stride, u32 x, u32 y);
int MHAL_RGN_GopGwinSetBufferXoffset(int gop, int gwin, u16 xoff);
int MHAL_RGN_GopGwinSetBuffer(int gop, int gwin, u64 addr);
int MHAL_RGN_GopGwinEnable(int gop, int gwin);
int MHAL_RGN_GopGwinDisable(int gop, int gwin);
int MHAL_RGN_GopSetColorkey(int gop, u8 en, u8 r, u8 g, u8 b);
int MHAL_RGN_GopSetAlphaType(int gop, int gwin, int type, u8 val);
int MHAL_RGN_GopSetArgb1555AlphaVal(int gop, int gwin, int which, u8 val);
int MHAL_RGN_GopSetAlphaZeroOpaque(int gop, u8 en, u8 const_alpha, int fmt);

/* ---- mi_sys (this build's MMA_Alloc takes heap and owner names; MemsetPa / MemcpyPa run on the BDMA engine) ---- */
int mi_sys_MMA_Alloc(const char *heap, const char *owner, u32 size, u64 *phy);
int mi_sys_MMA_Free(u64 phy);
int mi_sys_MemsetPa(u64 pa, u32 val, u32 len);
int mi_sys_MemcpyPa(u64 dst, u64 src, u32 len);
void *mi_sys_Vmap(u64 pa, u32 size, u8 cached);
void mi_sys_UnVmap(void *va);
int mi_sys_VFlushInvCache(void *va, u32 size);

/* ---- the user ABI (spec §2.2, §2.3) ---- */
#define E(x) ((int)(0xA0032000u | (x)))
#define E_DEVID		E(0x01)
#define E_PARAM		E(0x03)
#define E_NULL		E(0x06)
#define E_SUPPORT	E(0x08)
#define E_PERM		E(0x09)
#define E_NOMEM		E(0x0C)
#define E_NOTREADY	E(0x10)
#define E_BUSY		E(0x12)

enum { T_OSD, T_COVER };
enum { F_1555, F_4444, F_I2, F_I4, F_I8, F_565, F_8888 };
enum { ALPHA_PIXEL, ALPHA_CONST };		/* the API's alpha modes */
enum { GOP_ALPHA_CONST, GOP_ALPHA_PIXEL };	/* MHAL_RGN_GopSetAlphaType's */
#define HANDLES		1024
#define GWINS		8
#define COVERS		4

struct u_attr { u32 type, fmt, w, h; };
struct u_bitmap { u32 fmt, w, h, data; };
struct u_chnport { u32 mod; s32 dev, chn, port; };
struct u_param {
	u8 show, pad0[3];
	u32 x, y;
	union {
		struct { u32 layer, w, h, color; } cover;
		struct {
			u32 layer, alpha_mode;
			u8 a0, a1, pad1[2];	/* PIXEL: background, foreground alpha; CONSTANT: a0 */
			u8 inv_en, pad2[3];
			u32 inv_mode;
			u16 inv_luma, inv_wdiv, inv_hdiv, pad3;
		} osd;
		u8 raw[28];
	} u;
};
struct u_canvas { u64 phy; u32 va, w, h, stride, fmt, pad; };

/* ---- what mi_scl passes (spec §3.5; probed live: Process and Done alternate with up to two frames in flight) ---- */
struct rgn_cmd {
	u32 module, dev, chn, port;
	u8 enable, dbuf, del_delay, pad;
	u32 w, h, x, y, rsv;
	u32 path;	/* 0 SCL output port, 1 LDC */
	u32 idx;	/* the HW scaler output: picks the GOP */
};
/* more frames than mi_scl ever has in flight on a port: a frame further back was done, or dropped without a notice
 * (SCL drops the frames in flight when a port is disabled) */
#define IN_FLIGHT 2

/* ---- state ---- */
struct port {
	struct list_head node;
	struct u_chnport cp;
	struct list_head osd, cover;	/* windows */
	int n_osd, n_cover;
	bool scaled;
	u32 cw, ch, sw, sh;		/* ScaleRect: canvas (source) and screen sizes */
	u64 seq, done;			/* frames programmed, frames done (or beyond IN_FLIGHT) */
	unsigned long active;		/* jiffies of the last Process or Done */
	int gop, cover_hw;		/* last programmed, -1 none */
};

/* frames that may still read a canvas: done once every port's `done` passed its `seq` (or the port went idle) */
struct hold {
	struct port *p;
	u64 seq;
};
#define HOLDS 8

struct canvas {
	u64 phy;
	struct hold hold[HOLDS];
	int n_hold;
	u32 ver;		/* the update whose content it holds */
};
#define CANVASES	3

struct region {
	struct list_head node;
	u32 handle;
	struct u_attr attr;
	u32 stride, size;
	struct canvas cv[CANVASES];
	int n;			/* canvases */
	int front;		/* cv[front] is shown */
	int back;		/* cv[back] is written (valid while lent) */
	u32 ver;		/* updates so far */
	bool lent;
	struct list_head wins;	/* attachments */
};

struct win {
	struct list_head rnode, pnode;
	struct region *r;
	struct port *p;
	struct u_param param;
	struct u_param staged;	/* set while the canvas was lent: applied with the update */
	bool has_staged;
};

/* a canvas freed once nothing reads it any more */
struct gone {
	struct list_head node;
	struct canvas cv;
};

static DEFINE_MUTEX(api);		/* serialises the ioctls (the vendor module mutex) */
static DEFINE_MUTEX(lock);		/* everything Process / NotifyFrameDone read; never held across BDMA or allocation */
static DECLARE_WAIT_QUEUE_HEAD(frames);
static LIST_HEAD(regions);
static LIST_HEAD(ports);
static LIST_HEAD(graveyard);
static bool ready;			/* Init done */
static bool hw_ready;			/* the SCL GOPs initialised (once per boot) */
static struct file *owner;
static u8 gwin_on[GWINS];		/* per GOP: GWINs this driver enabled */
static u64 gwin_cv[GWINS][GWINS];	/* per GOP and GWIN: the canvas an enabled one scans */
static u8 cover_on[7];			/* per cover engine: layers enabled */
static u8 palette[1024];
static void reap(struct work_struct *w);
static DECLARE_DELAYED_WORK(reaper, reap);

static const int gop_of_idx[6] = { 0, 1, 2, 3, 6, 7 };	/* MHAL_RGN_GopType_e VPE_PORT0..3, 4, 5 */
static const int cover_of_idx[6] = { 0, 1, 2, 3, 5, 6 };

static u32 bpp(u32 fmt)
{
	static const u8 b[] = { 16, 16, 2, 4, 8, 16, 32 };
	return fmt < ARRAY_SIZE(b) ? b[fmt] : 0;
}

static struct region *region_find(u32 h)
{
	struct region *r;

	list_for_each_entry(r, &regions, node)
		if (r->handle == h)
			return r;
	return NULL;
}

static bool same_port(const struct u_chnport *a, const struct u_chnport *b)
{
	return a->mod == b->mod && a->dev == b->dev && a->chn == b->chn && a->port == b->port;
}

/* under `lock`; made on first use and never freed (a port's counters must outlive its windows) */
static struct port *port_get(const struct u_chnport *cp, bool make)
{
	struct port *p;

	list_for_each_entry(p, &ports, node)
		if (same_port(&p->cp, cp))
			return p;
	if (!make)
		return NULL;
	p = kzalloc(sizeof(*p), GFP_KERNEL);
	if (!p)
		return NULL;
	p->cp = *cp;
	INIT_LIST_HEAD(&p->osd);
	INIT_LIST_HEAD(&p->cover);
	p->gop = p->cover_hw = -1;
	p->active = jiffies;
	list_add_tail(&p->node, &ports);
	return p;
}

static struct win *win_find(struct region *r, const struct u_chnport *cp)
{
	struct win *w;

	list_for_each_entry(w, &r->wins, rnode)
		if (same_port(&w->p->cp, cp))
			return w;
	return NULL;
}

/* ---- canvas holds ---- */

/* `p` gets no frames any more: no sign of life for longer than any frame takes. A port with every frame done is not
 * idle: its next frame scans with the GOP registers its last Process set. */
static bool idle(struct port *p)
{
	return time_after(jiffies, p->active + HZ / 2);
}

/* a frame of the hold may still read its canvas: one up to `seq`, or the one after them, which scans with the GOP
 * registers they programmed (a frame's CMDQ writes take effect from the frame after it) */
static bool reading(const struct hold *h)
{
	return h->p->done <= h->seq && !idle(h->p);
}

static bool free_of_frames(struct canvas *c)
{
	int i;

	for (i = 0; i < c->n_hold; i++)
		if (reading(&c->hold[i]))
			return false;
	return true;
}

/* under `lock`: frames of `p` up to `seq` may read `c` */
static void hold_add(struct canvas *c, struct port *p, u64 seq)
{
	int i;

	for (i = 0; i < c->n_hold; i++)
		if (c->hold[i].p == p) {
			c->hold[i].seq = max(c->hold[i].seq, seq);
			return;
		}
	if (c->n_hold < HOLDS) {
		c->hold[c->n_hold].p = p;
		c->hold[c->n_hold].seq = seq;
		c->n_hold++;
		return;
	}
	/* full (more ports than any pipeline has): a finished entry makes room, else the newest frames win */
	for (i = 0; i < HOLDS; i++)
		if (!reading(&c->hold[i]))
			break;
	c->hold[i == HOLDS ? HOLDS - 1 : i] = (struct hold){ p, seq };
}

/* under `lock`: every frame programmed so far on a port showing `r` may read `c` */
static void hold_shown(struct region *r, struct canvas *c)
{
	struct win *w;

	list_for_each_entry(w, &r->wins, rnode)
		hold_add(c, w->p, w->p->seq);
}

/* a canvas other than the front that no frame reads, the newest such; -1 none */
static int free_back(struct region *r)
{
	int i, b = -1;

	mutex_lock(&lock);
	for (i = 0; i < r->n; i++)
		if (i != r->front && free_of_frames(&r->cv[i]) && (b < 0 || r->cv[i].ver > r->cv[b].ver))
			b = i;
	mutex_unlock(&lock);
	return b;
}

/* a back canvas writable and holding the front's content (`r->back`): waits for the frames still reading one */
static int back_ready(struct region *r)
{
	struct canvas *b;
	int i = -1;

	wait_event_timeout(frames, (i = free_back(r)) >= 0, HZ);
	/* no frame came for a second: the port is gone or stuck, nothing reads the old front any more */
	if (i < 0)
		i = (r->front + 1) % r->n;
	b = &r->cv[i];
	b->n_hold = 0;
	if (b->ver != r->cv[r->front].ver) {
		if (mi_sys_MemcpyPa(b->phy, r->cv[r->front].phy, r->size))
			return E_NOMEM;
		b->ver = r->cv[r->front].ver;
	}
	r->back = i;
	return 0;
}

/* the back canvas shown from the next frame on, with the display attributes set meanwhile */
static void flip(struct region *r)
{
	struct win *w;
	int old = r->front;

	mutex_lock(&lock);
	r->front = r->back;
	r->cv[r->front].ver = ++r->ver;
	hold_shown(r, &r->cv[old]);
	list_for_each_entry(w, &r->wins, rnode)
		if (w->has_staged) {
			w->param = w->staged;
			w->has_staged = false;
		}
	mutex_unlock(&lock);
}

/* ---- freeing ---- */

/* under `api`: both canvases to the graveyard, held by the frames that may still read them */
static void bury(struct region *r)
{
	int i;

	mutex_lock(&lock);
	hold_shown(r, &r->cv[r->front]);
	mutex_unlock(&lock);
	for (i = 0; i < r->n; i++) {
		struct gone *g = kzalloc(sizeof(*g), GFP_KERNEL);

		if (!g) {
			/* better leaked than freed under a GOP */
			pr_err("microcam_rgn: canvas %llx of region %u leaked\n", r->cv[i].phy, r->handle);
			continue;
		}
		g->cv = r->cv[i];
		mutex_lock(&lock);
		list_add_tail(&g->node, &graveyard);
		mutex_unlock(&lock);
	}
	schedule_delayed_work(&reaper, HZ / 20);
}

static void gwins_off(int gop, u8 keep);

/* under `lock`: an enabled GWIN still scans `c` (its registers change only in a frame's Process) */
static bool scanned(const struct canvas *c)
{
	int gop, g;

	for (gop = 0; gop < GWINS; gop++)
		for (g = 0; g < GWINS; g++)
			if ((gwin_on[gop] & BIT(g)) && gwin_cv[gop][g] == c->phy)
				return true;
	return false;
}

/* under `lock`: the GWINs of GOPs whose ports get no frames any more go off at once — no Process would turn them off,
 * and the canvases they scan could never be freed (a port disabled right after its last window left) */
static void gwins_of_idle_ports_off(void)
{
	struct port *p;
	int gop;

	for (gop = 0; gop < GWINS; gop++) {
		bool live = false;

		if (!gwin_on[gop])
			continue;
		list_for_each_entry(p, &ports, node)
			if (p->gop == gop && !idle(p))
				live = true;
		if (live)
			continue;
		gwins_off(gop, 0);
		list_for_each_entry(p, &ports, node)
			if (p->gop == gop)
				p->gop = -1;
	}
}

static void reap(struct work_struct *work)
{
	struct gone *g, *n;
	LIST_HEAD(dead);
	bool left;

	mutex_lock(&lock);
	gwins_of_idle_ports_off();
	list_for_each_entry_safe(g, n, &graveyard, node)
		if (free_of_frames(&g->cv) && !scanned(&g->cv))
			list_move(&g->node, &dead);
	left = !list_empty(&graveyard);
	mutex_unlock(&lock);
	list_for_each_entry_safe(g, n, &dead, node) {
		mi_sys_MMA_Free(g->cv.phy);
		kfree(g);
	}
	if (left)
		schedule_delayed_work(&reaper, HZ / 20);
}

/* ---- windows ---- */

static void win_drop(struct win *w)
{
	mutex_lock(&lock);
	list_del(&w->rnode);
	list_del(&w->pnode);
	if (w->r->attr.type == T_OSD)
		w->p->n_osd--;
	else
		w->p->n_cover--;
	mutex_unlock(&lock);
	kfree(w);
}

static void region_free(struct region *r)
{
	struct win *w, *n;

	/* the canvases' holds are taken while the windows still name their ports */
	if (r->attr.type == T_OSD)
		bury(r);
	list_for_each_entry_safe(w, n, &r->wins, rnode)
		win_drop(w);
	list_del(&r->node);
	kfree(r);
}

/* ---- the GOP and cover programming of one frame (spec §4.3–§4.5), under `lock` ---- */

/* the highest layer first: GWIN 0 shows over the others where windows overlap (the vendor's order, spec §4.7) */
static int by_layer(const void *a, const void *b)
{
	const struct win *x = *(const struct win **)a, *y = *(const struct win **)b;

	return (int)y->param.u.osd.layer - (int)x->param.u.osd.layer;
}

/* a cover coordinate, in the API's 1/8192 of the frame's size, in pixels to the nearest */
static u32 cover_px(u32 v, u32 size)
{
	return (v * size + 4096) >> 13;
}

static void program_covers(struct port *p, int engine, u32 W, u32 H)
{
	struct win *w;
	u8 on = 0;
	int n = 0, l;

	if (p->cover_hw >= 0 && p->cover_hw != engine) {
		for (l = 0; l < COVERS; l++)
			if (cover_on[p->cover_hw] & BIT(l))
				MHAL_RGN_CoverDisable(p->cover_hw, l);
		cover_on[p->cover_hw] = 0;
	}
	/* p->cover is kept sorted by layer */
	list_for_each_entry(w, &p->cover, pnode) {
		struct u_param *q = &w->param;
		u32 x, y, cw, ch;

		if (!q->show || n == COVERS)
			continue;
		x = cover_px(q->x, W);
		y = cover_px(q->y, H);
		cw = min(cover_px(q->u.cover.w, W), W - min(x, W));
		ch = min(cover_px(q->u.cover.h, H), H - min(y, H));
		MHAL_RGN_CoverSetWindow(engine, n, x, y, cw, ch);
		MHAL_RGN_CoverSetColor(engine, n, q->u.cover.color);
		MHAL_RGN_CoverEnable(engine, n);
		on |= BIT(n);
		n++;
	}
	for (l = n; l < COVERS; l++)
		if (cover_on[engine] & BIT(l))
			MHAL_RGN_CoverDisable(engine, l);
	cover_on[engine] = on;
	p->cover_hw = on ? engine : -1;
}

/* the GWINs of `gop` this driver turned on, but those in `keep`, off */
static void gwins_off(int gop, u8 keep)
{
	int g;

	for (g = 0; g < GWINS; g++)
		if ((gwin_on[gop] & BIT(g)) && !(keep & BIT(g))) {
			MHAL_RGN_GopGwinDisable(gop, g);
			gwin_cv[gop][g] = 0;
		}
	gwin_on[gop] &= keep;
}

/* a window as the GOP shows it: its region's front canvas, w × h of it at the window's place */
struct shown {
	struct region *r;
	struct u_param *q;
	u32 w, h;
};

/* GWIN g shows `s`: what it reads, where and how it blends are set before it is switched on — the GOP's registers are
 * double-buffered and a frame's writes reach them together in its CMDQ, and a GWIN switched on last never scans a
 * half-set window even without that */
static void gwin_show(int gop, int g, const struct shown *s)
{
	const struct u_param *q = s->q;
	u64 addr = s->r->cv[s->r->front].phy;
	/* the address register counts 16-byte units; the rest is an offset in pixels into the first unit */
	u32 head = (u32)addr & 15;

	MHAL_RGN_GopGwinSetBuffer(gop, g, addr - head);
	MHAL_RGN_GopGwinSetBufferXoffset(gop, g, head * 8 / bpp(s->r->attr.fmt));
	MHAL_RGN_GopGwinSetWindow(gop, g, s->w, s->h, s->r->stride, q->x, q->y);
	if (q->u.osd.alpha_mode == ALPHA_CONST) {
		MHAL_RGN_GopSetAlphaType(gop, g, GOP_ALPHA_CONST, q->u.osd.a0);
	} else {
		MHAL_RGN_GopSetAlphaType(gop, g, GOP_ALPHA_PIXEL, 0);
		/* ARGB1555 alone takes its pixels' alpha from the GWIN: the A bit picks a1 (1) or a0 (0) */
		if (s->r->attr.fmt == F_1555) {
			MHAL_RGN_GopSetArgb1555AlphaVal(gop, g, 1, q->u.osd.a1);
			MHAL_RGN_GopSetArgb1555AlphaVal(gop, g, 0, q->u.osd.a0);
		}
	}
	MHAL_RGN_GopGwinEnable(gop, g);
	gwin_cv[gop][g] = addr;
}

/* The port's shown OSD windows on its GOP, the highest layer on GWIN 0. All GWINs of a GOP take one pixel format, the
 * top window's: a window of another format is not shown. A window starts at its canvas's corner and is clipped on the
 * right and at the bottom to the canvas area; one wholly outside is not shown. The colour key stays as GopInitByType
 * left it, off. */
static void program_osd(struct port *p, int gop, u32 W, u32 H)
{
	struct win *list[GWINS], *w;
	struct shown s[GWINS];
	u32 cw = W, ch = H, sw = W, sh = H, fmt = 0;
	struct mhal_win src, dst;
	int n = 0, k = 0, i;
	u8 on;

	if (p->gop >= 0 && p->gop != gop) {
		/* the port moved to another HW scaler output: its old GOP lets go */
		gwins_off(p->gop, 0);
		p->gop = -1;
	}
	if (p->scaled) {
		cw = p->cw;
		ch = p->ch;
		sw = p->sw;
		sh = p->sh;
	}
	/* what is shown */
	list_for_each_entry(w, &p->osd, pnode)
		if (w->param.show && n < GWINS)
			list[n++] = w;
	sort(list, n, sizeof(list[0]), by_layer, NULL);
	for (i = 0; i < n; i++) {
		struct region *r = list[i]->r;
		struct u_param *q = &list[i]->param;

		if (q->x >= cw || q->y >= ch || (k && r->attr.fmt != fmt))
			continue;
		fmt = r->attr.fmt;
		s[k++] = (struct shown){ r, q, min(r->attr.w, cw - q->x), min(r->attr.h, ch - q->y) };
	}
	on = (1u << k) - 1;
	/* the GOP programmed for it: the GWINs that leave, the GOP's own settings, each window */
	gwins_off(gop, on);
	if (k) {
		/* one format field for all GWINs of an SCL GOP (spec §4.4): written through GWIN 0 */
		MHAL_RGN_GopGwinSetPixelFormat(gop, 0, fmt);
		/* the canvas area, and the screen it maps to (an SCL GOP does not stretch: it takes the first alone) */
		src = (struct mhal_win){ 0, 0, cw, ch };
		dst = (struct mhal_win){ 0, 0, sw, sh };
		MHAL_RGN_GopSetBaseWindow(gop, &src, &dst);
	}
	for (i = 0; i < k; i++)
		gwin_show(gop, i, &s[i]);
	gwin_on[gop] |= on;
	p->gop = k ? gop : -1;
}

/* GOP 7 (VPE_PORT5) shares the OSD blender with DIP and LDC: routed to it the vendor's way (spec §3.5) */
static bool gop_routed(int gop)
{
	u32 mask = 0;

	if (gop != 7)
		return true;
	MHAL_RGN_GetGopBindOsd(&mask);
	if (mask & 0x80)
		return true;
	return MHAL_RGN_SetGopBindOsd((mask & ~0x30) | 0x80) == 0;
}

int mi_rgn_Process(struct rgn_cmd *c, void *cmdq)
{
	struct u_chnport cp;
	struct port *p;
	int gop, engine;

	if (!c)
		return 0;
	cp = (struct u_chnport){ c->module, c->dev, c->chn, c->port };
	mutex_lock(&lock);
	p = port_get(&cp, false);
	if (!hw_ready) {
		mutex_unlock(&lock);
		return 0;
	}
	if (p) {
		p->seq++;
		p->active = jiffies;
		if (p->seq - p->done > IN_FLIGHT)
			p->done = p->seq - IN_FLIGHT;
	}
	if (!c->enable || c->path != 0 || c->idx >= ARRAY_SIZE(gop_of_idx)) {
		mutex_unlock(&lock);
		return 0;
	}
	gop = gop_of_idx[c->idx];
	engine = cover_of_idx[c->idx];
	if (!(p && (p->n_osd || p->n_cover || p->gop >= 0 || p->cover_hw >= 0)) && !gwin_on[gop] && !cover_on[engine]) {
		mutex_unlock(&lock);
		return 0;
	}
	MHAL_RGN_SetupCmdQ(cmdq, 0);
	if (p) {
		if (p->n_cover || p->cover_hw >= 0 || cover_on[engine])
			program_covers(p, engine, c->w, c->h);
		if (gop_routed(gop))
			program_osd(p, gop, c->w, c->h);
	} else {
		/* a port none of our windows are on, on a HW output something of ours was left on */
		int l;

		gwins_off(gop, 0);
		for (l = 0; l < COVERS; l++)
			if (cover_on[engine] & BIT(l))
				MHAL_RGN_CoverDisable(engine, l);
		cover_on[engine] = 0;
	}
	/* nothing else may write into this frame's queue once it is out of our hands */
	MHAL_RGN_SetupCmdQ(NULL, 0);
	mutex_unlock(&lock);
	return 0;
}
EXPORT_SYMBOL(mi_rgn_Process);

int mi_rgn_NotifyFrameDone(struct u_chnport *cp)
{
	struct port *p;

	if (!cp)
		return 0;
	mutex_lock(&lock);
	p = port_get(cp, false);
	if (p) {
		if (p->done < p->seq)
			p->done++;
		p->active = jiffies;
	}
	mutex_unlock(&lock);
	if (p)
		wake_up_all(&frames);
	return 0;
}
EXPORT_SYMBOL(mi_rgn_NotifyFrameDone);

/* ---- the IMPL calls (spec §3.4), under `api` ---- */

static void hw_init(void)
{
	int gop, fmt;

	if (hw_ready)
		return;
	mutex_lock(&lock);
	MHAL_RGN_GopInitByType(0);
	for (gop = 0; gop < 8; gop++)
		for (fmt = F_1555; fmt <= F_I8; fmt++)
			MHAL_RGN_GopSetAlphaZeroOpaque(gop, 0, 0, fmt);
	MHAL_RGN_CoverInit();
	hw_ready = true;
	mutex_unlock(&lock);
}

static int do_init(struct file *f, u32 upal)
{
	int gop, i;

	if (!upal)
		return E_NULL;
	if (ready)
		return E_BUSY;
	if (copy_from_user(palette, (void __user *)(uintptr_t)upal, sizeof(palette)))
		return E_NULL;
	hw_init();
	mutex_lock(&lock);
	for (gop = 0; gop < 8; gop++)
		for (i = 0; i < 256; i++)
			MHAL_RGN_GopSetPalette(gop, palette[4 * i], palette[4 * i + 1], palette[4 * i + 2], palette[4 * i + 3], i);
	mutex_unlock(&lock);
	ready = true;
	owner = f;
	return 0;
}

static int do_deinit(void)
{
	struct region *r, *n;

	if (!ready)
		return E_BUSY;
	list_for_each_entry_safe(r, n, &regions, node)
		region_free(r);
	ready = false;
	owner = NULL;
	return 0;
}

static int do_create(u32 h, struct u_attr *a)
{
	struct region *r;
	char name[24];
	int i;

	if (h >= HANDLES || region_find(h))
		return E_DEVID;
	if (a->type == T_OSD) {
		if (a->w < 1 || a->w > 4096 || a->h < 1 || a->h > 4096 || a->fmt > F_I8)
			return E_PARAM;
		if ((a->fmt == F_I2 && a->w % 4) || (a->fmt == F_I4 && a->w % 2))
			return E_PARAM;
	} else if (a->type != T_COVER) {
		return E_PARAM;
	}
	r = kzalloc(sizeof(*r), GFP_KERNEL);
	if (!r)
		return E_NOMEM;
	r->handle = h;
	r->attr = *a;
	INIT_LIST_HEAD(&r->wins);
	if (a->type == T_OSD) {
		r->stride = ALIGN((a->w * bpp(a->fmt) + 7) / 8, 16);
		r->size = r->stride * a->h;
		r->n = 2;
		for (i = 0; i < 2; i++) {
			snprintf(name, sizeof(name), "microcam-rgn%u_%d", h, i);
			if (mi_sys_MMA_Alloc(NULL, name, r->size, &r->cv[i].phy) == 0 &&
			    mi_sys_MemsetPa(r->cv[i].phy, 0, r->size) == 0)
				continue;
			if (r->cv[i].phy)
				mi_sys_MMA_Free(r->cv[i].phy);
			if (i)
				mi_sys_MMA_Free(r->cv[0].phy);
			kfree(r);
			return E_NOMEM;
		}
	}
	list_add_tail(&r->node, &regions);
	return 0;
}

static int do_destroy(u32 h)
{
	struct region *r = region_find(h);

	if (!r)
		return E_DEVID;
	region_free(r);
	return 0;
}

static int do_get_attr(u32 h, struct u_attr *a)
{
	struct region *r = region_find(h);

	if (!r)
		return E_DEVID;
	*a = r->attr;
	return 0;
}

static int do_set_bitmap(u32 h, struct u_bitmap *b)
{
	struct region *r = region_find(h);
	u32 w, hh, row, y;
	void *va;
	int ret;

	if (!r)
		return E_DEVID;
	if (!b->data)
		return E_NULL;
	if (r->attr.type != T_OSD)
		return E_PARAM;
	/* the vendor answers these with 0 and does nothing */
	if (r->lent || b->fmt > F_8888 || b->w == 0)
		return 0;
	if (b->h < 1 || b->h > 4096 || b->w > 4096 || (b->fmt == F_I2 && b->w % 4) || (b->fmt == F_I4 && b->w % 2))
		return E_PARAM;
	if (b->fmt != r->attr.fmt)
		return E_PARAM;
	w = min(b->w, r->attr.w);
	hh = min(b->h, r->attr.h);
	/* the vendor's reading: rows of the clipped width, packed */
	row = (w * bpp(b->fmt) + 7) / 8;
	if (!access_ok(VERIFY_READ, (void __user *)(uintptr_t)b->data, row * hh))
		return E_PARAM;
	ret = back_ready(r);
	if (ret)
		return ret;
	va = mi_sys_Vmap(r->cv[r->back].phy, r->size, 1);
	if (!va)
		return E_NOMEM;
	for (y = 0; y < hh; y++)
		if (copy_from_user(va + y * r->stride, (void __user *)(uintptr_t)(b->data + y * row), row))
			break;
	mi_sys_VFlushInvCache(va, r->size);
	mi_sys_UnVmap(va);
	flip(r);
	return 0;
}

static int check_param(struct region *r, struct u_param *q)
{
	if (r->attr.type == T_OSD) {
		struct u_attr *a = &r->attr;

		if (q->u.osd.inv_en) {
			u32 wd = q->u.osd.inv_wdiv, hd = q->u.osd.inv_hdiv;

			if (wd < DIV_ROUND_UP(a->w, 64) || wd > min(a->w, 256u) || hd < DIV_ROUND_UP(a->h, 64) ||
			    hd > min(a->h, 256u) || wd * hd > 2048)
				return E_PARAM;
		}
		if (q->u.osd.alpha_mode > ALPHA_CONST || q->x > 4095 || q->y > 4095)
			return E_PARAM;
		if ((a->fmt == F_I2 && q->x % 4) || (a->fmt == F_I4 && q->x % 2))
			return E_PARAM;
	} else {
		if (q->u.cover.w < 1 || q->u.cover.w > 8192 || q->u.cover.h < 1 || q->u.cover.h > 8192 || q->x > 8191 ||
		    q->y > 8191)
			return E_PARAM;
	}
	return 0;
}

/* under `lock`: covers kept in layer order */
static void cover_insert(struct port *p, struct win *w)
{
	struct win *o;

	list_for_each_entry(o, &p->cover, pnode)
		if (o->param.u.cover.layer > w->param.u.cover.layer)
			break;
	list_add_tail(&w->pnode, &o->pnode);
}

static int do_attach(u32 h, struct u_chnport *cp, struct u_param *q)
{
	struct region *r = region_find(h);
	struct port *p;
	struct win *w;
	int ret;

	if (!r)
		return E_DEVID;
	if (win_find(r, cp))
		return E_BUSY;
	ret = check_param(r, q);
	if (ret)
		return ret;
	w = kzalloc(sizeof(*w), GFP_KERNEL);
	if (!w)
		return E_NOMEM;
	mutex_lock(&lock);
	p = port_get(cp, true);
	if (!p)
		ret = E_NOMEM;
	else if (r->attr.type == T_OSD && p->n_osd >= GWINS)
		ret = E_SUPPORT;
	else if (r->attr.type == T_COVER && p->n_cover >= COVERS)
		ret = E_PARAM;
	if (ret) {
		mutex_unlock(&lock);
		kfree(w);
		return ret;
	}
	w->r = r;
	w->p = p;
	w->param = *q;
	list_add_tail(&w->rnode, &r->wins);
	if (r->attr.type == T_OSD) {
		list_add_tail(&w->pnode, &p->osd);
		p->n_osd++;
	} else {
		cover_insert(p, w);
		p->n_cover++;
	}
	mutex_unlock(&lock);
	return 0;
}

static int do_detach(u32 h, struct u_chnport *cp)
{
	struct region *r = region_find(h);
	struct win *w;

	if (!r)
		return E_DEVID;
	w = r ? win_find(r, cp) : NULL;
	if (!w)
		return E_BUSY;
	if (r->attr.type == T_OSD) {
		/* frames programmed so far on this port may read the front canvas */
		struct canvas *c = &r->cv[r->front];

		mutex_lock(&lock);
		hold_add(c, w->p, w->p->seq);
		mutex_unlock(&lock);
	}
	win_drop(w);
	return 0;
}

static int do_set_display(u32 h, struct u_chnport *cp, struct u_param *q)
{
	struct region *r = region_find(h);
	struct win *w;
	int ret;

	if (!r)
		return E_DEVID;
	ret = check_param(r, q);
	if (ret)
		return ret;
	w = win_find(r, cp);
	if (!w)
		return E_BUSY;
	mutex_lock(&lock);
	if (r->attr.type == T_OSD && r->lent) {
		w->staged = *q;
		w->has_staged = true;
		mutex_unlock(&lock);
		return 0;
	}
	w->param = *q;
	w->has_staged = false;
	if (r->attr.type == T_COVER) {
		list_del(&w->pnode);
		cover_insert(w->p, w);
	}
	mutex_unlock(&lock);
	return 0;
}

static int do_get_display(u32 h, struct u_chnport *cp, struct u_param *q)
{
	struct region *r = region_find(h);
	struct win *w;

	if (!r)
		return E_DEVID;
	w = win_find(r, cp);
	if (!w)
		return E_BUSY;
	*q = w->has_staged ? w->staged : w->param;
	return 0;
}

static int do_get_canvas(u32 h, struct u_canvas *c)
{
	struct region *r = region_find(h);
	int ret;

	if (!r)
		return E_DEVID;
	if (r->attr.type != T_OSD)
		return E_PARAM;
	if (!r->lent) {
		ret = back_ready(r);
		if (ret)
			return ret;
		r->lent = true;
	}
	*c = (struct u_canvas){ .phy = r->cv[r->back].phy, .w = r->attr.w, .h = r->attr.h, .stride = r->stride,
				.fmt = r->attr.fmt };
	return 0;
}

static int do_update_canvas(u32 h)
{
	struct region *r = region_find(h);

	if (!r)
		return E_DEVID;
	if (r->attr.type != T_OSD)
		return E_PARAM;
	if (!r->lent)
		return E_PERM;
	r->lent = false;
	flip(r);
	return 0;
}

static int do_scale_rect(struct u_chnport *cp, u32 *from, u32 *to)
{
	struct port *p;

	if (from[0] > 4096 || from[1] > 4096 || to[0] > 4096 || to[1] > 4096)
		return E_PARAM;
	if (from[1] != to[1] && from[0] > 1920)
		return E_PARAM;
	mutex_lock(&lock);
	p = port_get(cp, true);
	if (p) {
		p->cw = min(from[0], to[0]);
		p->ch = min(from[1], to[1]);
		p->sw = to[0];
		p->sh = to[1];
		p->scaled = true;
	}
	mutex_unlock(&lock);
	return p ? 0 : E_NOMEM;
}

/* ---- the device: the MI ioctl wrapper (docs/firmware/components/mi-user-abi.md §3.1) ---- */

struct wrap {
	u32 len, pad;
	u64 ptr;
};

static int dispatch(struct file *f, unsigned int nr, u32 *k)
{
	if (nr != 0 && nr != 1 && !ready)
		return E_NOTREADY;
	switch (nr) {
	case 0: return do_init(f, k[0]);
	case 1: return do_deinit();
	case 2: return do_create(k[0], (struct u_attr *)(k + 1));
	case 3: return do_destroy(k[0]);
	case 4: return do_get_attr(k[0], (struct u_attr *)(k + 1));
	case 5: return do_set_bitmap(k[0], (struct u_bitmap *)(k + 1));
	case 6: return do_attach(k[0], (struct u_chnport *)(k + 1), (struct u_param *)(k + 5));
	case 7: return do_detach(k[0], (struct u_chnport *)(k + 1));
	case 8: return do_set_display(k[0], (struct u_chnport *)(k + 1), (struct u_param *)(k + 5));
	case 9: return do_get_display(k[0], (struct u_chnport *)(k + 1), (struct u_param *)(k + 5));
	case 10: return do_get_canvas(k[0], (struct u_canvas *)(k + 2));
	case 11: return do_update_canvas(k[0]);
	case 12: return do_scale_rect((struct u_chnport *)k, k + 4, k + 6);
	}
	return -EIO;
}

/* the payload size of each command (the size field of the vendor's command numbers) */
static const u16 payload[13] = { 8, 0, 0x14, 4, 0x14, 0x14, 0x3c, 0x14, 0x3c, 0x3c, 0x28, 4, 0x20 };

static long rgn_ioctl(struct file *f, unsigned int cmd, unsigned long arg)
{
	unsigned int nr = _IOC_NR(cmd);
	struct wrap w;
	u32 *k = NULL;
	long ret;

	if (_IOC_TYPE(cmd) != 'i' || nr >= ARRAY_SIZE(payload))
		return -EIO;
	if (arg) {
		if (copy_from_user(&w, (void __user *)arg, sizeof(w)))
			return -EFAULT;
		if (w.len > _IOC_SIZE(cmd) || w.len < payload[nr])
			return -EINVAL;
		k = kzalloc(ALIGN(max_t(u32, w.len, 8), 8), GFP_KERNEL);
		if (!k)
			return -ENOMEM;
		if ((_IOC_DIR(cmd) & _IOC_WRITE) && copy_from_user(k, (void __user *)(uintptr_t)w.ptr, w.len)) {
			kfree(k);
			return -EFAULT;
		}
	} else if (payload[nr]) {
		return -EINVAL;
	}
	mutex_lock(&api);
	ret = dispatch(f, nr, k);
	mutex_unlock(&api);
	if (k && (_IOC_DIR(cmd) & _IOC_READ) && copy_to_user((void __user *)(uintptr_t)w.ptr, k, w.len))
		ret = -EFAULT;
	kfree(k);
	return ret;
}

static int rgn_release(struct inode *i, struct file *f)
{
	mutex_lock(&api);
	if (ready && f == owner)
		do_deinit();
	mutex_unlock(&api);
	return 0;
}

static const struct file_operations rgn_fops = {
	.owner = THIS_MODULE,
	.unlocked_ioctl = rgn_ioctl,
	.release = rgn_release,
};

static struct miscdevice rgn_dev = {
	.minor = MISC_DYNAMIC_MINOR,
	.name = "mi_rgn",
	.fops = &rgn_fops,
	.mode = 0660,
};

/* ---- /proc/microcam_rgn: what is where ---- */

static int show(struct seq_file *s, void *v)
{
	struct region *r;
	struct port *p;
	struct win *w;
	int g;

	mutex_lock(&api);
	mutex_lock(&lock);
	seq_printf(s, "ready %d hw %d\n", ready, hw_ready);
	list_for_each_entry(r, &regions, node) {
		seq_printf(s, "region %u %s fmt %u %ux%u stride %u canvases %d front %d update %u%s\n", r->handle,
			   r->attr.type == T_OSD ? "osd" : "cover", r->attr.fmt, r->attr.w, r->attr.h, r->stride, r->n,
			   r->front, r->ver, r->lent ? " lent" : "");
		list_for_each_entry(w, &r->wins, rnode)
			seq_printf(s, "  on %x/%d/%d/%d show %u at %u,%u layer %u\n", w->p->cp.mod, w->p->cp.dev,
				   w->p->cp.chn, w->p->cp.port, w->param.show, w->param.x, w->param.y,
				   w->param.u.osd.layer);
	}
	list_for_each_entry(p, &ports, node)
		seq_printf(s, "port %x/%d/%d/%d frames %llu done %llu osd %d cover %d gop %d cover-hw %d%s\n", p->cp.mod,
			   p->cp.dev, p->cp.chn, p->cp.port, p->seq, p->done, p->n_osd, p->n_cover, p->gop, p->cover_hw,
			   p->scaled ? " scaled" : "");
	for (g = 0; g < GWINS; g++)
		if (gwin_on[g])
			seq_printf(s, "gop %d gwins %02x\n", g, gwin_on[g]);
	seq_printf(s, "graveyard %d\n", !list_empty(&graveyard));
	mutex_unlock(&lock);
	mutex_unlock(&api);
	return 0;
}

static int show_open(struct inode *i, struct file *f)
{
	return single_open(f, show, NULL);
}

/* `canvases <handle> 3`: the region gets a third canvas (never fewer) */
static ssize_t proc_write(struct file *f, const char __user *buf, size_t len, loff_t *off)
{
	char cmd[48], name[24];
	struct region *r;
	u32 h, n;
	int ret = len;

	if (len >= sizeof(cmd) || copy_from_user(cmd, buf, len))
		return -EINVAL;
	cmd[len] = 0;
	if (sscanf(cmd, "canvases %u %u", &h, &n) != 2 || n > CANVASES)
		return -EINVAL;
	mutex_lock(&api);
	r = region_find(h);
	if (!r || r->attr.type != T_OSD) {
		ret = -ENOENT;
	} else {
		while (r->n < n) {
			struct canvas *c = &r->cv[r->n];

			snprintf(name, sizeof(name), "microcam-rgn%u_%d", h, r->n);
			memset(c, 0, sizeof(*c));
			if (mi_sys_MMA_Alloc(NULL, name, r->size, &c->phy)) {
				ret = -ENOMEM;
				break;
			}
			/* not the front's version: brought up to its content when first lent */
			c->ver = r->cv[r->front].ver - 1;
			mutex_lock(&lock);
			r->n++;
			mutex_unlock(&lock);
		}
	}
	mutex_unlock(&api);
	return ret;
}

static const struct file_operations proc_fops = {
	.open = show_open,
	.write = proc_write,
	.read = seq_read,
	.llseek = seq_lseek,
	.release = single_release,
};

static int __init microcam_rgn_init(void)
{
	int ret = misc_register(&rgn_dev);

	if (ret)
		return ret;
	proc_create("microcam_rgn", 0644, NULL, &proc_fops);
	pr_info("microcam_rgn: loaded\n");
	return 0;
}

static void __exit microcam_rgn_exit(void)
{
	struct port *p, *n;

	remove_proc_entry("microcam_rgn", NULL);
	misc_deregister(&rgn_dev);
	mutex_lock(&api);
	if (ready)
		do_deinit();
	mutex_unlock(&api);
	cancel_delayed_work_sync(&reaper);
	reap(NULL);
	cancel_delayed_work_sync(&reaper);
	list_for_each_entry_safe(p, n, &ports, node)
		kfree(p);
}

module_init(microcam_rgn_init);
module_exit(microcam_rgn_exit);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_DESCRIPTION("microcam: RGN (SCL OSD) driver in place of mi_rgn");
