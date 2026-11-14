/* Read and set ISP IQ records of the running pipeline (microcamd or the vendor app) from a second process.
 * usage: isptool [dev chn]                 hex dump of ColorToGray, APIBypassMode, NR3D
 *        isptool gray|nr3d <0|1>           set the record's enable word, read it back
 *        isptool nr3d save|load <file> | tf <v> | fill <off,count,size,value>
 * Signatures follow the camera's libmi_isp.so (DevId, Channel, data), not the older 2-arg headers. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int (*iq_fn)(unsigned dev, unsigned chn, void *data);
extern int MI_ISP_IQ_GetNR3D(unsigned, unsigned, void *);
extern int MI_ISP_IQ_GetColorToGray(unsigned, unsigned, void *);
extern int MI_ISP_IQ_GetAPIBypassMode(unsigned, unsigned, void *);
extern int MI_ISP_IQ_SetNR3D(unsigned, unsigned, void *);
extern int MI_ISP_IQ_SetColorToGray(unsigned, unsigned, void *);

static unsigned char buf[65536];

static void dump(const char *name, iq_fn f, unsigned dev, unsigned chn, int n)
{
    memset(buf, 0xA5, sizeof buf);
    int r = f(dev, chn, buf);
    printf("%s dev=%u chn=%u ret=0x%08x\n", name, dev, chn, (unsigned)r);
    for (int i = 0; i < n; i++)
        printf("%02x%s", buf[i], (i % 32 == 31) ? "\n" : " ");
    printf("\n");
    fflush(stdout);
}

/* Read the record, set its leading bEnable word, write it back, read again. */
static int set_enable(const char *name, iq_fn get, iq_fn set, unsigned v)
{
    memset(buf, 0, sizeof buf);
    int r = get(0, 0, buf);
    printf("%s get ret=0x%08x enable=%u\n", name, (unsigned)r, *(unsigned *)buf);
    if (r) return 1;
    *(unsigned *)buf = v;
    r = set(0, 0, buf);
    printf("%s set enable=%u ret=0x%08x\n", name, v, (unsigned)r);
    memset(buf, 0, sizeof buf);
    r = get(0, 0, buf);
    printf("%s get ret=0x%08x enable=%u\n", name, (unsigned)r, *(unsigned *)buf);
    return 0;
}

#define NR3D_SIZE 2116 /* bEnable, enOpType, 16 auto + 1 manual NR3D_PARAM_t of 124 B */
#define NR3D_PARAM(i) (buf + 8 + (i) * 124)

/* save <file>: live NR3D record to a file; load <file>: file back into the ISP; tf <v>: u8TfStr/u8TfStrEx of all 17 sets;
 * fill <off,count,elemsize,value>: count elements of elemsize bytes at NR3D_PARAM_t offset off, in all 17 sets. */
static int nr3d_cmd(const char *cmd, const char *arg)
{
    memset(buf, 0, sizeof buf);
    int r = MI_ISP_IQ_GetNR3D(0, 0, buf);
    printf("NR3D get ret=0x%08x enable=%u op=%u tf=%u/%u\n", (unsigned)r, *(unsigned *)buf, *(unsigned *)(buf + 4),
           NR3D_PARAM(16)[4], NR3D_PARAM(16)[5]);
    if (r) return 1;
    if (!strcmp(cmd, "save")) {
        FILE *f = fopen(arg, "wb");
        if (!f || fwrite(buf, 1, NR3D_SIZE, f) != NR3D_SIZE) return 1;
        fclose(f);
        printf("saved %s\n", arg);
        return 0;
    }
    if (!strcmp(cmd, "load")) {
        FILE *f = fopen(arg, "rb");
        if (!f || fread(buf, 1, NR3D_SIZE, f) != NR3D_SIZE) return 1;
        fclose(f);
    } else if (!strcmp(cmd, "tf")) {
        for (int i = 0; i < 17; i++)
            NR3D_PARAM(i)[4] = NR3D_PARAM(i)[5] = strtoul(arg, 0, 0);
    } else if (!strcmp(cmd, "fill")) {
        unsigned off, n, sz, v;
        if (sscanf(arg, "%u,%u,%u,%u", &off, &n, &sz, &v) != 4 || off + n * sz > 124) return 1;
        for (int i = 0; i < 17; i++)
            for (unsigned k = 0; k < n; k++)
                memcpy(NR3D_PARAM(i) + off + k * sz, &v, sz);
    } else
        return 1;
    r = MI_ISP_IQ_SetNR3D(0, 0, buf);
    printf("NR3D set ret=0x%08x\n", (unsigned)r);
    memset(buf, 0, sizeof buf);
    r = MI_ISP_IQ_GetNR3D(0, 0, buf);
    printf("NR3D get ret=0x%08x enable=%u op=%u tf=%u/%u\n", (unsigned)r, *(unsigned *)buf, *(unsigned *)(buf + 4),
           NR3D_PARAM(16)[4], NR3D_PARAM(16)[5]);
    return 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, 0, _IONBF, 0);
    if (argc == 3 && !strcmp(argv[1], "gray"))
        return set_enable("ColorToGray", MI_ISP_IQ_GetColorToGray, MI_ISP_IQ_SetColorToGray, strtoul(argv[2], 0, 0));
    if (argc == 4 && !strcmp(argv[1], "nr3d"))
        return nr3d_cmd(argv[2], argv[3]);
    if (argc == 3 && !strcmp(argv[1], "nr3d"))
        return set_enable("NR3D", MI_ISP_IQ_GetNR3D, MI_ISP_IQ_SetNR3D, strtoul(argv[2], 0, 0));
    unsigned dev = argc > 1 ? strtoul(argv[1], 0, 0) : 0;
    unsigned chn = argc > 2 ? strtoul(argv[2], 0, 0) : 0;
    dump("ColorToGray", MI_ISP_IQ_GetColorToGray, dev, chn, 32);
    dump("APIBypassMode", MI_ISP_IQ_GetAPIBypassMode, dev, chn, 32);
    dump("NR3D", MI_ISP_IQ_GetNR3D, dev, chn, 256);
    return 0;
}
