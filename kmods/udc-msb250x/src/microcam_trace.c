#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/spinlock.h>
#include <linux/jiffies.h>
#include <linux/sched.h>
#include <asm/io.h>
#include "msb250x_udc_reg.h"
#include "msb250x_udc.h"
#include "microcam_trace.h"

#define N 1024
struct ev { u32 t; u8 tag; u8 ep; u16 x; u32 a, b, c, d; };
static struct ev ring[N];
static unsigned pos;
static u32 total;
static unsigned view;   /* how many newest events to skip when showing (write "o <n>") */
static const char *names[] = { "?", "dma_in", "dma_out", "ep_irq", "rh", "done", "dmaset", "ok2rcv", "noowner", "queue", "ep0", "ep0setup", "ep0err", "halt" };
static u8 only[8];      /* record only these endpoint addresses (and ep0); none = all ("f 84 02" / "f") */
static int n_only;

bool microcam_trace_on = true;

void __microcam_trace(u8 tag, u8 ep, u16 x, u32 a, u32 b, u32 c, u32 d)
{
    struct ev *e;
    int i;

    if (n_only && (ep & 0x7f))
    {
        for (i = 0; i < n_only && only[i] != ep; i++)
            ;
        if (i == n_only)
            return;
    }
    e = &ring[pos];
    pos = (pos + 1) % N;
    total++;
    e->t = (u32) jiffies; e->tag = tag; e->ep = ep; e->x = x; e->a = a; e->b = b; e->c = c; e->d = d;
}

static ssize_t microcam_trace_show(struct device *dev, struct device_attribute *attr, char *buf)
{
    unsigned i, n = 0;
    ssize_t len = 0;
    /* live controller state at read time: autoNAK/config words 0x80..0x93, then per-endpoint RXCSR and RXCOUNT,
     * then the DMA channel control words */
    len += scnprintf(buf + len, PAGE_SIZE - len, "cfg");
    for (i = 0; i < 10; i++)
        len += scnprintf(buf + len, PAGE_SIZE - len, " %04x", ms_readw(MSB250X_OTG0_USB_CFG0_L + i * 4));
    len += scnprintf(buf + len, PAGE_SIZE - len, "\nrx");
    for (i = 1; i <= 6; i++)
        len += scnprintf(buf + len, PAGE_SIZE - len, " ep%u:%02x%02x/%u", i, ms_readb(MSB250X_OTG0_EP_RXCSR2_REG(i)),
                         ms_readb(MSB250X_OTG0_EP_RXCSR1_REG(i)),
                         ms_readb(MSB250X_OTG0_EP_RXCOUNT_L_REG(i)) | (ms_readb(MSB250X_OTG0_EP_RXCOUNT_H_REG(i)) << 8));
    len += scnprintf(buf + len, PAGE_SIZE - len, "\ndma");
    for (i = 1; i <= 4; i++)
        len += scnprintf(buf + len, PAGE_SIZE - len, " ch%u:%04x", i, ms_readw(MSB250X_OTG0_DMA_CNTL(i)));
    len += scnprintf(buf + len, PAGE_SIZE - len, "\nrecording");
    if (!n_only)
        len += scnprintf(buf + len, PAGE_SIZE - len, " all");
    for (i = 0; i < n_only; i++)
        len += scnprintf(buf + len, PAGE_SIZE - len, " %02x", only[i]);
    len += scnprintf(buf + len, PAGE_SIZE - len, "\n");
    for (i = N - 40 - min_t(unsigned, view, N - 40); i < N - min_t(unsigned, view, N - 40) && len < PAGE_SIZE - 90; i++)   /* 40 fit one page */
    {
        struct ev *e = &ring[(pos + i) % N];
        if (!e->tag) continue;
        len += scnprintf(buf + len, PAGE_SIZE - len, "%u %s ep%u%s x=%04x %08x %08x %08x %08x\n",
                         e->t, e->tag < ARRAY_SIZE(names) ? names[e->tag] : "?", e->ep & 0x7f,
                         (e->ep & 0x7f) ? ((e->ep & 0x80) ? "in" : "out") : "", e->x, e->a, e->b, e->c, e->d);
        n++;
    }
    len += scnprintf(buf + len, PAGE_SIZE - len, "total %u shown %u\n", total, n);
    return len;
}

static ssize_t microcam_trace_store(struct device *dev, struct device_attribute *attr, const char *buf, size_t count)
{
    unsigned i;
    if (sscanf(buf, "o %u", &view) == 1)
        return count;
    if (buf[0] == 'f')
    {
        n_only = sscanf(buf, "f %hhx %hhx %hhx %hhx %hhx %hhx %hhx %hhx", &only[0], &only[1], &only[2], &only[3],
                        &only[4], &only[5], &only[6], &only[7]);
        if (n_only < 0)
            n_only = 0;
        return count;
    }
    if (!strncmp(buf, "off", 3) || !strncmp(buf, "on", 2))
    {
        microcam_trace_on = buf[1] == 'n';
        return count;
    }
    view = 0;
    for (i = 0; i < N; i++) { ring[i].tag = 0; }
    pos = 0; total = 0;
    return count;
}

static DEVICE_ATTR(microcam_trace, 0644, microcam_trace_show, microcam_trace_store);

int microcam_trace_add_sysfs(struct device *dev) { return device_create_file(dev, &dev_attr_microcam_trace); }
void microcam_trace_remove_sysfs(struct device *dev) { device_remove_file(dev, &dev_attr_microcam_trace); }
