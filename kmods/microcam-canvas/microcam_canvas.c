// SPDX-License-Identifier: MIT-0 OR GPL-2.0
/*
 * microcam_canvas: the frames an encoder reads, owned by us.
 *
 * A pool per encoder output: canvas-sized NV12 frames in the media heap. The module puts two allocators of its own
 * into mi_sys, the shape mi_vdisp uses for its «normal channels»:
 *  - on the SCL output port that makes the live picture: each buffer the port asks for is a *view* — the live
 *    window's rectangle inside a free canvas, at the canvas's stride, at the window's x, y — so the port writes the
 *    live picture straight into the canvas. The view's size is the port's own request; one that would not fit the
 *    canvas, or starts off the port's 16-column grid, is refused;
 *  - on each reader's input port — the encoder's, and any more added with `reader` (a second encoder, DISP's input
 *    port): a plain `MI_SYS_ChnInputPortGetBufPa` there returns the whole canvas microcamd named with `next`, and the
 *    reader's release of it comes back here. Each reader takes the
 *    canvas `next` named once, so one canvas goes to every reader of the pool.
 * Every frame an encoder gets goes this way, the whole-frame live picture too (the view is then the whole canvas).
 *
 * A canvas has at most one writer — the port (a view out: being written, then held by microcamd's feed until it has
 * handed the canvas on) or microcamd (`take`: it draws into it) — and any number of encoder readers. The port is lent
 * only a canvas with no writer and no reader; `take` only succeeds on such a canvas; `next` names any canvas microcamd
 * has not taken. Canvases are allocated here (owner `microcam_canvas` in the heap's listing) and mapped by microcamd by their
 * physical addresses, which the status shows.
 *
 * /proc/microcam_canvas, kept open by microcamd for its life (a pool belongs to the open file that made it):
 *   pool <id> <w> <h> <rows> <n> <scl dev> <chn> <port> [<venc dev> <chn>]   a pool of n canvases w × rows (the encoder
 *        reads w × h of them; rows ≥ h, H.264 wants them to 32), both allocators put on. The port must not be bound.
 *   view <id> <x> <y> | view <id> none     where the port's window goes (x on the 16-column grid, y even); none: no
 *        view is lent (covered — the port is off)
 *   reader <id> <mod> <dev> <chn> <port>   one more reader's input port (MI module id: 2 VENC, 15 DISP), allocator put on
 *   unreader <id> <mod> <dev> <chn> <port> that reader's allocator taken off (its channel still there)
 *   next <id> <k>                          the canvas each reader's next GetBufPa gets
 *   take <id> <k> | give <id> <k>          microcamd draws into a canvas (-EBUSY while the port or an encoder has it)
 *   count <id> <n>                         canvases added, or the last ones retired (freed once nobody has them)
 *   close <id>                             both allocators taken off — the port disabled, the encoder still there
 *        (mi_sys checks the channel when an allocator is set) — and the canvases freed once nobody has them
 * /proc/microcam_canvas_lent (pollable): the newest lending of a view per pool, for a reader handed the canvas before the
 * port writes it (DISP).
 * Read: per pool its geometry, view, canvases (address, writer, readers) and counts; per reader the hand-overs and
 * «after-release <sum us>/<n>»: each hand-over's time after the reader's last release, summed (DISP's display phase),
 * and «released <ns>», the last release (ktime; DISP's: its pass after a vsync).
 * If the file is closed with pools still open (microcamd died), nothing of MI is called: mi_sys tears its channels down
 * and drops our allocators and allocations through their callbacks; the memory goes when nothing refers to it.
 *
 * Only `…Pa` calls may take these buffers (no user or kernel mapping is offered; a request for one is refused).
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/kallsyms.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/uaccess.h>
#include <linux/slab.h>
#include <linux/spinlock.h>
#include <linux/mutex.h>
#include <linux/workqueue.h>
#include <linux/ktime.h>
#include <linux/math64.h>
#include <linux/poll.h>
#include <linux/wait.h>

/* the camera's kernel exports no __memzero, which our tree's string.h makes the inline copy_from_user call */
void __memzero(void *p, __kernel_size_t n)
{
	(memset)(p, 0, n);
}

#define MOD_VENC 2
#define MOD_SCL 34
#define NV12 0xb
#define POOLS 4
#define READERS 4		/* DISP, USB's encoder, the recording's, the photo's during a recording */
#define NO_READER 0xffffffffu	/* a pool made without its first reader (DISP's is added with `reader`) */
#define CANVASES 8
#define GRID 16
#define HOLDS 256		/* a reader's last releases kept for /proc/microcam_canvas_holds */
#define MAP_VA (1u << 31)	/* bufconf flag: the buffer mapped for the caller */

