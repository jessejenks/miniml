#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <setjmp.h>
#include <time.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <unistd.h>
#include <sys/mman.h>
#endif

static unsigned long long allocations;
static unsigned long long allocated_bytes;
static unsigned long long collections;
static unsigned long long live_bytes;
static unsigned long long max_live_bytes;
static double gc_seconds;

#ifdef MINIML_TEST
static size_t guard_size;
#endif

#define ARG_STACK_SIZE ((1 << 20) * sizeof(uint64_t))
#define HEAP_SIZE (1 << 30)
#define DEFAULT_INITIAL_HEAP (1 << 20)

// header constants
#define MARK_BIT 0x100
#define CLOSURE_TAG 247
#define ENV_TAG 248
#define NO_SCAN_TAG 251
#define STRING_TAG 252
#define FREE_TAG 255

static size_t initial_heap = DEFAULT_INITIAL_HEAP;
static int gc_stress;

uint64_t *miniml_args;
static uint64_t *args_base;
static uint64_t *stack_base;

static uint64_t *heap_base;
static uint64_t *heap_top;
static uint64_t *heap_capacity;
static uint64_t *heap_end;
static uint64_t *free_list;

static uint8_t *object_starts; // bitset of where heap objects begin

static struct { uint64_t **items; size_t len, cap; } mark_stack;

#ifdef _MSC_VER
#define NOINLINE __declspec(noinline)
#else
#define NOINLINE __attribute__((noinline))
#endif

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
    fprintf(stderr, "peak heap: %llu\n", (unsigned long long)((heap_top - heap_base) * sizeof(uint64_t)));
    fprintf(stderr, "collections: %llu\n", collections);
    fprintf(stderr, "max live bytes: %llu\nfinal live bytes: %llu\n", max_live_bytes, live_bytes);
    fprintf(stderr, "gc seconds: %.3f\n", gc_seconds);
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
    heap_base = (uint64_t *)start;
    heap_top = heap_base;
    // round up to next word
    heap_capacity = (uint64_t *)(start + ((initial_heap + 7) & ~(size_t)7));
    heap_end = (uint64_t *)(start + HEAP_SIZE);
    object_starts = calloc(HEAP_SIZE / 64, 1);
    if (object_starts == NULL) {
        return -1;
    }
    return 0;
}

