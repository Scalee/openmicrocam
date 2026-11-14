/*------------------------- usb_gadget_ops ----------------------------------*/
#include <linux/delay.h>
#include <linux/dma-mapping.h>
#include "msb250x_udc_reg.h"
#include "msb250x_udc.h"

void msb250x_gadget_sync_request(struct usb_gadget* gadget, struct usb_request* req, int offset, int size)
{
    #ifdef __arch_pfn_to_dma
        dma_sync_single_range_for_cpu(gadget->dev.parent, req->dma, offset, size, DMA_FROM_DEVICE);
    #else
        dma_addr_t __dma = req->dma;
        //__dma = (MIU1_BUS_BASE_ADDR > __dma)? ((__dma - MIU0_BUS_BASE_ADDR) + MIU0_BASE_ADDR) : ((__dma - MIU1_BUS_BASE_ADDR) + MIU1_BASE_ADDR);
        __dma = MSB250X_BUS2PA(__dma);
        dma_sync_single_range_for_cpu(gadget->dev.parent, __dma, offset, size, DMA_FROM_DEVICE);
    #endif

}
EXPORT_SYMBOL(msb250x_gadget_sync_request);

extern void Chip_Flush_Cache_Range(unsigned long u32Addr, unsigned long u32Size);
extern void Chip_Flush_MIU_Pipe(void);

int msb250x_gadget_map_request(struct usb_gadget* gadget, struct usb_request* req, int is_in)
{
    int ret = usb_gadget_map_request(gadget, req, is_in);
    dma_addr_t __dma = req->dma;

    if (is_in && req->length)
    {
        /* microcam: as the vendor binary's map_request does: flush the CPU cache range,
         * then drain the MIU write pipe so the controller's DMA never reads data still on its way to DRAM */
        Chip_Flush_Cache_Range((unsigned long) req->buf, req->length);
        Chip_Flush_MIU_Pipe();
    }

#ifndef __arch_pfn_to_dma
    //__dma = (MIU1_BASE_ADDR > __dma)? (MIU0_BUS_BASE_ADDR | (__dma - MIU0_BASE_ADDR)) : (MIU1_BUS_BASE_ADDR + (__dma - MIU1_BASE_ADDR));
    __dma = MSB250X_PA2BUS(__dma);
    req->dma = __dma;
#endif

    return ret;
}
EXPORT_SYMBOL(msb250x_gadget_map_request);

void msb250x_gadget_unmap_request(struct usb_gadget* gadget, struct usb_request* req, int is_in)
{
#ifndef __arch_pfn_to_dma
        dma_addr_t __dma = req->dma;
        //__dma = (MIU1_BUS_BASE_ADDR > __dma)? ((__dma - MIU0_BUS_BASE_ADDR) + MIU0_BASE_ADDR) : ((__dma - MIU1_BUS_BASE_ADDR) + MIU1_BASE_ADDR);
        __dma = MSB250X_BUS2PA(__dma);
        req->dma = __dma;
#endif

    usb_gadget_unmap_request(gadget, req, is_in);
}
EXPORT_SYMBOL(msb250x_gadget_unmap_request);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_pullup_i
+------------------------------------------------------------------------------
| DESCRIPTION : internal software connection function
|
| RETURN      : NULL
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| is_on                 |x  |       | enable the software connection or not
|--------------------+---+---+-------------------------------------------------
*/
void msb250x_gadget_pullup_i(int is_on)
{
    u8 power = 0;
    struct otg0_usb_power* pst_power = (struct otg0_usb_power*) &power;

    power = ms_readb(MSB250X_OTG0_PWR_REG);
    pst_power->bSoftConn = is_on;

    ms_writeb(power, MSB250X_OTG0_PWR_REG);

    printk("<USB>[GADGET] PULL_UP(%s)\n", (0 == is_on)? "OFF" : "ON");
}

EXPORT_SYMBOL(msb250x_gadget_pullup_i);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_get_frame
+------------------------------------------------------------------------------
| DESCRIPTION : get frame count
|
| RETURN      : the current frame number
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| _gadget          |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
*/
int msb250x_gadget_get_frame(struct usb_gadget *g)
{
    return ms_readw(MSB250X_OTG0_FRAME_L_REG);
}
EXPORT_SYMBOL(msb250x_gadget_get_frame);

