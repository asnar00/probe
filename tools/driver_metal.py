# Run a compute kernel from a .metallib on this machine's GPU.
#   python3 tools/driver_metal.py lib.metallib kernel [n]
# Loads the library by URL, makes a pipeline for `kernel`, dispatches n
# threads over a buffer of n int32s initialised to 10*i, and prints the
# buffer afterwards.
#   python3 tools/driver_metal.py --kernel lib.metallib lib.air.json n [group]
# A probe program's __kernel: buffer 0 is the program's memory (its
# data image, n scratch slabs, then an area of n i64 words the program
# writes), buffer 1 the parameters (the area's offset); prints the area.
#   python3 tools/driver_metal.py --suite lib.metallib mem.bin size area_off area_len
# The suite's driver: the memory buffer is `size` bytes, its start the
# image in mem.bin; the parameters buffer holds the area's offset; one
# thread runs; the area's text (up to its first NUL) is printed.
import sys, ctypes, json, Metal, Foundation

def pipeline(path):
    dev = Metal.MTLCreateSystemDefaultDevice()
    lib, err = dev.newLibraryWithURL_error_(Foundation.NSURL.fileURLWithPath_(path), None)
    if lib is None:
        print("library error:", err, file=sys.stderr); sys.exit(1)
    pso, err = dev.newComputePipelineStateWithFunction_error_(lib.newFunctionWithName_("__kernel"), None)
    if pso is None:
        print("pipeline error:", err, file=sys.stderr); sys.exit(1)
    return dev, pso

def run(dev, pso, mem, params, n, group=None):
    # n threads, in groups of `group` (the program's choice, when it
    # uses its group) or whatever the device likes
    q = dev.newCommandQueue(); cb = q.commandBuffer(); enc = cb.computeCommandEncoder()
    enc.setComputePipelineState_(pso); enc.setBuffer_offset_atIndex_(mem, 0, 0); enc.setBuffer_offset_atIndex_(params, 0, 1)
    enc.dispatchThreads_threadsPerThreadgroup_(Metal.MTLSizeMake(n, 1, 1), Metal.MTLSizeMake(group or min(n, pso.maxTotalThreadsPerThreadgroup()), 1, 1))
    enc.endEncoding(); cb.commit(); cb.waitUntilCompleted()
    if cb.error() is not None:
        print("command buffer error:", cb.error(), file=sys.stderr); sys.exit(1)

#   python3 tools/driver_metal.py --batch lib.metallib mem.bin size area_off area_len n
# As --suite, but n threads, and the area's bytes go to stdout raw.
if sys.argv[1] == "--batch":
    path, image, size, area_off, area_len, n = sys.argv[2], open(sys.argv[3], "rb").read(), int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6]), int(sys.argv[7])
    dev, pso = pipeline(path)
    mem = dev.newBufferWithLength_options_(size, 0)
    view = memoryview(mem.contents().as_buffer(size))
    view[len(image):] = bytes(size - len(image))  # a new buffer is not promised to be zero
    view[:len(image)] = image
    params = dev.newBufferWithLength_options_(8, 0)
    memoryview(params.contents().as_buffer(8))[:] = area_off.to_bytes(8, "little")
    run(dev, pso, mem, params, n)
    sys.stdout.buffer.write(bytes(memoryview(mem.contents().as_buffer(size))[area_off:area_off + area_len]))
    sys.exit(0)
if sys.argv[1] == "--suite":
    path, image, size, area_off, area_len = sys.argv[2], open(sys.argv[3], "rb").read(), int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6])
    dev, pso = pipeline(path)
    mem = dev.newBufferWithLength_options_(size, 0)
    view = memoryview(mem.contents().as_buffer(size))
    view[len(image):] = bytes(size - len(image))  # a new buffer is not promised to be zero
    view[:len(image)] = image
    params = dev.newBufferWithLength_options_(8, 0)
    memoryview(params.contents().as_buffer(8))[:] = area_off.to_bytes(8, "little")
    run(dev, pso, mem, params, 1)
    text = bytes(memoryview(mem.contents().as_buffer(size))[area_off:area_off + area_len])
    sys.stdout.write(text.split(b"\0", 1)[0].decode("latin-1"))
    sys.exit(0)
if sys.argv[1] == "--kernel":
    # the area of n i64 words follows the data and the slabs in memory
    path, layout, n = sys.argv[2], json.load(open(sys.argv[3])), int(sys.argv[4])
    group = int(sys.argv[5]) if len(sys.argv) > 5 else None
    dev, pso = pipeline(path)
    data = bytes.fromhex(layout["data"])
    data_size = (len(data) + 15) & ~15
    area_off = (data_size + n * layout["slab"] + 15) & ~15
    mem_len = area_off + 8 * n
    mem = dev.newBufferWithLength_options_(mem_len, 0)
    view = memoryview(mem.contents().as_buffer(mem_len))
    view[len(data):] = bytes(mem_len - len(data))
    view[:len(data)] = data
    params = dev.newBufferWithLength_options_(8, 0)
    memoryview(params.contents().as_buffer(8))[:] = area_off.to_bytes(8, "little")
    run(dev, pso, mem, params, n, group)
    out = (ctypes.c_int64 * n).from_buffer(mem.contents().as_buffer(mem_len), area_off)
    print("area:", list(out))
    sys.exit(0)
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
