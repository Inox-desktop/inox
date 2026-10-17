//! Reading an image file into RGBA8 pixels.
//!
//! Two things make this module more than a call to `image::open`:
//!
//! - **The bound.** `MAX_PIXELS` caps how much memory one decode may commit.
//!   An unbounded decode is a denial of service on a desktop session: the file
//!   is a few kilobytes of a highly compressible format, the buffer is hundreds
//!   of megabytes, and the session that dies is the one running the wallpaper.
//!   The bound is checked twice — against the size the header declares, before
//!   any pixels are allocated, and again against the buffer the decoder
//!   actually produced.
//! - **The errors are typed.** Every failure is a [`DecodeError`] that the
//!   caller maps onto the degradation vocabulary
//!   ([`Unusable`]): nothing here logs and continues,
//!   and nothing here reaches for the theme background by itself.

use std::io;
use std::path::{Path, PathBuf};

use image::{ImageFormat, ImageReader, Limits};

use crate::model::{Size, Unusable};

/// The largest image the wallpaper will decode, in pixels.
///
/// 7680 × 4320 — 8K UHD — is the widest output Inox can drive, so an image
/// larger than that cannot be shown at its own resolution on any configuration
/// this session supports; allowing one would only buy a longer allocation. At
/// four bytes per pixel the bound caps a single RGBA8 frame at 126.5 MiB, which
/// a desktop session can survive, whereas the 512 MiB `image` allows by default
/// across several outputs is an out-of-memory kill. Two 8K wallpapers are the
/// workload this is sized for, not two hundred.
pub const MAX_PIXELS: u64 = 7680 * 4320;

/// The widest or tallest image that is even considered, in pixels.
///
/// A second, independent bound: [`MAX_PIXELS`] alone would admit a
/// 4 294 967 295 × 1 strip, which is a number the decoders accept and no output
/// can use. It is checked against the size the header declares, before any
/// pixels are allocated, and it is deliberately twice the widest mode Inox can
/// drive so that a legitimately unusual wallpaper is never the thing that fails.
pub const MAX_DIMENSION: u32 = 32_768;

/// The RGBA8 pixels of one image.
///
/// A frame is a decoded image and nothing else: it does not know where it will be
/// drawn, and it is never mutated. [`crate::apply`] moves its buffer into the
/// renderer's own handle, so the pixels exist exactly once from decode to
/// screen — which is also what makes "the new frame is complete before the old
/// one is released" a statement about one buffer being replaced, not about a
/// buffer being written over.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    /// Reports the size and the length, never the pixels.
    ///
    /// A frame is up to 126 MiB of bytes; a derived `Debug` would print all of
    /// them, and the one place a frame ends up in a log line is a `?` inside a
    /// larger value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("size", &self.size())
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

impl Frame {
    /// A frame from decoded RGBA8 pixels.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when `rgba` is not exactly `width * height * 4` bytes. The
    /// check exists because a frame whose length disagrees with its size would
    /// be drawn as garbage at best and panicked on at worst, deep inside the
    /// renderer; refusing it here keeps that class of mistake out of the
    /// interface, and it is what the "no half-drawn state" test exercises.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, FrameError> {
        let expected = (u64::from(width) * u64::from(height) * 4) as usize;

        if rgba.len() != expected {
            return Err(FrameError::Length {
                expected,
                found: rgba.len(),
            });
        }

        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// The frame's size in pixels.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The frame's width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The frame's height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, in RGBA order, row-major from the top-left corner.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The pixels, moved out of the frame.
    #[must_use]
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }
}

/// A frame whose pixel buffer does not match its size.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The buffer is the wrong length for the size it claims.
    #[error("a {expected}-byte RGBA8 frame was expected, found {found}")]
    Length {
        /// `width * height * 4`.
        expected: usize,
        /// The length of the buffer that was handed over.
        found: usize,
    },
}

