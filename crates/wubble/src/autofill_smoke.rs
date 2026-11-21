//! `--smoke autofill`: a corpus of sign-in pages served from a loopback port
//! (the vault knows only http and https origins, so not `file:` or `data:`
//! pages), each modelled on the markup of a common site or pattern, with
//! logins seeded for their origins in a vault on the fake keyring. Most
//! fixtures sit on an origin of their own (`<name>.localhost`, which Chromium
//! resolves to loopback), so one fixture's saves never meet another's logins.
//!
//! Each fixture is a plan of steps, run in turn on the one tab: open a page,
//! wait for the key chip to offer exactly the expected rows, pick one as a
//! click on its row would, wait for the page to confirm in its title what its
//! own listeners saw (React-style value tracking and composed events from
//! shadow roots included), then submit as a user does (a trusted click at a
//! fixed point, or Enter through the host's key path, once a frame painted
//! since: Chromium drops input a document gets before its first paint), wait
//! for the save or update prompt, save, and read the vault back. Some check
//! that nothing is offered (a foreign frame, a one-time-code page). One line
//! per fixture: `smoke: autofill <name> ok`, or the failure; the smoke passes
//! when all did. Every wait is bounded by counts of CEF work runs, never by
//! time.
//!
//! `--smoke autofill=probe --url URL` (by hand): the same machinery on a real
//! page, read-only. A fake login is seeded for the URL's origin so a chip can
//! show; nothing is typed, picked or submitted. It prints what the chip offers
//! (with the autofill trace on) and passes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use wubble_engine::input::Point;

/// Runs of CEF work a step may take before its fixture fails.
pub const MAX_STEP_RUNS: u32 = 2_000;
/// Runs after a page said it is ready in which no chip may show.
pub const NO_OFFER_RUNS: u32 = 150;
/// Runs the probe waits for a chip on a real page.
pub const PROBE_RUNS: u32 = 3_000;

/// The basic page's seeded login, which its menu offers.
pub const SEEDED_USER: &str = "alice";
pub const SEEDED_PASSWORD: &str = "s3cret-seed";
/// What the basic page then submits: a login the vault does not hold yet.
pub const NEW_USER: &str = "carol";
pub const NEW_PASSWORD: &str = "c4rol-pw";
/// The page's title once the seeded login filled its visible fields.
pub const FILLED_TITLE: &str = "filled ok";
/// Inside every fixture's submit control (view-relative logical px; `fixture.css` `.go`).
pub const SUBMIT_AT: Point = Point { x: 120.0, y: 320.0 };
/// Inside a show-password toggle (`fixture.css` `.toggle`).
pub const TOGGLE_AT: Point = Point { x: 420.0, y: 95.0 };
/// Inside the field a fixture's Enter is typed in (`fixture.css` `.enter`).
pub const FIELD_AT: Point = Point { x: 100.0, y: 171.0 };
/// The menu row of the password generator.
pub const GENERATE: &str = "+generate";
/// The probe's fake login.
pub const PROBE_USER: &str = "probe-user";

const LOGIN_PAGE: &str = include_str!("../assets/autofill.html");
const WELCOME_PAGE: &str = "<!doctype html><meta charset=utf-8><title>welcome</title><p>Signed in.";
/// A sign-in form on another origin (`localhost`, not `127.0.0.1`), framed
/// by the login page: it never takes the page's offer.
const FRAME_PAGE: &str = "<!doctype html><meta charset=utf-8><form method=post action=/welcome>\
    <input name=u><input type=password name=p></form>";

