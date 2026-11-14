//! bin2bmp for the browser: a handful of C-style exports, no bindings generator.
//!
//! From JavaScript: `p = b2b_alloc(n)`, copy the file into the module's memory at
//! `p`, `out = b2b_render(p, n, mode, param, hilbert)`, `b2b_free(p, n)`. `out` is
//! 0 if the input cannot be rendered, otherwise it points at `width: u32`,
//! `height: u32` (little endian) followed by `width * height` RGBA pixels; hand
//! it back with `b2b_free(out, 8 + width * height * 4)`.

use bin2bmp::{Layout, Mode, Options};

#[no_mangle]
pub extern "C" fn b2b_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// # Safety
/// `ptr` and `len` must describe a buffer handed out by this module that has not been freed.
#[no_mangle]
pub unsafe extern "C" fn b2b_free(ptr: *mut u8, len: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

/// `mode`: 0 bytes, 1 class, 2 bit (`param` = which bit), 3 entropy and 4 chi
/// (`param` = block size), 5 digraph.
///
/// # Safety
/// `data` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn b2b_render(
    data: *const u8,
    len: usize,
    mode: u32,
    param: u32,
    hilbert: u32,
) -> *mut u8 {
    let mode = match mode {
        0 => Mode::Bytes,
        1 => Mode::Class,
        2 => Mode::Bit { bit: param as u8 },
        3 => Mode::Entropy {
            block: param as usize,
        },
        4 => Mode::Chi {
            block: param as usize,
        },
        5 => Mode::Digraph,
        _ => return std::ptr::null_mut(),
    };
    let layout = if hilbert != 0 {
        Layout::Hilbert
    } else {
        Layout::Rows
    };
    let opts = Options {
        mode,
        layout,
        width: None,
    };
    let Ok(image) = bin2bmp::render(std::slice::from_raw_parts(data, len), &opts) else {
        return std::ptr::null_mut();
    };
    let mut out = Vec::with_capacity(8 + image.pixels.len() * 4);
    out.extend_from_slice(&image.width.to_le_bytes());
    out.extend_from_slice(&image.height.to_le_bytes());
    for &index in &image.pixels {
        let [r, g, b] = image.palette[index as usize];
        out.extend_from_slice(&[r, g, b, 255]);
    }
    Box::into_raw(out.into_boxed_slice()) as *mut u8
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn render(
        data: &[u8],
        mode: u32,
        param: u32,
        hilbert: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let input = b2b_alloc(data.len());
        std::ptr::copy_nonoverlapping(data.as_ptr(), input, data.len());
        let out = b2b_render(input, data.len(), mode, param, hilbert);
        b2b_free(input, data.len());
        if out.is_null() {
            return None;
        }
        let header = std::slice::from_raw_parts(out, 8);
        let width = u32::from_le_bytes(header[..4].try_into().unwrap());
        let height = u32::from_le_bytes(header[4..].try_into().unwrap());
        let len = (width * height * 4) as usize;
        let rgba = std::slice::from_raw_parts(out.add(8), len).to_vec();
        b2b_free(out, 8 + len);
        Some((width, height, rgba))
    }

    #[test]
    fn renders_rgba_with_a_size_header() {
        unsafe {
            let (width, height, rgba) = render(&[0, 255, 7], 0, 0, 0).unwrap();
            assert_eq!((width, height), (2, 2));
            let grey = |v| [v, v, v, 255];
            assert_eq!(rgba, [grey(0), grey(255), grey(7), grey(0)].concat());

            // class mode colours printable text blue, bit mode shows bit 7
            assert_eq!(render(b"a", 1, 0, 0).unwrap().2, [55, 126, 184, 255]);
            assert_eq!(render(&[0x80], 2, 7, 0).unwrap().2, [255; 4]);
            assert_eq!(
                render(&[9; 128], 0, 0, 1).unwrap().0,
                8,
                "hilbert crops to the used half"
            );
            assert_eq!(
                render(&[1, 2, 3], 5, 0, 0).map(|i| (i.0, i.1)),
                Some((256, 256))
            );
        }
    }

    #[test]
    fn refuses_what_cannot_be_rendered() {
        unsafe {
            assert!(render(&[], 0, 0, 0).is_none(), "no data");
            assert!(render(&[1], 3, 0, 0).is_none(), "block size 0");
            assert!(render(&[1], 2, 8, 0).is_none(), "bit 8");
            assert!(render(&[1], 99, 0, 0).is_none(), "unknown mode");
        }
    }
}
