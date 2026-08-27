#include <metal_stdlib>
using namespace metal;
kernel void fp(device float* a [[buffer(0)]], device float* b [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    float x = a[i], y = b[i];
    a[i] = sqrt(x) + fma(x, y, 1.0f) + x / y - fmod(x, y);
}
