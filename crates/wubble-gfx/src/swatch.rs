//! The colour check of the bundled smoke page (`crates/wubble/assets/smoke.html`,
//! `.swatches`): each swatch must show the exact colour the page asks for,
//! which catches a wrong channel order or a gamma error (mid grey reads 188
//! after a double sRGB encode). Used by `--smoke frames` on a window snapshot
//! and by `wubble-engine`'s windowless `frame_smoke` example on a probed
//! frame. A check, never on the frame path.

/// Each swatch: name, centre in page pixels, and the colour the page asks for.
const SWATCHES: [(&str, u32, u32, [u8; 3]); 4] = [
    ("red", 56, 56, [255, 0, 0]),
    ("green", 128, 56, [0, 255, 0]),
    ("blue", 200, 56, [0, 0, 255]),
    ("grey", 272, 56, [128, 128, 128]),
];

/// Largest per-channel error allowed for a swatch.
const TOLERANCE: u8 = 2;

/// Fails unless each swatch shows its colour. `read` gives the RGB at a page
/// pixel, `None` when it is outside what was captured.
pub fn check(read: impl Fn(u32, u32) -> Option<[u8; 3]>) -> Result<(), String> {
    for (name, x, y, want) in SWATCHES {
        let got = read(x, y).ok_or_else(|| format!("the {name} swatch is outside the capture"))?;
        if got.iter().zip(want).any(|(g, w)| g.abs_diff(w) > TOLERANCE) {
            return Err(format!(
                "the {name} swatch shows {got:?}, the page asked for {want:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(grey: u8) -> impl Fn(u32, u32) -> Option<[u8; 3]> {
        move |x, _| {
            Some(match x {
                56 => [255, 0, 0],
                128 => [0, 255, 0],
                200 => [0, 0, 255],
                _ => [grey; 3],
            })
        }
    }

    #[test]
    fn exact_and_near_colours_pass() {
        assert!(check(page(128)).is_ok());
        assert!(check(page(130)).is_ok());
    }

    #[test]
    fn double_encoded_grey_fails() {
        assert!(check(page(188)).unwrap_err().contains("grey"));
    }

    #[test]
    fn outside_the_capture_fails() {
        assert!(check(|_, _| None).unwrap_err().contains("outside"));
    }
}
