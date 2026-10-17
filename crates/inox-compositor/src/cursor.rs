//! Token-coloured software cursor, used when no client cursor surface is active.

use inox_config::{Rgba, Theme};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};

pub(crate) fn fallback(theme: &Theme) -> MemoryRenderBuffer {
    let (width, height, pixels) = arrow(theme);
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (width, height),
        1,
        Transform::Normal,
        None,
    )
}

fn arrow(theme: &Theme) -> (i32, i32, Vec<u8>) {
    // Bound allocations for untrusted theme tokens; normal dimensions use lg/xl.
    let width = i32::from(theme.spacing.lg).clamp(1, 256);
    let height = i32::from(theme.spacing.xl).clamp(1, 256);
    let mut pixels = vec![0; (width * height * 4) as usize];
    let outline = premultiplied(theme.colors.background);
    let fill = premultiplied(theme.colors.text);
    for y in 0..height {
        let edge = (y * width / height).min(width - 1);
        for x in 0..=edge {
            let pixel = if x == 0 || x == edge || y == height - 1 {
                outline
            } else {
                fill
            };
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    (width, height, pixels)
}

fn premultiplied(color: Rgba) -> [u8; 4] {
    let channel = |value: u8| (u16::from(value) * u16::from(color.a) / 255) as u8;
    [
        channel(color.b),
        channel(color.g),
        channel(color.r),
        color.a,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_uses_theme_tokens_and_has_a_transparent_outline() {
        let theme = Theme::builtin_light();
        let (width, height, bytes) = arrow(&theme);
        assert_eq!(width, i32::from(theme.spacing.lg));
        assert_eq!(height, i32::from(theme.spacing.xl));
        assert_eq!(bytes.len(), (width * height * 4) as usize);
        assert_eq!(&bytes[..4], &premultiplied(theme.colors.background));
        assert_eq!(
            &bytes[(width as usize - 1) * 4..width as usize * 4],
            &[0; 4]
        );
        assert!(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel == premultiplied(theme.colors.text))
        );
    }
}
