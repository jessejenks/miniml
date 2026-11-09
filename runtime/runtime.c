#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

static unsigned long long allocations;
static unsigned long long allocated_bytes;

uint64_t *miniml_args;

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
}

void miniml_init(void) {
    const char *stats = getenv("MINIML_STATS");
    if (stats != NULL && stats[0] != '\0' && stats[0] != '0') {
        atexit(print_stats);
    }
    miniml_args = malloc((1 << 20) * sizeof(uint64_t));
    if (miniml_args == NULL) {
        fputs("miniml: could not allocation argument stack\n", stderr);
        exit(1);
    }
}

void *miniml_alloc(size_t size) {
    allocations++;
    allocated_bytes += size;
    void *p = malloc(size);
    if (p == NULL) {
        fputs("miniml: out of memory\n", stderr);
        exit(1);
    }
    return p;
}
