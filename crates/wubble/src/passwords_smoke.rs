//! `--smoke passwords`: the passwords page as the owner uses it, across restarts, on a throwaway
//! profile whose vault key sits in a file keyring (`wubble_vault::FileKeyRing`, never the OS
//! one). Three processes on the same profile:
//! 1. Write: open the page, unlock it, add two logins by hand (one on `http` with a port), edit
//!    one (username and password), reveal it, delete the other, leave the page (it locks and
//!    forgets what it showed), come back, unlock, add a third, quit.
//! 2. Read: a new process unlocks and finds exactly the two logins, the edited one revealing
//!    its new password.
//! 3. Broken: the keyring cannot be reached; the page says so (unavailable, the reason shown),
//!    lists nothing, and an add is refused with a notice instead of pretending to save.
//! 4. Lost: the key is gone from the keyring (the parent removed the key file), the two logins
//!    are not: the page says they wait for their key, an add is refused, and no new key is
//!    made over them (the parent checks the key file is still absent).
//!
//! Every step is the page's own callback, as a click sends it. Each wait is "until the host
//! sees X", bounded by a count of CEF work runs, never by time.

use crate::nav_smoke::MAX_WORK_RUNS;

/// Which process of the smoke this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultPhase {
    Write,
    Read,
    Broken,
    Lost,
}

impl VaultPhase {
    /// Its `--smoke passwords-phase=N` argument.
    pub fn arg(self) -> &'static str {
        match self {
            VaultPhase::Write => "passwords-phase=1",
            VaultPhase::Read => "passwords-phase=2",
            VaultPhase::Broken => "passwords-phase=3",
            VaultPhase::Lost => "passwords-phase=4",
        }
    }
}

/// The logins the smoke types. `B` is edited from `B_USER`/`B_OLD` to `B_NEW_USER`/`B_NEW`.
pub const A_SITE: &str = "example.com";
pub const A_USER: &str = "alice";
pub const A_PASSWORD: &str = "a1ice-pw";
pub const B_SITE: &str = "http://127.0.0.1:8080";
pub const B_USER: &str = "bob";
pub const B_OLD: &str = "b0b-old";
pub const B_NEW_USER: &str = "bobby";
pub const B_NEW: &str = "b0b-new";
pub const C_SITE: &str = "https://c.test";
pub const C_USER: &str = "carol";
pub const C_PASSWORD: &str = "car0l-pw";

/// What the page shows of its vault.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shown {
    #[default]
    Locked,
    Unlocked,
    Unavailable,
    Unreadable,
    KeyLost,
}

/// One login row as the page draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub origin: String,
    pub username: String,
    /// The password the row shows, if it shows one.
    pub revealed: Option<String>,
}

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub ready: bool,
    /// The visible tab shows the passwords page.
    pub on_page: bool,
    pub shown: Shown,
    /// The status card's body.
    pub body: String,
    /// The line under the status card (an import's report, a refusal).
    pub notice: String,
    pub rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Waiting for the page, locked and empty.
    Opened,
    Unlocking,
    Adding,
    Editing,
    Revealing,
    Deleting,
    /// Left for another tab.
    Left,
    /// Back on the page: it must be locked and empty again.
    Back,
    Relocking,
    AddingMore,
    /// Broken: an add was asked; waiting for its refusal.
    Refusing,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordsSmoke {
    pub phase: VaultPhase,
    pub stage: Stage,
    runs: u32,
}

