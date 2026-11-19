//! A screenshot's bytes to their places, off the Slint thread: the PNG Chromium returned
//! (base64) to a file in the downloads folder and/or, decoded to RGBA, onto the clipboard; and
//! Copy Image's pixels onto the clipboard.
//!
//! One worker thread owns the clipboard for the app's life: on X11 and Wayland the copying
//! program serves the image until something else is copied, so its owner must outlive the copy.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc;

use base64::Engine as _;

/// The image of one job.
pub enum Picture {
    /// A PNG, base64 (`Page.captureScreenshot`'s `data`), written to `path` if given.
    Png {
        base64: String,
        path: Option<PathBuf>,
    },
    /// Straight RGBA rows (Copy Image's `View::download_image`).
    Rgba {
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    },
}

/// What to do with one screenshot or copied image.
pub struct Job {
    pub picture: Picture,
    /// Put it on the clipboard.
    pub clipboard: bool,
}

/// How it went: `None` where nothing was asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub file: Option<bool>,
    pub clipboard: Option<bool>,
    /// The image's size in pixels, if it decoded.
    pub size: Option<(u32, u32)>,
}

type Done = Box<dyn FnOnce(Saved) + Send>;

/// The worker; dropping it ends the thread.
pub struct Saver {
    jobs: mpsc::Sender<(Job, Done)>,
}

impl Saver {
    pub fn spawn() -> Self {
        let (jobs, rx) = mpsc::channel::<(Job, Done)>();
        let spawned = std::thread::Builder::new()
            .name("wubble-shots".into())
            .spawn(move || {
                let mut clipboard = None;
                for (job, done) in rx {
                    done(run(job, &mut clipboard));
                }
            });
        if let Err(e) = spawned {
            eprintln!("wubble: screenshot thread: {e}");
        }
        Self { jobs }
    }

    /// Runs `job` on the worker; `done` is called there with the outcome.
    pub fn save(&self, job: Job, done: impl FnOnce(Saved) + Send + 'static) {
        if let Err(mpsc::SendError((_, done))) = self.jobs.send((job, Box::new(done))) {
            done(Saved::default());
        }
    }
}

fn run(job: Job, clipboard: &mut Option<arboard::Clipboard>) -> Saved {
    // The file is written now; the pixels are decoded only for the clipboard.
    let (file, size, pixels) = match job.picture {
        Picture::Png { base64, path } => {
            let Ok(png) = base64::engine::general_purpose::STANDARD.decode(base64.trim()) else {
                eprintln!("wubble: screenshot: the image is not base64");
                return Saved::default();
            };
            let file = path.map(|path| {
                std::fs::write(&path, &png)
                    .map_err(|e| eprintln!("wubble: screenshot {}: {e}", path.display()))
                    .is_ok()
            });
            (file, png_size(&png), job.clipboard.then(|| rgba(&png)))
        }
        Picture::Rgba {
            width,
            height,
            bytes,
        } => (
            None,
            Some((width, height)),
            job.clipboard.then_some(Ok((width, height, bytes))),
        ),
    };
    let copied = pixels.map(|decoded| {
        let (width, height, bytes) = match decoded {
            Ok(image) => image,
            Err(e) => {
                eprintln!("wubble: screenshot: {e}");
                return false;
            }
        };
        let image = arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: bytes.into(),
        };
        let result = match clipboard {
            Some(c) => c.set_image(image),
            None => arboard::Clipboard::new().and_then(|mut c| {
                let r = c.set_image(image);
                *clipboard = Some(c);
                r
            }),
        };
        result
            .map_err(|e| eprintln!("wubble: image to the clipboard: {e}"))
            .is_ok()
    });
    Saved {
        file,
        clipboard: copied,
        size,
    }
}

/// The PNG's width and height, from its header alone.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let reader = png::Decoder::new(Cursor::new(png)).read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

/// The PNG as 8-bit RGBA rows, what the clipboard takes.
fn rgba(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    // A full page is large: allow up to 1 GiB of pixels.
    let limits = png::Limits { bytes: 1 << 30 };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(png), limits);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("an indexed image was not expanded".into()),
    };
    Ok((info.width, info.height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 x 1 RGB PNG.
    fn tiny_png() -> Vec<u8> {
        let mut out = vec![];
        let mut e = png::Encoder::new(&mut out, 2, 1);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        w.finish().unwrap();
        out
    }

    #[test]
    fn a_png_decodes_to_rgba_and_reports_its_size() {
        let png = tiny_png();
        assert_eq!(png_size(&png), Some((2, 1)));
        assert_eq!(
            rgba(&png).unwrap(),
            (2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255])
        );
        assert_eq!(png_size(b"nope"), None);
    }

    #[test]
    fn a_file_job_writes_the_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let png = tiny_png();
        let job = Job {
            picture: Picture::Png {
                base64: base64::engine::general_purpose::STANDARD.encode(&png),
                path: Some(path.clone()),
            },
            clipboard: false,
        };
        let saved = run(job, &mut None);
        assert_eq!(
            saved,
            Saved {
                file: Some(true),
                clipboard: None,
                size: Some((2, 1))
            }
        );
        assert_eq!(std::fs::read(path).unwrap(), png);
        let bad = Job {
            picture: Picture::Png {
                base64: "***".into(),
                path: None,
            },
            clipboard: false,
        };
        assert_eq!(run(bad, &mut None), Saved::default());
    }
}
