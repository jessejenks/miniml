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
#define CELL_SIZE 16
#define DEFAULT_INITIAL_HEAP (1 << 20)
#define POISON 0xD15EA5EDDEADBEEFull

static size_t initial_heap = DEFAULT_INITIAL_HEAP;
static int gc_stress;

uint64_t *miniml_args;
static uint64_t *args_base;
static uint64_t *stack_base;

static char *heap_base;
static char *heap_top;
static char *heap_capacity;
static char *heap_end;
static uint64_t *free_list;

static uint8_t *mark_bits; // bitset of marked cells
static unsigned long long marked_cell_count;
static struct { uint64_t **items; size_t len, cap; } mark_stack;

#ifdef _MSC_VER
#define NOINLINE __declspec(noinline)
#else
#define NOINLINE __attribute__((noinline))
#endif

static void print_stats(void) {
    fprintf(stderr, "allocations: %llu\nallocated bytes: %llu\n", allocations, allocated_bytes);
    fprintf(stderr, "peak heap: %llu\n", (unsigned long long)(heap_top - heap_base));
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

    mark_bits = calloc(HEAP_SIZE / CELL_SIZE / 8, 1);
    if (mark_bits == NULL) {
        fputs("miniml: could not allocate mark bits\n", stderr);
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
    free(mark_bits);
    mark_bits = NULL;
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

static int is_cell(uint64_t w) {
    return w != 0 && (w & 15) == 0
        && w >= (uint64_t)heap_base && w < (uint64_t)heap_top;
}

static uint64_t *containing_cell(uint64_t w) {
    if (w < (uint64_t)heap_base || w >= (uint64_t)heap_top) return NULL;
    return (uint64_t *)(w & ~(uint64_t)15);
}

static void push_mark_stack(uint64_t *c) {
    if (mark_stack.len == mark_stack.cap) {
        mark_stack.cap = mark_stack.cap == 0 ? 1024 : 2 * mark_stack.cap;
        mark_stack.items = realloc(mark_stack.items, mark_stack.cap * sizeof(uint64_t *));
        if (mark_stack.items == NULL) {
            fputs("miniml: could not grow mark stack\n", stderr);
            exit(1);
        }
    }
    mark_stack.items[mark_stack.len++] = c;
}

static uint64_t *pop_mark_stack(void) {
    return mark_stack.items[--mark_stack.len];
}

static void mark_cell(uint64_t *c) {
    size_t i = ((char *)c - heap_base) / CELL_SIZE;
    uint8_t bit = (uint8_t)(1 << (i % 8));
    size_t idx = i / 8;
    if (mark_bits[idx] & bit) return;
    mark_bits[idx] |= bit;
    marked_cell_count++;
    push_mark_stack(c);
}

static void sweep(void) {
    uint64_t *new_free_list = NULL;
    size_t cells = (heap_top - heap_base) / CELL_SIZE;
    // we sweep from highest address so that final free list starts at the lowest address
    for (size_t i = cells; i-- > 0;) {
        if (mark_bits[i / 8] & (1 << (i % 8))) continue;
        uint64_t *c = (uint64_t *)(heap_base + i * CELL_SIZE);
        // replace freed with [ ptr | 1 ][ poison ]
        // poison value is chosen to be odd so is_cell returns false
        // addresses are 16 byte aligned, so last 4 bits of c[0] are always 0001. See take_cell
        c[0] = (uint64_t)new_free_list | 1;
        c[1] = POISON;
        new_free_list = c;
    }
    free_list = new_free_list;
}

static NOINLINE void collect(void) {
    clock_t start = clock();

    // union to force 8-byte alignment
    union { jmp_buf buf; uint64_t align; } regs;
    // setjmp copies callee saved registers into `regs.buf`.
    // may want __builtin_unwind_init? https://maskray.me/blog/stack-unwinding
    // because -02 can change order of things depending on platform/version
    setjmp(regs.buf);

    marked_cell_count = 0;
    for (uint64_t *w = (uint64_t *)&regs; w < stack_base; w++) {
        uint64_t *c = containing_cell(*w);
        if (c) mark_cell(c);
    }
    for (uint64_t *w = args_base; w < miniml_args; w++) {
        if (is_cell(*w)) mark_cell((uint64_t *)*w);
    }
    while (mark_stack.len > 0) {
        uint64_t *c = pop_mark_stack();
        if (is_cell(c[0])) mark_cell((uint64_t *)c[0]);
        if (is_cell(c[1])) mark_cell((uint64_t *)c[1]);
    }

    live_bytes = marked_cell_count * CELL_SIZE;
    if (live_bytes > max_live_bytes) max_live_bytes = live_bytes;
    sweep();
    // num bits to set = ceil(heap size in bytes / 8)
    memset(mark_bits, 0, ((heap_top - heap_base) / CELL_SIZE + 7) / 8);
    collections++;
    gc_seconds += (double)(clock() - start) / CLOCKS_PER_SEC;
}

static void *take_cell(void) {
    if (free_list != NULL) {
        // pop from free list, if non-empty
        uint64_t *c = free_list;
        free_list = (uint64_t *)(c[0] & ~(uint64_t)1);
        return c;
    }
    if (heap_top + CELL_SIZE <= heap_capacity) {
        // otherwise take from existing heap capacity
        void *p = heap_top;
        heap_top += CELL_SIZE;
        return p;
    }
    return NULL;
}

void *miniml_alloc(size_t size) {
    allocations++;
    allocated_bytes += size;

    if (size != CELL_SIZE) {
        fputs("miniml: unsupported object size\n", stderr);
        exit(1);
    }

    if (gc_stress) collect();

    void *p = take_cell();
    if (p == NULL) {
        collect();
        p = take_cell();
    }
    if (p == NULL) {
        if (grow_heap() == -1) {
            fputs("miniml: out of memory\n", stderr);
            exit(1);
        }
        p = take_cell();
    }
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
