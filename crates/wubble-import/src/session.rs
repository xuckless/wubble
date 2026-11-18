//! Open tabs, pinned tabs, folders and (Zen) spaces from session files.
//!
//! Formats (all fields optional here; the formats are unversioned and Zen changes often):
//! - Firefox `sessionstore-backups/recovery.jsonlz4`, `sessionstore.jsonlz4`: mozLz4 JSON
//!   `windows[].tabs[]` with `entries[]` (`url`, `title`), 1-based `index`, `pinned`, `hidden`,
//!   `userContextId`, `groupId`; `windows[].groups[]` `{id,name,collapsed}`.
//!   Source: mozilla-firefox/firefox `browser/components/sessionstore/TabState.sys.mjs`,
//!   `TabGroupState.sys.mjs`, `SessionFile.sys.mjs` (MPL-2.0).
//! - Zen `zen-sessions.jsonlz4` (profile root): mozLz4 JSON `{tabs, folders, groups, spaces}`.
//!   Tabs add `zenWorkspace`, `zenEssential`, `zenIsEmpty`, `zenIsGlance`, `zenStaticLabel`,
//!   `_zenPinnedInitialState.entry`; folders `{id,name,collapsed,parentId,workspaceId}`;
//!   spaces `{uuid,name,icon,containerTabId,theme.gradientColors[].c}`.
//!   Source: zen-browser/desktop `src/zen/sessionstore/ZenSessionManager.sys.mjs`,
//!   `TabState-sys-mjs.patch`, `src/zen/folders/ZenFolders.mjs`, `src/zen/spaces/ZenSpaceManager.mjs`,
//!   `ZenGradientGenerator.mjs` (MPL-2.0); pinned-tab URL rule and folder chain walk with a
//!   loop guard ported from zephium-browser/Zephium `crates/zephium-import/src/zen.rs` (MPL-2.0).

use crate::{
    ImportError, ImportedFolder, ImportedSpace, ImportedTab, Layout, Placement, is_web_url, mozlz4,
};
use serde_json::Value;
use std::collections::HashMap;
use wubble_core::space::{ContainerId, Rgb};

