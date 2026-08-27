# Run a compute kernel from a .metallib on this machine's GPU:
#   python3 tools/driver_metal.py lib.metallib kernel [n]
# Loads the library by URL, makes a pipeline for `kernel`, dispatches n
# threads over a buffer of n int32s initialised to 10*i, and prints the
# buffer afterwards. The harness `probe test air` builds on this.
import sys, ctypes, Metal, Foundation
path, fname = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 8
dev = Metal.MTLCreateSystemDefaultDevice()
lib, err = dev.newLibraryWithURL_error_(Foundation.NSURL.fileURLWithPath_(path), None)
if lib is None:
    print("library error:", err); sys.exit(1)
pso, err = dev.newComputePipelineStateWithFunction_error_(lib.newFunctionWithName_(fname), None)
if pso is None:
    print("pipeline error:", err); sys.exit(1)
buf = dev.newBufferWithLength_options_(4 * n, 0)
arr = (ctypes.c_int32 * n).from_buffer(buf.contents().as_buffer(4 * n))
for i in range(n):
    arr[i] = 10 * i
q = dev.newCommandQueue(); cb = q.commandBuffer(); enc = cb.computeCommandEncoder()
enc.setComputePipelineState_(pso); enc.setBuffer_offset_atIndex_(buf, 0, 0)
enc.dispatchThreads_threadsPerThreadgroup_(Metal.MTLSizeMake(n, 1, 1), Metal.MTLSizeMake(pso.maxTotalThreadsPerThreadgroup(), 1, 1))
enc.endEncoding(); cb.commit(); cb.waitUntilCompleted()
print("result:", list(arr))
