//! `--smoke autofill`: a sign-in page served from a loopback port (the vault
//! knows only http and https origins, so not a `file:` or `data:` page), and
//! a login seeded for that origin in a vault on the fake keyring. The smoke
//! opens the key menu and picks the login as a click on its row would; the
//! page confirms the fill in its title, then stands in for the user signing
//! in as someone else (it types another account). Once a frame painted after
//! the fill is on screen (the fill can land before the page's first paint,
//! and Chromium drops input a document gets before then), a click on the page's submit button posts the form; the next page has no form, so the save
//! prompt must offer the new login. The smoke saves it and reads the row
//! back from the vault. Bounded by counts of CEF work runs, never by time.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use wubble_engine::input::Point;

/// Runs of CEF work a step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// The login seeded in the vault, which the menu offers.
pub const SEEDED_USER: &str = "alice";
pub const SEEDED_PASSWORD: &str = "s3cret-seed";
/// What the page then submits: a login the vault does not hold yet.
pub const NEW_USER: &str = "carol";
pub const NEW_PASSWORD: &str = "c4rol-pw";
/// The page's title once the seeded login filled its visible fields (and
/// not its hidden one).
pub const FILLED_TITLE: &str = "filled ok";
/// Inside the page's submit button (view-relative logical px).
pub const SUBMIT_AT: Point = Point { x: 120.0, y: 320.0 };

const LOGIN_PAGE: &str = include_str!("../assets/autofill.html");
const WELCOME_PAGE: &str = "<!doctype html><meta charset=utf-8><title>welcome</title><p>Signed in.";
/// A sign-in form on another origin (`localhost`, not `127.0.0.1`), framed
/// by the login page: it must never be offered or saved.
const FRAME_PAGE: &str = "<!doctype html><meta charset=utf-8><form method=post action=/welcome>\
    <input name=u><input type=password name=p></form>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Waiting for the key chip with the seeded login.
    Chip,
    /// Picked: waiting for the page to see the fill.
    Filled,
    /// Filled: waiting for a frame the page painted since, as a user sees the
    /// page before clicking it (Chromium drops input that reaches a document
    /// before its first paint).
    Shown,
    /// Submitted: waiting for the save prompt.
    Prompt,
    /// Saved: waiting for the vault's row.
    ReadBack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutofillSmoke {
    At { step: Step, runs: u32 },
    Done,
}

impl Default for AutofillSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Chip,
            runs: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// CEF did a run of work; what the visible tab's autofill shows now.
    Worked {
        /// The key chip offers exactly the seeded login.
        chip: bool,
        /// The prompt asks to save a new login.
        save_prompt: bool,
        /// A new frame reached the screen on this run.
        painted: bool,
    },
    Title(&'a str),
    /// The vault holds the new login with its password, and the seeded one.
    ReadBack(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the key menu and choose its first row.
    Pick,
    /// Click the page's submit button.
    Submit,
    /// Answer the prompt with Save.
    Save,
    /// Ask the vault for the rows.
    ReadBack,
    Pass,
    Fail(String),
}

pub fn next(state: AutofillSmoke, input: Input) -> (AutofillSmoke, Vec<Action>) {
    let AutofillSmoke::At { step, runs } = state else {
        return (state, vec![]);
    };
    let to = |step| AutofillSmoke::At { step, runs: 0 };
    match (step, input) {
        (Step::Chip, Input::Worked { chip: true, .. }) => (to(Step::Filled), vec![Action::Pick]),
        (Step::Filled, Input::Title(title)) if title == FILLED_TITLE => (to(Step::Shown), vec![]),
        (Step::Shown, Input::Worked { painted: true, .. }) => {
            (to(Step::Prompt), vec![Action::Submit])
        }
        (
            Step::Prompt,
            Input::Worked {
                save_prompt: true, ..
            },
        ) => (to(Step::ReadBack), vec![Action::Save, Action::ReadBack]),
        (Step::ReadBack, Input::ReadBack(true)) => (AutofillSmoke::Done, vec![Action::Pass]),
        (Step::ReadBack, Input::ReadBack(false)) => (
            AutofillSmoke::Done,
            vec![Action::Fail(format!(
                "the vault does not hold {NEW_USER} with the submitted password next to {SEEDED_USER}"
            ))],
        ),
        (_, Input::Worked { .. }) if runs >= MAX_STEP_RUNS => (
            AutofillSmoke::Done,
            vec![Action::Fail(format!(
                "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work"
            ))],
        ),
        (_, Input::Worked { .. }) => (
            AutofillSmoke::At {
                step,
                runs: runs + 1,
            },
            vec![],
        ),
        _ => (state, vec![]),
    }
}

/// Serves the smoke's pages on `127.0.0.1` at a free port, from a thread that
/// ends with the process; the port. Loopback only, nothing is fetched from
/// outside. Each connection is answered on its own thread: Chromium opens
/// connections ahead of its requests (preconnects) and can hold one idle for
/// minutes, which would stall every later request queued behind it.
pub fn serve() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::Builder::new()
        .name("autofill-smoke-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let reply = move || {
                    if let Err(e) = answer(stream, port) {
                        eprintln!("smoke: autofill page server: {e}");
                    }
                };
                if let Err(e) = std::thread::Builder::new()
                    .name("autofill-smoke-http-conn".into())
                    .spawn(reply)
                {
                    eprintln!("smoke: autofill page server: {e}");
                }
            }
        })?;
    Ok(port)
}

