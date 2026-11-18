//! uBO's list preprocessor: `!#if <expr>` / `!#else` / `!#endif` blocks are
//! kept or dropped for wubble's environment before adblock-rust parses a list
//! (adblock 0.13 reads them as comments, so both branches would load).
//!
//! Ported from gorhill/uBlock `src/js/static-filtering-parser.js`
//! (`utils.preparser`: `preparserTokens`, `evaluateExprToken`, `evaluateExpr`,
//! `splitter`, `prune`), GPL-3.0. The splitter works on lines here; directive
//! lines are dropped (they are comments either way).

/// uBO's token table: list token to environment value (`false`: never true).
const TOKENS: &[(&str, &str)] = &[
    ("ext_ublock", "ublock"),
    ("ext_ubol", "ubol"),
    ("ext_devbuild", "devbuild"),
    ("env_brave", "brave"),
    ("env_chromium", "chromium"),
    ("env_edge", "edge"),
    ("env_firefox", "firefox"),
    ("env_legacy", "legacy"),
    ("env_mobile", "mobile"),
    ("env_mv3", "mv3"),
    ("env_safari", "safari"),
    ("cap_html_filtering", "html_filtering"),
    ("cap_ipaddress", "ipaddress"),
    ("false", "false"),
    ("ext_abp", "false"),
    ("adguard", "adguard"),
    ("adguard_app_android", "false"),
    ("adguard_app_cli", "false"),
    ("adguard_app_ios", "false"),
    ("adguard_app_mac", "false"),
    ("adguard_app_windows", "false"),
    ("adguard_ext_android_cb", "false"),
    ("adguard_ext_chromium", "chromium"),
    ("adguard_ext_chromium_mv3", "mv3"),
    ("adguard_ext_edge", "edge"),
    ("adguard_ext_firefox", "firefox"),
    ("adguard_ext_opera", "chromium"),
    ("adguard_ext_safari", "false"),
];

/// wubble's environment: a Chromium browser running uBO's resources. No HTML
/// filtering, no `$ipaddress`.
const ENV: &[&str] = &["ublock", "chromium"];

/// `evaluateExprToken`: `None` for a token uBO does not know.
fn token(token: &str) -> Option<bool> {
    let (not, name) = match token.strip_prefix('!') {
        Some(name) => (true, name),
        None => (false, token),
    };
    let state = match TOKENS.iter().find(|(t, _)| *t == name) {
        Some((_, state)) => *state,
        None if name.starts_with("cap_") => "false",
        None => return None,
    };
    Some((state == "false" && not) || (ENV.contains(&state) != not))
}

/// `evaluateExpr`: tokens joined by `&&` / `||`, left to right, in optional
/// outer parentheses. `None` when malformed or a token is unknown.
fn expr(expr: &str) -> Option<bool> {
    let expr = expr
        .strip_prefix('(')
        .and_then(|e| e.strip_suffix(')'))
        .unwrap_or(expr);
    let mut words = expr.split_whitespace();
    let first = words.next()?;
    if first.starts_with('|') || first.starts_with('&') {
        return None;
    }
    let mut result = token(first);
    while let Some(op) = words.next() {
        let state = token(words.next()?)?;
        result = Some(match op {
            "||" => result? || state,
            "&&" => result? && state,
            _ => return None,
        });
    }
    result
}

/// One open `!#if`: whether its condition is known and whether its current
/// branch is dropped.
struct Block {
    known: bool,
    discard: bool,
}

/// The directive on `line` (`if`, `else`, `endif`) and its argument.
fn directive(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("!#")?;
    ["if", "else", "endif"].into_iter().find_map(|d| {
        let arg = rest.strip_prefix(d)?;
        let boundary = arg
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        boundary.then_some((d, arg))
    })
}

/// The list without the branches wubble's environment excludes and without
/// lines containing any of `drop`.
pub(crate) fn prune(text: &str, drop: &[&str]) -> String {
    let mut stack: Vec<Block> = Vec::new();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        match directive(line) {
            Some(("if", arg)) => {
                let result = expr(arg.trim());
                stack.push(Block {
                    known: result.is_some(),
                    discard: result == Some(false),
                });
            }
            Some(("else", _)) => {
                if let Some(top) = stack.last_mut() {
                    top.discard = !top.discard;
                }
            }
            Some(_) => {
                stack.pop();
            }
            None => {
                let dropped = stack.iter().any(|b| b.known && b.discard)
                    || drop.iter().any(|d| line.contains(d));
                if !dropped {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_follow_wubble_s_environment() {
        assert_eq!(token("env_chromium"), Some(true));
        assert_eq!(token("ext_ublock"), Some(true));
        assert_eq!(token("env_firefox"), Some(false));
        assert_eq!(token("!env_firefox"), Some(true));
        assert_eq!(token("cap_html_filtering"), Some(false));
        assert_eq!(token("cap_something_new"), Some(false));
        assert_eq!(token("false"), Some(false));
        assert_eq!(token("!false"), Some(true));
        assert_eq!(token("whatever"), None);
    }

    #[test]
    fn expressions() {
        assert_eq!(expr("env_chromium && !env_mobile"), Some(true));
        assert_eq!(expr("(env_firefox || env_safari)"), Some(false));
        assert_eq!(expr("env_firefox || ext_ublock"), Some(true));
        assert_eq!(expr("&& env_firefox"), None);
        assert_eq!(expr("env_firefox ^^ ext_ublock"), None);
        assert_eq!(expr("env_chromium && unknown"), None);
        assert_eq!(expr(""), None);
    }

    #[test]
    fn branches_are_kept_or_dropped() {
        let list = "\
a
!#if env_firefox
firefox
!#else
not-firefox
!#endif
!#if cap_html_filtering
html
!#if env_chromium
nested
!#endif
!#endif
!#if unknown_token
kept
!#endif
!#iffy
z
";
        assert_eq!(prune(list, &[]), "a\nnot-firefox\nkept\n!#iffy\nz\n");
    }

    #[test]
    fn dropped_lines() {
        let list = "##.a\nx.com##+js(trusted-click-element, b)\n##.c\n";
        assert_eq!(prune(list, &["+js("]), "##.a\n##.c\n");
    }
}
