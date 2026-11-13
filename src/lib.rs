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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub mode: Mode,
    /// Image width in pixels; `None` picks a near-square canvas.
    pub width: Option<u32>,
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
        std::array::from_fn(|i| [i as u8; 3])
    }

    /// Number of pixels produced for `len` input bytes.
    pub fn pixel_count(self, len: u64) -> u64 {
        len
    }
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// Feeds the pixel values for `input` to `sink` in order, chunk by chunk.
pub fn pixels(
    mode: Mode,
    mut input: impl Read,
    mut sink: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut buf = vec![0u8; 1 << 18];
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        match mode {
            Mode::Bytes => sink(&buf[..n])?,
        }
    }
}

fn dims(count: u64, width: Option<u32>) -> io::Result<(u32, u32)> {
    if count == 0 {
        return Err(invalid("no input data"));
    }
    let width = match width {
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

/// Renders `data` into an in-memory image.
pub fn render(data: &[u8], opts: &Options) -> io::Result<Image> {
    let count = opts.mode.pixel_count(data.len() as u64);
    let (width, height) = dims(count, opts.width)?;
    let mut px = Vec::with_capacity(width as usize * height as usize);
    pixels(opts.mode, data, |v| {
        px.extend_from_slice(v);
        Ok(())
    })?;
    px.resize(width as usize * height as usize, 0);
    Ok(Image {
        width,
        height,
        pixels: px,
        palette: opts.mode.palette(),
    })
}

/// Streams `input` (exactly `len` bytes) to `out` as a BMP and returns its
/// width and height. Memory use is constant regardless of `len`.
pub fn write_bmp(
    input: impl Read,
    len: u64,
    out: impl Write,
    opts: &Options,
) -> io::Result<(u32, u32)> {
    let (width, height) = dims(opts.mode.pixel_count(len), opts.width)?;
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
    fn refuses_what_bmp_cannot_hold() {
        let opts = Options::default();
        assert!(write_bmp(io::empty(), 0, io::sink(), &opts).is_err());
        assert!(write_bmp(io::empty(), 5 << 30, io::sink(), &opts).is_err());
    }
}