typedef void *(*getdev_t)(int mod, int dev, int, int);
extern void generic_allocator_init(void *allocator, void *ops);
extern int generic_allocator_on_ref(void *, ...);
extern int generic_allocator_on_unref(void *);
extern void generic_allocation_init(void *allocation, void *ops);
extern void generic_allocation_on_ref(void *, ...);
extern void generic_allocation_on_unref(void *);
extern int mi_sys_SetOutputPortUsrAllocator(void *moddev, u32 chn, u32 port, void *allocator);
extern int mi_sys_SetInputPortUsrAllocator(void *moddev, u32 chn, s32 pass, u32 port, void *allocator);
extern int mi_sys_MMA_Alloc(const char *heap, const char *owner, u32 size, u64 *phys);
extern int mi_sys_MMA_Free(u64 phys);

struct pool;

/* mi_sys's allocator: the generic header (ref +0, magic +4, list +8, ops +0x10) and two words it tags (+0x18, +0x1c) */
struct allocator {
	u8 generic[0x20];
	struct pool *pool;
	int live;		/* mi_sys still holds it */
	int idx;		/* a reader's index; -1 for the port's */
};

/* an input port reading whole canvases: an encoder's or DISP's */
struct reader {
	struct allocator a;
	void *dev;
	u32 mod, ndev, chn, port;
	int next;		/* the canvas its next GetBufPa gets, -1 none */
	int used;
	u32 handed;
	/* its last releases: when each canvas was handed to it and when it let it go (ktime ns), oldest overwritten */
	struct hold { u64 handed, released; int canvas; } holds[HOLDS];
	u32 nholds;
	/* its last release (ktime ns), and each hand-over's time after the last release summed (us) with their count:
	 * DISP lets a canvas go on its pass after a vsync, so for DISP this is the hand-over's phase in the display period */
	u64 released, after_sum;
	u32 after_n;
	/* each hand-over's time after the port was lent that canvas (us): sum, count, least, most */
	u64 lent_sum;
	u32 lent_n, lent_min, lent_max;
};

/* mi_sys's allocation: generic header (magic, ref, ops ×8) then the MI_SYS_BufInfo_t the buffer reference copies */
struct allocation {
	u8 generic[0x28];
	u8 bufinfo[0x108];	/* pts +0, type +0x10, fmt +0x24, layout +0x34, w +0x38, h +0x3a, plane addresses +0x48/+0x50, strides +0x60/+0x64, size +0x6c */
	u8 misys[0x10];		/* +0x130 the original OnRelease, +0x134 the port: written by mi_sys */
	struct pool *pool;
	int canvas;
	int enc;		/* a reader's whole canvas (its index + 1), else 0: a port's view */
	u64 t0;
};

enum writer { NONE, PORT, MICROCAMD };

struct canvas {
	u64 phys;		/* 0: none */
	enum writer writer;
	int readers;
	int retired;		/* freed once nobody has it */
	u64 lent;		/* when the port was last lent it (ktime ns) */
};

struct pool {
	int id;
	struct file *owner;	/* NULL once closed or orphaned */
	u32 w, h, rows, size;
	void *scl;		/* the mi_sys device the port's allocator was put on */
	u32 scl_chn, scl_port;
	struct allocator port_alloc;
	struct reader rd[READERS];
	int view;		/* a view is lent */
	u32 x, y;
	struct canvas cv[CANVASES];
	int n;			/* canvases in use (the rest retired or none) */
	int allocations;	/* alive in mi_sys */
	/* the newest lending of a view to the port: its number (from 1), canvas and time (ktime ns) */
	u32 lent_seq;
	int lent_k;
	u64 lent_ns;
	/* counts */
	u32 views, handed, no_canvas, refused, no_next;
	u64 hold_sum, hold_max;
	u32 hold_n;
	struct list_head node;	/* on `all`, open or closing, until reaped */
	spinlock_t lock;
};

static getdev_t getdev;
static struct pool *pools[POOLS];	/* by id, the newest pool (a closing one until another takes its id) */
static LIST_HEAD(all);			/* every pool not yet freed */
static DEFINE_MUTEX(cmd_lock);		/* commands, allocation of memory, reaping, `all` */
static void *ops_port[5], *ops_enc[5], *ops_allocation[8];
/* /proc/microcam_canvas_lent: woken at every lending; `lendings` counts them over all pools */
static DECLARE_WAIT_QUEUE_HEAD(lent_wq);
static atomic_t lendings = ATOMIC_INIT(0);

/* --- memory: allocated and freed in process context only (the release callbacks may run anywhere) --- */