/// Why an image could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file could not be opened or read.
    #[error("`{path}` cannot be read: {detail}")]
    Unreadable {
        /// The path that was asked for.
        path: PathBuf,
        /// The operating system's own explanation.
        detail: String,
    },
    /// The bytes are not an image this build decodes.
    #[error("`{path}` is not a PNG or a JPEG: {detail}")]
    Undecodable {
        /// The path that was asked for.
        path: PathBuf,
        /// What the decoder recognised, or why it recognised nothing.
        detail: String,
    },
    /// The image has more pixels than [`MAX_PIXELS`] allows.
    #[error("`{path}` is {size} ({pixels} pixels), above the {limit}-pixel bound")]
    TooLarge {
        /// The path that was asked for.
        path: PathBuf,
        /// The size the header declared.
        size: Size,
        /// How many pixels that is.
        pixels: u64,
        /// The bound that was exceeded.
        limit: u64,
    },
    /// One side of the image is longer than [`MAX_DIMENSION`] allows.
    #[error("`{path}` is {size}, whose longest side is above the {limit}-pixel bound")]
    TooLongSide {
        /// The path that was asked for.
        path: PathBuf,
        /// The size the header declared.
        size: Size,
        /// The bound that was exceeded.
        limit: u32,
    },
    /// The decoded pixels do not describe the image the header promised.
    #[error("`{path}` decoded into an unusable buffer: {source}")]
    Malformed {
        /// The path that was asked for.
        path: PathBuf,
        /// The frame-level error.
        #[source]
        source: FrameError,
    },
}

impl DecodeError {
    /// The path this failure is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Unreadable { path, .. }
            | Self::Undecodable { path, .. }
            | Self::TooLarge { path, .. }
            | Self::TooLongSide { path, .. }
            | Self::Malformed { path, .. } => path,
        }
    }
}

impl From<DecodeError> for Unusable {
    /// The reason to show, in the vocabulary the interface renders.
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::Unreadable { detail, .. } => Self::Unreadable { detail },
            DecodeError::Undecodable { detail, .. } => Self::Undecodable { detail },
            DecodeError::TooLarge {
                size,
                pixels,
                limit,
                ..
            } => Self::TooLarge {
                size,
                pixels,
                limit,
            },
            DecodeError::TooLongSide { size, limit, .. } => Self::TooLongSide { size, limit },
            DecodeError::Malformed { source, .. } => Self::Undecodable {
                detail: source.to_string(),
            },
        }
    }
}

/// Decodes the PNG or JPEG at `path` into RGBA8.
///
/// The format is taken from the file's own content, not from its extension: a
/// JPEG named `.png` is a file the user can see and expects to work, and the
/// extension of a wallpaper is not a security boundary. Only PNG and JPEG are
/// accepted, because those are the two decoders ADR 0018 enables.
///
/// # Errors
///
/// [`DecodeError`] for an unreadable file, an unrecognised or malformed image,
/// and an image above [`MAX_PIXELS`].
pub fn decode(path: &Path) -> Result<Frame, DecodeError> {
    let size = dimensions(path)?;
    let pixels = size.pixels();

    if pixels > MAX_PIXELS {
        return Err(DecodeError::TooLarge {
            path: path.to_owned(),
            size,
            pixels,
            limit: MAX_PIXELS,
        });
    }

    // Re-opened rather than reused: `into_dimensions` consumes the reader, and
    // reading the header first is only worth anything if no pixel buffer has
    // been allocated yet.
    let decoded = reader(path)?
        .decode()
        .map_err(|error| DecodeError::Undecodable {
            path: path.to_owned(),
            detail: error.to_string(),
        })?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    let rgba = decoded.into_raw();

    // The header can lie, or a decoder can disagree with it; the buffer that
    // actually exists is what decides, so the bound is checked twice.
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(DecodeError::TooLarge {
            path: path.to_owned(),
            size: Size::new(width, height),
            pixels: u64::from(width) * u64::from(height),
            limit: MAX_PIXELS,
        });
    }

    Frame::new(width, height, rgba).map_err(|source| DecodeError::Malformed {
        path: path.to_owned(),
        source,
    })
}

