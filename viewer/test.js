// node viewer/test.js — checks the DOM-free half of index.html.
const assert = require("node:assert");
const html = require("node:fs").readFileSync(__dirname + "/index.html", "utf8");
const pure = html.match(/<script id="pure">([\s\S]*?)<\/script>/)[1];
const module_ = {};
new Function("module", pure)(module_);
const { MODES, tally, place, cloud, byteMap, human } = module_.exports;

// trigraph: 1 2 3 1 2 3 holds the triple 1,2,3 twice
let t = tally(Uint8Array.of(1, 2, 3, 1, 2, 3), "tri", 1e6);
assert.deepStrictEqual([...t.counts], [[0x010203, 2], [0x020301, 1], [0x030102, 1]]);
assert.strictEqual(t.first.get(0x010203), 0);
assert.deepStrictEqual(place("tri", 0x010203, 0), [1 / 255, 2 / 255, 3 / 255]);

// digraph + height: the pair sits at (first, second) and rises with its count
t = tally(Uint8Array.of(9, 7, 9, 7, 5), "height", 1e6);
assert.deepStrictEqual([...t.counts], [[0x0907, 2], [0x0709, 1], [0x0705, 1]]);
assert.deepStrictEqual(place("height", 0x0907, 0.5), [9 / 255, 0.5, 7 / 255]);

// layers: the first pair is in the bottom slice, the last pair in the top one
const ramp = Uint8Array.from({ length: 1025 }, (_, i) => i);
t = tally(ramp, "layers", 1e6);
const slices = [...t.counts.keys()].map(k => k >> 16);
assert.strictEqual(slices[0], 0);
assert.strictEqual(slices.at(-1), 255);
assert.strictEqual(place("layers", 255 << 16 | 0x0102, 0)[1], 1);

// big inputs are sampled, tiny ones give nothing instead of reading past the end
assert.strictEqual(tally(new Uint8Array(1000), "tri", 100).stride, 10);
assert.strictEqual(tally(Uint8Array.of(1, 2), "tri", 100).counts.size, 0);

// cloud: rare keys can be hidden, too many are thinned, buffers match
t = tally(Uint8Array.of(9, 7, 9, 7, 5), "height", 1e6);
let c = cloud(t, "height", "count", 2, 10);
assert.strictEqual(c.unique, 1);
assert.strictEqual(c.max, 2);
assert.deepStrictEqual([...c.positions].map(v => +v.toFixed(4)), [+(9 / 255 - 0.5).toFixed(4), 0.5, +(7 / 255 - 0.5).toFixed(4)]);
c = cloud(tally(ramp, "tri", 1e6), "tri", "where", 1, 50);
assert.strictEqual(c.unique, 256);  // the ramp wraps at 256
assert.strictEqual(c.positions.length, 150);
assert.ok(c.colours.every(v => v >= 0 && v <= 1));

// thumbnail: 4 bytes on a 2×2 canvas, opaque grey
assert.deepStrictEqual([...byteMap(Uint8Array.of(0, 255, 128, 7), 2)], [0, 0, 0, 255, 255, 255, 255, 255, 128, 128, 128, 255, 7, 7, 7, 255]);
assert.strictEqual(byteMap(new Uint8Array(0), 2)[3], 255);

assert.deepStrictEqual([human(12), human(2048), human(5 * 1024 * 1024)], ["12 bytes", "2.0 KB", "5.0 MB"]);
assert.ok(Object.values(MODES).every(m => m.axes.length === 3));
console.log("viewer: ok");
