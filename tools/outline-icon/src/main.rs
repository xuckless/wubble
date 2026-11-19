//! Writes wubble's symbolic icon: the mark's ring and W (`assets/icons/wubble-mark-16.svg`, the
//! small-size geometry) as strokes outlined into filled paths with `kurbo::stroke`, on a 16 px
//! grid, one colour (`#bebebe`, the GTK symbolic convention: the shell recolours it).

use kurbo::{Affine, BezPath, Cap, Join, Stroke, StrokeOpts};

/// The mark's 120-unit drawing scaled to the 16 px icon grid.
const SCALE: f64 = 16.0 / 120.0;
/// The ring (`wubble-mark.svg`'s outline), thick enough to read at 16 px.
const RING: &str = "M61 9C89 8 112 30 111 59C110 90 90 112 60 111C29 110 9 92 9 61C9 31 32 10 61 9Z";
const RING_WIDTH: f64 = 9.0;
/// The W (`wubble-mark-16.svg`).
const W: &str = "M28 44Q36 88 46 88Q53 88 60 64Q67 88 74 88Q84 88 92 44";
const W_WIDTH: f64 = 17.0;

fn outline(d: &str, width: f64) -> BezPath {
    let path = BezPath::from_svg(d).expect("the mark's path parses");
    let style = Stroke::new(width)
        .with_caps(Cap::Round)
        .with_join(Join::Round);
    let mut out = kurbo::stroke(path, &style, &StrokeOpts::default(), 0.05);
    out.apply_affine(Affine::scale(SCALE));
    out
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/icons/linux/hicolor/symbolic/apps/wubble-symbolic.svg".into());
    let ring = outline(RING, RING_WIDTH);
    let w = outline(W, W_WIDTH);
    let svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!-- wubble symbolic icon: the mark's ring and W outlined (tools/outline-icon). -->\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\">\n\
         <path fill=\"#bebebe\" d=\"{}\"/>\n\
         <path fill=\"#bebebe\" d=\"{}\"/>\n\
         </svg>\n",
        ring.to_svg(),
        w.to_svg()
    );
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).expect("the icon's folder");
    }
    std::fs::write(&out, svg).expect("the icon is written");
    println!("wrote {out}");
}
