/* Read-only dump of the live DISP / HDMI / SCL-port settings (raw bytes) to verify struct layouts before changing anything. */
#include <stdio.h>
#include <string.h>

extern int MI_DISP_GetPubAttr(int dev, void *attr);            /* 52 B */
extern int MI_DISP_GetVideoLayerAttr(int layer, void *attr);   /* 20 B */
extern int MI_DISP_GetInputPortAttr(int layer, unsigned port, void *attr); /* 12 B */
extern int MI_DISP_GetPlayToleration(int layer, unsigned *tol);
extern int MI_HDMI_GetAttr(int id, void *attr);                /* 48 B */
extern int MI_SCL_GetOutputPortParam(unsigned dev, unsigned chn, unsigned port, void *p); /* 24 B */

static void hex(const char *name, int r, const unsigned char *b, int n)
{
    printf("%-18s ret=0x%08x:", name, (unsigned)r);
    for (int i = 0; i < n; i++) printf("%s%02x", (i % 4) ? "" : " ", b[i]);
    printf("\n");
}

int main(void)
{
    unsigned char b[256]; int r; unsigned tol = ~0u;
    setvbuf(stdout, 0, _IONBF, 0);
    memset(b, 0xee, sizeof b); r = MI_DISP_GetPubAttr(0, b);            hex("DISP PubAttr", r, b, 52);
    memset(b, 0xee, sizeof b); r = MI_DISP_GetVideoLayerAttr(0, b);     hex("DISP LayerAttr", r, b, 20);
    memset(b, 0xee, sizeof b); r = MI_DISP_GetInputPortAttr(0, 0, b);   hex("DISP InPortAttr", r, b, 12);
    r = MI_DISP_GetPlayToleration(0, &tol); printf("%-18s ret=0x%08x: %u\n", "DISP Toleration", (unsigned)r, tol);
    memset(b, 0xee, sizeof b); r = MI_HDMI_GetAttr(0, b);               hex("HDMI Attr", r, b, 48);
    memset(b, 0xee, sizeof b); r = MI_SCL_GetOutputPortParam(0, 0, 0, b); hex("SCL OutPort0", r, b, 24);
    return 0;
}
