/* Fixture for vgi-symbols DWARF inline-expansion tests.
   Two always_inline callees folded into apply() so one machine address fans
   out to a multi-frame inline chain. */
#include <stdint.h>

__attribute__((noinline)) int sink(int x);

__attribute__((always_inline)) static inline int inner_lo(int x) {
    return sink(x * 2) + 1;        /* line for inner_lo */
}

__attribute__((always_inline)) static inline int inner_hi(int x) {
    return inner_lo(x) ^ 7;        /* inner_lo inlined into inner_hi */
}

__attribute__((noinline)) int apply(int x) {
    return inner_hi(x) + inner_hi(x + 1);   /* inner_hi inlined into apply */
}

int sink(int x) { return x ^ 0x55; }

int main(int argc, char **argv) {
    (void)argv;
    return apply(argc);
}
