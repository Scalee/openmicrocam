/* microcam: in-memory event ring for diagnosing a receive stall of the OUT endpoints. Written from the ISR and the request paths
 * under dev->lock, read later through the platform device's sysfs file `microcam_trace`. Cheap enough to stay in.
 * `ep` is the endpoint's USB address (number | 0x80 for IN, 0 = ep0), shown as ep4in / ep2out; writing "f 84 02" records
 * only those endpoints (ep0 events always), "f" alone records all again. */
#ifndef __MICROCAM_TRACE_H
#define __MICROCAM_TRACE_H
#include <linux/types.h>
#include <linux/device.h>

enum microcam_tag {
    LT_DMA_IN = 1,  /* ep, ch | ctrl<<8, a=dma addr, b=bytesleft, c=owner req, d=head req */
    LT_DMA_OUT,     /* ep, ch, a=bytesdone, b=actual, c=length, d=is_last | bad<<8 | owner_is_head<<16 */
    LT_EP_IRQ,      /* ep, using_dma, a=csr1 | csr2<<8, b=counts, c=halted | autoNAK0 ctrl<<16, d=head req */
    LT_RH,          /* ep, kind, a=bytes, b=csr, c=halted, d=req */
    LT_DONE,        /* ep, aborted, a=status, b=actual, c=length, d=req */
    LT_DMASET,      /* ep, ch (0 = refused), a=bytes, b=start, c=ctrl word, d=req */
    LT_OK2RCV,      /* bulk-OUT ep of the autoNAK slot, cfg, a=pkt_num, b=autoNAK0 ctrl read back */
    LT_DMA_NOOWNER, /* 0, ch, a=ctrl */
    LT_QUEUE,       /* ep, 0, a=length, b=queue was empty, c=halted, d=req */
    LT_EP0,         /* 0, ep0state, a=csr0, b=count0, c=queue empty, d=head req */
    LT_EP0SETUP,    /* 0, ep0state after, a=bRequestType | bRequest<<8 | wValue<<16, b=wIndex | wLength<<16 */
    LT_EP0ERR,      /* 0, ep0state, a=count0, b=csr0; freezes the ring */
    LT_HALT,        /* ep, value, a=csr1 before, b=result (0, -EAGAIN = fifo busy), c=halted after */
};

void __microcam_trace(u8 tag, u8 ep, u16 x, u32 a, u32 b, u32 c, u32 d);
extern bool microcam_trace_on;   /* write "off"/"on" to the sysfs file; off skips even the argument reads */
#define microcam_trace(...) do { if (microcam_trace_on) __microcam_trace(__VA_ARGS__); } while (0)
int microcam_trace_add_sysfs(struct device *dev);
void microcam_trace_remove_sysfs(struct device *dev);
#endif
