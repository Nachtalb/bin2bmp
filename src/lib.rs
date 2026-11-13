//! Visualize binary data as an 8-bit indexed image.
//!
//! ```no_run
//! use bin2bmp::{render, write_bmp, Options};
//!
//! // In memory: palette indices plus palette, ready for any image encoder.
//! let image = render(b"some bytes", &Options::default())?;
//!
//! // Streamed: constant memory, input length must be known up front.
//! let file = std::fs::File::open("firmware.bin")?;
//! let len = file.metadata()?.len();
//! write_bmp(file, len, std::io::stdout().lock(), &Options::default())?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io::{self, Read, Write};

pub type Palette = [[u8; 3]; 256];

/// What one pixel means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// One pixel per byte, value = brightness.
    #[default]
    Bytes,
    /// One pixel per byte, coloured by class: 0x00 black, 0xFF white,
    /// printable ASCII blue, everything else red.
    Class,
    /// One pixel per `block` bytes, brightness = Shannon entropy of the block.
    /// White is the most a block of that size can reach, so compressed or
    /// encrypted data is bright and padding or repetition is dark.
    Entropy { block: usize },
    /// Always 256×256: pixel (x, y) is how often byte `x` is followed by
    /// byte `y`, on a log scale. File formats have recognisable shapes.
    Digraph,
}

/// Where pixel number `i` lands in the image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Left to right, top to bottom. Streams in constant memory.
    #[default]
    Rows,
    /// Along a Hilbert curve, so data that is close in the file stays close
    /// in the image and each region becomes a compact patch instead of a
    /// stripe. The whole image is held in memory.
    Hilbert,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub mode: Mode,
    /// Ignored by [`Mode::Digraph`], which has its own fixed layout.
    pub layout: Layout,
    /// Image width in pixels; `None` picks a near-square canvas.
    /// Only used by [`Layout::Rows`].
    pub width: Option<u32>,
}

impl Options {
    fn hilbert(&self) -> bool {
        self.layout == Layout::Hilbert && self.mode != Mode::Digraph
    }
}

/// Row-major palette indices, top row first.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub palette: Palette,
}

impl Mode {
    pub fn palette(self) -> Palette {
        std::array::from_fn(|i| match (self, i as u8) {
            (Mode::Bytes | Mode::Entropy { .. } | Mode::Digraph, v) => [v; 3],
            (Mode::Class, 0x00) => [0; 3],
            (Mode::Class, 0xFF) => [255; 3],
            (Mode::Class, b'\t' | b'\n' | b'\r' | 0x20..=0x7E) => [55, 126, 184],
            (Mode::Class, _) => [228, 26, 28],
        })
    }

    /// Number of pixels produced for `len` input bytes.
    pub fn pixel_count(self, len: u64) -> u64 {
        match self {
            Mode::Bytes | Mode::Class => len,
            Mode::Entropy { block } => len.div_ceil(block.max(1) as u64),
            Mode::Digraph => 65536 * len.min(1),
        }
    }

    fn check(self) -> io::Result<()> {
        match self {
            Mode::Entropy { block: 0 } => Err(invalid("block size must be at least 1")),
            _ => Ok(()),
        }
    }
}

struct Entropy {
    block: usize,
    filled: usize,
    hist: [usize; 256],
    /// `c * log2(c)` for small counts; a log per count would dominate small blocks.
    clog: Vec<f64>,
    out: Vec<u8>,
}

impl Entropy {
    fn new(block: usize) -> Self {
        Self {
            block,
            filled: 0,
            hist: [0; 256],
            clog: (0..=block.min(4096)).map(Self::clog).collect(),
            out: Vec::new(),
        }
    }

    fn clog(count: usize) -> f64 {
        if count == 0 {
            0.0
        } else {
            count as f64 * (count as f64).log2()
        }
    }