fn json(bytes: &[u8]) -> Result<Value, ImportError> {
    serde_json::from_slice(&mozlz4::decode(bytes)?)
        .map_err(|_| ImportError::Unreadable("session JSON"))
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn default_space(name: &str) -> ImportedSpace {
    ImportedSpace {
        name: name.to_owned(),
        icon: None,
        tint: None,
        container: ContainerId::DEFAULT,
        profile: None,
    }
}

/// Current entry of a tab: `entries[index - 1]`, with `index` clamped (Firefox `TabState`).
fn current_entry(tab: &Value) -> Option<&Value> {
    let entries = items(tab, "entries");
    let index = tab["index"].as_u64().unwrap_or(entries.len() as u64) as usize;
    entries.get(index.clamp(1, entries.len().max(1)) - 1)
}

/// Shared by both formats. `None` when the tab has no restorable page.
fn tab_from(tab: &Value, space: usize, folder: Option<usize>) -> Option<ImportedTab> {
    let current = current_entry(tab);
    // Zen pinned tabs navigate away from their pin; the pin's own page is the initial state.
    let entry = Some(&tab["_zenPinnedInitialState"]["entry"])
        .filter(|e| e["url"].as_str().is_some_and(is_web_url))
        .or(current)?;
    let url = entry["url"].as_str().filter(|u| is_web_url(u))?;
    let title = tab["zenStaticLabel"]
        .as_str()
        .filter(|t| !t.is_empty())
        .or(entry["title"].as_str())
        .unwrap_or_default();
    let placement = if tab["zenEssential"].as_bool() == Some(true) {
        Placement::Essential
    } else if tab["pinned"].as_bool() == Some(true) {
        Placement::Pinned
    } else {
        Placement::Normal
    };
    Some(ImportedTab {
        space,
        folder,
        placement,
        container: ContainerId(tab["userContextId"].as_u64().unwrap_or(0) as u32),
        url: url.to_owned(),
        title: title.to_owned(),
    })
}

/// Firefox: every window's tabs go into one space; tab groups become folders.
pub fn parse_firefox(bytes: &[u8]) -> Result<Layout, ImportError> {
    let root = json(bytes)?;
    let mut layout = Layout {
        spaces: vec![default_space("Imported")],
        ..Layout::default()
    };
    for window in items(&root, "windows") {
        // Group ids are only unique within a window.
        let mut groups = HashMap::new();
        for g in items(window, "groups") {
            if let Some(id) = g["id"].as_str() {
                groups.insert(id, layout.folders.len());
                layout.folders.push(ImportedFolder {
                    space: 0,
                    parent: None,
                    name: g["name"].as_str().unwrap_or_default().to_owned(),
                    collapsed: g["collapsed"].as_bool().unwrap_or(false),
                });
            }
        }
        for tab in items(window, "tabs") {
            let folder = tab["groupId"].as_str().and_then(|g| groups.get(g)).copied();
            layout.tabs.extend(tab_from(tab, 0, folder));
        }
    }
    Ok(layout)
}

pub fn parse_zen(bytes: &[u8]) -> Result<Layout, ImportError> {
    let root = json(bytes)?;
    let mut layout = Layout::default();
    let mut space_of = HashMap::new();
    for s in items(&root, "spaces") {
        if let Some(uuid) = s["uuid"].as_str() {
            space_of.insert(uuid, layout.spaces.len());
        }
        layout.spaces.push(ImportedSpace {
            name: s["name"].as_str().unwrap_or("Space").to_owned(),
            icon: s["icon"]
                .as_str()
                .filter(|i| !i.is_empty())
                .map(str::to_owned),
            tint: first_colour(&s["theme"]),
            container: ContainerId(s["containerTabId"].as_u64().unwrap_or(0) as u32),
            profile: None,
        });
    }
    if layout.spaces.is_empty() {
        layout.spaces.push(default_space("Imported"));
    }

    let raw = items(&root, "folders");
    let index_of: HashMap<&str, usize> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, f)| Some((f["id"].as_str()?, i)))
        .collect();
    let parent_of = |i: usize| -> Option<usize> {
        raw[i]["parentId"]
            .as_str()
            .and_then(|p| index_of.get(p))
            .copied()
    };
    for (i, f) in raw.iter().enumerate() {
        // Walk up; a chain longer than the folder count has looped back on itself.
        let mut cur = parent_of(i);
        let mut steps = 0;
        while let Some(p) = cur.filter(|_| steps <= raw.len()) {
            steps += 1;
            cur = parent_of(p);
        }
        let looped = cur.is_some();
        layout.folders.push(ImportedFolder {
            space: f["workspaceId"]
                .as_str()
                .and_then(|w| space_of.get(w))
                .copied()
                .unwrap_or(0),
            parent: if looped { None } else { parent_of(i) },
            name: f["name"].as_str().unwrap_or_default().to_owned(),
            collapsed: f["collapsed"].as_bool().unwrap_or(false),
        });
    }

    for tab in items(&root, "tabs") {
        if tab["zenIsEmpty"].as_bool() == Some(true) || tab["zenIsGlance"].as_bool() == Some(true) {
            continue;
        }
        let space = tab["zenWorkspace"]
            .as_str()
            .and_then(|w| space_of.get(w))
            .copied()
            .unwrap_or(0);
        let folder = tab["groupId"]
            .as_str()
            .and_then(|g| index_of.get(g))
            .copied();
        layout.tabs.extend(tab_from(tab, space, folder));
    }
    Ok(layout)
}

