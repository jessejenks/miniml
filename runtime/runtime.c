#include <stdio.h>
#include <stdlib.h>

static unsigned long long allocations;
static unsigned long long allocated_bytes;

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
}

void miniml_init(void) {
    const char *stats = getenv("MINIML_STATS");
    if (stats != NULL && stats[0] != '\0' && stats[0] != '0') {
        atexit(print_stats);
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
