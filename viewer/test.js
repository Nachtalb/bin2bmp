// node viewer/test.js — checks the DOM-free half of index.html.
const assert = require("node:assert");
const html = require("node:fs").readFileSync(__dirname + "/index.html", "utf8");
const pure = html.match(/<script id="pure">([\s\S]*?)<\/script>/)[1];
const module_ = {};
new Function("module", pure)(module_);
const { MODES, MAPS, tables, count, select, place, thin, exportFactor, snapAngle, byteMap, human } = module_.exports;

const T = tables();
const whole = { start: 0, length: 100, step: 1, phase: 0 };
const counted = () => [...T.touched.subarray(0, T.nTouched)].map(k => [k, T.cnt[k]]);

// trigraph: 1 2 3 1 2 3 holds the triple 1,2,3 twice
let r = count(Uint8Array.of(1, 2, 3, 1, 2, 3), { ...whole, mode: "Trigraph" }, T, 1e6);
assert.deepStrictEqual(counted(), [[0x010203, 2], [0x020301, 1], [0x030102, 1]]);
assert.deepStrictEqual([r.lo, r.hi, r.n, r.sampled], [0, 4, 4, false]);
assert.deepStrictEqual(place("Trigraph", 0x010203, 0), [1 / 255, 3 / 255, 2 / 255]);

// digraph: the table is reused, so the trigraph counts above must be gone
count(Uint8Array.of(9, 7, 9, 7, 5), { ...whole, mode: "Digraph" }, T, 1e6);
assert.deepStrictEqual(counted(), [[0x0907, 2], [0x0709, 1], [0x0705, 1]]);
assert.strictEqual(T.cnt[0x010203], 0);
assert.deepStrictEqual(place("Digraph", 0x0907, 0.5), [9 / 255, 0.5, 7 / 255]);

// filter and thinning
assert.deepStrictEqual(select(T, 2, 0, 10), { keep: [0x0907], unique: 1, max: 2 });
assert.deepStrictEqual(select(T, 1, 1, 10).keep, [0x0709, 0x0705]);
const thinned = select(T, 1, 0, 2);
assert.deepStrictEqual([thinned.keep.length, thinned.unique], [2, 3]);

// step and phase: every 2nd byte from offset 1 sees only the 7→9 and 7→5 pairs
count(Uint8Array.of(9, 7, 9, 7, 5), { ...whole, mode: "Digraph", step: 2, phase: 1 }, T, 1e6);
assert.deepStrictEqual(counted(), [[0x0709, 1], [0x0705, 1]]);

// window: the second half of the file only
r = count(Uint8Array.from({ length: 100 }, (_, i) => i), { ...whole, mode: "Digraph", start: 50, length: 50 }, T, 1e6);
assert.deepStrictEqual([r.lo, r.hi, T.first[50 << 8 | 51]], [50, 99, 50]);

// layers: the first pair is in the bottom slice, the last pair in the top one
const ramp = Uint8Array.from({ length: 1025 }, (_, i) => i);
count(ramp, { ...whole, mode: "Digraph over file" }, T, 1e6);
const slices = counted().map(([k]) => k >> 16);
assert.deepStrictEqual([slices[0], slices.at(-1)], [0, 255]);
assert.deepStrictEqual(place("Digraph over file", 255 << 16 | 0x0102, 0), [1 / 255, 1, 2 / 255]);

// over the cap the positions are sampled, and the same ones every time
r = count(ramp, { ...whole, mode: "Trigraph" }, T, 100);
assert.deepStrictEqual([r.n, r.sampled], [100, true]);
const once = counted();
count(ramp, { ...whole, mode: "Trigraph" }, T, 100);
assert.deepStrictEqual(counted(), once);

// too short for a single pair or triple
assert.strictEqual(count(Uint8Array.of(1, 2), { ...whole, mode: "Trigraph" }, T, 100).n, 0);
assert.strictEqual(count(new Uint8Array(0), { ...whole, mode: "Digraph" }, T, 100).n, 0);

// thinning for the 2D view: small inputs pass through untouched, big ones keep every stride-th byte
const small = Uint8Array.of(1, 2, 3);
assert.strictEqual(thin(small, 3).data, small);
assert.deepStrictEqual(thin(Uint8Array.of(0, 1, 2, 3, 4, 5, 6), 3), { stride: 3, data: Uint8Array.of(0, 3, 6) });
assert.strictEqual(thin(small, Infinity).stride, 1);
// the wasm module numbers the modes in this order
assert.deepStrictEqual(MAPS.map(m => m.name), ["bytes", "class", "bit", "entropy", "chi", "digraph"]);

// export scale: as asked if it fits, smaller if not, never below 1
assert.deepStrictEqual([exportFactor(1280, 800, 2, 16384), exportFactor(1280, 800, 4, 4096), exportFactor(5000, 100, 3, 4096)], [2, 3, 1]);

// view snapping: nearest step, unless an extra stop is closer
assert.deepStrictEqual([snapAngle(50, 45), snapAngle(-70, 45), snapAngle(100, 90), snapAngle(30, 45, [35.26, -35.26]), snapAngle(10, 45, [35.26])], [45, -90, 90, 35.26, 0]);

// thumbnail: 4 bytes on a 2×2 canvas, opaque grey
assert.deepStrictEqual([...byteMap(Uint8Array.of(0, 255, 128, 7), 2)], [0, 0, 0, 255, 255, 255, 255, 255, 128, 128, 128, 255, 7, 7, 7, 255]);
assert.strictEqual(byteMap(new Uint8Array(0), 2)[3], 255);

assert.deepStrictEqual([human(12), human(2048), human(5 * 1024 * 1024)], ["12 B", "2.0 KB", "5.0 MB"]);
assert.strictEqual(MODES.length, 3);
console.log("viewer: ok");
