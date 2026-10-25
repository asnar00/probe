#!/usr/bin/env node
// Run a probe program's kernel, printed as WGSL (src/emit_wgsl.rs), on this
// machine's GPU through WebGPU in a headless Chrome: the second GPU path
// beside tools/driver_metal.py, and the only WGSL validator on the machine.
// No package: Chrome's own debugging protocol over node's WebSocket.
//
//   node tools/driver_webgpu.js --kernel x.wgsl x.wgsl.json n [g] [n g ...]
//     buffer 0 is the program's memory, laid out as driver_metal.py --kernel
//     lays it out: the data image, n slabs, then an area of n i64 words the
//     program writes; the uniform's first words are the area's offset. n
//     threads in workgroups of g (by default the largest divisor of n up to
//     256: WebGPU dispatches whole workgroups). One line `area: [...]` per
//     (n, g), in the Metal driver's form, all of them in one Chrome.
//     With PROBE_TIME=k in the environment, each area is followed by
//     `time: <ms> ms`, the dispatch from its submit to the area read back,
//     the least of k dispatches of the same kernel on the same memory.
//   node tools/driver_webgpu.js --check x.wgsl
//     compile only: what Chrome's WGSL compiler says, nothing run.
//   node tools/driver_webgpu.js --adapter
//     what the adapter says of itself and of subgroups.
//
// Chrome runs on a debugging port the system chooses (read back from the
// profile's DevToolsActivePort) with a profile of its own made under
// $TMPDIR for this run, so it cannot meet any other Chrome on the machine;
// it is killed when the cases are done and on any error, and the profile
// goes with it.
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const LIMIT_MS = 60000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function usage() {
  process.stderr.write("usage: driver_webgpu.js --kernel x.wgsl x.wgsl.json n [g] [n g ...] | --check x.wgsl | --adapter\n");
  process.exit(2);
}

