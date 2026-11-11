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

#ifdef MINIML_TEST
static size_t guard_size;
#endif

#define ARG_STACK_SIZE ((1 << 20) * sizeof(uint64_t))
#define HEAP_SIZE (1 << 30)
#define CELL_SIZE 16
#define DEFAULT_INITIAL_HEAP (1 << 20)

static size_t initial_heap = DEFAULT_INITIAL_HEAP;

uint64_t *miniml_args;
static uint64_t *args_base;
static uint64_t *stack_base;

static char *heap_base;
static char *heap_top;
static char *heap_capacity;
static char *heap_end;

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
    fprintf(stderr, "peak heap: %llu\n", (unsigned long long)(heap_top - heap_base));
}

static char *reserve_arg_stack(void) {
#ifdef _WIN32
    SYSTEM_INFO info;
    GetSystemInfo(&info);
    size_t page_size = info.dwPageSize;
    char *start = VirtualAlloc(NULL, ARG_STACK_SIZE + 2 * page_size, MEM_RESERVE|MEM_COMMIT, PAGE_READWRITE);
    if (start == NULL) return NULL;
    DWORD old;
    if (!VirtualProtect(start, page_size, PAGE_NOACCESS, &old) ||
        !VirtualProtect(start + page_size + ARG_STACK_SIZE, page_size, PAGE_NOACCESS, &old)) return NULL;
#else
    size_t page_size = sysconf(_SC_PAGESIZE);
    char* start = (char*)mmap(NULL, ARG_STACK_SIZE + 2 * page_size, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (start == MAP_FAILED) return NULL;
    // | PROT_NONE | PROT_READ|PROT_WRITE | PROT_NONE |
    // | page      | arg stack            | page      |
    if (mprotect(start, page_size, PROT_NONE) == -1) return NULL;
    if (mprotect(start + page_size + ARG_STACK_SIZE, page_size, PROT_NONE) == -1) return NULL;
#endif
#ifdef MINIML_TEST
    guard_size = page_size;
#endif
    return start + page_size;
}

static int reserve_heap(void) {
#ifdef _WIN32
    // have to reserve full, and separate commit used part
    // mmap will lazily do this for us.
    char *start = VirtualAlloc(NULL, HEAP_SIZE, MEM_RESERVE, PAGE_READWRITE);
    if (start == NULL) return -1;
    if (VirtualAlloc(start, initial_heap, MEM_COMMIT, PAGE_READWRITE) == NULL) return -1;
#else
    char* start = (char*)mmap(NULL, HEAP_SIZE, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE, -1, 0);
    if (start == MAP_FAILED) return -1;
#endif
    heap_base = start;
    heap_top = start;
    heap_capacity = start + initial_heap;
    heap_end = start + HEAP_SIZE;
    return 0;
}

static int grow_heap(void) {
    // try to grow the heap until we hit HEAP_SIZE
    if (heap_capacity == heap_end) return -1;
    size_t grow_size = heap_capacity - heap_base;
    if (grow_size > (size_t)(heap_end - heap_capacity)) grow_size = heap_end - heap_capacity;
#ifdef _WIN32
    if (VirtualAlloc(heap_capacity, grow_size, MEM_COMMIT, PAGE_READWRITE) == NULL) return -1;
#endif
    heap_capacity += grow_size;
    return 0;
}


void miniml_init(void) {
    const char *initial = getenv("MINIML_INITIAL_HEAP");
    if (initial != NULL) {
        char *end;
        unsigned long long n = strtoull(initial, &end, 10);
        if (end == initial || *end != '\0' || n == 0 || n > HEAP_SIZE) {
            fputs("miniml: MINIML_INITIAL_HEAP must be a byte count between 1 and the heap size\n", stderr);
            exit(1);
        }
        initial_heap = n;
    }

    miniml_args = (uint64_t*)reserve_arg_stack();
    if (miniml_args == NULL) {
        fputs("miniml: could not allocate argument stack\n", stderr);
        exit(1);
    }
    args_base = miniml_args;

    if (reserve_heap() == -1) {
        fputs("miniml: could not allocate heap\n", stderr);
        exit(1);
    }

    const char *stats = getenv("MINIML_STATS");
    if (stats != NULL && stats[0] != '\0' && stats[0] != '0') {
        atexit(print_stats);
    }
}

#ifdef MINIML_TEST
void miniml_reset(void) {
    char *start = (char*)args_base - guard_size;
#ifdef _WIN32
    VirtualFree(start, 0, MEM_RELEASE);
#else
    munmap(start, ARG_STACK_SIZE + 2 * guard_size);
#endif
#ifdef _WIN32
    VirtualFree(heap_base, 0, MEM_RELEASE);
#else
    munmap(heap_base, HEAP_SIZE);
#endif
    miniml_args = NULL;
    args_base = NULL;
    stack_base = NULL;
    heap_base = NULL;
    heap_top = NULL;
    heap_capacity = NULL;
    heap_end = NULL;
    allocations = 0;
    allocated_bytes = 0;
    initial_heap = DEFAULT_INITIAL_HEAP;
}

void miniml_set_initial_heap(size_t bytes) {
    initial_heap = bytes;
}
#endif

void *miniml_alloc(size_t size) {
    allocations++;
    allocated_bytes += size;

    if (size != CELL_SIZE) {
        fputs("miniml: unsupported object size\n", stderr);
        exit(1);
    }

    if (heap_top + size > heap_capacity) {
        if (grow_heap() == -1) {
            fputs("miniml: out of memory\n", stderr);
            exit(1);
        }
    }

    void *p = heap_top;
    heap_top += size;
    return p;
}

int64_t miniml_run(int64_t (*entry)(void)) {
    volatile uint64_t marker = 0;
    stack_base = (uint64_t *)&marker;
    miniml_init();
    int64_t result = entry();
    return result;
}

#ifndef MINIML_TEST
int64_t miniml_main(void);

int main(void) {
    return (int)miniml_run(miniml_main);
}
#endif
