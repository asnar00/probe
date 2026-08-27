#include <metal_stdlib>
using namespace metal;
int helper(int v) { return v * 3 + 1; }
kernel void ints(device long* a [[buffer(0)]], device half* h [[buffer(1)]], device uchar* c [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    long v = a[i];
    long s = 0;
    for (int k = 0; k < (int)(v & 7); k++) s += helper(k);
    a[i] = v * 12345678901L + s / 7 + (v % 5);
    h[i] = h[i] * half(2.0);
    c[i] = c[i] + 200;
}
