//! The one definition of a usable page icon, shared by the engine (which
//! produces icons) and the store (which keeps them).

/// Largest icon side kept, in pixels.
pub const MAX_ICON_PX: u32 = 64;

/// True if `width` x `height` is a usable icon size (both sides in
/// `1..=MAX_ICON_PX`) and `len` is exactly `width * height * 4` RGBA bytes.
pub fn valid_rgba(width: u32, height: u32, len: usize) -> bool {
    let side = 1..=MAX_ICON_PX;
    side.contains(&width) && side.contains(&height) && len == (width * height * 4) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_lengths() {
        assert!(valid_rgba(2, 3, 24));
        assert!(valid_rgba(64, 64, 64 * 64 * 4));
        assert!(!valid_rgba(0, 3, 0));
        assert!(!valid_rgba(65, 1, 65 * 4));
        assert!(!valid_rgba(2, 3, 23));
    }
}