/* microcam: can this endpoint take the descriptor — unclaimed, its type and direction in the caps, packet and FIFO big enough */
static int msb250x_gadget_ep_fits(struct msb250x_ep *ep,
                                  struct usb_endpoint_descriptor *desc,
                                  int maxpacket, int mult)
{
    const struct usb_ep_caps *caps = &ep->ep.caps;
    int type_ok;

    switch (usb_endpoint_type(desc))
    {
        case USB_ENDPOINT_XFER_ISOC:
            type_ok = caps->type_iso;
            break;
        case USB_ENDPOINT_XFER_BULK:
            type_ok = caps->type_bulk;
            break;
        case USB_ENDPOINT_XFER_INT:
            type_ok = caps->type_int;
            break;
        default:
            type_ok = caps->type_control;
            break;
    }

    return type_ok && (usb_endpoint_dir_in(desc) ? caps->dir_in : caps->dir_out) && !ep->ep.claimed &&
           maxpacket <= ep->ep.maxpacket_limit && maxpacket * mult <= ep->fifo_size;
}

struct usb_ep*
msb250x_gadget_match_ep(struct usb_gadget *g,
                        struct usb_endpoint_descriptor *desc,
                        struct usb_ss_ep_comp_descriptor *ep_comp)
{
    struct msb250x_udc* dev = to_msb250x_udc(g);
    struct msb250x_ep* select = NULL;

    int maxpacket = usb_endpoint_maxp(desc) & 0x07FF;
    int mult = usb_endpoint_maxp_mult(desc);
    int index, pass;

    if (!maxpacket)
    {
        switch (usb_endpoint_type(desc))
        {
            case USB_ENDPOINT_XFER_ISOC:
                maxpacket = 1024;
                break;
            case USB_ENDPOINT_XFER_BULK:
                maxpacket = 512;
                break;
            default:
                maxpacket = 64;
                break;
        }
    }

    /* microcam: bulk first looks only at endpoints with a FIFO of their own for one direction (ep5in, ep5out), so a function's
     * bulk pair lands there whatever the bind order and the shared-FIFO endpoints stay free for the next function. Then,
     * and for every other type, the original rule: the smallest maxpacket_limit that fits, the later one among equals. */
    for (pass = usb_endpoint_xfer_bulk(desc) ? 0 : 1; pass < 2 && !select; pass++)
    {
        for (index = 1; index < MSB250X_EP_COUNT; index++)
        {
            struct msb250x_ep* cand = &dev->ep[index];

            if (0 == pass && cand->ep.caps.dir_in == cand->ep.caps.dir_out)
                continue;

            if (!msb250x_gadget_ep_fits(cand, desc, maxpacket, mult))
                continue;

            if (!select || select->ep.maxpacket_limit >= cand->ep.maxpacket_limit)
            {
                select = cand;
            }
        }
    }

    if (select)
    {
        printk(KERN_DEBUG"<USB>[GADGET] Match %s(%d x %d)", select->ep.name, mult, maxpacket);
        return &select->ep;
    }

    printk("<USB_ERR>[GADGET] No Match EP(%d x %d)\n", mult, maxpacket);
    return NULL;
}
EXPORT_SYMBOL(msb250x_gadget_match_ep);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_wakeup
+------------------------------------------------------------------------------
| DESCRIPTION : tries to wake up the host connected to this gadget
|
| RETURN      : zero on success, else negative error code
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| _gadget          |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
*/
int msb250x_gadget_wakeup(struct usb_gadget *g)
{
    printk("Entered %s\n", __FUNCTION__);
    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_wakeup);


