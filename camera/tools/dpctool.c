/* ISP defect-pixel correction records of the running pipeline, from a second process (same call path as isptool.c).
 * usage: dpctool dp|cl get                 hex dump of DynamicDP (756 B) / DynamicDP_CLUSTER (1164 B)
 *        dpctool dp|cl save|load <file>    record to / from a file
 *        dpctool dp|cl poke <off> <size> <value>   write a 1/2/4-byte value at a byte offset, set, read back */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int (*iq_fn)(unsigned dev, unsigned chn, void *data);
extern int MI_ISP_IQ_GetDynamicDP(unsigned, unsigned, void *);
extern int MI_ISP_IQ_SetDynamicDP(unsigned, unsigned, void *);
extern int MI_ISP_IQ_GetDynamicDP_CLUSTER(unsigned, unsigned, void *);
extern int MI_ISP_IQ_SetDynamicDP_CLUSTER(unsigned, unsigned, void *);

static unsigned char buf[65536];

static void hex(int n)
{
    for (int i = 0; i < n; i++)
        printf("%s%02x%s", i % 32 ? "" : "", buf[i], (i % 32 == 31) ? "\n" : " ");
    printf("\n");
}

int main(int argc, char **argv)
{
    setvbuf(stdout, 0, _IONBF, 0);
    if (argc < 3) return 2;
    int cl = !strcmp(argv[1], "cl");
    iq_fn get = cl ? MI_ISP_IQ_GetDynamicDP_CLUSTER : MI_ISP_IQ_GetDynamicDP;
    iq_fn set = cl ? MI_ISP_IQ_SetDynamicDP_CLUSTER : MI_ISP_IQ_SetDynamicDP;
    int n = cl ? 1164 : 756;
    memset(buf, 0, sizeof buf);
    int r = get(0, 0, buf);
    printf("%s get ret=0x%08x\n", cl ? "DPC_CLUSTER" : "DynamicDP", (unsigned)r);
    if (r) return 1;
    if (!strcmp(argv[2], "get")) {
        hex(n);
        return 0;
    }
    if (!strcmp(argv[2], "save") && argc == 4) {
        FILE *f = fopen(argv[3], "wb");
        if (!f || fwrite(buf, 1, n, f) != (size_t)n) return 1;
        fclose(f);
        printf("saved %s\n", argv[3]);
        return 0;
    }
    if (!strcmp(argv[2], "load") && argc == 4) {
        FILE *f = fopen(argv[3], "rb");
        if (!f || fread(buf, 1, n, f) != (size_t)n) return 1;
        fclose(f);
    } else if (!strcmp(argv[2], "poke") && argc == 6) {
        unsigned off = strtoul(argv[3], 0, 0), sz = strtoul(argv[4], 0, 0), v = strtoul(argv[5], 0, 0);
        if (off + sz > (unsigned)n || (sz != 1 && sz != 2 && sz != 4)) return 1;
        memcpy(buf + off, &v, sz);
    } else
        return 2;
    r = set(0, 0, buf);
    printf("set ret=0x%08x\n", (unsigned)r);
    memset(buf, 0, sizeof buf);
    r = get(0, 0, buf);
    printf("get ret=0x%08x\n", (unsigned)r);
    hex(n);
    return 0;
}