/* One work item for the module: a release callback schedules it and never touches its pool after unlocking, so no
 * work runs on a freed pool. It walks every pool: idle canvases retired or ownerless go, and a pool with no owner, no
 * live allocator, no allocation and no canvas goes with them. */
static void reap(struct work_struct *w)
{
	struct pool *p, *t;

	mutex_lock(&cmd_lock);
	list_for_each_entry_safe(p, t, &all, node) {
		u64 gone[CANVASES];
		int i, n = 0, last;
		unsigned long f;

		spin_lock_irqsave(&p->lock, f);
		for (i = 0; i < CANVASES; i++) {
			struct canvas *c = &p->cv[i];
			if (c->phys && (c->retired || !p->owner) && c->writer == NONE && !c->readers) {
				gone[n++] = c->phys;
				c->phys = 0;
				c->retired = 0;
			}
		}
		last = !p->owner && !p->port_alloc.live && !p->allocations;
		for (i = 0; i < READERS; i++)
			if (p->rd[i].a.live)
				last = 0;
		for (i = 0; i < CANVASES && last; i++)
			if (p->cv[i].phys)
				last = 0;
		spin_unlock_irqrestore(&p->lock, f);
		for (i = 0; i < n; i++)
			mi_sys_MMA_Free(gone[i]);
		if (!last)
			continue;
		list_del(&p->node);
		if (pools[p->id] == p)
			pools[p->id] = NULL;
		pr_info("microcam_canvas: pool %d gone\n", p->id);
		kfree(p);
		module_put(THIS_MODULE);
	}
	mutex_unlock(&cmd_lock);
}

static DECLARE_WORK(reaper, reap);

/* a reader is still to take canvas k (lock held) */
static int pending(struct pool *p, int k)
{
	int i;

	for (i = 0; i < READERS; i++)
		if (p->rd[i].used && p->rd[i].next == k)
			return 1;
	return 0;
}

/* --- mi_sys's callbacks --- */

static void allocation_release(void *a)
{
	struct allocation *al = a;
	struct pool *p = al->pool;
	struct canvas *c = &p->cv[al->canvas];
	unsigned long f;
	int due;

	spin_lock_irqsave(&p->lock, f);
	if (al->enc) {
		u64 now = ktime_get_ns(), d = div_u64(now - al->t0, 1000);
		struct reader *rd = &p->rd[al->enc - 1];
		struct hold *h = &rd->holds[rd->nholds++ % HOLDS];
		h->handed = al->t0;
		h->released = now;
		h->canvas = al->canvas;
		rd->released = now;
		c->readers--;
		p->hold_sum += d;
		p->hold_n++;
		if (d > p->hold_max)
			p->hold_max = d;
	} else {
		c->writer = NONE;
	}
	p->allocations--;
	due = (c->writer == NONE && !c->readers && (c->retired || !p->owner)) || (!p->owner && !p->allocations);
	spin_unlock_irqrestore(&p->lock, f);
	kfree(al);
	if (due)
		schedule_work(&reaper);
}

static void fill(struct allocation *al, u64 pts, u32 w, u32 h, u64 y, u64 uv, u32 stride, u32 size)
{
	u8 *b = al->bufinfo;

	*(u64 *)(b + 0) = pts;
	*(u32 *)(b + 0x10) = 1;			/* E_MI_SYS_BUFDATA_FRAME */
	*(u32 *)(b + 0x24) = NV12;
	*(u32 *)(b + 0x34) = 2;			/* the plane layout mi_vdisp sets */
	*(u16 *)(b + 0x38) = w;
	*(u16 *)(b + 0x3a) = h;
	*(u64 *)(b + 0x48) = y;
	*(u64 *)(b + 0x50) = uv;
	*(u32 *)(b + 0x60) = stride;
	*(u32 *)(b + 0x64) = stride;
	*(u32 *)(b + 0x6c) = size;
}

static struct allocation *new_allocation(struct pool *p, int k, int enc)
{
	struct allocation *al = kzalloc(sizeof(*al), GFP_ATOMIC);

	if (!al)
		return NULL;
	generic_allocation_init(al, ops_allocation);
	al->pool = p;
	al->canvas = k;
	al->enc = enc;
	return al;
}

/* alloc(allocator, name, bufconf, ext-conf): bufconf = type +0, flags +4, target PTS +8, frame config +0x18 (u16 w,
 * u16 h, scan, format +0x20). The ext-conf is mi_sys's own and only read. */
