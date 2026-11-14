/* Read/set the output-port depth (user fifo depth, buffer queue depth) of a running MI channel from a second process.
 * usage: portdepth <mod> <dev> <chn> <port> [user queue]    SCL mod=34, ISP mod=?, VIF mod=?  (ids in mi_common_datatype.h) */
#include <stdio.h>
#include <stdlib.h>

typedef struct { unsigned mod, dev, chn, port; } ChnPort;
extern int MI_SYS_SetChnOutputPortDepth(unsigned short soc, ChnPort *p, unsigned user, unsigned queue);
extern int MI_SYS_GetChnOutputPortDepth(unsigned short soc, ChnPort *p, unsigned *user, unsigned *queue);

int main(int argc, char **argv)
{
    setvbuf(stdout, 0, _IONBF, 0);
    if (argc < 5) { fprintf(stderr, "usage: portdepth <mod> <dev> <chn> <port> [user queue]\n"); return 2; }
    ChnPort p = { strtoul(argv[1], 0, 0), strtoul(argv[2], 0, 0), strtoul(argv[3], 0, 0), strtoul(argv[4], 0, 0) };
    unsigned u = ~0u, q = ~0u;
    int r = MI_SYS_GetChnOutputPortDepth(0, &p, &u, &q);
    printf("get mod=%u dev=%u chn=%u port=%u ret=0x%08x user=%u queue=%u\n", p.mod, p.dev, p.chn, p.port, (unsigned)r, u, q);
    if (argc >= 7) {
        r = MI_SYS_SetChnOutputPortDepth(0, &p, strtoul(argv[5], 0, 0), strtoul(argv[6], 0, 0));
        printf("set ret=0x%08x\n", (unsigned)r);
        u = q = ~0u;
        r = MI_SYS_GetChnOutputPortDepth(0, &p, &u, &q);
        printf("get ret=0x%08x user=%u queue=%u\n", (unsigned)r, u, q);
    }
    return 0;
}
