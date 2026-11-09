// On-device FairPlay decryption core for the WarpSign decrypt helper.
//
// Written from FairPlayed's public description (no code copied); proven on
// iOS 27.2 (docs/spikes/fairplay-ondevice.md). For one Mach-O embedded in the
// helper's bundle (with its SC_Info/<name>.sinf replicated alongside), ask the
// kernel to run its encrypted pages through FairPlay via mremap_encrypted with
// cryptid=2, read the decrypted pages back, and write them into a destination
// copy with cryptid cleared.

#include "decrypt_core.h"

#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>

#include <libkern/OSByteOrder.h>
#include <mach/mach.h>
#include <mach/machine.h>
#include <mach-o/fat.h>
#include <mach-o/loader.h>

// Real libsystem symbol; no public prototype in the SDK.
extern int mremap_encrypted(void *addr, size_t len, uint32_t cryptid,
                            uint32_t cputype, uint32_t cpusubtype);

typedef struct {
    uint64_t slice_off;
    uint32_t cryptoff, cryptsize, cryptid, cputype, cpusubtype;
    uint64_t cryptoff_in_file;    // slice_off + cryptoff
    uint64_t cryptid_off_in_file; // file offset of the cryptid u32
} slice_t;

static double region_entropy(const uint8_t *p, size_t n) {
    if (n == 0) return 0.0;
    size_t counts[256] = {0};
    for (size_t i = 0; i < n; i++) counts[p[i]]++;
    double e = 0.0;
    for (int i = 0; i < 256; i++) {
        if (counts[i]) {
            double pr = (double)counts[i] / (double)n;
            e -= pr * log2(pr);
        }
    }
    return e;
}

// Parse the arm64 slice's offset + LC_ENCRYPTION_INFO_64. Returns:
//   0  found an encrypted region (cryptid != 0)
//   1  no encryption / already decrypted (cryptid 0 or no LC)
//  <0  parse error
static int parse(int fd, slice_t *s) {
    uint32_t magic = 0;
    if (pread(fd, &magic, 4, 0) != 4) return -1;
    uint64_t base = 0;
    if (magic == FAT_CIGAM || magic == FAT_CIGAM_64) {
        struct fat_header fh;
        if (pread(fd, &fh, sizeof fh, 0) != (ssize_t)sizeof fh) return -2;
        uint32_t n = OSSwapBigToHostInt32(fh.nfat_arch);
        if (n > 32) return -3;
        int found = 0;
        for (uint32_t i = 0; i < n && !found; i++) {
            if (magic == FAT_CIGAM) {
                struct fat_arch fa;
                if (pread(fd, &fa, sizeof fa, sizeof(struct fat_header) + (off_t)i * sizeof fa) != (ssize_t)sizeof fa) return -4;
                if (OSSwapBigToHostInt32(fa.cputype) == CPU_TYPE_ARM64) { base = OSSwapBigToHostInt32(fa.offset); found = 1; }
            } else {
                struct fat_arch_64 fa;
                if (pread(fd, &fa, sizeof fa, sizeof(struct fat_header) + (off_t)i * sizeof fa) != (ssize_t)sizeof fa) return -4;
                if (OSSwapBigToHostInt32(fa.cputype) == CPU_TYPE_ARM64) { base = OSSwapBigToHostInt64(fa.offset); found = 1; }
            }
        }
        if (!found) return 1;  // no arm64 slice: nothing we decrypt
    } else if (magic != MH_MAGIC_64) {
        return 1;  // not a 64-bit Mach-O (scripts, nibs, etc.)
    }

    struct mach_header_64 mh;
    if (pread(fd, &mh, sizeof mh, (off_t)base) != (ssize_t)sizeof mh) return -7;
    if (mh.magic != MH_MAGIC_64 || mh.cputype != CPU_TYPE_ARM64) return 1;
    s->slice_off = base;
    s->cputype = (uint32_t)mh.cputype;
    s->cpusubtype = (uint32_t)mh.cpusubtype & ~CPU_SUBTYPE_MASK;

    uint64_t pos = base + sizeof mh;
    for (uint32_t i = 0; i < mh.ncmds; i++) {
        struct load_command lc;
        if (pread(fd, &lc, sizeof lc, (off_t)pos) != (ssize_t)sizeof lc) return -9;
        if (lc.cmd == LC_ENCRYPTION_INFO_64) {
            struct encryption_info_command_64 e;
            if (pread(fd, &e, sizeof e, (off_t)pos) != (ssize_t)sizeof e) return -10;
            if (e.cryptid == 0) return 1;  // already decrypted
            s->cryptoff = e.cryptoff;
            s->cryptsize = e.cryptsize;
            s->cryptid = e.cryptid;
            s->cryptoff_in_file = (uint32_t)(base) + e.cryptoff;  // absolute
            s->cryptid_off_in_file = (uint32_t)(pos + 16);
            return 0;
        }
        if (lc.cmdsize < sizeof(struct load_command)) return -11;
        pos += lc.cmdsize;
    }
    return 1;  // no LC_ENCRYPTION_INFO_64
}

