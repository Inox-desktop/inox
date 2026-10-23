//! The compositor-side "waydriver" host harness (`ADR 0036` D02, phase 2d).
//!
//! The host parses the binary PPM images the compositor's `--test-input`
//! `dump-frame` command writes, and compares their pixels against the scene the
//! headless compositor is known to render. The decode and the comparison are
//! intentionally pure functions with unit tests, so the integration test in
//! `../waydriver_host.rs` only has to drive a real subprocess and read a file.
//!
//! Everything here is test code: it lives under `tests/`, may use
//! `assert!`/`panic!` freely, and is not part of the shipped crate.

/// Maximum PPM (`P6`) dimension the decoder accepts, in pixels.
///
/// The compositor's headless output is at most [`crate::config::MAX_DIMENSION`]
/// (`16_384`) square, and the `P6` header stores dimensions as ASCII. This bound
/// keeps the pixel-count arithmetic (`width * height * 3`) from overflowing on a
/// header we were handed.
pub const MAX_DIMENSION: usize = 1_000_000;

/// Why a byte slice could not be decoded as the PPM image `to_ppm` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpmDecodeError {
    /// The file did not start with the `P6` magic.
    BadMagic,
    /// The header did not carry exactly `width height` and a `255` max value.
    BadHeader,
    /// A numeric field in the header was out of range or malformed.
    InvalidField,
    /// The pixel data is shorter than the geometry in the header requires.
    ShortFrame {
        /// Number of RGB bytes the geometry requires.
        expected: usize,
        /// Number of bytes actually present after the header.
        actual: usize,
    },
    /// The geometry exceeds [`MAX_DIMENSION`].
    TooLarge {
        /// The width read from the header.
        width: usize,
        /// The height read from the header.
        height: usize,
    },
}

/// Decoded binary PPM (`P6`) image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ppm {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Interleaved `RGB` bytes, `width * height` pixels in row-major order.
    pub pixels: Vec<u8>,
}

/// Decode the exact `P6` layout that `crate::test_input::to_ppm` produces.
///
/// The encoder writes `P6\n{width} {height}\n255\n` followed by one RGB triplet
/// per pixel, with the alpha channel dropped. The decoder reads that back as
/// interleaved RGB bytes so a test can compare pixels without pulling in an
/// image crate.
///
/// # Errors
///
/// Returns [`PpmDecodeError`] when the magic, header, geometry or trailing
/// length does not match what the encoder writes.
pub fn decode_ppm(bytes: &[u8]) -> Result<Ppm, PpmDecodeError> {
    if !bytes.starts_with(b"P6") {
        return Err(PpmDecodeError::BadMagic);
    }
    let header_end = find_header_end(bytes).ok_or(PpmDecodeError::BadHeader)?;
    let header = &bytes[..header_end];
    let pixel_bytes = &bytes[header_end..];

    let mut fields = header.split(|b| b.is_ascii_whitespace());
    let magic = fields.next().ok_or(PpmDecodeError::BadHeader)?;
    if magic != b"P6" {
        return Err(PpmDecodeError::BadMagic);
    }
    let width = parse_field(fields.next().ok_or(PpmDecodeError::BadHeader)?)?;
    let height = parse_field(fields.next().ok_or(PpmDecodeError::BadHeader)?)?;
    if fields.next() != Some(b"255") {
        return Err(PpmDecodeError::BadHeader);
    }
    if width == 0 || height == 0 {
        return Err(PpmDecodeError::InvalidField);
    }

    let pixels = width
        .checked_mul(height)
        .ok_or(PpmDecodeError::TooLarge { width, height })?;
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(PpmDecodeError::TooLarge { width, height });
    }
    let expected = pixels
        .checked_mul(3)
        .ok_or(PpmDecodeError::TooLarge { width, height })?;

    if pixel_bytes.len() < expected {
        return Err(PpmDecodeError::ShortFrame {
            expected,
            actual: pixel_bytes.len(),
        });
    }

    Ok(Ppm {
        width: width as u32,
        height: height as u32,
        pixels: pixel_bytes[..expected].to_vec(),
    })
}

/// The offset just past the header line that `to_ppm` writes, or `None` when the
/// header is malformed or truncated.
///
/// The header is exactly four whitespace-separated tokens — `P6`, width, height,
/// `255` — followed by one byte of trailing whitespace. Parsing it token by token
/// avoids mistaking a `255\n` inside the width/height or the pixel data for the
/// end of the header.
fn find_header_end(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"P6") || bytes.get(2)? < &b'0' {
        return None;
    }
    let mut index = 3;
    // Width.
    let end = skip_whitespace(bytes, index)?;
    index = end;
    index = take_field(bytes, index)?;
    if index == end {
        return None;
    }
    index = skip_whitespace(bytes, index)?;
    // Height.
    let height_start = index;
    index = take_field(bytes, index)?;
    if index == height_start {
        return None;
    }
    index = skip_whitespace(bytes, index)?;
    // Max value; must be `255`.
    if bytes.get(index..index.checked_add(3)?) != Some(b"255") {
        return None;
    }
    index += 3;
    // One byte of trailing whitespace ends the header.
    if !bytes.get(index)?.is_ascii_whitespace() {
        return None;
    }
    Some(index + 1)
}

