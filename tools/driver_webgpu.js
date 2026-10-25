#!/usr/bin/env node
// A probe program's kernel, printed as WGSL (src/emit_wgsl.rs), compiled by
// WebGPU in a headless Chrome: the only WGSL validator on the machine.
// No package: Chrome's own debugging protocol over node's WebSocket.
//
//   node tools/driver_webgpu.js --check x.wgsl
//     compile only: what Chrome's WGSL compiler says, nothing run.
//   node tools/driver_webgpu.js --adapter
//     what the adapter says of itself and of subgroups.
//
// Chrome runs on a debugging port the system chooses (read back from the
// profile's DevToolsActivePort) with a profile of its own made under
// $TMPDIR for this run, so it cannot meet any other Chrome on the machine;
// it is killed when the answer is in and on any error, and the profile
// goes with it.
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const LIMIT_MS = 60000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function usage() {
  process.stderr.write("usage: driver_webgpu.js --check x.wgsl | --adapter\n");
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
  const dev = await a.requestDevice({ requiredFeatures: features });
  const mod = dev.createShaderModule({ code: job.wgsl });
  const info = await mod.getCompilationInfo();
  out.messages = info.messages.map((m) => `wgsl:${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`);
  if (info.messages.some((m) => m.type === "error")) return { ...out, error: "WGSL does not compile" };
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
  } else usage();
  job.subgroups = /^\s*enable\s+subgroups\s*;/m.test(job.wgsl || "");

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
  } catch (e) {
    process.stderr.write("driver_webgpu: " + e.message + "\n");
    process.exitCode = 1;
  } finally {
    clearTimeout(timer);
    await finish();
  }
}

main();