int dh_decrypt_into(const char *src_path, const char *dst_path, char *msg, size_t msglen) {
    int fd = open(src_path, O_RDONLY);
    if (fd < 0) { snprintf(msg, msglen, "open src: %s", strerror(errno)); return -1; }
    slice_t s = {0};
    int pr = parse(fd, &s);
    if (pr == 1) { snprintf(msg, msglen, "not encrypted"); close(fd); return 0; }
    if (pr < 0) { snprintf(msg, msglen, "parse error %d", pr); close(fd); return -1; }

    uint64_t pg = (uint64_t)getpagesize();
    if (s.cryptoff % pg || s.cryptsize % pg || s.cryptsize == 0) {
        snprintf(msg, msglen, "unaligned cryptoff/size"); close(fd); return -1;
    }

    size_t map_len = (size_t)s.cryptoff + s.cryptsize;
    uint8_t *base = mmap(NULL, map_len, PROT_READ, MAP_PRIVATE, fd, (off_t)s.slice_off);
    if (base == MAP_FAILED) { snprintf(msg, msglen, "mmap: %s", strerror(errno)); close(fd); return -1; }

    errno = 0;
    int mr = mremap_encrypted(base + s.cryptoff, s.cryptsize, 2, s.cputype, s.cpusubtype);
    if (mr != 0) {
        snprintf(msg, msglen, "mremap_encrypted: %s", strerror(errno));
        munmap(base, map_len); close(fd); return -1;
    }

    uint8_t *dec = malloc(s.cryptsize);
    uint64_t ok = 0, total = s.cryptsize / pg;
    for (uint64_t off = 0; off < s.cryptsize; off += pg) {
        vm_size_t n = (off + pg <= s.cryptsize) ? (vm_size_t)pg : (vm_size_t)(s.cryptsize - off), got = 0;
        if (vm_read_overwrite(mach_task_self(), (vm_address_t)(base + s.cryptoff + off), n,
                              (vm_address_t)(dec + off), &got) == KERN_SUCCESS && got == n) ok++;
    }
    double ent = region_entropy(dec, s.cryptsize);
    munmap(base, map_len); close(fd);

    if (ok != total) {
        snprintf(msg, msglen, "page-in failed %llu/%llu", (unsigned long long)ok, (unsigned long long)total);
        free(dec); return -1;
    }

    // Write the decrypted region into the destination copy and clear cryptid.
    int out = open(dst_path, O_RDWR);
    if (out < 0) { snprintf(msg, msglen, "open dst: %s", strerror(errno)); free(dec); return -1; }
    int rc = 0;
    if (pwrite(out, dec, s.cryptsize, (off_t)s.cryptoff_in_file) != (ssize_t)s.cryptsize) rc = -1;
    uint32_t zero = 0;
    if (pwrite(out, &zero, 4, (off_t)s.cryptid_off_in_file) != 4) rc = -1;
    close(out);
    free(dec);
    if (rc != 0) { snprintf(msg, msglen, "write dst failed"); return -1; }

    snprintf(msg, msglen, "decrypted %llu pages, entropy %.2f",
             (unsigned long long)total, ent);
    return 1;
}
