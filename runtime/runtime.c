#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/mman.h>

static unsigned long long allocations;
static unsigned long long allocated_bytes;

uint64_t *miniml_args;

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
}

void miniml_init(void) {
    size_t page_size = sysconf(_SC_PAGESIZE);
    size_t stack_size = (1 << 20) * sizeof(uint64_t);
    char* start = (char*)mmap(NULL, stack_size + 2 * page_size, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (start == MAP_FAILED) {
        fputs("miniml: could not allocate argument stack\n", stderr);
        exit(1);
    }
    // | PROT_NONE | PROT_READ|PROT_WRITE | PROT_NONE |
    // | page      | arg stack            | page      |
    if (mprotect(start, page_size, PROT_NONE) == -1) {
        fputs("miniml: could not allocate argument stack\n", stderr);
        exit(1);
    }
    if (mprotect(start + page_size + stack_size, page_size, PROT_NONE) == -1) {
        fputs("miniml: could not allocate argument stack\n", stderr);
        exit(1);
    }
    miniml_args = (uint64_t*)(start + page_size);

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
