#include <metal_stdlib>
using namespace metal;
kernel void ids(device uint* out [[buffer(0)]], uint gid [[thread_position_in_grid]], uint lid [[thread_position_in_threadgroup]], uint tg [[threadgroup_position_in_grid]], uint n [[threads_per_grid]], uint sid [[thread_index_in_simdgroup]]) {
    out[gid] = gid + lid * 1000 + tg * 100000 + n * 10000000 + sid;
}