static void *port_alloc_cb(void *allocator, const char *name, u8 *bufconf, void *ext)
{
	struct pool *p = ((struct allocator *)allocator)->pool;
	u32 w = *(u16 *)(bufconf + 0x18), h = *(u16 *)(bufconf + 0x1a);
	struct allocation *al;
	unsigned long f;
	int i, k = -1;

	spin_lock_irqsave(&p->lock, f);
	if (!p->owner || !p->view) {
		spin_unlock_irqrestore(&p->lock, f);
		return NULL;
	}
	if (*(u32 *)bufconf != 1 || *(u32 *)(bufconf + 4) & MAP_VA || *(u32 *)(bufconf + 0x20) != NV12 ||
	    p->x % GRID || p->y & 1 || p->x + w > p->w || p->y + h > p->h) {
		p->refused++;
		spin_unlock_irqrestore(&p->lock, f);
		return NULL;
	}
	for (i = 0; i < p->n; i++) {
		struct canvas *c = &p->cv[i];
		if (c->phys && !c->retired && c->writer == NONE && !c->readers && !pending(p, i)) {
			k = i;
			break;
		}
	}
	if (k < 0) {
		p->no_canvas++;
		spin_unlock_irqrestore(&p->lock, f);
		return NULL;
	}
	al = new_allocation(p, k, 0);
	if (al) {
		u64 base = p->cv[k].phys;
		p->cv[k].writer = PORT;
		p->cv[k].lent = ktime_get_ns();
		p->lent_seq++;
		p->lent_k = k;
		p->lent_ns = p->cv[k].lent;
		p->allocations++;
		p->views++;
		fill(al, *(u64 *)(bufconf + 8), w, h, base + (u64)p->y * p->w + p->x,
		     base + (u64)p->w * p->rows + (u64)(p->y / 2) * p->w + p->x, p->w, w * h * 3 / 2);
	}
	spin_unlock_irqrestore(&p->lock, f);
	if (al) {
		atomic_inc(&lendings);
		wake_up_interruptible(&lent_wq);
	}
	return al;
}

static void *enc_alloc_cb(void *allocator, const char *name, u8 *bufconf, void *ext)
{
	struct pool *p = ((struct allocator *)allocator)->pool;
	struct reader *rd = &p->rd[((struct allocator *)allocator)->idx];
	struct allocation *al = NULL;
	unsigned long f;
	int k;

	spin_lock_irqsave(&p->lock, f);
	k = rd->next;
	rd->next = -1;
	if (!p->owner || *(u32 *)bufconf != 1 || *(u32 *)(bufconf + 4) & MAP_VA) {
		p->refused++;
	} else if (k < 0 || !p->cv[k].phys || p->cv[k].writer == MICROCAMD) {
		p->no_next++;
	} else if ((al = new_allocation(p, k, rd - p->rd + 1))) {
		u64 base = p->cv[k].phys;
		p->cv[k].readers++;
		p->allocations++;
		p->handed++;
		rd->handed++;
		al->t0 = ktime_get_ns();
		if (rd->released) {
			rd->after_sum += div_u64(al->t0 - rd->released, 1000);
			rd->after_n++;
		}
		if (p->cv[k].lent) {
			u32 d = div_u64(al->t0 - p->cv[k].lent, 1000);
			rd->lent_sum += d;
			if (!rd->lent_n++ || d < rd->lent_min)
				rd->lent_min = d;
			if (d > rd->lent_max)
				rd->lent_max = d;
		}
		fill(al, *(u64 *)(bufconf + 8), p->w, p->h, base, base + (u64)p->w * p->rows, p->w, p->size);
	}
	spin_unlock_irqrestore(&p->lock, f);
	return al;
}

static int suit_cb(void *allocator, ...)
{
	return 0x7fffffff;
}

/* mi_sys has dropped one of our allocators (its last reference: the allocator taken off, or its channel gone) */
static void allocator_release(void *a)
{
	struct allocator *al = a;
	struct pool *p = al->pool;
	unsigned long f;
	int due;

	spin_lock_irqsave(&p->lock, f);
	al->live = 0;
	due = !p->owner;
	spin_unlock_irqrestore(&p->lock, f);
	if (due)
		schedule_work(&reaper);
}

/* --- commands --- */

static int alloc_canvases(struct pool *p, int n)
{
	int i;

	for (i = 0; i < n; i++) {
		struct canvas *c = &p->cv[i];
		u64 phys;
		unsigned long f;
		if (c->phys) {
			spin_lock_irqsave(&p->lock, f);
			c->retired = 0;
			spin_unlock_irqrestore(&p->lock, f);
			continue;
		}
		if (mi_sys_MMA_Alloc(NULL, "microcam_canvas", p->size, &phys)) {
			pr_err("microcam_canvas: pool %d: no memory for canvas %d (%u B)\n", p->id, i, p->size);
			return -ENOMEM;
		}
		spin_lock_irqsave(&p->lock, f);
		c->phys = phys;
		c->writer = NONE;
		c->readers = 0;
		c->retired = 0;
		spin_unlock_irqrestore(&p->lock, f);
	}
	return 0;
}