const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// One request, one answer, then the connection closes.
fn answer(mut stream: TcpStream, port: u16) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    // The posted form is read and dropped.
    reader.take(length).read_to_end(&mut Vec::new())?;
    let page = match path.as_str() {
        "/" => LOGIN_PAGE.replace("{PORT}", &port.to_string()),
        "/welcome" => WELCOME_PAGE.to_owned(),
        "/frame" => FRAME_PAGE.to_owned(),
        // `--smoke block` is served from here too: it needs an http page.
        "/block" | "/block/next" => crate::BLOCK_PAGE.to_owned(),
        // `--measure memory`'s article, on every `t<n>.localhost` site.
        "/memory" => crate::ARTICLE_PAGE.to_owned(),
        // `--smoke parity`'s page, in a secure context (the async Clipboard API needs one;
        // a `data:` page is not), and the copy button it frames from another origin.
        "/parity" => crate::PARITY_PAGE.to_owned(),
        "/parity/frame" => crate::parity::FRAME_PAGE.to_owned(),
        // `--smoke mcp`'s pages and redirects: an agent session navigates to them on loopback.
        p if p.starts_with("/mcp") => match crate::mcp_smoke::route(p, port) {
            Some(crate::mcp_smoke::Route::Page(page)) => page,
            Some(crate::mcp_smoke::Route::Redirect(to)) => {
                return write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\
                     Connection: close\r\n\r\n"
                );
            }
            None => return stream.write_all(NOT_FOUND),
        },
        _ => return stream.write_all(NOT_FOUND),
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worked(chip: bool, save_prompt: bool) -> Input<'static> {
        Input::Worked {
            chip,
            save_prompt,
            painted: true,
        }
    }

    fn unpainted() -> Input<'static> {
        Input::Worked {
            chip: false,
            save_prompt: false,
            painted: false,
        }
    }

    fn run(inputs: &[Input]) -> (AutofillSmoke, Vec<Action>) {
        inputs
            .iter()
            .fold((AutofillSmoke::default(), vec![]), |(s, mut all), &i| {
                let (s, a) = next(s, i);
                all.extend(a);
                (s, all)
            })
    }

    #[test]
    fn passes_through_pick_fill_submit_save_and_read_back() {
        let (s, actions) = run(&[
            worked(false, false),
            worked(true, false),
            Input::Title("sign in"),
            Input::Title(FILLED_TITLE),
            worked(false, false),
            worked(false, false),
            worked(false, true),
            Input::ReadBack(true),
        ]);
        assert_eq!(s, AutofillSmoke::Done);
        assert_eq!(
            actions,
            [
                Action::Pick,
                Action::Submit,
                Action::Save,
                Action::ReadBack,
                Action::Pass
            ]
        );
    }

    #[test]
    fn a_title_before_the_pick_does_not_submit() {
        let (_, actions) = run(&[Input::Title(FILLED_TITLE), worked(true, false)]);
        assert_eq!(actions, [Action::Pick]);
    }

    /// A fill can land before the page's first paint; a click then would be
    /// dropped by Chromium, so the submit waits for a frame painted after it.
    #[test]
    fn the_submit_waits_for_the_page_on_screen() {
        let (s, actions) = run(&[
            worked(true, false),
            Input::Title(FILLED_TITLE),
            unpainted(),
            unpainted(),
        ]);
        assert_eq!(actions, [Action::Pick]);
        assert_eq!(
            s,
            AutofillSmoke::At {
                step: Step::Shown,
                runs: 2
            }
        );
        let (_, actions) = next(s, worked(false, false));
        assert_eq!(actions, [Action::Submit]);
    }

    #[test]
    fn a_missing_row_fails() {
        let (s, actions) = run(&[
            worked(true, false),
            Input::Title(FILLED_TITLE),
            worked(false, false),
            worked(false, true),
            Input::ReadBack(false),
        ]);
        assert_eq!(s, AutofillSmoke::Done);
        assert!(matches!(actions.last(), Some(Action::Fail(_))));
    }

    #[test]
    fn a_stuck_step_fails_once() {
        let mut inputs = vec![worked(true, false)];
        inputs.extend(vec![worked(false, false); MAX_STEP_RUNS as usize + 3]);
        let (s, actions) = run(&inputs);
        assert_eq!(s, AutofillSmoke::Done);
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[1], Action::Fail(e) if e.contains("Filled")));
    }

    #[test]
    fn the_page_matches_the_smoke() {
        for needle in [
            SEEDED_USER,
            SEEDED_PASSWORD,
            NEW_USER,
            NEW_PASSWORD,
            FILLED_TITLE,
            "{PORT}",
            "/welcome",
        ] {
            assert!(LOGIN_PAGE.contains(needle), "{needle}");
        }
    }

    #[test]
    fn the_server_answers_each_page() {
        let port = serve().unwrap();
        let get = |request: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let login = get("GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(login.starts_with("HTTP/1.1 200"));
        assert!(login.contains(&format!("localhost:{port}/frame")));
        let welcome = get("POST /welcome HTTP/1.1\r\nContent-Length: 5\r\n\r\nu=a&p");
        assert!(welcome.contains("<title>welcome</title>"));
        assert!(get("GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }
}