impl PasswordsSmoke {
    pub fn new(phase: VaultPhase) -> Self {
        Self {
            phase,
            stage: Stage::Start,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenPage,
    /// Another tab (the start page): the passwords page is left.
    Leave,
    Unlock,
    Add {
        site: &'static str,
        user: &'static str,
        password: &'static str,
    },
    Edit {
        id: String,
        site: &'static str,
        user: &'static str,
        password: &'static str,
    },
    Reveal(String),
    Delete(String),
    Pass,
    Fail(String),
}

fn has(rows: &[Row], origin: &str, user: &str) -> Option<String> {
    rows.iter()
        .find(|r| r.origin == origin && r.username == user)
        .map(|r| r.id.clone())
}

/// The two logins the write phase leaves, as listed (`https://` added to a bare site).
fn kept(rows: &[Row]) -> bool {
    rows.len() == 2
        && has(rows, B_SITE, B_NEW_USER).is_some()
        && has(rows, C_SITE, C_USER).is_some()
}

pub fn next(s: PasswordsSmoke, seen: &Seen) -> (PasswordsSmoke, Vec<Action>) {
    use Stage::*;
    let to = |stage| PasswordsSmoke {
        phase: s.phase,
        stage,
        runs: 0,
    };
    let a_origin = format!("https://{A_SITE}");
    let locked_empty = seen.on_page && seen.shown == Shown::Locked && seen.rows.is_empty();
    let unlocked = seen.on_page && seen.shown == Shown::Unlocked;
    let step: Option<(Stage, Vec<Action>)> = match (s.phase, s.stage) {
        (_, Start) if seen.ready => Some((Opened, vec![Action::OpenPage])),
        (_, Opened) if locked_empty => Some((Unlocking, vec![Action::Unlock])),
        (_, Opened | Back)
            if seen.on_page && !seen.rows.is_empty() && seen.shown == Shown::Locked =>
        {
            Some((
                Done,
                vec![Action::Fail(format!(
                    "a locked page lists logins: {seen:?}"
                ))],
            ))
        }
        // write
        (VaultPhase::Write, Unlocking) if unlocked && seen.rows.is_empty() => Some((
            Adding,
            vec![
                Action::Add {
                    site: A_SITE,
                    user: A_USER,
                    password: A_PASSWORD,
                },
                Action::Add {
                    site: B_SITE,
                    user: B_USER,
                    password: B_OLD,
                },
            ],
        )),
        (VaultPhase::Write, Adding) if unlocked && seen.rows.len() == 2 => {
            match (
                has(&seen.rows, &a_origin, A_USER),
                has(&seen.rows, B_SITE, B_USER),
            ) {
                (Some(_), Some(b)) => Some((
                    Editing,
                    vec![Action::Edit {
                        id: b,
                        site: B_SITE,
                        user: B_NEW_USER,
                        password: B_NEW,
                    }],
                )),
                _ => Some((
                    Done,
                    vec![Action::Fail(format!("the added logins: {seen:?}"))],
                )),
            }
        }
        (VaultPhase::Write, Editing) if unlocked => has(&seen.rows, B_SITE, B_NEW_USER)
            .filter(|_| seen.rows.len() == 2)
            .map(|b| (Revealing, vec![Action::Reveal(b)])),
        (VaultPhase::Write, Revealing) if unlocked => {
            let b = seen.rows.iter().find(|r| r.username == B_NEW_USER);
            match b.and_then(|r| r.revealed.as_deref()) {
                Some(B_NEW) => {
                    has(&seen.rows, &a_origin, A_USER).map(|a| (Deleting, vec![Action::Delete(a)]))
                }
                Some(other) => Some((
                    Done,
                    vec![Action::Fail(format!(
                        "the edited login reveals {} characters, not its new password",
                        other.len()
                    ))],
                )),
                None => None,
            }
        }
        (VaultPhase::Write, Deleting) if unlocked && seen.rows.len() == 1 => {
            Some((Left, vec![Action::Leave]))
        }
        (VaultPhase::Write, Left) if seen.ready && !seen.on_page => {
            Some((Back, vec![Action::OpenPage]))
        }
        (VaultPhase::Write, Back) if locked_empty => Some((Relocking, vec![Action::Unlock])),
        (VaultPhase::Write, Relocking) if unlocked && seen.rows.len() == 1 => {
            if has(&seen.rows, B_SITE, B_NEW_USER).is_some() {
                Some((
                    AddingMore,
                    vec![Action::Add {
                        site: C_SITE,
                        user: C_USER,
                        password: C_PASSWORD,
                    }],
                ))
            } else {
                Some((
                    Done,
                    vec![Action::Fail(format!("after the relock: {seen:?}"))],
                ))
            }
        }
        (VaultPhase::Write, AddingMore) if unlocked && kept(&seen.rows) => {
            Some((Done, vec![Action::Pass]))
        }
        // read
        (VaultPhase::Read, Unlocking) if unlocked && !seen.rows.is_empty() => {
            if kept(&seen.rows) {
                has(&seen.rows, B_SITE, B_NEW_USER).map(|b| (Revealing, vec![Action::Reveal(b)]))
            } else {
                Some((
                    Done,
                    vec![Action::Fail(format!("after the restart: {seen:?}"))],
                ))
            }
        }
        (VaultPhase::Read, Revealing) if unlocked => {
            let b = seen.rows.iter().find(|r| r.username == B_NEW_USER);
            match b.and_then(|r| r.revealed.as_deref()) {
                Some(B_NEW) => Some((Done, vec![Action::Pass])),
                Some(_) => Some((
                    Done,
                    vec![Action::Fail(
                        "the login reveals another password after the restart".into(),
                    )],
                )),
                None => None,
            }
        }
        (_, Unlocking) if seen.on_page && seen.shown == Shown::Unreadable => Some((
            Done,
            vec![Action::Fail(format!(
                "the saved logins cannot be read: {seen:?}"
            ))],
        )),
        // broken keyring
        (VaultPhase::Broken, Unlocking) if seen.on_page && seen.shown == Shown::Unavailable => {
            if !seen.rows.is_empty() || !seen.body.contains("cannot be used") {
                Some((
                    Done,
                    vec![Action::Fail(format!("the unavailable page: {seen:?}"))],
                ))
            } else {
                Some((
                    Refusing,
                    vec![Action::Add {
                        site: C_SITE,
                        user: C_USER,
                        password: C_PASSWORD,
                    }],
                ))
            }
        }
        // the key gone, the logins kept
        (VaultPhase::Lost, Unlocking) if seen.on_page && seen.shown == Shown::KeyLost => {
            if !seen.rows.is_empty()
                || !seen
                    .body
                    .starts_with("2 saved logins were saved with a key")
            {
                Some((
                    Done,
                    vec![Action::Fail(format!("the key-lost page: {seen:?}"))],
                ))
            } else {
                Some((
                    Refusing,
                    vec![Action::Add {
                        site: C_SITE,
                        user: C_USER,
                        password: C_PASSWORD,
                    }],
                ))
            }
        }
        (VaultPhase::Lost, Unlocking) if unlocked => Some((
            Done,
            vec![Action::Fail(
                "a lost key was replaced: the vault unlocked over logins sealed with another"
                    .into(),
            )],
        )),
        (VaultPhase::Lost, Refusing) if seen.on_page && !seen.notice.is_empty() => {
            if seen.rows.is_empty() && seen.shown == Shown::KeyLost {
                Some((Done, vec![Action::Pass]))
            } else {
                Some((
                    Done,
                    vec![Action::Fail(format!("after a refused add: {seen:?}"))],
                ))
            }
        }
        (VaultPhase::Broken, Unlocking) if unlocked => Some((
            Done,
            vec![Action::Fail("a broken keyring unlocked the vault".into())],
        )),
        (VaultPhase::Broken, Refusing) if seen.on_page && !seen.notice.is_empty() => {
            if seen.rows.is_empty() && seen.shown == Shown::Unavailable {
                Some((Done, vec![Action::Pass]))
            } else {
                Some((
                    Done,
                    vec![Action::Fail(format!("after a refused add: {seen:?}"))],
                ))
            }
        }
        _ => None,
    };
    match step {
        Some((stage, actions)) => (to(stage), actions),
        None if s.stage == Done => (s, vec![]),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Done),
            vec![Action::Fail(format!(
                "{:?} stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.phase, s.stage
            ))],
        ),
        None => (
            PasswordsSmoke {
                runs: s.runs + 1,
                ..s
            },
            vec![],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, origin: &str, user: &str, revealed: Option<&str>) -> Row {
        Row {
            id: id.into(),
            origin: origin.into(),
            username: user.into(),
            revealed: revealed.map(Into::into),
        }
    }

    fn page(shown: Shown, rows: Vec<Row>) -> Seen {
        Seen {
            ready: true,
            on_page: true,
            shown,
            rows,
            ..Seen::default()
        }
    }

    fn drive(phase: VaultPhase, seen: &[Seen]) -> (PasswordsSmoke, Vec<Action>) {
        seen.iter()
            .fold((PasswordsSmoke::new(phase), vec![]), |(s, mut all), x| {
                let (s, a) = next(s, x);
                all.extend(a);
                (s, all)
            })
    }

    const A: &str = "https://example.com";

    #[test]
    fn the_write_phase_adds_edits_reveals_deletes_relocks_and_adds() {
        let two = || vec![row("a", A, A_USER, None), row("b", B_SITE, B_USER, None)];
        let edited = |r| vec![row("a", A, A_USER, None), row("b", B_SITE, B_NEW_USER, r)];
        let one = vec![row("b", B_SITE, B_NEW_USER, None)];
        let left = Seen {
            ready: true,
            ..Seen::default()
        };
        let (s, actions) = drive(
            VaultPhase::Write,
            &[
                left.clone(),
                page(Shown::Locked, vec![]),
                page(Shown::Unlocked, vec![]),
                page(Shown::Unlocked, two()),
                page(Shown::Unlocked, edited(None)),
                page(Shown::Unlocked, edited(Some(B_NEW))),
                page(Shown::Unlocked, one.clone()),
                left,
                page(Shown::Locked, vec![]),
                page(Shown::Unlocked, one),
                page(
                    Shown::Unlocked,
                    vec![
                        row("b", B_SITE, B_NEW_USER, None),
                        row("c", C_SITE, C_USER, None),
                    ],
                ),
            ],
        );
        assert_eq!(s.stage, Stage::Done);
        let kinds: Vec<String> = actions
            .iter()
            .map(|a| {
                format!("{a:?}")
                    .split([' ', '('])
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "OpenPage", "Unlock", "Add", "Add", "Edit", "Reveal", "Delete", "Leave",
                "OpenPage", "Unlock", "Add", "Pass"
            ]
        );
    }

    #[test]
    fn a_page_that_lost_its_logins_after_a_restart_fails() {
        let (_, actions) = drive(
            VaultPhase::Read,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                page(
                    Shown::Unreadable,
                    vec![row("b", B_SITE, "(cannot be read)", None)],
                ),
            ],
        );
        assert!(matches!(actions.last(), Some(Action::Fail(e)) if e.contains("cannot be read")));
        let (_, actions) = drive(
            VaultPhase::Read,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                page(Shown::Unlocked, vec![row("b", B_SITE, B_NEW_USER, None)]),
            ],
        );
        assert!(matches!(actions.last(), Some(Action::Fail(e)) if e.contains("restart")));
    }

    #[test]
    fn the_broken_keyring_must_be_said_and_refuse_the_add() {
        let unavailable = |notice: &str| Seen {
            body: "… it cannot be used: no keyring.".into(),
            notice: notice.into(),
            ..page(Shown::Unavailable, vec![])
        };
        let (s, actions) = drive(
            VaultPhase::Broken,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                unavailable(""),
                unavailable(""),
                unavailable("vault unavailable: no keyring"),
            ],
        );
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(actions.last(), Some(&Action::Pass));
        let (_, actions) = drive(
            VaultPhase::Broken,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                page(Shown::Unlocked, vec![]),
            ],
        );
        assert!(matches!(actions.last(), Some(Action::Fail(_))));
    }

    #[test]
    fn a_lost_key_must_be_said_keep_the_logins_and_refuse_the_add() {
        let lost = |notice: &str| Seen {
            body: "2 saved logins were saved with a key that the keyring does not have now".into(),
            notice: notice.into(),
            ..page(Shown::KeyLost, vec![])
        };
        let (s, actions) = drive(
            VaultPhase::Lost,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                lost(""),
                lost(""),
                lost("2 saved logins wait for the key they were saved with"),
            ],
        );
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(actions.last(), Some(&Action::Pass));
        // a new key made over them (the vault unlocked) fails
        let (_, actions) = drive(
            VaultPhase::Lost,
            &[
                page(Shown::Locked, vec![]),
                page(Shown::Locked, vec![]),
                page(Shown::Unlocked, vec![]),
            ],
        );
        assert!(matches!(actions.last(), Some(Action::Fail(_))));
    }

    #[test]
    fn a_stuck_step_fails_once() {
        let seen = vec![page(Shown::Locked, vec![]); MAX_WORK_RUNS as usize + 5];
        let (s, actions) = drive(VaultPhase::Read, &seen);
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(
            actions
                .iter()
                .filter(|a| matches!(a, Action::Fail(_)))
                .count(),
            1
        );
    }
}