/// The corpus pages, by path (any host serves them).
const PAGES: &[(&str, &str)] = &[
    ("/fixture.js", include_str!("../assets/autofill/fixture.js")),
    (
        "/fixture.css",
        include_str!("../assets/autofill/fixture.css"),
    ),
    ("/f/welcome", WELCOME_PAGE),
    (
        "/f/github/login",
        include_str!("../assets/autofill/github.html"),
    ),
    (
        "/f/wp/wp-login.php",
        include_str!("../assets/autofill/wordpress.html"),
    ),
    (
        "/f/google/identifier",
        include_str!("../assets/autofill/google-identifier.html"),
    ),
    (
        "/f/google/pwd",
        include_str!("../assets/autofill/google-password.html"),
    ),
    (
        "/f/live/login",
        include_str!("../assets/autofill/live.html"),
    ),
    (
        "/f/amazon/ap/signin",
        include_str!("../assets/autofill/amazon-email.html"),
    ),
    (
        "/f/amazon/ap/password",
        include_str!("../assets/autofill/amazon-password.html"),
    ),
    (
        "/f/reddit/login",
        include_str!("../assets/autofill/reddit.html"),
    ),
    (
        "/f/react/login",
        include_str!("../assets/autofill/react.html"),
    ),
    ("/f/signup", include_str!("../assets/autofill/signup.html")),
    (
        "/f/settings/password",
        include_str!("../assets/autofill/change.html"),
    ),
    ("/f/spa", include_str!("../assets/autofill/spa.html")),
    ("/f/late", include_str!("../assets/autofill/late.html")),
    ("/f/frames", include_str!("../assets/autofill/frames.html")),
    (
        "/f/frames/inner",
        include_str!("../assets/autofill/frames-inner.html"),
    ),
    ("/f/xframe", include_str!("../assets/autofill/xframe.html")),
    (
        "/f/xframe/inner",
        include_str!("../assets/autofill/frames-inner.html"),
    ),
    (
        "/f/xforeign",
        include_str!("../assets/autofill/xframe.html"),
    ),
    (
        "/f/xforeign/inner",
        include_str!("../assets/autofill/frames-inner.html"),
    ),
    (
        "/f/honeypot",
        include_str!("../assets/autofill/honeypot.html"),
    ),
    ("/f/otp", include_str!("../assets/autofill/otp.html")),
    (
        "/f/otp/code",
        include_str!("../assets/autofill/otp-code.html"),
    ),
    ("/f/picker", include_str!("../assets/autofill/picker.html")),
];

/// What a vault row's password must be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Want {
    Is(String),
    /// Changed from this (a generated one).
    Not(String),
}

/// One step of a fixture.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    /// Navigate the tab.
    Open(String),
    /// Wait for the chip to offer exactly `rows` (in any order; [`GENERATE`] for
    /// the generator), filling a password or not; then choose the row `pick`.
    Offer {
        rows: Vec<String>,
        password: bool,
        pick: Option<String>,
    },
    /// Wait for the page's title `ready`, then for [`NO_OFFER_RUNS`] without a chip.
    NoOffer { ready: String },
    /// Wait for the page's title.
    Title(String),
    /// A trusted click, once a frame painted since the step began.
    Click(Point),
    /// Enter through the host's key path, once a frame painted since the step began.
    Enter,
    /// Wait for the save (or update) prompt naming `user` on `origin`; answer Save.
    Prompt {
        update: bool,
        user: String,
        origin: String,
    },
    /// The vault holds `user` on `origin` with this password.
    Vault {
        origin: String,
        user: String,
        password: Want,
    },
    /// Print what the chip offers, as soon as it shows or after [`PROBE_RUNS`].
    Probe,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fixture {
    pub name: String,
    /// Seeded before the run: (origin, username, password).
    pub logins: Vec<(String, String, String)>,
    pub steps: Vec<Do>,
}

/// What the visible tab's key chip offers (it shows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferView {
    pub rows: Vec<String>,
    pub password: bool,
}

/// The prompt on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptView {
    pub update: bool,
    pub user: String,
    pub origin: String,
}