static int grow_heap(void) {
    // try to grow the heap until we hit HEAP_SIZE
    if (heap_capacity == heap_end) return -1;
    size_t grow_size = heap_capacity - heap_base;
    if (grow_size > (size_t)(heap_end - heap_capacity)) grow_size = heap_end - heap_capacity;
#ifdef _WIN32
    if (VirtualAlloc(heap_capacity, grow_size * sizeof(uint64_t), MEM_COMMIT, PAGE_READWRITE) == NULL) return -1;
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

    const char *stress = getenv("MINIML_GC_STRESS");
    if (stress != NULL) {
        gc_stress = stress[0] != '\0' && stress[0] != '0';
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
    free_list = NULL;
    free(object_starts);
    object_starts = NULL;
    free(mark_stack.items);
    mark_stack.items = NULL;
    mark_stack.len = 0;
    mark_stack.cap = 0;
    allocations = 0;
    allocated_bytes = 0;
    collections = 0;
    live_bytes = 0;
    max_live_bytes = 0;
    gc_seconds = 0;
    initial_heap = DEFAULT_INITIAL_HEAP;
    gc_stress = 0;
}

void miniml_set_initial_heap(size_t bytes) {
    initial_heap = bytes;
}

void miniml_set_gc_stress(int on) {
    gc_stress = on;
}
#endif

static uint64_t header_size(uint64_t header) {
    return header >> 10;
}

static uint8_t header_tag(uint64_t header) {
    return header & 255;
}

static uint8_t header_mark(uint64_t header) {
    return (header & MARK_BIT) >> 8;
}

static uint8_t is_object_start(uint64_t *p) {
    size_t i = p - heap_base;
    uint8_t bit = (uint8_t)(1 << (i % 8));
    size_t idx = i / 8;
    return object_starts[idx] & bit;
}

static void record_object_start(uint64_t *p) {
    size_t i = p - heap_base;
    uint8_t bit = (uint8_t)(1 << (i % 8));
    size_t idx = i / 8;
    object_starts[idx] |= bit;
}

static int is_object(uint64_t w) {
    return (w & 7) == 0
        && w >= (uint64_t)heap_base && w < (uint64_t)heap_top
        && is_object_start((uint64_t *)w);
}

static uint64_t *containing_object(uint64_t w) {
    if (w < (uint64_t)heap_base || w >= (uint64_t)heap_top) return NULL;
    // nearest possible start address
    uint64_t *start = (uint64_t *)(w & ~(uint64_t)7);
    // find actual nearest object start
    while (start > heap_base && !is_object_start(start)) start--;
    if (start == heap_base) return NULL;
    if ((uint64_t *)w >= start + header_size(start[-1])) return NULL;
    return start;
}

static void push_mark_stack(uint64_t *p) {
    if (mark_stack.len == mark_stack.cap) {
        mark_stack.cap = mark_stack.cap == 0 ? 1024 : 2 * mark_stack.cap;
        mark_stack.items = realloc(mark_stack.items, mark_stack.cap * sizeof(uint64_t *));
        if (mark_stack.items == NULL) {
            fputs("miniml: could not grow mark stack\n", stderr);
            exit(1);
        }
    }
    mark_stack.items[mark_stack.len++] = p;
}

static uint64_t *pop_mark_stack(void) {
    return mark_stack.items[--mark_stack.len];
}

static void mark(uint64_t *p) {
    if (header_mark(p[-1])) return;
    p[-1] |= MARK_BIT;
    live_bytes += (header_size(p[-1]) + 1) * sizeof(uint64_t);
    push_mark_stack(p);
}

static void sweep(void) {
    uint64_t *p;
    for (p = heap_base + 1; p - 1 < heap_top; p += header_size(p[-1]) + 1) {
        p[-1] &= ~(uint64_t)MARK_BIT;
    }
    if (p - 1 != heap_top) abort();
}

static NOINLINE void collect(void) {
    clock_t start = clock();

    // union to force 8-byte alignment
    union { jmp_buf buf; uint64_t align; } regs;
    // setjmp copies callee saved registers into `regs.buf`.
    // may want __builtin_unwind_init? https://maskray.me/blog/stack-unwinding
    // because -02 can change order of things depending on platform/version
    setjmp(regs.buf);

    live_bytes = 0;
    for (uint64_t *w = (uint64_t *)&regs; w < stack_base; w++) {
        uint64_t *p = containing_object(*w);
        if (p) mark(p);
    }
    for (uint64_t *w = args_base; w < miniml_args; w++) {
        if (is_object(*w)) mark((uint64_t *)*w);
    }
    while (mark_stack.len > 0) {
        uint64_t *p = pop_mark_stack();
        if (header_tag(p[-1]) < NO_SCAN_TAG) {
            for (uint64_t i = 0; i < header_size(p[-1]); i++) {
                if (is_object(p[i])) mark((uint64_t *)p[i]);
            }
        }
    }

    if (live_bytes > max_live_bytes) max_live_bytes = live_bytes;
    sweep();
    // if we've used more than half of remaining capacity, double heap size.
    // note: an error from grow_heap is ignored and caught later in miniml_alloc
    if (live_bytes > (unsigned long long)(heap_capacity - heap_base) * sizeof(uint64_t) / 2) grow_heap();
    collections++;
    gc_seconds += (double)(clock() - start) / CLOCKS_PER_SEC;
}

static uint64_t *take_cell(uint64_t words) {
    // TODO
    // Find first free slot with `words + 1` many words available
    if (heap_top + 1 + words <= heap_capacity) {
        uint64_t *p = heap_top;
        heap_top += 1 + words;
        return p;
    }
    return NULL;
}

void *miniml_alloc(uint64_t words, uint64_t tag) {
    allocations++;
    allocated_bytes += (1 + words) * sizeof(uint64_t);

    uint64_t header = words << 10 | (tag & 255);

    if (gc_stress) collect();

    uint64_t *p = take_cell(words);
    if (p == NULL) {
        collect();
        p = take_cell(words);
    }
    if (p == NULL) {
        if (grow_heap() == -1) {
            fputs("miniml: out of memory\n", stderr);
            exit(1);
        }
        p = take_cell(words);
    }
    p[0] = header;
    // record start from first field
    record_object_start(p + 1);
    return p + 1;
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
