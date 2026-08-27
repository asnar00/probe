#include <metal_stdlib>
using namespace metal;
kernel void add1(device int* buf [[buffer(0)]], uint tid [[thread_position_in_grid]]) {
    buf[tid] = buf[tid] + 1;
}
