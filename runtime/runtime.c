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
#define TUPLE_TAG 0
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

static void clear_object_start(uint64_t *p) {
    size_t i = p - heap_base;
    uint8_t bit = (uint8_t)(1 << (i % 8));
    size_t idx = i / 8;
    object_starts[idx] &= ~bit;
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
    uint64_t *new_free_list = NULL;
    uint64_t *tail = NULL;
    uint64_t *p = heap_base + 1;
    uint64_t run_length = 0;
    uint64_t *run_start = p;
    while (p - 1 < heap_top) {
        uint8_t is_marked = header_mark(p[-1]);
        uint8_t is_free = header_tag(p[-1]) == FREE_TAG;
        uint64_t size = header_size(p[-1]);
        if (is_free || !is_marked) {
            clear_object_start(p);
            if (run_length == 0) run_start = p;
            run_length += size + 1;
        } else {
            if (run_length > 0) {
                run_start[-1] = ((run_length - 1) << 10) | FREE_TAG;
                run_start[0] = 0;
                if (tail != NULL) tail[0] = (uint64_t)run_start;
                else new_free_list = run_start;
                tail = run_start;
                run_length = 0;
            }
            p[-1] &= ~(uint64_t)MARK_BIT;
        }
        p += size + 1;
    }
    if (run_length > 0) {
        run_start[-1] = ((run_length - 1) << 10) | FREE_TAG;
        run_start[0] = 0;
        if (tail != NULL) tail[0] = (uint64_t)run_start;
        else new_free_list = run_start;
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
    // Find first free slot with `words + 1` many words available
    if (free_list != NULL) {
        uint64_t *curr = free_list;
        uint64_t *prev = NULL;
        uint64_t size;
        while (curr != NULL) {
            size = header_size(curr[-1]);
            if (size == words) {
                // replace free region and update free list
                if (prev == NULL) free_list = (uint64_t *)curr[0];
                else prev[0] = curr[0];
                return curr - 1;
            }
            if (size > words + 1) {
                // split region
                // +1 so that we have room for 1 field and new header
                // update size
                curr[-1] = ((size - words - 1) << 10) | FREE_TAG;
                // take from tail so that we can keep free list pointer accurate
                return curr + (size - words - 1);
            }
            prev = curr;
            curr = (uint64_t *)curr[0];
        }
    }
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

int64_t miniml_equal(uint64_t left, uint64_t right) {
    if (left == right) return 1;
    if ((left & 1) || (right & 1)) return 0;
    uint64_t *p = (uint64_t *)left;
    uint64_t *q = (uint64_t *)right;
    if (header_tag(p[-1]) != header_tag(q[-1])) return 0;
    uint64_t size = header_size(p[-1]);
    if (size != header_size(q[-1])) return 0;
    if (header_tag(p[-1]) == STRING_TAG) {
        return memcmp(p, q, size * sizeof(uint64_t)) == 0;
    }
    if (header_tag(p[-1]) < CLOSURE_TAG) {
        for (uint64_t i = 0; i < size; i++) {
            if (!miniml_equal(p[i], q[i])) {
                return 0;
            }
        }
        return 1;
    }
    return 0;
}

static uint64_t string_length(uint64_t *s) {
    uint64_t bytes = header_size(s[-1]) * sizeof(uint64_t);
    return bytes - 1 - ((uint8_t *)s)[bytes - 1];
}

uint64_t miniml_len(uint64_t w) {
    return (string_length((uint64_t *)w) << 1) | 1; // box
}

static void pad_string(char *dst, uint64_t len, uint64_t words) {
    uint64_t k = words * 8 - 1 - len;
    memset(dst + len, 0, k);
    dst[words * 8 - 1] = (char)k;
}

uint64_t miniml_string_concat(uint64_t left, uint64_t right) {
    uint64_t len_l = string_length((uint64_t *)left);
    uint64_t len_r = string_length((uint64_t *)right);
    uint64_t len = len_l + len_r;
    uint64_t words = len / 8 + 1;
    void *p = miniml_alloc(words, STRING_TAG);
    char *dst = (char *)p;
    memcpy(dst, (char *)left, len_l);
    memcpy(dst + len_l, (char *)right, len_r);
    pad_string(dst, len, words);
    return (uint64_t)p;
}

static uint64_t make_string(const char *bytes, uint64_t len) {
    uint64_t words = len / 8 + 1;
    char *dst = miniml_alloc(words, STRING_TAG);
    memcpy(dst, bytes, len);
    pad_string(dst, len, words);
    return (uint64_t)dst;
}

uint64_t miniml_unit_to_str() {
    // make global?
    return make_string("()", 2);
}

uint64_t miniml_bool_to_str(uint64_t w) {
    // make globals?
    if (w >> 1) return make_string("true", 4);
    else return make_string("false", 5);
}

static int encode_char(uint64_t c, char *buf) {
    if (c < 0x80) {
        buf[0] = (char)c;
        return 1;
    }
    if (c < 0x800) {
        // 110xxxxx 10xxxxxx
        buf[0] = (char)(0xC0 | (c >> 6));
        buf[1] = (char)(0x80 | (c & 0x3F));
        return 2;
    }
    if (c < 0x10000) {
        // 1110xxxx 10xxxxxx 10xxxxxx
        buf[0] = (char)(0xE0 | (c >> 12));
        buf[1] = (char)(0x80 | ((c >> 6) & 0x3F));
        buf[2] = (char)(0x80 | (c & 0x3F));
        return 3;
    }
    // 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
    buf[0] = (char)(0xF0 | (c >> 18));
    buf[1] = (char)(0x80 | ((c >> 12) & 0x3F));
    buf[2] = (char)(0x80 | ((c >> 6) & 0x3F));
    buf[3] = (char)(0x80 | (c & 0x3F));
    return 4;
}

uint64_t miniml_char_to_str(uint64_t w) {
    char buf[4];
    int len = encode_char(w >> 1, buf);
    return make_string(buf, len);
}

// matches `escape_str` in lexer.rs
static void print_escaped(const char *bytes, uint64_t len, char quote) {
    for (uint64_t i = 0; i < len; i++) {
        char b = bytes[i];
        switch (b) {
        case '\n': fputs("\\n", stdout); break;
        case '\r': fputs("\\r", stdout); break;
        case '\t': fputs("\\t", stdout); break;
        case '\0': fputs("\\0", stdout); break;
        case '\\': fputs("\\\\", stdout); break;
        default:
            if (b == quote) putchar('\\');
            putchar(b);
        }
    }
}

void miniml_print_char(uint64_t c) {
    char buf[4];
    int len = encode_char(c, buf);
    print_escaped(buf, len, '\'');
}

void miniml_print_string(uint64_t s) {
    fwrite((char *)s, 1, string_length((uint64_t *)s), stdout);
}

void miniml_print_string_escaped(uint64_t s) {
    print_escaped((char *)s, string_length((uint64_t *)s), '"');
}

uint64_t miniml_int_to_str(uint64_t w) {
    char buf[32];
    int len = snprintf(buf, sizeof(buf), "%lld", (long long)((int64_t)w >> 1));
    return make_string(buf, len);
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
