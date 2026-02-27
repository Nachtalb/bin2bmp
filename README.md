# bin2bmp

Visualize any binary file as a grayscale bitmap. Each byte becomes one pixel — value 0x00 is black, 0xFF is white. The canvas is as square as possible.

## Usage

```sh
bin2bmp [INPUT] [OUTPUT]
```

- `INPUT`  — path to a binary file, or `-` for stdin (default: stdin)
- `OUTPUT` — path for the `.bmp` output, or `-` for stdout (default: stdout)

**Examples:**

```sh
bin2bmp firmware.bin firmware.bmp
bin2bmp /usr/bin/ls ls.bmp
cat binary | bin2bmp - output.bmp
bin2bmp input.bin | magick - output.png   # pipe to ImageMagick for PNG
```

## Output format

8-bit indexed grayscale BMP with a 256-entry palette. One byte per pixel — keeps the output roughly the same size as the input and avoids the 4 GB BMP header overflow that 24-bit would hit on large files.

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

## Release profile

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

No dependencies — pure Rust, no crates.