static void retire_from(struct pool *p, int n)
{
	unsigned long f;
	int i;

	spin_lock_irqsave(&p->lock, f);
	p->n = n;
	for (i = n; i < CANVASES; i++)
		if (p->cv[i].phys)
			p->cv[i].retired = 1;
	for (i = 0; i < READERS; i++)
		if (p->rd[i].next >= n)
			p->rd[i].next = -1;
	spin_unlock_irqrestore(&p->lock, f);
	schedule_work(&reaper);
}

static void reader_off(struct pool *p, struct reader *rd)
{
	int r;

	if (!rd->a.live)
		return;
	r = mi_sys_SetInputPortUsrAllocator(rd->dev, rd->chn, 0, rd->port, NULL);
	if (r)
		pr_err("microcam_canvas: pool %d: reader %u %u/%u/%u's allocator not taken off: %x\n", p->id, rd->mod, rd->ndev,
		       rd->chn, rd->port, r);
}

static void allocators_off(struct pool *p)
{
	int r, i;

	for (i = 0; i < READERS; i++)
		reader_off(p, &p->rd[i]);
	if (p->port_alloc.live) {
		r = mi_sys_SetOutputPortUsrAllocator(p->scl, p->scl_chn, p->scl_port, NULL);
		if (r)
			pr_err("microcam_canvas: pool %d: port's allocator not taken off: %x\n", p->id, r);
	}
}

static int add_reader(struct pool *p, u32 mod, u32 ndev, u32 chn, u32 port)
{
	struct reader *rd = NULL;
	void *dev;
	int i, r;

	for (i = 0; i < READERS; i++) {
		if (p->rd[i].used && p->rd[i].mod == mod && p->rd[i].ndev == ndev && p->rd[i].chn == chn && p->rd[i].port == port)
			return -EEXIST;
		if (!p->rd[i].used && !p->rd[i].a.live && !rd)
			rd = &p->rd[i];
	}
	if (!rd)
		return -ENOSPC;
	dev = getdev(mod, ndev, 0, 0);
	if (!dev)
		return -ENODEV;
	rd->dev = dev;
	rd->mod = mod;
	rd->ndev = ndev;
	rd->chn = chn;
	rd->port = port;
	rd->next = -1;
	rd->handed = 0;
	rd->nholds = 0;
	rd->released = rd->after_sum = 0;
	rd->after_n = 0;
	rd->a.pool = p;
	rd->a.idx = rd - p->rd;
	generic_allocator_init(&rd->a, ops_enc);
	r = mi_sys_SetInputPortUsrAllocator(dev, chn, 0, port, &rd->a);
	if (r) {
		pr_err("microcam_canvas: pool %d: %u %u/%u/%u takes no allocator: %x\n", p->id, mod, ndev, chn, port, r);
		return -EIO;
	}
	rd->a.live = 1;
	rd->used = 1;
	return 0;
}

static int make_pool(struct file *file, u32 id, u32 w, u32 h, u32 rows, u32 n, u32 sdev, u32 schn, u32 sport, u32 vdev,
		     u32 vchn)
{
	struct pool *p;
	void *scl;
	int r, i;

	if (id >= POOLS || !n || n > CANVASES || !w || w % GRID || !h || h & 1 || rows < h || rows & 1)
		return -EINVAL;
	if (pools[id] && pools[id]->owner)
		return -EBUSY;
	pools[id] = NULL;	/* one its owner left stays on `all` until mi_sys lets it go */
	scl = getdev(MOD_SCL, sdev, 0, 0);
	if (!scl || (vdev != NO_READER && !getdev(MOD_VENC, vdev, 0, 0)))
		return -ENODEV;
	p = kzalloc(sizeof(*p), GFP_KERNEL);
	if (!p)
		return -ENOMEM;
	if (!try_module_get(THIS_MODULE)) {
		kfree(p);
		return -ENODEV;
	}
	p->id = id;
	p->owner = file;
	p->w = w;
	p->h = h;
	p->rows = rows;
	p->size = w * rows * 3 / 2;
	p->n = n;
	spin_lock_init(&p->lock);
	list_add_tail(&p->node, &all);
	p->scl = scl;
	p->scl_chn = schn;
	p->scl_port = sport;
	p->port_alloc.pool = p;
	p->port_alloc.idx = -1;
	for (i = 0; i < READERS; i++)
		p->rd[i].next = -1;
	generic_allocator_init(&p->port_alloc, ops_port);
	pools[id] = p;
	r = alloc_canvases(p, n);
	if (!r) {
		r = mi_sys_SetOutputPortUsrAllocator(scl, schn, sport, &p->port_alloc);
		if (!r)
			p->port_alloc.live = 1;
		else
			pr_err("microcam_canvas: pool %d: SCL %u/%u/%u takes no allocator: %x\n", id, sdev, schn, sport, r);
	}
	if (!r && vdev != NO_READER)
		r = add_reader(p, MOD_VENC, vdev, vchn, 0);
	if (r) {
		allocators_off(p);
		p->owner = NULL;
		schedule_work(&reaper);
		return r > 0 ? -EIO : r;
	}
	pr_info("microcam_canvas: pool %d: %u canvases %ux%u (rows %u), SCL %u/%u/%u → encoder %u/%u\n", id, n, w, h, rows,
		sdev, schn, sport, vdev, vchn);
	return 0;
}