    fn feed(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() {
            let n = (self.block - self.filled).min(chunk.len());
            for &b in &chunk[..n] {
                self.hist[b as usize] += 1;
            }
            chunk = &chunk[n..];
            self.filled += n;
            if self.filled == self.block {
                self.emit();
            }
        }
    }

    fn emit(&mut self) {
        if self.filled == 0 {
            return;
        }
        let n = self.filled as f64;
        let sum: f64 = self
            .hist
            .iter()
            .map(|&c| self.clog.get(c).copied().unwrap_or_else(|| Self::clog(c)))
            .sum();
        let bits = n.log2() - sum / n;
        let max_bits = (self.block as f64).log2().min(8.0);
        let level = if max_bits > 0.0 { bits / max_bits } else { 0.0 };
        self.out.push((level * 255.0).round() as u8);
        self.hist = [0; 256];
        self.filled = 0;
    }
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

struct Digraph {
    counts: Vec<u64>,
    prev: Option<u8>,
}

impl Digraph {
    fn feed(&mut self, mut chunk: &[u8]) {
        let mut prev = match (self.prev, chunk.split_first()) {
            (Some(prev), _) => prev,
            (None, Some((&first, rest))) => {
                chunk = rest;
                first
            }
            (None, None) => return,
        };
        for &b in chunk {
            self.counts[(b as usize) << 8 | prev as usize] += 1;
            prev = b;
        }
        self.prev = Some(prev);
    }

    fn pixels(&self) -> Vec<u8> {
        let max = self.counts.iter().copied().max().unwrap_or(0).max(1);
        let scale = 254.0 / (max as f64).ln_1p();
        self.counts
            .iter()
            .map(|&c| {
                if c == 0 {
                    0
                } else {
                    1 + ((c as f64).ln_1p() * scale) as u8
                }
            })
            .collect()
    }
}

enum State {
    PerByte,
    Entropy(Box<Entropy>),
    Digraph(Digraph),
}

/// Feeds the pixel values for `input` to `sink` in order, chunk by chunk.
pub fn pixels(
    mode: Mode,
    mut input: impl Read,
    mut sink: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    mode.check()?;
    let mut buf = vec![0u8; 1 << 18];
    let mut state = match mode {
        Mode::Bytes | Mode::Class => State::PerByte,
        Mode::Entropy { block } => State::Entropy(Box::new(Entropy::new(block))),
        Mode::Digraph => State::Digraph(Digraph {
            counts: vec![0; 65536],
            prev: None,
        }),
    };
    let mut any = false;
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        any = true;
        match &mut state {
            State::PerByte => sink(&buf[..n])?,
            State::Entropy(e) => {
                e.feed(&buf[..n]);
                sink(&e.out)?;
                e.out.clear();
            }
            State::Digraph(d) => d.feed(&buf[..n]),
        }
    }
    match state {
        State::PerByte => Ok(()),
        State::Entropy(mut e) => {
            e.emit();
            sink(&e.out)
        }
        State::Digraph(d) if any => sink(&d.pixels()),
        State::Digraph(_) => Ok(()),
    }
}

fn dims(opts: &Options, len: u64) -> io::Result<(u32, u32)> {
    let count = opts.mode.pixel_count(len);
    if count == 0 {
        return Err(invalid("no input data"));
    }
    let width = match opts.width {
        _ if opts.mode == Mode::Digraph => 256,
        Some(0) => return Err(invalid("width must be at least 1")),
        Some(w) => w as u64,
        None => {
            let mut w = (count as f64).sqrt() as u64;
            while w * w < count {
                w += 1;
            }
            w
        }
    };
    let height = count.div_ceil(width);
    if width > i32::MAX as u64 || height > i32::MAX as u64 {
        return Err(invalid("image dimensions exceed the BMP limit"));
    }
    Ok((width as u32, height as u32))
}

const PIXEL_OFFSET: u32 = 14 + 40 + 256 * 4;

fn stride(width: u32) -> u64 {
    (width as u64).div_ceil(4) * 4
}

