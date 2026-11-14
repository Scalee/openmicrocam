/* fbhide: give U-Boot's logo area back to Linux and hide fb0's window — the GOP U-Boot shows its boot logo in — through fbdev's FBIOSET_SHOW, before app-start.sh
 * unloads fbdev for microcamd (HDMI's menu is in microcamd's own frames; nothing else hides the logo, and an
 * unloaded fbdev leaves its window on). fbdev believes the window shown (U-Boot's) — shown again first, so its state
 * and the hardware agree, then hidden [R 2026-11-10: the logo gone from the write-back]. Built by build.sh-style
 * linking against the camera's glibc only. */
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <linux/fb.h>
#define FBIOSET_SHOW _IOW('F', 0x63, unsigned char)
int main(void)
{
    int fd = open("/dev/fb0", O_RDWR);
    if (fd < 0) { perror("fbhide: /dev/fb0"); return 1; }
    unsigned char on = 1, off = 0;
    /* fbdev gives U-Boot's logo area (`mmap_reserved=fb`, 8 MiB) back to Linux at its first pan, nowhere else */
    struct fb_var_screeninfo v;
    int r = ioctl(fd, FBIOGET_VSCREENINFO, &v);
    if (!r) { v.xoffset = v.yoffset = 0; r = ioctl(fd, FBIOPAN_DISPLAY, &v); if (r) perror("fbhide: FBIOPAN_DISPLAY"); }
    r |= ioctl(fd, FBIOSET_SHOW, &on) | ioctl(fd, FBIOSET_SHOW, &off);
    close(fd);
    if (r) perror("fbhide: FBIOSET_SHOW");
    return r ? 1 : 0;
}