static struct pool *own(struct file *file, u32 id)
{
	return id < POOLS && pools[id] && pools[id]->owner == file ? pools[id] : NULL;
}

static ssize_t proc_write(struct file *file, const char __user *buf, size_t len, loff_t *off)
{
	char cmd[128];
	u32 a[11];
	char none[8];
	struct pool *p;
	unsigned long f;
	int r = 0, k, i;

	if (len >= sizeof(cmd) || copy_from_user(cmd, buf, len))
		return -EINVAL;
	cmd[len] = 0;
	mutex_lock(&cmd_lock);
	if (sscanf(cmd, "next %u %d", &a[0], &k) == 2) {
		p = own(file, a[0]);
		if (!p || k < -1 || k >= p->n)
			r = -EINVAL;
		else {
			spin_lock_irqsave(&p->lock, f);
			for (i = 0; i < READERS; i++)
				p->rd[i].next = k;
			spin_unlock_irqrestore(&p->lock, f);
		}
	} else if (sscanf(cmd, "reader %u %u %u %u %u", &a[0], &a[1], &a[2], &a[3], &a[4]) == 5) {
		p = own(file, a[0]);
		r = p ? add_reader(p, a[1], a[2], a[3], a[4]) : -EINVAL;
	} else if (sscanf(cmd, "unreader %u %u %u %u %u", &a[0], &a[1], &a[2], &a[3], &a[4]) == 5) {
		p = own(file, a[0]);
		r = -EINVAL;
		for (i = 0; p && i < READERS; i++) {
			struct reader *rd = &p->rd[i];
			if (rd->used && rd->mod == a[1] && rd->ndev == a[2] && rd->chn == a[3] && rd->port == a[4]) {
				reader_off(p, rd);
				spin_lock_irqsave(&p->lock, f);
				rd->used = 0;
				rd->next = -1;
				spin_unlock_irqrestore(&p->lock, f);
				r = 0;
			}
		}
	} else if (sscanf(cmd, "pool %u %u %u %u %u %u %u %u %u %u", &a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6],
			  &a[7], &a[8], &a[9]) == 10) {
		r = make_pool(file, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9]);
	} else if (sscanf(cmd, "pool %u %u %u %u %u %u %u %u", &a[0], &a[1], &a[2], &a[3], &a[4], &a[5], &a[6], &a[7]) == 8) {
		r = make_pool(file, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], NO_READER, 0);
	} else if (sscanf(cmd, "view %u %7s", &a[0], none) == 2 && !strcmp(none, "none")) {
		p = own(file, a[0]);
		if (!p)
			r = -EINVAL;
		else
			p->view = 0;
	} else if (sscanf(cmd, "view %u %u %u", &a[0], &a[1], &a[2]) == 3) {
		p = own(file, a[0]);
		if (!p || a[1] % GRID || a[2] & 1 || a[1] >= p->w || a[2] >= p->h)
			r = -EINVAL;
		else {
			spin_lock_irqsave(&p->lock, f);
			p->x = a[1];
			p->y = a[2];
			p->view = 1;
			spin_unlock_irqrestore(&p->lock, f);
		}
	} else if (sscanf(cmd, "take %u %d", &a[0], &k) == 2) {
		p = own(file, a[0]);
		if (!p || k < 0 || k >= p->n)
			r = -EINVAL;
		else {
			spin_lock_irqsave(&p->lock, f);
			if (p->cv[k].writer != NONE || p->cv[k].readers || pending(p, k))
				r = -EBUSY;
			else
				p->cv[k].writer = MICROCAMD;
			spin_unlock_irqrestore(&p->lock, f);
		}
	} else if (sscanf(cmd, "give %u %d", &a[0], &k) == 2) {
		p = own(file, a[0]);
		if (!p || k < 0 || k >= CANVASES || p->cv[k].writer != MICROCAMD)
			r = -EINVAL;
		else {
			spin_lock_irqsave(&p->lock, f);
			p->cv[k].writer = NONE;
			spin_unlock_irqrestore(&p->lock, f);
			if (p->cv[k].retired)
				schedule_work(&reaper);
		}
	} else if (sscanf(cmd, "count %u %u", &a[0], &a[1]) == 2) {
		p = own(file, a[0]);
		if (!p || !a[1] || a[1] > CANVASES)
			r = -EINVAL;
		else if (a[1] > p->n) {
			r = alloc_canvases(p, a[1]);
			if (!r) {
				spin_lock_irqsave(&p->lock, f);
				p->n = a[1];
				spin_unlock_irqrestore(&p->lock, f);
			} else
				retire_from(p, p->n);	/* the canvases this call made go */
		} else
			retire_from(p, a[1]);
	} else if (sscanf(cmd, "close %u", &a[0]) == 1) {
		p = own(file, a[0]);
		if (!p)
			r = -EINVAL;
		else {
			allocators_off(p);
			spin_lock_irqsave(&p->lock, f);
			p->owner = NULL;
			p->view = 0;
			spin_unlock_irqrestore(&p->lock, f);
			schedule_work(&reaper);
		}
	} else {
		r = -EINVAL;
	}
	mutex_unlock(&cmd_lock);
	return r ? r : len;
}

