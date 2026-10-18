// Puts the VFP unit in full IEEE mode on every thread of a Vita process.
//
// The Vita starts every thread with FPSCR = 0x03000000: flush-to-zero (FZ, bit 24) and
// default-NaN (DN, bit 25). Desktop x86_64 keeps subnormals and NaN payloads, and
// skate-core relies on that (it flushes explicitly where the Xbox VMX code did). New
// threads do not inherit FPSCR from their creator, so this clears FZ and DN:
// - on the main thread, from a constructor that runs before main;
// - on every thread started through pthread_create (all Rust std threads), by
//   wrapping it. Link with -Wl,--wrap=pthread_create.
// Rounding mode and the other FPSCR fields are left as the system set them.

#include <pthread.h>
#include <stdlib.h>

#define FPSCR_FZ (1u << 24)
#define FPSCR_DN (1u << 25)

void vita_fpu_set_ieee(void) {
	unsigned int fpscr;
	__asm__ volatile("vmrs %0, fpscr" : "=r"(fpscr));
	fpscr &= ~(FPSCR_FZ | FPSCR_DN);
	__asm__ volatile("vmsr fpscr, %0" : : "r"(fpscr));
}

__attribute__((constructor)) static void vita_fpu_init_main_thread(void) {
	vita_fpu_set_ieee();
}

struct start {
	void *(*routine)(void *);
	void *arg;
};

static void *trampoline(void *p) {
	struct start s = *(struct start *)p;
	free(p);
	vita_fpu_set_ieee();
	return s.routine(s.arg);
}

int __real_pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                          void *(*routine)(void *), void *arg);

int __wrap_pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                          void *(*routine)(void *), void *arg) {
	struct start *s = malloc(sizeof *s);
	if (!s)
		return __real_pthread_create(thread, attr, routine, arg);
	s->routine = routine;
	s->arg = arg;
	int ret = __real_pthread_create(thread, attr, trampoline, s);
	if (ret != 0)
		free(s);
	return ret;
}