/// The size the image's own header declares, without decoding any pixels.
///
/// # Errors
///
/// As [`decode`], minus the failures that only a full decode can produce.
/// The side bound ([`MAX_DIMENSION`]) is applied here, because it is a property
/// of the header alone; the pixel bound ([`MAX_PIXELS`]) belongs to [`decode`],
/// which checks it against the size this returns before it allocates anything.
pub fn dimensions(path: &Path) -> Result<Size, DecodeError> {
    let (width, height) =
        reader(path)?
            .into_dimensions()
            .map_err(|error| DecodeError::Undecodable {
                path: path.to_owned(),
                detail: error.to_string(),
            })?;

    if width == 0 || height == 0 {
        return Err(DecodeError::Undecodable {
            path: path.to_owned(),
            detail: "the header declares a zero-sized image".to_owned(),
        });
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(DecodeError::TooLongSide {
            path: path.to_owned(),
            size: Size::new(width, height),
            limit: MAX_DIMENSION,
        });
    }

    Ok(Size::new(width, height))
}

/// An image reader for `path`, with the format sniffed from the content and the
/// decoder's own allocation limited.
fn reader(path: &Path) -> Result<ImageReader<std::io::BufReader<std::fs::File>>, DecodeError> {
    let reader = ImageReader::open(path)
        .map_err(|error| DecodeError::Unreadable {
            path: path.to_owned(),
            detail: error.to_string(),
        })?
        .with_guessed_format()
        .map_err(|error| DecodeError::Unreadable {
            path: path.to_owned(),
            detail: error.to_string(),
        })?;

    match reader.format() {
        Some(ImageFormat::Png) | Some(ImageFormat::Jpeg) => Ok(reader),
        Some(other) => Err(DecodeError::Undecodable {
            path: path.to_owned(),
            detail: format!("the decoder recognised {other:?}, which this build does not enable"),
        }),
        None => Err(DecodeError::Undecodable {
            path: path.to_owned(),
            detail: "neither the content nor the extension identifies a known format".to_owned(),
        }),
    }
    .map(|mut reader| {
        // The decoder enforces the same bound while it works, so a header that
        // under-reports its size still cannot allocate past it. The width and
        // height limits are left off on purpose: `image` checks those before
        // this crate can see the numbers, and a refusal there would have to be
        // reported without the size that caused it, which is exactly the
        // diagnosis [`decode`] is trying to give.
        let mut limits = Limits::default();
        limits.max_image_width = None;
        limits.max_image_height = None;
        limits.max_alloc = Some(MAX_PIXELS * 4);
        reader.limits(limits);

        reader
    })
}

