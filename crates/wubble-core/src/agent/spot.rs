//! Where an action on an element lands: from the element's boxes as the page lays them out now
//! (DevTools' `DOM.getContentQuads`, viewport CSS px, after `DOM.scrollIntoViewIfNeeded`), the
//! centre of the first box that shows in the viewport, or a point the agent names inside a
//! canvas.
//!
//! Design reference, re-expressed: puppeteer's `ElementHandle.clickablePoint`
//! (`packages/puppeteer-core/src/api/ElementHandle.ts`, Apache-2.0): the content quads cut to
//! the viewport, the first with an area above one pixel, its centre.

/// A box in viewport CSS px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn of(quad: &[f64; 8]) -> Rect {
        let xs = [quad[0], quad[2], quad[4], quad[6]];
        let ys = [quad[1], quad[3], quad[5], quad[7]];
        let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
        let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
        Rect {
            x: min(xs),
            y: min(ys),
            w: max(xs) - min(xs),
            h: max(ys) - min(ys),
        }
    }

    /// This box cut to a viewport of `size`; `None` when less than a pixel of it is left.
    fn cut(self, size: (f64, f64)) -> Option<Rect> {
        let (x0, y0) = (self.x.max(0.0), self.y.max(0.0));
        let (x1, y1) = ((self.x + self.w).min(size.0), (self.y + self.h).min(size.1));
        (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then_some(Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    fn centre(self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Why an action has no place to land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreachable {
    /// No box of the element shows in the viewport (zero size, hidden, clipped away).
    Hidden,
    /// The point named is outside the element's box.
    Outside,
    /// The point named is inside the element but out of the viewport.
    OffScreen,
}

impl Unreachable {
    pub fn note(self) -> &'static str {
        match self {
            Unreachable::Hidden => "the element has no visible box (zero size, hidden or clipped)",
            Unreachable::Outside => "the point is outside the canvas",
            Unreachable::OffScreen => "the point is inside the canvas but out of view",
        }
    }
}

/// `DOM.getContentQuads`' answer (`{"quads": [[x1, y1, .. x4, y4], ..]}`) as quads; malformed
/// entries are left out.
pub fn quads(answer: &serde_json::Value) -> Vec<[f64; 8]> {
    answer
        .get("quads")
        .and_then(serde_json::Value::as_array)
        .map(|qs| {
            qs.iter()
                .filter_map(|q| {
                    let v: Vec<f64> = q.as_array()?.iter().filter_map(|n| n.as_f64()).collect();
                    v.try_into().ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The part of the element that shows: its first box with a pixel in a viewport of `size`.
pub fn visible(quads: &[[f64; 8]], size: (f64, f64)) -> Option<Rect> {
    quads.iter().find_map(|q| Rect::of(q).cut(size))
}

/// Where an action lands: the centre of the element's visible part, or, with `offset`, that
/// point from the top-left of the element's first box (a canvas: the agent's coordinates mean
/// nothing anywhere else), which must lie inside the box and the viewport.
pub fn point(
    quads: &[[f64; 8]],
    size: (f64, f64),
    offset: Option<(f64, f64)>,
) -> Result<(f64, f64), Unreachable> {
    let Some((dx, dy)) = offset else {
        return visible(quads, size)
            .map(Rect::centre)
            .ok_or(Unreachable::Hidden);
    };
    let first = quads.first().map(Rect::of).ok_or(Unreachable::Hidden)?;
    if !(0.0..first.w).contains(&dx) || !(0.0..first.h).contains(&dy) {
        return Err(Unreachable::Outside);
    }
    let (x, y) = (first.x + dx, first.y + dy);
    if (0.0..size.0).contains(&x) && (0.0..size.1).contains(&y) {
        Ok((x, y))
    } else {
        Err(Unreachable::OffScreen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: (f64, f64) = (1000.0, 800.0);

    fn quad(x: f64, y: f64, w: f64, h: f64) -> [f64; 8] {
        [x, y, x + w, y, x + w, y + h, x, y + h]
    }

    #[test]
    fn quads_are_read_from_the_answer() {
        let v = serde_json::json!({"quads": [[16, 64, 138.5, 64, 138.5, 98, 16, 98], [1, 2]]});
        assert_eq!(
            quads(&v),
            [[16.0, 64.0, 138.5, 64.0, 138.5, 98.0, 16.0, 98.0]]
        );
        assert!(quads(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_centre_of_what_shows() {
        assert_eq!(
            point(&[quad(16.0, 64.0, 120.0, 34.0)], VIEW, None),
            Ok((76.0, 81.0))
        );
        // half above the top: the centre of the half that shows
        assert_eq!(
            point(&[quad(0.0, -20.0, 100.0, 40.0)], VIEW, None),
            Ok((50.0, 10.0))
        );
        // a wrapped link: its first line is out of view, its second shows
        let lines = [quad(900.0, -30.0, 100.0, 20.0), quad(0.0, -5.0, 50.0, 20.0)];
        assert_eq!(point(&lines, VIEW, None), Ok((25.0, 7.5)));
        for hidden in [
            vec![],
            vec![quad(10.0, 10.0, 0.0, 0.0)],
            vec![quad(0.0, 900.0, 10.0, 10.0)],
        ] {
            assert_eq!(point(&hidden, VIEW, None), Err(Unreachable::Hidden));
        }
    }

    #[test]
    fn a_point_inside_a_canvas() {
        let canvas = [quad(100.0, 700.0, 300.0, 200.0)];
        assert_eq!(point(&canvas, VIEW, Some((10.0, 20.0))), Ok((110.0, 720.0)));
        assert_eq!(
            point(&canvas, VIEW, Some((300.0, 0.0))),
            Err(Unreachable::Outside)
        );
        assert_eq!(
            point(&canvas, VIEW, Some((-1.0, 0.0))),
            Err(Unreachable::Outside)
        );
        assert_eq!(
            point(&canvas, VIEW, Some((10.0, 150.0))),
            Err(Unreachable::OffScreen)
        );
    }
}
