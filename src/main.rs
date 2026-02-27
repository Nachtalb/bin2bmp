use std::{
    env, fs,
    io::{self, Read, Write},
};

/// Write an 8-bit indexed grayscale BMP (1 byte per pixel, 256-entry palette).
/// This keeps the file size at ~1× input size instead of 3×, avoiding the
/// 4 GB BMP header overflow that occurs with 24-bit on large inputs.
/// BMP rows are bottom-up and padded to a 4-byte boundary.
fn encode_bmp(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row_stride = (width + 3) / 4 * 4;           // 1 byte/px, pad to 4
    let pixel_data_size = row_stride as u64 * height as u64;
    let palette_size: u32 = 256 * 4;                 // 256 RGBQUAD entries
    let pixel_offset: u32 = 14 + 40 + palette_size;
    let file_size = pixel_offset as u64 + pixel_data_size;

    if file_size > u32::MAX as u64 {
        eprintln!(
            "warning: output is {:.1} GB — exceeds 4 GB BMP limit; \
             consider splitting the input",
            file_size as f64 / 1e9
        );
    }

    let mut out = Vec::with_capacity(file_size.min(1 << 30) as usize);

    // ---- File header (14 bytes) ----
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes()); // wraps if >4 GB, but we warned
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    out.extend_from_slice(&pixel_offset.to_le_bytes());

    // ---- DIB header / BITMAPINFOHEADER (40 bytes) ----
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());  // color planes
    out.extend_from_slice(&8u16.to_le_bytes());  // bits per pixel (8-bit indexed)
    out.extend_from_slice(&0u32.to_le_bytes());  // compression (none)
    out.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // ~72 DPI x
    out.extend_from_slice(&2835u32.to_le_bytes()); // ~72 DPI y
    out.extend_from_slice(&256u32.to_le_bytes()); // colors in table
    out.extend_from_slice(&0u32.to_le_bytes());   // important colors

    // ---- Grayscale palette: 256 × RGBQUAD (B, G, R, reserved) ----
    for i in 0u8..=255 {
        out.push(i); // B
        out.push(i); // G
        out.push(i); // R
        out.push(0); // reserved
    }

    // ---- Pixel data (bottom-up rows) ----
    for row in (0..height).rev() {
        for col in 0..width {
            let idx = (row * width + col) as usize;
            out.push(pixels.get(idx).copied().unwrap_or(0));
        }
        // pad row to 4-byte boundary
        for _ in width..row_stride {
            out.push(0);
        }
    }

    out
}

fn square_dims(n: usize) -> (u32, u32) {
    let w = (n as f64).sqrt().ceil() as u32;
    let h = n.div_ceil(w as usize) as u32;
    (w, h)
}

fn usage(bin: &str) {
    eprintln!("Usage: {bin} [INPUT] [OUTPUT]");
    eprintln!();
    eprintln!("  INPUT   path to binary file, or '-' for stdin  (default: stdin)");
    eprintln!("  OUTPUT  path for .bmp output, or '-' for stdout (default: stdout)");
    eprintln!();
    eprintln!("Each byte becomes one grayscale pixel. Canvas is as square as possible.");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage(&args[0]);
        return;
    }

    let input_path = args.get(1).map(String::as_str);
    let output_path = args.get(2).map(String::as_str);

    // Read input
    let data: Vec<u8> = match input_path {
        Some("-") | None => {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .expect("failed to read stdin");
            buf
        }
        Some(path) => fs::read(path).expect("failed to read input file"),
    };

    if data.is_empty() {
        eprintln!("error: no input data");
        std::process::exit(1);
    }

    let (width, height) = square_dims(data.len());
    let bmp = encode_bmp(&data, width, height);

    // Write output
    match output_path {
        Some("-") | None => {
            io::stdout()
                .write_all(&bmp)
                .expect("failed to write stdout");
        }
        Some(path) => fs::write(path, &bmp).expect("failed to write output file"),
    }

    eprintln!("{width}x{height} ({} bytes → {} pixels)", data.len(), width * height);
}