/// Whether `error` is the operating system saying the file is not there.
///
/// Used by [`crate::source`] to tell "nothing here" from "here but unreadable",
/// which are different diagnoses for the user.
#[must_use]
pub fn is_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    /// A PNG of `width` × `height` solid pixels, written to `dir`.
    fn png(dir: &Path, name: &str, width: u32, height: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        let buffer = image::RgbaImage::from_pixel(width, height, image::Rgba([12, 34, 56, 255]));
        buffer.save(&path).expect("the fixture is written");
        path
    }

    /// The same PNG cut off `keep` bytes into its image data.
    fn truncated(dir: &Path, source: &Path, name: &str, keep: usize) -> std::path::PathBuf {
        let bytes = fs::read(source).expect("the fixture is written");
        let start = bytes
            .windows(4)
            .position(|window| window == b"IDAT")
            .expect("a PNG has image data")
            + 4;
        let cut = (start + keep).min(bytes.len() - 1);
        let path = dir.join(name);
        fs::write(&path, &bytes[..cut]).expect("the fixture is truncated");
        path
    }

    /// The CRC-32/PNG of `bytes`, computed a bit at a time.
    ///
    /// A fixture helper: the crate itself never checksums anything.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    /// A PNG whose header declares `width` × `height` and whose image data stops
    /// right after it starts.
    ///
    /// Building the 400-megapixel buffer a real fixture would need is precisely
    /// the allocation the bound exists to prevent, so the fixture *is* the
    /// header: a decoder reads the size out of it, and nothing beyond it is ever
    /// reached.
    fn header_only_png(width: u32, height: u32) -> Vec<u8> {
        let mut file = Vec::from([0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

        let mut ihdr = Vec::from(*b"IHDR");
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlacing

        file.extend_from_slice(&13u32.to_be_bytes());
        file.extend_from_slice(&ihdr);
        file.extend_from_slice(&crc32(&ihdr).to_be_bytes());
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"IDAT");
        file
    }

    #[test]
    fn a_png_decodes_into_rgba_pixels_of_the_right_length() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = png(dir.path(), "wall.png", 4, 3);

        let frame = decode(&path).expect("a decodable fixture");

        assert_eq!(frame.size(), Size::new(4, 3));
        assert_eq!(frame.width(), 4);
        assert_eq!(frame.height(), 3);
        assert_eq!(frame.rgba().len(), 4 * 3 * 4);
        assert_eq!(&frame.rgba()[..4], &[12, 34, 56, 255]);
    }

    #[test]
    fn a_jpeg_decodes_into_rgba_pixels() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("wall.jpg");
        // JPEG has no alpha channel, so the fixture is an RGB image.
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50]))
            .save(&path)
            .expect("the fixture is written");

        let frame = decode(&path).expect("a decodable fixture");

        assert_eq!(frame.size(), Size::new(8, 8));
        assert_eq!(frame.rgba().len(), 8 * 8 * 4);
        // JPEG is lossy, so only the neighbourhood is asserted.
        assert!(frame.rgba()[0] > 180, "{}", frame.rgba()[0]);
    }

    #[test]
    fn the_format_is_sniffed_from_the_content_not_the_extension() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // A real JPEG with a PNG name: the user can see the file and expects it
        // to work, and the name is not a security boundary.
        let png_named = dir.path().join("actually-a-jpeg.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30]))
            .save(&png_named)
            .expect("the fixture is written");

        assert_eq!(
            dimensions(&png_named).expect("the content says JPEG"),
            Size::new(8, 8)
        );
    }

    #[test]
    fn a_file_that_is_not_an_image_is_reported_as_undecodable() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("notes.png");
        fs::write(&path, b"this is not an image, it is a note\n").expect("the fixture is written");

        let error = decode(&path).expect_err("a text file is not an image");

        assert!(matches!(error, DecodeError::Undecodable { .. }));
        assert_eq!(error.path(), path.as_path());
        assert!(
            error.to_string().contains("not a PNG or a JPEG"),
            "`{error}`"
        );
    }

    #[test]
    fn a_truncated_png_is_reported_as_undecodable_and_never_panics() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let whole = png(dir.path(), "whole.png", 64, 64);

        // Cutting one byte into the image data and half way through it: both
        // are points where a decoder has committed to a size it cannot fill.
        let broken = [
            truncated(dir.path(), &whole, "cut-early.png", 1),
            truncated(dir.path(), &whole, "cut-middle.png", 40),
        ];

        for path in broken {
            match decode(&path) {
                Ok(frame) => panic!("{} decoded into {frame:?}", path.display()),
                Err(error) => assert_eq!(error.path(), path.as_path()),
            }
        }
    }

    #[test]
    fn a_file_that_is_not_a_png_at_all_is_reported_as_undecodable() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("empty.png");
        fs::write(&path, b"").expect("the fixture is written");

        assert!(matches!(
            decode(&path),
            Err(DecodeError::Undecodable { .. })
        ));
    }

    #[test]
    fn a_missing_file_is_reported_as_unreadable() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("absent.png");

        let error = decode(&path).expect_err("the file does not exist");

        assert!(matches!(error, DecodeError::Unreadable { .. }));
    }

    #[test]
    fn an_oversized_image_is_rejected_before_it_is_decoded() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // 20 000 × 20 000 is 400 megapixels: twelve times the bound.
        let path = dir.path().join("huge.png");
        fs::write(&path, header_only_png(20_000, 20_000)).expect("the fixture is written");

        let error = decode(&path).expect_err("above the bound");

        match &error {
            DecodeError::TooLarge {
                size,
                pixels,
                limit,
                ..
            } => {
                assert_eq!(*size, Size::new(20_000, 20_000));
                assert_eq!(*pixels, 400_000_000);
                assert_eq!(*limit, MAX_PIXELS);
            }
            other => panic!("expected the pixel bound, got {other:?}"),
        }
        assert_eq!(
            Unusable::from(error),
            Unusable::TooLarge {
                size: Size::new(20_000, 20_000),
                pixels: 400_000_000,
                limit: MAX_PIXELS,
            }
        );
    }

    #[test]
    fn the_bound_admits_exactly_eight_k() {
        assert_eq!(MAX_PIXELS, Size::new(7680, 4320).pixels());
    }

    #[test]
    fn an_image_wider_than_the_dimension_bound_is_rejected_and_names_its_size() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // 40 000 × 8 is only 320 000 pixels, so the pixel bound alone would
        // admit it and no fixture of that shape is ever decoded.
        let path = dir.path().join("wide.png");
        fs::write(&path, header_only_png(40_000, 8)).expect("the fixture is written");

        let error = decode(&path).expect_err("wider than the dimension bound");

        match &error {
            DecodeError::TooLongSide { size, limit, .. } => {
                assert_eq!(*size, Size::new(40_000, 8));
                assert_eq!(*limit, MAX_DIMENSION);
            }
            other => panic!("expected the dimension bound, got {other:?}"),
        }

        // The diagnosis has to name the bound that was actually crossed: this
        // image is 320 000 pixels, so "above the 33177600-pixel bound" would be
        // a sentence about a limit it does not exceed.
        match Unusable::from(error) {
            Unusable::TooLongSide { size, limit } => {
                assert_eq!(size, Size::new(40_000, 8));
                assert_eq!(limit, MAX_DIMENSION);
            }
            other => panic!("expected the dimension bound, got {other:?}"),
        }
    }

    #[test]
    fn the_dimension_bound_admits_a_side_of_exactly_the_bound() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("edge.png");
        fs::write(&path, header_only_png(MAX_DIMENSION, 1)).expect("the fixture is written");

        assert_eq!(
            dimensions(&path).expect("the bound is inclusive"),
            Size::new(MAX_DIMENSION, 1)
        );
    }

    #[test]
    fn the_header_is_enough_to_reject_a_huge_image_without_decoding_it() {
        // The property that matters: the size comes from the header, so the
        // 400-megapixel buffer is never allocated at all.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("huge.png");
        fs::write(&path, header_only_png(20_000, 20_000)).expect("the fixture is written");

        assert_eq!(
            dimensions(&path).expect("the header is well formed"),
            Size::new(20_000, 20_000)
        );
    }

    #[test]
    fn a_frame_refuses_a_buffer_that_does_not_match_its_size() {
        let error = Frame::new(2, 2, vec![0; 15]).expect_err("15 is not 2 * 2 * 4");

        assert_eq!(
            error,
            FrameError::Length {
                expected: 16,
                found: 15
            }
        );
        assert!(Frame::new(2, 2, vec![0; 16]).is_ok());
    }

    #[test]
    fn a_frame_hands_its_pixels_over_without_copying() {
        let frame = Frame::new(1, 1, vec![1, 2, 3, 4]).expect("a well-formed frame");

        assert_eq!(frame.rgba(), &[1, 2, 3, 4]);
        assert_eq!(frame.into_rgba(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_frame_error_becomes_a_reason_the_interface_can_show() {
        let reason = Unusable::from(DecodeError::Malformed {
            path: std::path::PathBuf::from("/w.png"),
            source: FrameError::Length {
                expected: 16,
                found: 15,
            },
        });

        match reason {
            Unusable::Undecodable { detail } => assert!(detail.contains("16"), "`{detail}`"),
            other => panic!("expected an undecodable reason, got {other:?}"),
        }
    }

    #[test]
    fn each_failure_maps_onto_its_own_reason() {
        let unreadable =
            Unusable::from(decode(&std::path::PathBuf::from("/nope/none.png")).unwrap_err());
        assert!(matches!(unreadable, Unusable::Unreadable { .. }));

        let dir = tempfile::tempdir().expect("a temporary directory");
        let note = dir.path().join("note.png");
        let mut file = fs::File::create(&note).expect("a fixture");
        file.write_all(b"not an image").expect("a fixture");
        let undecodable = Unusable::from(decode(&note).unwrap_err());
        assert!(matches!(undecodable, Unusable::Undecodable { .. }));
    }

    #[test]
    fn the_not_found_helper_tells_absence_from_denial() {
        assert!(is_not_found(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!is_not_found(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }
}