fn write_header(
    out: &mut impl Write,
    width: u32,
    height: u32,
    palette: &Palette,
) -> io::Result<()> {
    let data_size = stride(width) * height as u64;
    let file_size = u32::try_from(PIXEL_OFFSET as u64 + data_size).map_err(|_| {
        invalid(format!(
            "output would be {:.1} GB, over the 4 GB BMP limit",
            data_size as f64 / 1e9
        ))
    })?;

    let mut h = Vec::with_capacity(PIXEL_OFFSET as usize);
    h.extend_from_slice(b"BM");
    h.extend_from_slice(&file_size.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(&PIXEL_OFFSET.to_le_bytes());
    h.extend_from_slice(&40u32.to_le_bytes());
    h.extend_from_slice(&(width as i32).to_le_bytes());
    // Negative height = rows stored top-down, so they stream in input order.
    h.extend_from_slice(&(-(height as i32)).to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&8u16.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(&(data_size as u32).to_le_bytes());
    h.extend_from_slice(&2835u32.to_le_bytes());
    h.extend_from_slice(&2835u32.to_le_bytes());
    h.extend_from_slice(&256u32.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    for [r, g, b] in palette {
        h.extend_from_slice(&[*b, *g, *r, 0]);
    }
    out.write_all(&h)
}

/// Cuts a pixel stream into BMP rows (padded to 4 bytes) and zero-fills the tail.
struct RowWriter<W> {
    out: W,
    width: usize,
    pad: usize,
    col: usize,
    left: u64,
}

impl<W: Write> RowWriter<W> {
    fn new(mut out: W, width: u32, height: u32, palette: &Palette) -> io::Result<Self> {
        write_header(&mut out, width, height, palette)?;
        Ok(Self {
            out,
            width: width as usize,
            pad: (stride(width) - width as u64) as usize,
            col: 0,
            left: width as u64 * height as u64,
        })
    }

    fn push(&mut self, mut values: &[u8]) -> io::Result<()> {
        while !values.is_empty() && self.left > 0 {
            let n = (self.width - self.col).min(values.len());
            self.out.write_all(&values[..n])?;
            values = &values[n..];
            self.col += n;
            self.left -= n as u64;
            if self.col == self.width {
                self.out.write_all(&[0; 3][..self.pad])?;
                self.col = 0;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        let zeros = vec![0u8; self.width];
        while self.left > 0 {
            let n = self.width - self.col;
            self.push(&zeros[..n])?;
        }
        self.out.flush()
    }
}

impl Image {
    pub fn write_bmp(&self, out: impl Write) -> io::Result<()> {
        let mut rows = RowWriter::new(out, self.width, self.height, &self.palette)?;
        rows.push(&self.pixels)?;
        rows.finish()
    }
}

/// Lays `values` along the Hilbert curve through the `size`-wide square whose
/// first cell is `(x, y)`. The curve sets off along `a` and ends along `b`.
fn hilbert_walk(
    values: &mut std::slice::Iter<u8>,
    px: &mut [u8],
    width: usize,
    (x, y): (isize, isize),
    a: (isize, isize),
    b: (isize, isize),
    size: isize,
) {
    let at = |i: isize, j: isize| (x + a.0 * i + b.0 * j, y + a.1 * i + b.1 * j);
    if size == 2 {
        for ((x, y), &v) in [at(0, 0), at(1, 0), at(1, 1), at(0, 1)]
            .into_iter()
            .zip(values)
        {
            px[y as usize * width + x as usize] = v;
        }
    } else if values.len() > 0 {
        let h = size / 2;
        hilbert_walk(values, px, width, at(0, 0), b, a, h);
        hilbert_walk(values, px, width, at(h, 0), a, b, h);
        hilbert_walk(values, px, width, at(h, h), a, b, h);
        hilbert_walk(
            values,
            px,
            width,
            at(h - 1, size - 1),
            (-b.0, -b.1),
            (-a.0, -a.1),
            h,
        );
    }
}

fn hilbert(values: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut side = 1;
    while side * side < values.len() {
        side *= 2;
    }
    if side == 1 {
        return (1, 1, values.to_vec());
    }
    // The curve starts downwards, so its first half stays in the left half.
    let width = if values.len() * 2 <= side * side {
        side / 2
    } else {
        side
    };
    let mut px = vec![0; width * side];
    hilbert_walk(
        &mut values.iter(),
        &mut px,
        width,
        (0, 0),
        (0, 1),
        (1, 0),
        side as isize,
    );
    (width as u32, side as u32, px)
}

fn collect(input: impl Read, len: u64, opts: &Options) -> io::Result<Image> {
    opts.mode.check()?;
    let (mut width, mut height) = dims(opts, len)?;
    let mut px = Vec::with_capacity(opts.mode.pixel_count(len) as usize);
    pixels(opts.mode, input, |v| {
        px.extend_from_slice(v);
        Ok(())
    })?;
    if opts.hilbert() {
        (width, height, px) = hilbert(&px);
    } else {
        px.resize(width as usize * height as usize, 0);
    }
    Ok(Image {
        width,
        height,
        pixels: px,
        palette: opts.mode.palette(),
    })
}

/// Renders `data` into an in-memory image.
pub fn render(data: &[u8], opts: &Options) -> io::Result<Image> {
    collect(data, data.len() as u64, opts)
}

/// Writes `input` (exactly `len` bytes) to `out` as a BMP and returns its
/// width and height. With [`Layout::Rows`] this streams in constant memory.
pub fn write_bmp(
    input: impl Read,
    len: u64,
    out: impl Write,
    opts: &Options,
) -> io::Result<(u32, u32)> {
    if opts.hilbert() {
        let image = collect(input, len, opts)?;
        image.write_bmp(out)?;
        return Ok((image.width, image.height));
    }
    opts.mode.check()?;
    let (width, height) = dims(opts, len)?;
    let mut rows = RowWriter::new(out, width, height, &opts.mode.palette())?;
    pixels(opts.mode, input, |v| rows.push(v))?;
    rows.finish()?;
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(data: &[u8], opts: &Options) -> Vec<u8> {
        let mut out = Vec::new();
        write_bmp(data, data.len() as u64, &mut out, opts).unwrap();
        out
    }

    #[test]
    fn rows_are_top_down_padded_and_zero_filled() {
        let bmp = stream(&[1, 2, 3, 4, 5], &Options::default());
        assert_eq!(&bmp[18..22], &3i32.to_le_bytes());
        assert_eq!(&bmp[22..26], &(-2i32).to_le_bytes());
        assert_eq!(&bmp[2..6], &(bmp.len() as u32).to_le_bytes());
        assert_eq!(&bmp[PIXEL_OFFSET as usize..], &[1, 2, 3, 0, 4, 5, 0, 0]);
    }

    #[test]
    fn streamed_and_in_memory_output_match() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 31 % 251) as u8).collect();
        for width in [None, Some(1), Some(7), Some(256)] {
            let opts = Options {
                width,
                ..Options::default()
            };
            let mut mem = Vec::new();
            render(&data, &opts).unwrap().write_bmp(&mut mem).unwrap();
            assert_eq!(stream(&data, &opts), mem, "width {width:?}");
        }
    }

    #[test]
    fn class_mode_only_swaps_the_palette() {
        let data = [0x00, b'a', 0x90, 0xFF];
        let class = Options {
            mode: Mode::Class,
            ..Options::default()
        };
        let image = render(&data, &class).unwrap();
        assert_eq!(image.pixels, data);
        let colours = data.map(|b| image.palette[b as usize]);
        assert_eq!(colours, [[0; 3], [55, 126, 184], [228, 26, 28], [255; 3]]);
    }

    #[test]
    fn entropy_spans_black_to_white_per_block() {
        let mut data = vec![7u8; 256];
        data.extend(0..=255u8);
        data.extend([1, 2, 1, 2]);
        let opts = Options {
            mode: Mode::Entropy { block: 256 },
            ..Options::default()
        };
        // constant block, every value once, then a 4-byte tail holding 1 bit/byte
        assert_eq!(render(&data, &opts).unwrap().pixels[..3], [0, 255, 32]);
    }

    #[test]
    fn entropy_blocks_ignore_read_boundaries() {
        struct OneByOne<'a>(&'a [u8]);
        impl Read for OneByOne<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(1);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let data: Vec<u8> = (0..5000u32).map(|i| (i * i % 253) as u8).collect();
        let mode = Mode::Entropy { block: 7 };
        let collect = |input: &mut dyn Read| {
            let mut out = Vec::new();
            pixels(mode, input, |v| {
                out.extend_from_slice(v);
                Ok(())
            })
            .unwrap();
            out
        };
        let whole = collect(&mut &data[..]);
        assert_eq!(whole.len() as u64, mode.pixel_count(data.len() as u64));
        assert_eq!(collect(&mut OneByOne(&data)), whole);
    }

    #[test]
    fn digraph_plots_byte_pairs() {
        let opts = Options {
            mode: Mode::Digraph,
            layout: Layout::Hilbert,
            width: Some(3),
        };
        let image = render(&[1, 2, 1, 2, 1, 9], &opts).unwrap();
        assert_eq!((image.width, image.height), (256, 256));
        let at = |x: usize, y: usize| image.pixels[y * 256 + x];
        // 1→2 twice (brightest), 2→1 twice, 1→9 once, nothing else
        assert_eq!((at(1, 2), at(2, 1)), (255, 255));
        assert!(at(1, 9) > 0 && at(1, 9) < 255);
        assert_eq!(image.pixels.iter().filter(|&&p| p > 0).count(), 3);
    }

    #[test]
    fn hilbert_curve_visits_every_cell_through_neighbours() {
        let values: Vec<u8> = (0..=255).collect();
        let (width, height, px) = hilbert(&values);
        assert_eq!((width, height), (16, 16));
        let cell = |v: u8| {
            let i = px.iter().position(|&p| p == v).unwrap();
            (i % 16, i / 16)
        };
        assert_eq!(cell(0), (0, 0));
        for v in 0..255 {
            let ((x0, y0), (x1, y1)) = (cell(v), cell(v + 1));
            assert_eq!(x0.abs_diff(x1) + y0.abs_diff(y1), 1, "step {v}");
        }
    }

    #[test]
    fn hilbert_layout_keeps_every_pixel_and_crops_the_unused_half() {
        let hilbert = Options {
            layout: Layout::Hilbert,
            ..Options::default()
        };
        let full = render(&[9; 200], &hilbert).unwrap();
        assert_eq!((full.width, full.height), (16, 16));
        assert_eq!(full.pixels.iter().filter(|&&p| p == 9).count(), 200);
        let half = render(&[9; 128], &hilbert).unwrap();
        assert_eq!((half.width, half.height), (8, 16));
        assert_eq!(half.pixels, [9; 128]);
        assert_eq!(render(&[9], &hilbert).unwrap().pixels, [9]);

        let mut bmp = Vec::new();
        let dims = write_bmp(&[9u8; 128][..], 128, &mut bmp, &hilbert).unwrap();
        assert_eq!(dims, (8, 16));
        assert_eq!(bmp.len(), PIXEL_OFFSET as usize + 128);
    }

    #[test]
    fn refuses_what_bmp_cannot_hold() {
        let opts = Options::default();
        assert!(write_bmp(io::empty(), 0, io::sink(), &opts).is_err());
        let digraph = Options {
            mode: Mode::Digraph,
            ..opts
        };
        assert!(write_bmp(io::empty(), 0, io::sink(), &digraph).is_err());
        assert!(write_bmp(io::empty(), 5 << 30, io::sink(), &opts).is_err());
        let opts = Options {
            mode: Mode::Entropy { block: 0 },
            ..Options::default()
        };
        assert!(render(&[1], &opts).is_err());
    }
}
