# bin2bmp

Visualize any binary file as a bitmap, so its structure shows: code, text, tables, padding, compressed blobs. The canvas is as square as possible.

## Usage

```sh
bin2bmp [OPTIONS] [INPUT] [OUTPUT]
```

- `INPUT`  — path to a binary file, or `-` for stdin (default: stdin)
- `OUTPUT` — path for the `.bmp` output, or `-` for stdout (default: stdout)

- `-m, --mode MODE` — what a pixel shows (default: `bytes`)
- `-b, --block N` — bytes per pixel in `entropy` mode (default: 256)

| Mode | Pixel | Shows |
|------|-------|-------|
| `bytes` | one per byte, value = brightness (0x00 black, 0xFF white) | the texture of the data |
| `class` | one per byte: 0x00 black, 0xFF white, printable ASCII blue, everything else red | where text, padding and binary data sit |
| `entropy` | one per block, brightness = Shannon entropy of the block | where the compressed or encrypted blobs are: they come out white, padding and repetition dark |
| `digraph` | always 256×256: pixel (x, y) = how often byte x is followed by byte y, log scale | what kind of data it is; text, machine code and images each draw a recognisable shape |

**Examples:**

```sh
bin2bmp firmware.bin firmware.bmp
bin2bmp -m class /usr/bin/ls ls.bmp
bin2bmp -m entropy -b 4096 disk.img disk.bmp   # 4096× smaller than the input
cat binary | bin2bmp - output.bmp
bin2bmp input.bin | magick - output.png   # pipe to ImageMagick for PNG
```

A regular file is streamed in constant memory. Stdin and other inputs of unknown length are buffered first, because the BMP header needs the size.

## Library

The CLI is a thin wrapper around the `bin2bmp` crate:

```toml
[dependencies]
bin2bmp = { git = "https://github.com/Nachtalb/bin2bmp" }
```

```rust
use bin2bmp::{render, write_bmp, Options};

// In memory: palette indices plus palette, ready for any image encoder or canvas.
let image = render(&data, &Options::default())?;
println!("{}x{}", image.width, image.height);

// Streamed straight to any `Write`, in constant memory.
let file = std::fs::File::open("firmware.bin")?;
let len = file.metadata()?.len();
write_bmp(file, len, std::io::stdout().lock(), &Options::default())?;
```

`pixels()` gives the raw pixel stream chunk by chunk if you want to draw it yourself.

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