// what runs in the page: an adapter and a device, the module compiled and
// its messages read, each case dispatched and its area read back
async function inPage(job) {
  const out = {};
  const a = await navigator.gpu?.requestAdapter();
  if (!a) return { error: "no WebGPU adapter" };
  out.adapter = { vendor: a.info.vendor, architecture: a.info.architecture, subgroups: a.features.has("subgroups"), subgroupMinSize: a.info.subgroupMinSize, subgroupMaxSize: a.info.subgroupMaxSize };
  if (job.adapter) return out;
  const features = [];
  if (job.subgroups) {
    if (!a.features.has("subgroups")) return { ...out, error: "the adapter has no subgroups, and the kernel uses subgroup operations" };
    features.push("subgroups");
  }
  let need = 1 << 16, groups = 1;
  for (const [n, g] of job.cases) {
    need = Math.max(need, ((job.dataLen + 15) & ~15) + n * job.slab + 16 + 8 * n);
    groups = Math.max(groups, g);
  }
  const limits = { maxStorageBufferBindingSize: Math.min(need, a.limits.maxStorageBufferBindingSize), maxBufferSize: Math.min(need, a.limits.maxBufferSize), maxComputeInvocationsPerWorkgroup: Math.min(Math.max(groups, 256), a.limits.maxComputeInvocationsPerWorkgroup), maxComputeWorkgroupSizeX: Math.min(Math.max(groups, 256), a.limits.maxComputeWorkgroupSizeX) };
  const dev = await a.requestDevice({ requiredFeatures: features, requiredLimits: limits });
  let lost = null;
  dev.lost.then((i) => (lost = i.message));
  const mod = dev.createShaderModule({ code: job.wgsl });
  const info = await mod.getCompilationInfo();
  out.messages = info.messages.map((m) => `wgsl:${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`);
  if (info.messages.some((m) => m.type === "error")) return { ...out, error: "WGSL does not compile" };
  if (job.check) return out;
  const bin = atob(job.data);
  const data = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i);
  out.areas = [];
  for (const [n, g] of job.cases) {
    const dataSize = (data.length + 15) & ~15;
    const areaOff = (dataSize + n * job.slab + 15) & ~15;
    const memLen = areaOff + 8 * n;
    dev.pushErrorScope("validation");
    dev.pushErrorScope("out-of-memory");
    // a WebGPU buffer starts as zeros
    const mem = dev.createBuffer({ size: memLen, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC, mappedAtCreation: true });
    new Uint8Array(mem.getMappedRange()).set(data);
    mem.unmap();
    const params = dev.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM, mappedAtCreation: true });
    new Uint32Array(params.getMappedRange()).set([areaOff, 0, 0, 0]);
    params.unmap();
    const read = dev.createBuffer({ size: 8 * n, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
    let pipe;
    try {
      pipe = await dev.createComputePipelineAsync({ layout: "auto", compute: { module: mod, entryPoint: "main", constants: { group_size: g } } });
    } catch (e) {
      return { ...out, error: "pipeline: " + e.message };
    }
    const bind = dev.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: mem } }, { binding: 1, resource: { buffer: params } }] });
    const enc = dev.createCommandEncoder();
    const pass = enc.beginComputePass();
    pass.setPipeline(pipe);
    pass.setBindGroup(0, bind);
    pass.dispatchWorkgroups(n / g);
    pass.end();
    enc.copyBufferToBuffer(mem, areaOff, read, 0, 8 * n);
    const t0 = performance.now();
    dev.queue.submit([enc.finish()]);
    const oom = await dev.popErrorScope();
    const bad = await dev.popErrorScope();
    if (bad) return { ...out, error: "validation: " + bad.message };
    if (oom) return { ...out, error: "out of memory: " + oom.message };
    try {
      await read.mapAsync(GPUMapMode.READ);
    } catch (e) {
      return { ...out, error: "reading the area back: " + (lost || e.message) };
    }
    let least = performance.now() - t0;
    const words = new BigInt64Array(read.getMappedRange().slice(0));
    read.unmap();
    // (timed, the same dispatch again, job.reps in all, and the least
    // kept: the first carries the GPU waking and Chrome's scheduling)
    for (let r = 1; r < job.reps; r++) {
      const e2 = dev.createCommandEncoder();
      const p2 = e2.beginComputePass();
      p2.setPipeline(pipe);
      p2.setBindGroup(0, bind);
      p2.dispatchWorkgroups(n / g);
      p2.end();
      e2.copyBufferToBuffer(mem, areaOff, read, 0, 8 * n);
      const t1 = performance.now();
      dev.queue.submit([e2.finish()]);
      await read.mapAsync(GPUMapMode.READ);
      least = Math.min(least, performance.now() - t1);
      read.unmap();
    }
    (out.times ||= []).push(least);
    mem.destroy();
    params.destroy();
    read.destroy();
    out.areas.push([...words].map(String));
  }
  return out;
}