/// Advance `index` past any ASCII whitespace, returning the offset of the next
/// non-whitespace byte, or `None` when the slice ends first.
fn skip_whitespace(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    (index < bytes.len()).then_some(index)
}

/// Advance `index` past one ASCII field, returning the offset just past it, or
/// the unchanged `index` when a non-whitespace run never starts.
fn take_field(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    Some(index)
}

/// Parse one ASCII decimal field from the header.
fn parse_field(field: &[u8]) -> Result<usize, PpmDecodeError> {
    if field.is_empty() || !field.iter().all(|b| b.is_ascii_digit()) {
        return Err(PpmDecodeError::InvalidField);
    }
    std::str::from_utf8(field)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or(PpmDecodeError::InvalidField)
}

/// Compare a decoded marker pixel against an expected RGBA colour.
///
/// PPM has no alpha channel, so the `alpha` byte of `expected` is ignored. The
/// tolerance margins the float-to-unorm rounding difference between GL drivers
/// that `tests/headless_render.rs` documents.
pub fn pixel_channel_difference(ppm: &Ppm, x: u32, y: u32, channel: usize) -> u8 {
    assert!(
        channel < 3,
        "a PPM has only R, G, B channels, got {channel}"
    );
    let idx = (y as usize) * ppm.width as usize * 3 + (x as usize) * 3 + channel;
    ppm.pixels[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic two-row RGB frame: row 0 is a horizontal gradient, row 1
    /// is a solid magenta.
    fn sample_pixels() -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..2u32 {
            for x in 0..4u32 {
                pixels.extend_from_slice(&[(x * 40) as u8, (x * 40 + 10) as u8, (y * 200) as u8]);
            }
        }
        pixels
    }

    #[test]
    fn header_round_trips() {
        let pixels = sample_pixels();
        let mut bytes = b"P6\n4 2\n255\n".to_vec();
        bytes.extend_from_slice(&pixels);
        let ppm = decode_ppm(&bytes).expect("a well-formed P6 must decode");
        assert_eq!((ppm.width, ppm.height), (4, 2));
        assert_eq!(ppm.pixels, pixels);
    }

    #[test]
    fn trailing_bytes_are_ignored() {
        let mut bytes = b"P6\n1 1\n255\n".to_vec();
        bytes.extend_from_slice(&[1, 2, 3]);
        bytes.extend_from_slice(&[9, 9, 9]);
        let ppm = decode_ppm(&bytes).expect("extra trailing data must be ignored");
        assert_eq!(ppm.pixels, vec![1, 2, 3]);
    }

    #[test]
    fn rejects_a_non_ppm_magic() {
        assert_eq!(
            decode_ppm(b"GIF\n1 1\n255\n"),
            Err(PpmDecodeError::BadMagic)
        );
    }

    #[test]
    fn rejects_a_truncated_header() {
        assert_eq!(decode_ppm(b"P6\n1"), Err(PpmDecodeError::BadHeader));
        assert_eq!(decode_ppm(b"P6\n1 1"), Err(PpmDecodeError::BadHeader));
        assert_eq!(decode_ppm(b"P6\n1 1\n"), Err(PpmDecodeError::BadHeader));
        assert_eq!(decode_ppm(b"P6\n1 1\n255"), Err(PpmDecodeError::BadHeader));
    }

    #[test]
    fn rejects_a_bad_and_oversized_geometry() {
        assert_eq!(
            decode_ppm(b"P6\n0 1\n255\n"),
            Err(PpmDecodeError::InvalidField)
        );
        let bytes = b"P6\n1000001 1\n255\n".to_vec();
        assert!(matches!(
            decode_ppm(&bytes),
            Err(PpmDecodeError::TooLarge {
                width: 1_000_001,
                ..
            })
        ));
    }

    #[test]
    fn rejects_pixel_data_shorter_than_the_geometry() {
        let mut bytes = b"P6\n4 2\n255\n".to_vec();
        bytes.extend_from_slice(&sample_pixels()[..3]); // only one of 24 bytes
        assert!(matches!(
            decode_ppm(&bytes),
            Err(PpmDecodeError::ShortFrame {
                expected: 24,
                actual: 3
            })
        ));
    }

    #[test]
    fn marker_pixel_read_is_row_major() {
        let mut bytes = b"P6\n2 2\n255\n".to_vec();
        bytes.extend_from_slice(&[
            255, 0, 0, //
            0, 255, 0, //
            0, 0, 255, //
            255, 255, 0, //
        ]);
        let ppm = decode_ppm(&bytes).expect("the sample must decode");
        assert_eq!(pixel_channel_difference(&ppm, 0, 0, 0), 255);
        assert_eq!(pixel_channel_difference(&ppm, 1, 0, 1), 255);
        assert_eq!(pixel_channel_difference(&ppm, 0, 1, 2), 255);
        assert_eq!(pixel_channel_difference(&ppm, 1, 1, 1), 255);
    }
}