/// First gradient colour of a Zen space theme (`theme.gradientColors[].c`), if it is
/// `[r,g,b]` or `#rrggbb`. Other CSS colour syntaxes are ignored.
fn first_colour(theme: &Value) -> Option<Rgb> {
    let c = &items(theme, "gradientColors").first()?["c"];
    if let Some([r, g, b]) = c.as_array().map(Vec::as_slice).and_then(|a| a.get(..3))
        && let (Some(r), Some(g), Some(b)) = (r.as_u64(), g.as_u64(), b.as_u64())
    {
        return Some(Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8));
    }
    let hex = c.as_str()?.strip_prefix('#').filter(|h| h.len() == 6)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pack(v: &Value) -> Vec<u8> {
        mozlz4::encode(v.to_string().as_bytes())
    }

    fn tab(url: &str) -> Value {
        json!({"entries": [{"url": url, "title": "t"}], "index": 1})
    }

    #[test]
    fn firefox_fixture() {
        let l = parse_firefox(&mozlz4::encode(include_bytes!(
            "../tests/fixtures/firefox_session.json"
        )))
        .unwrap();
        assert_eq!(l.spaces.len(), 1);
        assert_eq!(l.folders.len(), 1);
        assert_eq!(l.folders[0].name, "Reading");
        assert!(l.folders[0].collapsed);
        let tabs: Vec<_> = l
            .tabs
            .iter()
            .map(|t| (t.url.as_str(), t.placement, t.container.0, t.folder))
            .collect();
        assert_eq!(
            tabs,
            [
                ("https://pinned.example/", Placement::Pinned, 0, None),
                ("https://work.example/b", Placement::Normal, 2, None),
                ("https://hidden.example/", Placement::Normal, 0, None),
                ("https://grouped.example/", Placement::Normal, 0, Some(0)),
            ]
        );
        // `index` 2 selects the second entry of the work tab.
        assert_eq!(l.tabs[1].title, "Work B");
    }

    #[test]
    fn firefox_index_clamps_and_empty_entries_skip() {
        let v = json!({"windows": [{"tabs": [
            {"entries": [{"url": "https://a/"}, {"url": "https://b/"}], "index": 9},
            {"entries": [{"url": "https://a/"}, {"url": "https://b/"}], "index": 0},
            {"entries": []},
            {},
            {"entries": [{"url": "about:blank"}]},
            {"entries": [{"url": "javascript:1"}]},
        ]}]});
        let l = parse_firefox(&pack(&v)).unwrap();
        let urls: Vec<_> = l.tabs.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://b/", "https://a/"]);
    }

    #[test]
    fn firefox_bad_input() {
        assert!(parse_firefox(b"junk").is_err());
        assert!(parse_firefox(&mozlz4::encode(b"not json")).is_err());
        assert_eq!(parse_firefox(&pack(&json!({}))).unwrap().tabs.len(), 0);
    }

    #[test]
    fn zen_fixture() {
        let l = parse_zen(&mozlz4::encode(include_bytes!(
            "../tests/fixtures/zen_session.json"
        )))
        .unwrap();
        assert_eq!(l.spaces.len(), 2);
        assert_eq!(l.spaces[0].name, "Home");
        assert_eq!(l.spaces[0].icon.as_deref(), Some("🏠"));
        assert_eq!(l.spaces[0].tint, Some(Rgb(10, 20, 30)));
        assert_eq!(l.spaces[1].tint, Some(Rgb(0xff, 0x80, 0x00)));
        assert_eq!(l.spaces[1].container, ContainerId(2));

        // Nested folders via parentId; the looping pair is cut to roots.
        let f: Vec<_> = l
            .folders
            .iter()
            .map(|f| (f.name.as_str(), f.space, f.parent))
            .collect();
        assert_eq!(
            f,
            [
                ("Outer", 0, None),
                ("Inner", 0, Some(0)),
                ("Loop A", 1, None),
                ("Loop B", 1, None),
            ]
        );

        let t: Vec<_> = l
            .tabs
            .iter()
            .map(|t| {
                (
                    t.url.as_str(),
                    t.title.as_str(),
                    t.space,
                    t.placement,
                    t.folder,
                )
            })
            .collect();
        assert_eq!(
            t,
            [
                (
                    "https://essential.example/",
                    "Essential",
                    0,
                    Placement::Essential,
                    None
                ),
                // pin URL wins over the page the tab navigated to; static label wins over title
                (
                    "https://pin.example/",
                    "My Pin",
                    0,
                    Placement::Pinned,
                    Some(1)
                ),
                (
                    "https://normal.example/",
                    "Normal",
                    1,
                    Placement::Normal,
                    None
                ),
                // unknown workspace falls back to the first space
                ("https://orphan.example/", "", 0, Placement::Normal, None),
            ]
        );
        assert_eq!(l.tabs[1].container, ContainerId(2));
    }

    #[test]
    fn zen_without_spaces_gets_default_space() {
        let l = parse_zen(&pack(&json!({"tabs": [tab("https://a/")]}))).unwrap();
        assert_eq!(l.spaces.len(), 1);
        assert_eq!(l.tabs.len(), 1);
    }

    #[test]
    fn zen_colour_forms() {
        let c = |v: Value| first_colour(&json!({"gradientColors": [{"c": v}]}));
        assert_eq!(c(json!([1, 2, 3])), Some(Rgb(1, 2, 3)));
        assert_eq!(c(json!("#0a0b0c")), Some(Rgb(10, 11, 12)));
        assert_eq!(c(json!("rgb(1, 2, 3)")), None);
        assert_eq!(c(json!([1, 2])), None);
        assert_eq!(first_colour(&json!({})), None);
    }

    #[test]
    fn zen_self_parent_is_cut() {
        let v = json!({"folders": [{"id": "x", "parentId": "x", "name": "X"}]});
        assert_eq!(parse_zen(&pack(&v)).unwrap().folders[0].parent, None);
    }
}