async function main() {
  const args = process.argv.slice(2);
  const job = { cases: [] };
  if (args[0] === "--adapter") {
    job.adapter = true;
  } else if (args[0] === "--check" && args.length === 2) {
    job.check = true;
    job.wgsl = fs.readFileSync(args[1], "utf8");
  } else if (args[0] === "--kernel" && args.length >= 4) {
    job.wgsl = fs.readFileSync(args[1], "utf8");
    const layout = JSON.parse(fs.readFileSync(args[2], "utf8"));
    const data = Buffer.from(layout.data, "hex");
    job.data = data.toString("base64");
    job.dataLen = data.length;
    job.slab = layout.slab;
    const nums = args.slice(3).map(Number);
    if (nums.some((x) => !Number.isInteger(x) || x <= 0)) usage();
    if (nums.length === 1) {
      const n = nums[0];
      let g = Math.min(n, 256);
      while (n % g) g--;
      job.cases.push([n, g]);
    } else {
      if (nums.length % 2) usage();
      for (let i = 0; i < nums.length; i += 2) {
        if (nums[i] % nums[i + 1]) throw new Error(`${nums[i + 1]} does not divide ${nums[i]}: WebGPU dispatches whole workgroups`);
        job.cases.push([nums[i], nums[i + 1]]);
      }
    }
  } else usage();
  job.subgroups = /^\s*enable\s+subgroups\s*;/m.test(job.wgsl || "");
  job.reps = Math.max(1, parseInt(process.env.PROBE_TIME || "1", 10) || 1);

  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "probe-wgsl-"));
  const page = path.join(profile, "page.html");
  // a file: page is a secure context, which WebGPU wants; about:blank is not
  fs.writeFileSync(page, "<!doctype html><title>probe wgsl</title>");
  const chrome = spawn(CHROME, ["--headless=new", "--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "--user-data-dir=" + profile, "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--hide-scrollbars", "file://" + page], { stdio: "ignore" });
  let gone = false;
  const exited = new Promise((r) => chrome.once("exit", () => r((gone = true))));
  const finish = async () => {
    if (!gone) {
      chrome.kill("SIGTERM");
      await Promise.race([exited, sleep(2000)]);
      if (!gone) {
        chrome.kill("SIGKILL");
        await exited;
      }
    }
    if (path.dirname(profile) === path.resolve(os.tmpdir()) && path.basename(profile).startsWith("probe-wgsl-")) {
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    }
  };
  for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => finish().then(() => process.exit(1)));
  const timer = setTimeout(() => {
    process.stderr.write(`driver_webgpu: no answer in ${LIMIT_MS / 1000} s\n`);
    finish().then(() => process.exit(1));
  }, LIMIT_MS);
  try {
    let port;
    for (let i = 0; i < 200 && !port; i++) {
      await sleep(25);
      try {
        port = fs.readFileSync(path.join(profile, "DevToolsActivePort"), "utf8").split("\n")[0].trim();
      } catch (_) {}
      if (gone) throw new Error("Chrome exited before its debugging port was up");
    }
    if (!port) throw new Error("Chrome did not say its debugging port");
    let target;
    for (let i = 0; i < 200 && !target; i++) {
      try {
        target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === "page" && t.url.startsWith("file:"));
      } catch (_) {}
      if (!target) await sleep(25);
    }
    if (!target) throw new Error("Chrome's page did not come up");
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      ws.onopen = res;
      ws.onerror = () => rej(new Error("the debugging socket did not open"));
    });
    let next = 1;
    const waiting = new Map();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && waiting.has(msg.id)) {
        const w = waiting.get(msg.id);
        waiting.delete(msg.id);
        msg.error ? w.rej(new Error(msg.error.message)) : w.res(msg.result);
      }
    };
    const send = (method, params = {}) =>
      new Promise((res, rej) => {
        const id = next++;
        waiting.set(id, { res, rej });
        ws.send(JSON.stringify({ id, method, params }));
      });
    const r = await send("Runtime.evaluate", { expression: `(${inPage.toString()})(${JSON.stringify(job)})`, awaitPromise: true, returnByValue: true });
    ws.close();
    if (r.exceptionDetails) throw new Error("in the page: " + (r.exceptionDetails.exception?.description || r.exceptionDetails.text));
    const out = r.result.value;
    if (job.adapter) {
      const a = out.adapter;
      console.log(a ? `adapter: ${a.vendor} ${a.architecture}, subgroups ${a.subgroups ? `yes, ${a.subgroupMinSize} to ${a.subgroupMaxSize} threads` : "no"}` : out.error);
    }
    for (const m of out.messages || []) process.stderr.write(m + "\n");
    if (out.error) throw new Error(out.error);
    if (job.check) console.log("compiles");
    // PROBE_TIME=k: after each area the dispatch's milliseconds, from the
    // submit to the area mapped back, the least of k (`probe zero <store> kernel`)
    (out.areas || []).forEach((words, i) => {
      console.log(`area: [${words.join(", ")}]`);
      if (process.env.PROBE_TIME) console.log(`time: ${out.times[i].toFixed(3)} ms`);
    });
  } catch (e) {
    process.stderr.write("driver_webgpu: " + e.message + "\n");
    process.exitCode = 1;
  } finally {
    clearTimeout(timer);
    await finish();
  }
}

main();
