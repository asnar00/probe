#include <metal_stdlib>
using namespace metal;
kernel void shared_sum(device int* out [[buffer(0)]], uint lid [[thread_position_in_threadgroup]]) {
    threadgroup int tmp[64];
    tmp[lid] = lid;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    out[lid] = tmp[63 - lid];
}
