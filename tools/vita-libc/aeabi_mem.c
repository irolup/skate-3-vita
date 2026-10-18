/* skate_vita (M3): strong __aeabi_mem* definitions for the Vita.
 *
 * Rust's compiler_builtins provides these as weak symbols implemented as one-word
 * loops; on the Cortex-A9 that made every compiler-emitted zero-fill and struct copy
 * cost ~0.5 cycles per byte (1 KB zeroed: 1.25 us, copied: 1.7 us; M3 measurements).
 * The plain memcpy/memset/memmove symbols already go to the firmware's SceLibc
 * routines; these route the AEABI entry points there too. Same bytes, same results:
 * only the copy loop changes.
 */
#include <stddef.h>
#include <psp2/kernel/clib.h>

#ifdef AEABI_COUNT
/* Diagnostic build only (sim-bench feature mem-count): calls and bytes per entry
 * kind [copy, move, set] and per size class [<64, <256, <1024, >=1024]. */
unsigned long long skate_vita_aeabi_calls[3][4], skate_vita_aeabi_bytes[3][4];
/* Call sites by return address (open addressing; diagnostic). */
#define SITES 1024
struct site { unsigned addr; unsigned kind; unsigned long long calls, bytes; };
struct site skate_vita_aeabi_sites[SITES];
static inline void count_at(int kind, size_t n, unsigned addr) {
    int c = n < 64 ? 0 : n < 256 ? 1 : n < 1024 ? 2 : 3;
    skate_vita_aeabi_calls[kind][c]++;
    skate_vita_aeabi_bytes[kind][c] += n;
    unsigned h = (addr >> 1) * 2654435761u % SITES;
    for (int i = 0; i < SITES; i++, h = (h + 1) % SITES) {
        struct site *e = &skate_vita_aeabi_sites[h];
        if (e->addr == addr || e->addr == 0) {
            e->addr = addr; e->kind = kind; e->calls++; e->bytes += n;
            return;
        }
    }
}
#define count(kind, n) count_at(kind, n, (unsigned)__builtin_return_address(0))
#else
#define count(kind, n) ((void)0)
#endif

void __aeabi_memcpy(void *dst, const void *src, size_t n) { count(0, n); sceClibMemcpy(dst, src, n); }
void __aeabi_memcpy4(void *dst, const void *src, size_t n) { count(0, n); sceClibMemcpy(dst, src, n); }
void __aeabi_memcpy8(void *dst, const void *src, size_t n) { count(0, n); sceClibMemcpy(dst, src, n); }

void __aeabi_memmove(void *dst, const void *src, size_t n) { count(1, n); sceClibMemmove(dst, src, n); }
void __aeabi_memmove4(void *dst, const void *src, size_t n) { count(1, n); sceClibMemmove(dst, src, n); }
void __aeabi_memmove8(void *dst, const void *src, size_t n) { count(1, n); sceClibMemmove(dst, src, n); }

/* AEABI order: (dest, n, c), unlike memset(dest, c, n). */
void __aeabi_memset(void *dst, size_t n, int c) { count(2, n); sceClibMemset(dst, c, n); }
void __aeabi_memset4(void *dst, size_t n, int c) { count(2, n); sceClibMemset(dst, c, n); }
void __aeabi_memset8(void *dst, size_t n, int c) { count(2, n); sceClibMemset(dst, c, n); }

void __aeabi_memclr(void *dst, size_t n) { count(2, n); sceClibMemset(dst, 0, n); }
void __aeabi_memclr4(void *dst, size_t n) { count(2, n); sceClibMemset(dst, 0, n); }
void __aeabi_memclr8(void *dst, size_t n) { count(2, n); sceClibMemset(dst, 0, n); }
