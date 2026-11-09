#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#include <sys/mman.h>
#endif

static unsigned long long allocations;
static unsigned long long allocated_bytes;

uint64_t *miniml_args;

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
}

static char *reserve_guarded(size_t stack_size) {
#ifdef _WIN32
    SYSTEM_INFO info;
    GetSystemInfo(&info);
    size_t page_size = info.dwPageSize;
    char *start = VirtualAlloc(NULL, stack_size + 2 * page_size, MEM_RESERVE|MEM_COMMIT, PAGE_READWRITE);
    if (start == NULL) return NULL;
    DWORD old;
    if (!VirtualProtect(start, page_size, PAGE_NOACCESS, &old) ||
        !VirtualProtect(start + page_size + stack_size, page_size, PAGE_NOACCESS, &old)) return NULL;
#else
    size_t page_size = sysconf(_SC_PAGESIZE);
    char* start = (char*)mmap(NULL, stack_size + 2 * page_size, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (start == MAP_FAILED) return NULL;
    // | PROT_NONE | PROT_READ|PROT_WRITE | PROT_NONE |
    // | page      | arg stack            | page      |
    if (mprotect(start, page_size, PROT_NONE) == -1) return NULL;
    if (mprotect(start + page_size + stack_size, page_size, PROT_NONE) == -1) return NULL;
#endif
    return start + page_size;
}

void miniml_init(void) {
    miniml_args = (uint64_t*)reserve_guarded((1 << 20) * sizeof(uint64_t));
    if (miniml_args == NULL) {
        fputs("miniml: could not allocate argument stack\n", stderr);
        exit(1);
    }

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
