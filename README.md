# bin2bmp

Visualize any binary file as a bitmap, so its structure shows: code, text, tables, padding, compressed blobs. The canvas is as square as possible.

## Usage

```sh
bin2bmp [OPTIONS] [INPUT] [OUTPUT]
```

- `INPUT`  — path to a binary file, or `-` for stdin (default: stdin)
- `OUTPUT` — path for the `.bmp` output, or `-` for stdout (default: stdout)

- `-m, --mode MODE` — what a pixel shows (default: `bytes`)
- `-l, --layout LAYOUT` — where the pixels go (default: `rows`)
- `-b, --block N` — bytes per pixel in `entropy` mode (default: 256) and `chi` mode (default: 4096)
- `--bit N` — which bit `bit` mode shows, 0 = lowest (default: 0)
- `-w, --width N` — image width for the `rows` layout (default: as square as possible); a width matching the data's record size lines columns up

| Mode | Pixel | Shows |
|------|-------|-------|
| `bytes` | one per byte, value = brightness (0x00 black, 0xFF white) | the texture of the data |
| `class` | one per byte: 0x00 black, 0xFF white, printable ASCII blue, everything else red | where text, padding and binary data sit |
| `bit` | one per byte: white if one chosen bit is set | data hidden in the low bits of raw pixels or audio (LSB steganography): the hidden part is even static, the untouched part still shows traces of the picture |
| `entropy` | one per block, brightness = Shannon entropy of the block | where the compressed or encrypted blobs are: they come out white, padding and repetition dark |
| `chi` | one per block: white where the values 2k and 2k+1 occur equally often (chi-square test) | which part of raw pixels or audio has had its lowest bits overwritten; compressed or encrypted data is white everywhere, so only use it on raw data |
| `digraph` | always 256×256: pixel (x, y) = how often byte x is followed by byte y, log scale | what kind of data it is; text, machine code and images each draw a recognisable shape |

| Layout | Placement | Trade-off |
|--------|-----------|-----------|
| `rows` | left to right, top to bottom | offsets are easy to read off, but a region of the file smears into a stripe across several rows; streams in constant memory |
| `hilbert` | along a [Hilbert curve](https://en.wikipedia.org/wiki/Hilbert_curve) on a power-of-two square | bytes close in the file stay close in the image, so each region is one compact patch; the image is held in memory |

**Examples:**

```sh
bin2bmp firmware.bin firmware.bmp
bin2bmp -m class /usr/bin/ls ls.bmp
bin2bmp -m entropy -b 4096 disk.img disk.bmp   # 4096× smaller than the input
bin2bmp -m class -l hilbert firmware.bin map.bmp
cat binary | bin2bmp - output.bmp
bin2bmp input.bin | magick - output.png   # pipe to ImageMagick for PNG
```

With the `rows` layout a regular file is streamed in constant memory. Stdin and other inputs of unknown length are buffered first, because the BMP header needs the size.

## Library

The CLI is a thin wrapper around the `bin2bmp` crate:

```toml
[dependencies]
bin2bmp = { git = "https://github.com/Nachtalb/bin2bmp" }
```

```rust
use bin2bmp::{render, write_bmp, Layout, Mode, Options};

// In memory: palette indices plus palette, ready for any image encoder or canvas.
let opts = Options { mode: Mode::Entropy { block: 256 }, layout: Layout::Hilbert, width: None };
let image = render(&data, &opts)?;
println!("{}x{}", image.width, image.height);

// Straight to any `Write`; the rows layout streams in constant memory.
let file = std::fs::File::open("firmware.bin")?;
let len = file.metadata()?.len();
write_bmp(file, len, std::io::stdout().lock(), &Options::default())?;
```

`pixels()` gives the raw pixel stream chunk by chunk if you want to draw it yourself.

## Viewer

Live at <https://bin2bmp.pages.dev>, redeployed on every push to `master`.

`viewer/index.html` is a standalone page for looking at files in 3D, and in 2D through this library compiled to wasm. Open it in a browser and drop files on it; they stay on your machine. It loads [three.js](https://threejs.org) and [lil-gui](https://lil-gui.georgealways.com) from a CDN, so it needs a connection the first time.

- **Trigraph** — every three neighbouring bytes are one point in a cube. Text is a small clump, raw audio or pixels a rod along the diagonal, compressed or encrypted data fills the cube.
- **Digraph** — the flat digraph with each pair raised by how often it occurs.
- **Digraph over file** — the digraph stacked by position in the file, so the file's sections show up as layers.

The panel narrows the graph to a window of the file, samples every n-th byte, filters by count, and switches colouring between count and file offset. Each opened file is listed with its name, size and a preview: the picture itself for images the browser can decode, a byte map for everything else.

**2D maps** are the CLI's modes (`bytes`, `class`, `bit`, `entropy`, `chi`, `digraph`, rows or Hilbert layout), rendered in the page by the library itself. The page loads the wasm module from its own folder. A lone copy of the page opened from disk fetches it from the hosted site instead, so it needs a connection. To build and serve it yourself:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release -p bin2bmp-wasm --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/bin2bmp_wasm.wasm viewer/bin2bmp.wasm
python3 -m http.server -d viewer        # then open http://localhost:8000
```

The module is the `wasm/` crate: three small C-style exports around `render`, no bindings generator, about 40 KB. The 3D graphs work without it, straight from the file.

**Save PNG** exports the current view: the 3D graph re-rendered at 2× its on-screen resolution (1× to 4×), a 2D map with every pixel enlarged by the same factor.

For exact screenshots, "snap on release" in the Camera folder glides the view to the nearest 15°, 45° or 90° when you let go, and the current angle is shown bottom left.

The panel can be made see-through with "panel opacity", so the graph stays visible behind it while you change things. The page also works on a phone: one finger turns the graph or moves the map, two fingers zoom.

`node viewer/test.js` checks the counting and placement logic.

## Output format

8-bit indexed BMP with a 256-entry palette, rows stored top-down. One byte per pixel keeps the output roughly the same size as the input. BMP cannot hold more than 4 GB, so larger outputs are refused with an error; use `entropy` mode for inputs that big.

## Build

```sh
cargo build --release
```

With CPU-specific optimizations:

```sh
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Static binary (musl):

```sh
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C target-cpu=native" cargo build --release --target x86_64-unknown-linux-musl
```

No dependencies — pure Rust, no crates.