#[derive(Clone, Debug)]
pub enum Input<'a> {
    /// CEF did a run of work; what the visible tab's autofill shows now.
    Worked {
        offer: Option<OfferView>,
        prompt: Option<PromptView>,
        /// A new frame reached the screen on this run.
        painted: bool,
    },
    Title(&'a str),
    /// The vault holds what the `ReadBack` asked for.
    ReadBack(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(String),
    /// Open the key menu and choose this row.
    Pick(usize),
    Click(Point),
    Enter,
    /// Answer the prompt with Save.
    Save,
    ReadBack {
        origin: String,
        user: String,
        password: Want,
    },
    Print(String),
    Pass,
    Fail(String),
}

/// Where the run is: the fixture, its step, the runs the step took, whether
/// the step's condition has started (a `NoOffer`'s page is ready, a `Vault`
/// asked), and the fixtures that failed.
#[derive(Clone, Debug, PartialEq)]
pub struct AutofillSmoke {
    plan: Vec<Fixture>,
    fixture: usize,
    step: usize,
    runs: u32,
    begun: bool,
    failed: Vec<String>,
    done: bool,
}

impl AutofillSmoke {
    /// The run of `plan`. Its first step, when it opens a page, is the tab's
    /// first page ([`Self::start_url`]) and is not taken again.
    pub fn new(plan: Vec<Fixture>) -> Self {
        let skip = matches!(
            plan.first().and_then(|f| f.steps.first()),
            Some(Do::Open(_))
        );
        Self {
            plan,
            fixture: 0,
            step: usize::from(skip),
            runs: 0,
            begun: false,
            failed: vec![],
            done: false,
        }
    }

    /// A probe of a real page (its trace is printed).
    pub fn is_probe(&self) -> bool {
        self.plan.iter().any(|f| f.steps.contains(&Do::Probe))
    }

    pub fn start_url(&self) -> Option<&str> {
        match self.plan.first()?.steps.first()? {
            Do::Open(url) => Some(url),
            _ => None,
        }
    }

    fn current(&self) -> Option<&Do> {
        self.plan.get(self.fixture)?.steps.get(self.step)
    }

    fn name(&self) -> &str {
        self.plan.get(self.fixture).map_or("", |f| f.name.as_str())
    }

    /// The next step, or the next fixture with a line for this one.
    fn advance(mut self, mut actions: Vec<Action>) -> (Self, Vec<Action>) {
        self.step += 1;
        self.runs = 0;
        self.begun = false;
        if self.step >= self.plan[self.fixture].steps.len() {
            if !matches!(self.plan[self.fixture].steps.last(), Some(Do::Probe)) {
                actions.push(Action::Print(format!("smoke: autofill {} ok", self.name())));
            }
            return self.next_fixture(actions);
        }
        (self, actions)
    }

    fn fail(mut self, why: String) -> (Self, Vec<Action>) {
        let name = self.name().to_owned();
        self.failed.push(name.clone());
        self.next_fixture(vec![Action::Print(format!(
            "smoke: autofill {name} FAILED: {why}"
        ))])
    }

    fn next_fixture(mut self, mut actions: Vec<Action>) -> (Self, Vec<Action>) {
        self.fixture += 1;
        self.step = 0;
        self.runs = 0;
        self.begun = false;
        if self.fixture >= self.plan.len() {
            self.done = true;
            actions.push(if self.failed.is_empty() {
                Action::Pass
            } else {
                Action::Fail(format!(
                    "{} of {} fixtures failed: {}",
                    self.failed.len(),
                    self.plan.len(),
                    self.failed.join(", ")
                ))
            });
        }
        (self, actions)
    }
}

fn same_rows(seen: &[String], want: &[String]) -> bool {
    let mut seen = seen.to_vec();
    let mut want = want.to_vec();
    seen.sort();
    want.sort();
    seen == want
}

pub fn next(state: AutofillSmoke, input: Input) -> (AutofillSmoke, Vec<Action>) {
    if state.done {
        return (state, vec![]);
    }
    let Some(step) = state.current().cloned() else {
        return (state, vec![]);
    };
    match (&step, &input) {
        (Do::Open(url), Input::Worked { .. }) => state.advance(vec![Action::Open(url.clone())]),
        (
            Do::Offer {
                rows,
                password,
                pick,
            },
            Input::Worked { offer: Some(o), .. },
        ) if same_rows(&o.rows, rows) && o.password == *password => {
            let chosen = pick.as_ref().map(|p| o.rows.iter().position(|r| r == p));
            match chosen {
                Some(Some(i)) => state.advance(vec![Action::Pick(i)]),
                Some(None) => state.fail(format!("no row {pick:?} in {:?}", o.rows)),
                None => state.advance(vec![]),
            }
        }
        (Do::NoOffer { ready }, Input::Title(t)) if t == ready => {
            let mut state = state;
            state.begun = true;
            state.runs = 0;
            (state, vec![])
        }
        (Do::NoOffer { .. }, Input::Worked { offer: Some(o), .. }) if state.begun => {
            state.fail(format!("a chip offers {:?}", o.rows))
        }
        (Do::NoOffer { .. }, Input::Worked { .. })
            if state.begun && state.runs >= NO_OFFER_RUNS =>
        {
            state.advance(vec![])
        }
        (Do::Title(want), Input::Title(t)) if t == want => state.advance(vec![]),
        (Do::Click(at), Input::Worked { painted: true, .. }) => {
            state.advance(vec![Action::Click(*at)])
        }
        (Do::Enter, Input::Worked { painted: true, .. }) => state.advance(vec![Action::Enter]),
        (
            Do::Prompt {
                update,
                user,
                origin,
            },
            Input::Worked {
                prompt: Some(p), ..
            },
        ) if p.update == *update && p.user == *user && p.origin == *origin => {
            state.advance(vec![Action::Save])
        }
        (
            Do::Vault {
                origin,
                user,
                password,
            },
            Input::Worked { .. },
        ) if !state.begun => {
            let mut state = state;
            state.begun = true;
            let ask = Action::ReadBack {
                origin: origin.clone(),
                user: user.clone(),
                password: password.clone(),
            };
            (state, vec![ask])
        }
        (Do::Vault { .. }, Input::ReadBack(true)) => state.advance(vec![]),
        (Do::Vault { user, origin, .. }, Input::ReadBack(false)) => state.fail(format!(
            "the vault does not hold {user} on {origin} as submitted"
        )),
        (Do::Probe, Input::Worked { offer: Some(o), .. }) => {
            let line = format!(
                "smoke: autofill probe {}: chip offers {:?} (fills a password: {})",
                state.name(),
                o.rows,
                o.password
            );
            state.advance(vec![Action::Print(line)])
        }
        (Do::Probe, Input::Title(t)) => {
            let line = format!("smoke: autofill probe {}: title {t:?}", state.name());
            (state, vec![Action::Print(line)])
        }
        (Do::Probe, Input::Worked { .. }) if state.runs >= PROBE_RUNS => {
            let line = format!(
                "smoke: autofill probe {}: no chip after {PROBE_RUNS} runs of CEF work",
                state.name()
            );
            state.advance(vec![Action::Print(line)])
        }
        (_, Input::Worked { offer, prompt, .. })
            if state.runs >= MAX_STEP_RUNS && step != Do::Probe =>
        {
            let why = format!(
                "stuck at step {} ({step:?}) for {MAX_STEP_RUNS} runs of CEF work; \
                 chip {offer:?}, prompt {prompt:?}",
                state.step
            );
            state.fail(why)
        }
        (_, Input::Worked { .. }) => {
            let mut state = state;
            state.runs += 1;
            (state, vec![])
        }
        _ => (state, vec![]),
    }
}

// ------------------------------------------------------------ the corpus

fn s(v: &str) -> String {
    v.to_owned()
}

fn rows(v: &[&str]) -> Vec<String> {
    v.iter().map(|r| s(r)).collect()
}

fn pick(r: &[&str], password: bool, choose: &str) -> Do {
    Do::Offer {
        rows: rows(r),
        password,
        pick: Some(s(choose)),
    }
}

fn title(t: &str) -> Do {
    Do::Title(s(t))
}

fn prompt(update: bool, user: &str, origin: &str) -> Do {
    Do::Prompt {
        update,
        user: s(user),
        origin: s(origin),
    }
}

fn vault(origin: &str, user: &str, password: Want) -> Do {
    Do::Vault {
        origin: s(origin),
        user: s(user),
        password,
    }
}

fn is(p: &str) -> Want {
    Want::Is(s(p))
}

/// Every fixture, in run order, for the server on `port`.
pub fn corpus(port: u16) -> Vec<Fixture> {
    let at = |host: &str| format!("http://{host}:{port}");
    let fixture = |name: &str, host: &str, logins: &[(&str, &str)], steps: Vec<Do>| Fixture {
        name: s(name),
        logins: logins.iter().map(|(u, p)| (at(host), s(u), s(p))).collect(),
        steps,
    };
    let open = |host: &str, path: &str| Do::Open(format!("{}{path}", at(host)));
    let loopback = "127.0.0.1";
    vec![
        // The original page: a honeypot field, another origin's form framed beside it.
        fixture(
            "basic",
            loopback,
            &[(SEEDED_USER, SEEDED_PASSWORD)],
            vec![
                open(loopback, "/"),
                pick(&[SEEDED_USER], true, SEEDED_USER),
                title(FILLED_TITLE),
                Do::Click(SUBMIT_AT),
                prompt(false, NEW_USER, &at(loopback)),
                vault(&at(loopback), NEW_USER, is(NEW_PASSWORD)),
                vault(&at(loopback), SEEDED_USER, is(SEEDED_PASSWORD)),
            ],
        ),
        // Enter in the password field; a changed password is an update.
        fixture(
            "github",
            "github.localhost",
            &[("octocat", "0ld-octo")],
            vec![
                open("github.localhost", "/f/github/login"),
                pick(&["octocat"], true, "octocat"),
                title("filled ok"),
                Do::Click(FIELD_AT),
                Do::Enter,
                prompt(true, "octocat", &at("github.localhost")),
                vault(&at("github.localhost"), "octocat", is("n3w-octo")),
            ],
        ),
        // The show-password toggle is no submit; the field shown as text is still the password.
        fixture(
            "wordpress",
            "wp.localhost",
            &[("admin", "wp-admin-pw")],
            vec![
                open("wp.localhost", "/f/wp/wp-login.php"),
                pick(&["admin"], true, "admin"),
                title("filled ok"),
                Do::Click(TOGGLE_AT),
                title("shown"),
                Do::Click(SUBMIT_AT),
                prompt(false, "editor", &at("wp.localhost")),
                vault(&at("wp.localhost"), "editor", is("ed1tor-pw")),
            ],
        ),
        // Two pages: the identifier (a type=button Next outside the form), then the password
        // alone; the update is saved with the first page's username.
        fixture(
            "google",
            "google.localhost",
            &[("alice@gmail.test", "g00gle-pw")],
            vec![
                open("google.localhost", "/f/google/identifier"),
                pick(&["alice@gmail.test"], false, "alice@gmail.test"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                pick(&["alice@gmail.test"], true, "alice@gmail.test"),
                title("filled ok 2"),
                Do::Click(SUBMIT_AT),
                prompt(true, "alice@gmail.test", &at("google.localhost")),
                vault(
                    &at("google.localhost"),
                    "alice@gmail.test",
                    is("g00gle-new"),
                ),
            ],
        ),
        // One document: the password field revealed after the username step.
        fixture(
            "microsoft",
            "live.localhost",
            &[("bob@outlook.test", "ms-pw")],
            vec![
                open("live.localhost", "/f/live/login"),
                pick(&["bob@outlook.test"], false, "bob@outlook.test"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                pick(&["bob@outlook.test"], true, "bob@outlook.test"),
                title("filled ok 2"),
                Do::Click(SUBMIT_AT),
                prompt(true, "bob@outlook.test", &at("live.localhost")),
                vault(&at("live.localhost"), "bob@outlook.test", is("ms-new")),
            ],
        ),
        // Two pages, another account typed on the first: a new login under that username.
        fixture(
            "amazon",
            "amazon.localhost",
            &[("carol@amazon.test", "amz-pw")],
            vec![
                open("amazon.localhost", "/f/amazon/ap/signin"),
                pick(&["carol@amazon.test"], false, "carol@amazon.test"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                pick(&["carol@amazon.test"], true, "carol@amazon.test"),
                title("filled ok 2"),
                Do::Click(SUBMIT_AT),
                prompt(false, "dave@amazon.test", &at("amazon.localhost")),
                vault(&at("amazon.localhost"), "dave@amazon.test", is("dave-pw")),
            ],
        ),
        // Open shadow roots, nested, hydrated late; the submit inside a shadow root.
        fixture(
            "reddit",
            "reddit.localhost",
            &[("erin", "r3ddit")],
            vec![
                open("reddit.localhost", "/f/reddit/login"),
                pick(&["erin"], true, "erin"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                prompt(false, "erin2", &at("reddit.localhost")),
                vault(&at("reddit.localhost"), "erin2", is("r3ddit2")),
            ],
        ),
        // React-style value tracking; a single-page sign-in seen when the form goes.
        fixture(
            "react",
            "react.localhost",
            &[("frank@react.test", "re4ct")],
            vec![
                open("react.localhost", "/f/react/login"),
                pick(&["frank@react.test"], true, "frank@react.test"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                prompt(true, "frank@react.test", &at("react.localhost")),
                vault(&at("react.localhost"), "frank@react.test", is("re4ct-new")),
            ],
        ),
        // The site's saved login is never offered on its sign-up form; the generator is.
        fixture(
            "signup",
            "signup.localhost",
            &[("grace", "gr4ce-pw")],
            vec![
                open("signup.localhost", "/f/signup"),
                pick(&[GENERATE], false, GENERATE),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                prompt(false, "henry@signup.test", &at("signup.localhost")),
                vault(
                    &at("signup.localhost"),
                    "henry@signup.test",
                    Want::Not(s("gr4ce-pw")),
                ),
            ],
        ),
        // The current password filled, never the new ones; then generated new ones: an update.
        fixture(
            "change",
            "change.localhost",
            &[("ivan", "1van-old")],
            vec![
                open("change.localhost", "/f/settings/password"),
                pick(&["ivan", GENERATE], true, "ivan"),
                title("filled ok"),
                pick(&["ivan", GENERATE], true, GENERATE),
                title("generated ok"),
                Do::Click(SUBMIT_AT),
                prompt(true, "ivan", &at("change.localhost")),
                vault(&at("change.localhost"), "ivan", Want::Not(s("1van-old"))),
            ],
        ),
        // No form element; Enter signs in by script and the fields leave.
        fixture(
            "formless",
            "spa.localhost",
            &[("judy@spa.test", "judy-pw")],
            vec![
                open("spa.localhost", "/f/spa"),
                pick(&["judy@spa.test"], true, "judy@spa.test"),
                title("filled ok"),
                Do::Click(FIELD_AT),
                Do::Enter,
                prompt(true, "judy@spa.test", &at("spa.localhost")),
                vault(&at("spa.localhost"), "judy@spa.test", is("judy-new")),
            ],
        ),
        // The form inserted after load.
        fixture(
            "late",
            "late.localhost",
            &[("kate", "k4te-pw")],
            vec![
                open("late.localhost", "/f/late"),
                pick(&["kate"], true, "kate"),
                title("filled ok"),
            ],
        ),
        // A same-origin iframe; it signs in the whole page.
        fixture(
            "same-origin-frame",
            "frames.localhost",
            &[("mallory", "m4llory")],
            vec![
                open("frames.localhost", "/f/frames"),
                pick(&["mallory"], true, "mallory"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                prompt(false, "mike", &at("frames.localhost")),
                vault(&at("frames.localhost"), "mike", is("m1ke-pw")),
            ],
        ),
        // A cross-origin iframe on another loopback host: offered its own origin's login, not
        // the page's (alice, on 127.0.0.1), and saved for its origin.
        fixture(
            "cross-origin-frame",
            "localhost",
            &[("nina", "n1na-pw")],
            vec![
                open(loopback, "/f/xframe"),
                pick(&["nina"], true, "nina"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                prompt(false, "oscar", &at("localhost")),
                vault(&at("localhost"), "oscar", is("0scar-pw")),
            ],
        ),
        // A cross-origin iframe without logins of its own: nothing offered, not the page's.
        fixture(
            "foreign-frame",
            "evil.localhost",
            &[],
            vec![
                open(loopback, "/f/xforeign"),
                Do::NoOffer { ready: s("ready") },
            ],
        ),
        // Decoys: only the real fields are filled.
        fixture(
            "honeypot",
            "honey.localhost",
            &[("pat", "p4t-pw")],
            vec![
                open("honey.localhost", "/f/honeypot"),
                pick(&["pat"], true, "pat"),
                title("filled ok"),
            ],
        ),
        // A one-time code is no password; its page alone offers nothing.
        fixture(
            "one-time-code",
            "otp.localhost",
            &[("quinn", "qu1nn-pw")],
            vec![
                open("otp.localhost", "/f/otp"),
                pick(&["quinn"], true, "quinn"),
                title("filled ok"),
                Do::Click(SUBMIT_AT),
                Do::NoOffer {
                    ready: s("ready code"),
                },
            ],
        ),
        // Two logins for the site: the menu offers both, the second is filled.
        fixture(
            "picker",
            "picker.localhost",
            &[("rita", "r1ta-pw"), ("sam", "s4m-pw")],
            vec![
                open("picker.localhost", "/f/picker"),
                pick(&["rita", "sam"], true, "sam"),
                title("filled ok"),
            ],
        ),
    ]
}

/// The probe's plan: open `url` and print what the chip offers, with a fake
/// login seeded for its origin.
pub fn probe(url: &str) -> Vec<Fixture> {
    let origin = wubble_core::autofill::web_origin(url);
    vec![Fixture {
        name: s(url),
        logins: origin
            .map(|o| (o, s(PROBE_USER), s("probe-password")))
            .into_iter()
            .collect(),
        steps: vec![Do::Open(s(url)), Do::Probe],
    }]
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

/// A corpus page with its placeholders filled: the port, and for a framing
/// page the frame's host and path (`/f/xframe` frames `localhost`,
/// `/f/xforeign` frames `evil.localhost`).
fn corpus_page(path: &str, port: u16) -> Option<(String, &'static str)> {
    let (_, page) = PAGES.iter().find(|(p, _)| *p == path)?;
    let (frame_host, frame_path) = match path {
        "/f/xforeign" => ("evil.localhost", "xforeign"),
        _ => ("localhost", "xframe"),
    };
    let page = page
        .replace("{PORT}", &port.to_string())
        .replace("{FRAME_HOST}", frame_host)
        .replace("{FRAME_PATH}", frame_path);
    let kind = match path.rsplit('.').next() {
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        _ => "text/html",
    };
    Some((page, kind))
}

/// One request, one answer, then the connection closes.
fn answer(mut stream: TcpStream, port: u16) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = target.split('?').next().unwrap_or("/").to_owned();
    let mut length = 0;
    // `--smoke mcp` echoes the cookies a request carried
    let mut cookie = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if header.to_ascii_lowercase().starts_with("cookie:") {
            cookie = header["cookie:".len()..].trim().to_owned();
        }
    }
    // The posted form is read and dropped.
    reader.take(length).read_to_end(&mut Vec::new())?;
    if let Some((page, kind)) = corpus_page(&path, port) {
        return write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
    }
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
        // `--smoke pinch`'s page (`?prevent` read by the page itself).
        "/pinch" => crate::PINCH_PAGE.to_owned(),
        // `--smoke mcp`'s pages and redirects: an agent session navigates to them on loopback.
        p if p.starts_with("/mcp") => match crate::mcp_smoke::route(&target, port, &cookie) {
            Some(crate::mcp_smoke::Route::Page(page)) => page,
            Some(crate::mcp_smoke::Route::SetCookies { cookies, to }) => {
                let set: String = cookies
                    .iter()
                    .map(|c| format!("Set-Cookie: {c}\r\n"))
                    .collect();
                return write!(
                    stream,
                    "HTTP/1.1 302 Found\r\n{set}Location: {to}\r\nContent-Length: 0\r\n\
                     Connection: close\r\n\r\n"
                );
            }
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

    fn worked(offer: Option<(&[&str], bool)>, prompt: Option<PromptView>) -> Input<'static> {
        Input::Worked {
            offer: offer.map(|(r, password)| OfferView {
                rows: rows(r),
                password,
            }),
            prompt,
            painted: true,
        }
    }

    fn idle() -> Input<'static> {
        worked(None, None)
    }

    fn unpainted() -> Input<'static> {
        Input::Worked {
            offer: None,
            prompt: None,
            painted: false,
        }
    }

    fn run(state: AutofillSmoke, inputs: &[Input]) -> (AutofillSmoke, Vec<Action>) {
        inputs.iter().fold((state, vec![]), |(s, mut all), i| {
            let (s, a) = next(s, i.clone());
            all.extend(a);
            (s, all)
        })
    }

    fn one(steps: Vec<Do>) -> AutofillSmoke {
        AutofillSmoke::new(vec![Fixture {
            name: s("one"),
            logins: vec![],
            steps,
        }])
    }

    fn basic() -> AutofillSmoke {
        AutofillSmoke::new(corpus(8080).into_iter().take(1).collect())
    }

    const ORIGIN: &str = "http://127.0.0.1:8080";

    #[test]
    fn the_basic_fixture_passes_through_pick_fill_submit_save_and_read_back() {
        let smoke = basic();
        assert_eq!(smoke.start_url(), Some("http://127.0.0.1:8080/"));
        let new = PromptView {
            update: false,
            user: s(NEW_USER),
            origin: s(ORIGIN),
        };
        let (s, actions) = run(
            smoke,
            &[
                idle(),
                worked(Some((&[SEEDED_USER], true)), None),
                Input::Title("sign in"),
                Input::Title(FILLED_TITLE),
                idle(),
                idle(),
                worked(None, Some(new)),
                idle(),
                Input::ReadBack(true),
                idle(),
                Input::ReadBack(true),
            ],
        );
        assert!(s.done);
        assert_eq!(
            actions,
            [
                Action::Pick(0),
                Action::Click(SUBMIT_AT),
                Action::Save,
                Action::ReadBack {
                    origin: ORIGIN.into(),
                    user: NEW_USER.into(),
                    password: Want::Is(NEW_PASSWORD.into()),
                },
                Action::ReadBack {
                    origin: ORIGIN.into(),
                    user: SEEDED_USER.into(),
                    password: Want::Is(SEEDED_PASSWORD.into()),
                },
                Action::Print("smoke: autofill basic ok".into()),
                Action::Pass,
            ]
        );
    }

    #[test]
    fn a_title_before_the_pick_does_not_advance() {
        let (_, actions) = run(
            basic(),
            &[
                Input::Title(FILLED_TITLE),
                worked(Some((&[SEEDED_USER], true)), None),
            ],
        );
        assert_eq!(actions, [Action::Pick(0)]);
    }

    #[test]
    fn an_offer_must_match_its_rows_and_what_it_fills() {
        let smoke = one(vec![pick(&["a", "b"], true, "b")]);
        let (s, actions) = run(
            smoke,
            &[
                worked(Some((&["a"], true)), None),
                worked(Some((&["b", "a"], false)), None),
                worked(Some((&["b", "a"], true)), None),
            ],
        );
        assert_eq!(
            actions[0],
            Action::Pick(0),
            "rows in any order, picked by name"
        );
        assert!(s.done);
    }

    /// A fill can land before the page's first paint; a click then would be
    /// dropped by Chromium, so the submit waits for a frame painted after it.
    #[test]
    fn a_submit_waits_for_a_painted_frame() {
        let smoke = one(vec![title("t"), Do::Click(SUBMIT_AT), Do::Enter]);
        let (s, actions) = run(smoke, &[Input::Title("t"), unpainted(), unpainted()]);
        assert!(actions.is_empty());
        let (s, actions) = run(s, &[idle(), unpainted(), idle()]);
        assert_eq!(actions[..2], [Action::Click(SUBMIT_AT), Action::Enter]);
        assert!(s.done);
    }

    #[test]
    fn no_offer_waits_for_the_page_then_counts_runs_without_a_chip() {
        let smoke = one(vec![Do::NoOffer { ready: s("ready") }]);
        let (s, actions) = run(smoke.clone(), &vec![idle(); NO_OFFER_RUNS as usize + 5]);
        assert!(
            actions.is_empty() && !s.done,
            "not before the page is ready"
        );
        let mut inputs = vec![Input::Title("ready")];
        inputs.extend(vec![idle(); NO_OFFER_RUNS as usize + 1]);
        let (s, actions) = run(smoke.clone(), &inputs);
        assert!(s.done);
        assert_eq!(actions.last(), Some(&Action::Pass));
        let (_, actions) = run(
            smoke,
            &[
                Input::Title("ready"),
                worked(Some((&["alice"], true)), None),
            ],
        );
        assert!(matches!(actions.last(), Some(Action::Fail(_))));
    }

    #[test]
    fn a_failed_fixture_is_reported_and_the_next_one_runs() {
        let smoke = AutofillSmoke::new(vec![
            Fixture {
                name: s("first"),
                logins: vec![],
                steps: vec![title("never")],
            },
            Fixture {
                name: s("second"),
                logins: vec![],
                steps: vec![Do::Open(s("http://x.test/")), title("t")],
            },
        ]);
        let mut inputs = vec![idle(); MAX_STEP_RUNS as usize + 1];
        inputs.extend([idle(), Input::Title("t")]);
        let (state, actions) = run(smoke, &inputs);
        assert!(state.done);
        assert!(
            matches!(&actions[0], Action::Print(l) if l.starts_with("smoke: autofill first FAILED: stuck at step 0"))
        );
        assert_eq!(actions[1], Action::Open(s("http://x.test/")));
        assert_eq!(actions[2], Action::Print(s("smoke: autofill second ok")));
        assert!(
            matches!(&actions[3], Action::Fail(e) if e.contains("1 of 2") && e.contains("first"))
        );
    }

    #[test]
    fn a_wrong_prompt_or_vault_row_fails() {
        let smoke = one(vec![
            prompt(true, "bob", ORIGIN),
            vault(ORIGIN, "bob", is("pw")),
        ]);
        let wrong = PromptView {
            update: false,
            user: s("bob"),
            origin: s(ORIGIN),
        };
        let right = PromptView {
            update: true,
            ..wrong.clone()
        };
        let (s, actions) = run(smoke, &[worked(None, Some(wrong))]);
        assert!(actions.is_empty(), "a save prompt is not the update prompt");
        let (_, actions) = run(
            s,
            &[worked(None, Some(right)), idle(), Input::ReadBack(false)],
        );
        assert!(matches!(&actions[2], Action::Print(l) if l.contains("FAILED")));
    }

    #[test]
    fn the_probe_prints_what_it_saw_and_passes() {
        let smoke = AutofillSmoke::new(probe("https://example.test/login"));
        assert_eq!(smoke.start_url(), Some("https://example.test/login"));
        assert_eq!(
            smoke.plan[0].logins,
            [(
                s("https://example.test"),
                s(PROBE_USER),
                s("probe-password")
            )]
        );
        let (_, actions) = run(smoke.clone(), &[worked(Some((&[PROBE_USER], true)), None)]);
        assert!(matches!(&actions[0], Action::Print(l) if l.contains("chip offers")));
        assert_eq!(actions[1], Action::Pass);
        let (_, actions) = run(smoke, &vec![idle(); PROBE_RUNS as usize + 1]);
        assert!(matches!(&actions[0], Action::Print(l) if l.contains("no chip")));
    }

    #[test]
    fn every_fixture_page_is_served_and_names_what_the_plan_expects() {
        let plan = corpus(8080);
        let mut names: Vec<&str> = plan.iter().map(|f| f.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), plan.len(), "fixture names are unique");
        for f in &plan {
            let Some(Do::Open(url)) = f.steps.first() else {
                panic!("{} opens no page", f.name);
            };
            let path = &url[url.find("/f/").unwrap_or(url.len() - 1)..];
            let page = if path == "/" {
                LOGIN_PAGE.to_owned()
            } else {
                corpus_page(path, 8080).expect(path).0
            };
            for step in &f.steps {
                if let Do::Title(t) = step {
                    // The page (or the page it frames, or the next page) says it.
                    let said = PAGES.iter().any(|(_, p)| p.contains(&format!("\"{t}\"")))
                        || page.contains(t);
                    assert!(said, "{}: no page says {t:?}", f.name);
                }
            }
        }
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
        assert!(
            corpus_page("/f/xforeign", 1)
                .unwrap()
                .0
                .contains("http://evil.localhost:1/f/xforeign/inner")
        );
        assert!(
            corpus_page("/f/xframe", 1)
                .unwrap()
                .0
                .contains("http://localhost:1/f/xframe/inner")
        );
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
        let pwd = get("POST /f/google/pwd?x=1 HTTP/1.1\r\nContent-Length: 3\r\n\r\na=b");
        assert!(pwd.starts_with("HTTP/1.1 200") && pwd.contains("Passwd"));
        let js = get("GET /fixture.js HTTP/1.1\r\n\r\n");
        assert!(js.contains("Content-Type: text/javascript"));
        assert!(get("GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }
}
