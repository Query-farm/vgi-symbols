/* Second fixture (distinct build-id) for LRU-eviction tests. */
__attribute__((noinline)) int compute(int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) acc += i * 3;
    return acc;
}
int main(int argc, char **argv) { (void)argv; return compute(argc); }