static int proc_show(struct seq_file *m, void *v)
{
	static const char *const who[] = { "-", "port", "microcamd" };
	struct pool *p;
	int k;

	mutex_lock(&cmd_lock);
	list_for_each_entry(p, &all, node) {
		unsigned long f;
		spin_lock_irqsave(&p->lock, f);
		seq_printf(m, "pool %d %ux%u rows %u size %u canvases %d %s", p->id, p->w, p->h, p->rows, p->size, p->n,
			   p->owner ? "open" : "closing");
		if (p->view)
			seq_printf(m, " view %u %u\n", p->x, p->y);
		else
			seq_puts(m, " view none\n");
		for (k = 0; k < CANVASES; k++)
			if (p->cv[k].phys)
				seq_printf(m, "  canvas %d phys %llx writer %s readers %d%s\n", k, p->cv[k].phys,
					   who[p->cv[k].writer], p->cv[k].readers, p->cv[k].retired ? " retired" : "");
		seq_printf(m, "  views %u handed %u no-canvas %u refused %u no-next %u held %llu/%llu us allocations %d\n",
			   p->views, p->handed, p->no_canvas, p->refused, p->no_next,
			   p->hold_n ? div_u64(p->hold_sum, p->hold_n) : 0, p->hold_max, p->allocations);
		for (k = 0; k < READERS; k++)
			if (p->rd[k].used)
				seq_printf(m, "  reader %u %u/%u/%u handed %u after-release %llu/%u released %llu after-lent %llu/%u %u-%u%s\n",
					   p->rd[k].mod, p->rd[k].ndev, p->rd[k].chn, p->rd[k].port, p->rd[k].handed,
					   p->rd[k].after_sum, p->rd[k].after_n, p->rd[k].released,
					   p->rd[k].lent_sum, p->rd[k].lent_n, p->rd[k].lent_min, p->rd[k].lent_max,
					   p->rd[k].a.live ? "" : " (dropped)");
		spin_unlock_irqrestore(&p->lock, f);
	}
	mutex_unlock(&cmd_lock);
	return 0;
}

static int proc_open(struct inode *inode, struct file *file)
{
	return single_open(file, proc_show, NULL);
}

/* each reader's last releases, oldest first: «pool reader canvas handed_ns released_ns» */
static int holds_show(struct seq_file *m, void *v)
{
	struct pool *p;
	int k;
	u32 i, from;

	mutex_lock(&cmd_lock);
	list_for_each_entry(p, &all, node) {
		unsigned long f;
		spin_lock_irqsave(&p->lock, f);
		for (k = 0; k < READERS; k++) {
			struct reader *rd = &p->rd[k];
			if (!rd->used)
				continue;
			seq_printf(m, "# pool %d reader %u %u/%u/%u\n", p->id, rd->mod, rd->ndev, rd->chn, rd->port);
			from = rd->nholds > HOLDS ? rd->nholds - HOLDS : 0;
			for (i = from; i < rd->nholds; i++) {
				struct hold *h = &rd->holds[i % HOLDS];
				seq_printf(m, "%d %d %d %llu %llu\n", p->id, k, h->canvas, h->handed, h->released);
			}
		}
		spin_unlock_irqrestore(&p->lock, f);
	}
	mutex_unlock(&cmd_lock);
	return 0;
}

