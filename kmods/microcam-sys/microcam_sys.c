// SPDX-License-Identifier: MIT-0 OR GPL-2.0
/*
 * microcam_sys: what microcamd needs of mi_gfx that libmi_gfx does not offer to user space.
 *
 * `echo gewait <tag> <ms> > /proc/microcam_sys`: wait until the 2D engine (GE) has finished the job with that tag (the
 * fence `MI_GFX_BitBlit` / `MI_GFX_QuickFill` return), at most `ms` milliseconds — without mi_gfx's module mutex.
 * mi_gfx's own `MI_GFX_IOCTL_WaitAllDone` takes that mutex and sleeps in it until the tag is done, so every other GE
 * call of the process (a BitBlit from another thread) waits for the sleeper's whole job. This waits on the driver's own interrupt wait queue (or polls the tag when the queue's
 * symbol is not found), and leaves the mutex to the short calls. Returns the length written, or -ETIMEDOUT.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/kallsyms.h>
#include <linux/proc_fs.h>
#include <linux/uaccess.h>
#include <linux/wait.h>
#include <linux/delay.h>
#include <linux/sched.h>

/* the camera's kernel exports no __memzero, which our tree's string.h makes the inline copy_from_user call */
void __memzero(void *p, __kernel_size_t n)
{
	(memset)(p, 0, n);
}

typedef int (*tagdone_t)(u16 tag);
static tagdone_t tagdone;
static wait_queue_head_t *geq;

static int gewait(u32 tag, u32 ms)
{
	unsigned long end = jiffies + msecs_to_jiffies(ms) + 1;

	if (!tagdone)
		return -ENODEV;
	if (geq) {
		if (wait_event_timeout(*geq, tagdone((u16)tag), msecs_to_jiffies(ms) + 1) == 0 && !tagdone((u16)tag))
			return -ETIMEDOUT;
		return 0;
	}
	while (!tagdone((u16)tag)) {
		if (time_after(jiffies, end))
			return -ETIMEDOUT;
		usleep_range(50, 150);
	}
	return 0;
}

static ssize_t proc_write(struct file *f, const char __user *buf, size_t len, loff_t *off)
{
	char cmd[48];
	u32 tag, v;
	int r;

	if (len >= sizeof(cmd) || copy_from_user(cmd, buf, len))
		return -EINVAL;
	cmd[len] = 0;
	if (sscanf(cmd, "gewait %u %u", &tag, &v) != 2)
		return -EINVAL;
	r = gewait(tag, v ? v : 1000);
	return r ? r : len;
}

static const struct file_operations proc_fops = {
	.write = proc_write,
};

static int __init microcam_sys_init(void)
{
	tagdone = (tagdone_t)kallsyms_lookup_name("MApi_GE_TagIsFinish");
	geq = (wait_queue_head_t *)kallsyms_lookup_name("g_ge_isr_wait_queue");
	pr_info("microcam_sys: GE tag check %s, wait queue %s\n", tagdone ? "found" : "missing", geq ? "found" : "missing (polling)");
	proc_create("microcam_sys", 0200, NULL, &proc_fops);
	return 0;
}

static void __exit microcam_sys_exit(void)
{
	remove_proc_entry("microcam_sys", NULL);
}

module_init(microcam_sys_init);
module_exit(microcam_sys_exit);
MODULE_LICENSE("Dual MIT/GPL");
MODULE_DESCRIPTION("microcam: mi_gfx calls user space has no way to");
