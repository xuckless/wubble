//! The scene on a real wgpu device with no window (any adapter, llvmpipe
//! included): 90 frames of fluid, two glass panels, a press wave and the
//! border beam, read back and written as a PPM for a visual check.
//!
//!     cargo run -p wubble-gfx --example headless -- out.ppm
//!
//! Source: the-wallpaper-project `src/wubble/slint/crate/examples/headless.rs`.

#[path = "../tests/support/headless.rs"]
mod headless;

use wubble_gfx::scene::Quality;

fn main() {
    let Some((device, queue, name)) = headless::device() else {
        eprintln!("no GPU adapter");
        std::process::exit(1);
    };
    let run = headless::Run {
        glass: true,
        quality: Quality::Full,
        frames: 90,
    };
    let (w, h, data) = headless::render(&device, &queue, run);
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for px in data.chunks(4) {
        ppm.extend_from_slice(&px[..3]);
    }
    let path = std::env::args().nth(1).unwrap_or("headless.ppm".into());
    std::fs::write(&path, ppm).expect("write the image");
    let c = (((h / 2) * w + w / 2) * 4) as usize;
    println!(
        "{name}: {w}x{h} centre rgba {:?} -> {path}",
        &data[c..c + 4]
    );
}