static int holds_open(struct inode *inode, struct file *file)
{
	return single_open_size(file, holds_show, NULL, POOLS * READERS * (HOLDS + 1) * 64);
}

static const struct file_operations holds_fops = {
	.open = holds_open,
	.read = seq_read,
	.llseek = seq_lseek,
	.release = single_release,
};

/* the opener is gone: its pools are left to mi_sys's teardown (no MI call from here) and freed when it lets go */
static int proc_release(struct inode *inode, struct file *file)
{
	int i;

	mutex_lock(&cmd_lock);
	for (i = 0; i < POOLS; i++) {
		struct pool *p = pools[i];
		unsigned long f;
		if (!p || p->owner != file)
			continue;
		spin_lock_irqsave(&p->lock, f);
		p->owner = NULL;
		p->view = 0;
		spin_unlock_irqrestore(&p->lock, f);
		pr_warn("microcam_canvas: pool %d left open by its owner; freed when mi_sys lets it go\n", i);
		schedule_work(&reaper);
	}
	mutex_unlock(&cmd_lock);
	return single_release(inode, file);
}

/* /proc/microcam_canvas_lent: a read at offset 0 gives, per open pool that has lent the port a view, «<pool> <number>
 * <canvas> <ns>» — the newest lending; poll is readable while lendings came since this file's last read. Read with
 * pread at 0 (any other offset reads nothing). */
static int lent_open(struct inode *inode, struct file *file)
{
	file->private_data = (void *)(long)atomic_read(&lendings);
	return 0;
}

static unsigned int lent_poll(struct file *file, poll_table *wait)
{
	poll_wait(file, &lent_wq, wait);
	return atomic_read(&lendings) != (int)(long)file->private_data ? POLLIN | POLLRDNORM : 0;
}

static ssize_t lent_read(struct file *file, char __user *buf, size_t len, loff_t *off)
{
	char out[POOLS * 48];
	int i, n = 0;

	if (*off)
		return 0;
	file->private_data = (void *)(long)atomic_read(&lendings);
	mutex_lock(&cmd_lock);
	for (i = 0; i < POOLS; i++) {
		struct pool *p = pools[i];
		unsigned long f;
		if (!p)
			continue;
		spin_lock_irqsave(&p->lock, f);
		if (p->owner && p->lent_seq)
			n += scnprintf(out + n, sizeof(out) - n, "%d %u %d %llu\n", p->id, p->lent_seq, p->lent_k, p->lent_ns);
		spin_unlock_irqrestore(&p->lock, f);
	}
	mutex_unlock(&cmd_lock);
	if (n > len)
		return -EINVAL;
	if (copy_to_user(buf, out, n))
		return -EFAULT;
	*off = n;
	return n;
}

static const struct file_operations lent_fops = {
	.open = lent_open,
	.read = lent_read,
	.poll = lent_poll,
	.llseek = default_llseek,
};

static const struct file_operations proc_fops = {
	.open = proc_open,
	.read = seq_read,
	.llseek = seq_lseek,
	.write = proc_write,
	.release = proc_release,
};

static int __init microcam_canvas_init(void)
{
	getdev = (getdev_t)kallsyms_lookup_name("_MI_SYS_IMPL_GetModDevInfoById");
	if (!getdev) {
		pr_err("microcam_canvas: no _MI_SYS_IMPL_GetModDevInfoById\n");
		return -ENOENT;
	}
	ops_port[0] = generic_allocator_on_ref;
	ops_port[1] = generic_allocator_on_unref;
	ops_port[2] = allocator_release;
	ops_port[3] = port_alloc_cb;
	ops_port[4] = suit_cb;
	memcpy(ops_enc, ops_port, sizeof(ops_enc));
	ops_enc[3] = enc_alloc_cb;
	ops_allocation[0] = generic_allocation_on_ref;
	ops_allocation[1] = generic_allocation_on_unref;
	ops_allocation[2] = allocation_release;
	proc_create("microcam_canvas", 0600, NULL, &proc_fops);
	proc_create("microcam_canvas_holds", 0400, NULL, &holds_fops);
	proc_create("microcam_canvas_lent", 0400, NULL, &lent_fops);
	return 0;
}

static void __exit microcam_canvas_exit(void)
{
	/* a pool holds a reference to the module until it is gone, so none is left here */
	remove_proc_entry("microcam_canvas_lent", NULL);
	remove_proc_entry("microcam_canvas_holds", NULL);
	remove_proc_entry("microcam_canvas", NULL);
	flush_work(&reaper);
}

module_init(microcam_canvas_init);
module_exit(microcam_canvas_exit);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_DESCRIPTION("microcam: the frames an encoder reads, the live picture written straight into them");
