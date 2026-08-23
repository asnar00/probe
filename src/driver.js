// Node runner for compiled wasm modules. Invoked by `probe test`:
//   node driver.js module.wasm '{"cases":[...]}'
// Each case: {"func": name, "args": [...], "rets": ["i64"|"i32"...]}
// Arg forms: {"t":"i64","v":"123"}  {"t":"i32","v":"..."}
//            {"t":"ptr","a64":["1","2"]}  {"t":"ptr","a32":[...]}
// Array args are copied into the module's linear memory; the offset is
// passed as the pointer. Results print one line per case, comma-separated;
// i64 prints signed, i32/i1/ptr print zero-extended (matching the suite's
// convention from the native backend).
const fs = require("fs");
const bytes = fs.readFileSync(process.argv[2]);
const spec = JSON.parse(process.argv[3]);
const mod = new WebAssembly.Module(bytes);
for (const c of spec.cases) {
  const inst = new WebAssembly.Instance(mod);
  const mem = inst.exports.memory;
  let bump = 8;
  const args = c.args.map((a) => {
    if (a.t === "i64") return BigInt(a.v);
    if (a.t === "i32") return Number(BigInt.asIntN(32, BigInt(a.v)));
    if (a.a64) {
      bump = (bump + 7) & ~7;
      const off = bump;
      const arr = new BigInt64Array(mem.buffer, off, a.a64.length);
      a.a64.forEach((x, i) => (arr[i] = BigInt.asIntN(64, BigInt(x))));
      bump += a.a64.length * 8;
      return off;
    }
    if (a.a32) {
      bump = (bump + 3) & ~3;
      const off = bump;
      const arr = new Int32Array(mem.buffer, off, a.a32.length);
      a.a32.forEach((x, i) => (arr[i] = Number(BigInt.asIntN(32, BigInt(x)))));
      bump += a.a32.length * 4;
      return off;
    }
    throw new Error("bad arg spec");
  });
  try {
    let r = inst.exports[c.func](...args);
    if (r === undefined) r = [];
    else if (!Array.isArray(r)) r = [r];
    const out = r.map((v, i) =>
      c.rets[i] === "i64"
        ? BigInt.asIntN(64, v).toString()
        : (Number(v) >>> 0).toString()
    );
    console.log(out.join(", "));
  } catch (e) {
    console.log("trap: " + e.message);
  }
}