/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_set_selfpowered
+------------------------------------------------------------------------------
| DESCRIPTION : sets the device selfpowered feature
|
| RETURN      : zero on success, else negative error code
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| gadget                 |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
| value                 |x  |       | set this feature or not
|--------------------+---+---+-------------------------------------------------
*/
int msb250x_gadget_set_selfpowered(struct usb_gadget *g,
                                   int value)
{
    struct msb250x_udc *udc = to_msb250x_udc(g);

    if (value)
        udc->devstatus |= (1 << USB_DEVICE_SELF_POWERED);
    else
        udc->devstatus &= ~(1 << USB_DEVICE_SELF_POWERED);

    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_set_selfpowered);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_pullup
+------------------------------------------------------------------------------
| DESCRIPTION : software-controlled connect to USB host
|
| RETURN      : zero on success, else negative error code
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| gadget                 |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
| is_on                 |x  |       | set software-controlled connect to USB host or not
|--------------------+---+---+-------------------------------------------------
*/
int msb250x_gadget_pullup(struct usb_gadget *g,
                          int is_on)
{
    struct msb250x_udc *dev = to_msb250x_udc(g);

    u8 power = ms_readb(MSB250X_OTG0_PWR_REG);
    struct otg0_usb_power* pst_power = (struct otg0_usb_power*) &power;

    dev->soft_conn = pst_power->bSoftConn;

    if (1 == is_on)
    {
        if (0 == dev->soft_conn)
        {
            msb250x_gadget_pullup_i(is_on);
        }
    }
    else
    {
        if (1 == dev->soft_conn)
        {
            msb250x_gadget_pullup_i(is_on);
        }
    }

    dev->soft_conn = is_on;

    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_pullup);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_vbus_session
+------------------------------------------------------------------------------
| DESCRIPTION : establish the USB session
|
| RETURN      : zero on success, else negative error code
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| gadget                 |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
| is_active          |x  |       | establish the session or not
|--------------------+---+---+-------------------------------------------------
*/

int msb250x_gadget_vbus_session(struct usb_gadget *g,
                                int is_active)
{
    printk("Entered %s\n", __FUNCTION__);
    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_vbus_session);

/*
+------------------------------------------------------------------------------
| FUNCTION    : msb250x_gadget_vbus_draw
+------------------------------------------------------------------------------
| DESCRIPTION : constrain controller's VBUS power usage
|
| RETURN      : zero on success, else negative error code
+------------------------------------------------------------------------------
| Variable Name      |IN |OUT|                   Usage
|--------------------+---+---+-------------------------------------------------
| _gadget          |x  |       | usb_gadget struct point
|--------------------+---+---+-------------------------------------------------
| ma                 |x  |       | milliAmperes
|--------------------+---+---+-------------------------------------------------
*/
int msb250x_gadget_vbus_draw(struct usb_gadget *g,
                             unsigned ma)
{
    printk("Entered %s\n", __FUNCTION__);
    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_vbus_draw);


int msb250x_gadget_udc_start(struct usb_gadget *g,
                             struct usb_gadget_driver *driver)
{
    struct msb250x_udc *dev = NULL;

    if (!g)
    {
        printk("<USB_ERR>[GADGET] ENODEV!\n");
        return -ENODEV;
    }

    dev = to_msb250x_udc(g);

    dev->driver = driver;
    dev->gadget.dev.driver = &driver->driver;

    /* microcam: no PHY reset/re-init here — the vendor module programs the PHY once, in probe (docs/firmware/components/usb-gadget.md §2.3) */
    msb250x_udc_init_usb_ctrl();
    msb250x_udc_reset_otg();
    msb250x_udc_init_otg(dev);

    mdelay(1);

    printk("<USB>[GADGET] UDC start\n");
    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_udc_start);

int msb250x_gadget_udc_stop(struct usb_gadget *g)
{
    struct msb250x_udc *dev = NULL;
    int i;

    if (!g)
    {
        printk("<USB_ERR>[GADGET] ENODEV!\n");
        return -ENODEV;
    }

    dev = to_msb250x_udc(g);

    for (i = 0; i < MSB250X_EP_COUNT; i++)
    {
        dev->ep[i].ep.desc = NULL;
    }
    dev->driver = NULL;
    dev->gadget.dev.driver = NULL;

    msb250x_udc_init_usb_ctrl();
    return 0;
}
EXPORT_SYMBOL(msb250x_gadget_udc_stop);

