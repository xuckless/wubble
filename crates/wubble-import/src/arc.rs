//! Arc `StorableSidebar.json` (Spaces, Favorites, pinned and unpinned tabs, folders).
//!
//! Field mapping ported from mozilla-firefox/firefox
//! `browser/components/migration/ArcMigrationUtils.sys.mjs` (`parseArcSidebar`, `pairs`,
//! `arcProfileId`), zen-browser/desktop `src/zen/migration/ZenMigrationUtils.sys.mjs`
//! (`arcSpaceTitle`, `arcSpaceIcon`, `arcColor`, `arcSpaceTheme`) and zephium-browser/Zephium
//! `crates/zephium-import/src/arc.rs` (`marked_ids`, `newContainerIDs`, favourites mirror).
//! All MPL-2.0. Arc itself is closed source; the format is observed from files.
//!
//! Layout: `sidebar.containers` is `[kind, container, kind, container, ...]`; the container
//! whose kind has `global` holds `items` (`[id, item, ...]`), `spaces` (`[id, space, ...]`) and
//! `topAppsContainerIDs` (`[profileMarker, containerId, ...]`, Favorites). A space's
//! `containerIDs` is `[marker, id, marker, id]` with markers `pinned` / `unpinned`, written
//! either as strings or as `{"pinned": {...}}` objects. Items are tabs (`data.tab`), folders
//! (`data.list`) or split views (`data.splitView`, flattened into their parent).
//!
//! Arc profiles are Chromium profile directories, not containers: spaces carry a `profile`
//! string and the output has no containers.

use crate::{
    ImportError, ImportedFolder, ImportedSpace, ImportedTab, Layout, Placement, is_web_url,
};
use serde_json::Value;
use std::collections::HashMap;
use wubble_core::space::{ContainerId, Rgb};

const MAX_DEPTH: usize = 32;
const DEFAULT_PROFILE: &str = "Default";

/// `[a, b, c, d]` -> `(a, b), (c, d)`.
fn pairs(v: &Value) -> impl Iterator<Item = (&Value, &Value)> {
    v.as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[k, v]| (k, v))
}

/// Profile of a marker such as `{"default": true}` or
/// `{"custom": {"_0": {"directoryBasename": "Profile 1"}}}`.
fn profile_name(marker: &Value) -> String {
    marker["custom"]["_0"]["directoryBasename"]
        .as_str()
        .unwrap_or(DEFAULT_PROFILE)
        .to_owned()
}

fn is_marker(v: &Value, name: &str) -> bool {
    v.as_str() == Some(name) || v.get(name).is_some()
}

/// Ids following a `kind` marker in a space's `containerIDs` / `newContainerIDs`.
fn marked_ids<'a>(space: &'a Value, kind: &str) -> Vec<&'a str> {
    ["containerIDs", "newContainerIDs"]
        .iter()
        .flat_map(|k| pairs(&space[k]))
        .filter(|(m, _)| is_marker(m, kind))
        .filter_map(|(_, id)| id.as_str())
        .collect()
}

fn rgb(v: &Value) -> Option<Rgb> {
    let ch = |k: &str| {
        v[k].as_f64()
            .map(|f| (f.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    if v["alpha"].as_f64().is_some_and(|a| a <= 0.0) {
        return None;
    }
    Some(Rgb(ch("red")?, ch("green")?, ch("blue")?))
}

/// First `{red, green, blue}` object in the subtree.
fn find_colour(v: &Value) -> Option<Rgb> {
    if let Some(c) = rgb(v) {
        return Some(c);
    }
    match v {
        Value::Object(m) => m.values().find_map(find_colour),
        Value::Array(a) => a.iter().find_map(find_colour),
        _ => None,
    }
}

fn space_tint(theme: &Value) -> Option<Rgb> {
    let style = &theme["background"]["single"]["_0"]["style"]["color"]["_0"];
    let node = if style.is_null() { theme } else { style };
    let preferred = &node["blendedSingleColor"]["_0"]["color"];
    let gradient = &node["blendedGradient"]["_0"]["baseColors"];
    [preferred, gradient, node]
        .into_iter()
        .find_map(find_colour)
}

/// `emoji_v2` is a string, older `emoji` a code point number (or a string).
fn space_icon(info: &Value) -> Option<String> {
    let t = &info["iconType"];
    [&t["emoji_v2"], &t["emoji"]]
        .into_iter()
        .find_map(|e| match e {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => char::from_u32(n.as_u64()? as u32).map(String::from),
            _ => None,
        })
}

struct Walk<'a> {
    items: HashMap<&'a str, &'a Value>,
    /// Item ids in file order, for the `parentID` fallback.
    order: Vec<&'a str>,
    layout: Layout,
}

impl<'a> Walk<'a> {
    fn children(&self, id: &str) -> Vec<&'a str> {
        match self.items.get(id).map(|i| &i["childrenIds"]) {
            Some(c) if c.is_array() => c
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect(),
            _ => self
                .order
                .iter()
                .copied()
                .filter(|c| self.items[c]["parentID"].as_str() == Some(id))
                .collect(),
        }
    }

    fn walk(
        &mut self,
        id: &str,
        space: usize,
        folder: Option<usize>,
        place: Placement,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for child in self.children(id) {
            let Some(&item) = self.items.get(child) else {
                continue;
            };
            let data = &item["data"];
            if data["tab"].is_object() {
                let tab = &data["tab"];
                let Some(url) = tab["savedURL"].as_str().filter(|u| is_web_url(u)) else {
                    continue;
                };
                // The item title is the user's rename; savedTitle is the page title.
                let title = [&item["title"], &tab["savedTitle"]]
                    .into_iter()
                    .find_map(|t| t.as_str().filter(|t| !t.is_empty()))
                    .unwrap_or(url);
                self.layout.tabs.push(ImportedTab {
                    space,
                    folder,
                    placement: place,
                    container: ContainerId::DEFAULT,
                    url: url.to_owned(),
                    title: title.to_owned(),
                });
            } else if data["list"].is_object() {
                self.layout.folders.push(ImportedFolder {
                    space,
                    parent: folder,
                    name: item["title"].as_str().unwrap_or_default().to_owned(),
                    collapsed: false,
                });
                let f = self.layout.folders.len() - 1;
                self.walk(child, space, Some(f), place, depth + 1);
            } else if data["splitView"].is_object() {
                self.walk(child, space, folder, place, depth + 1);
            }
        }
    }
}

pub fn parse(bytes: &[u8]) -> Result<Layout, ImportError> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|_| ImportError::Unreadable("StorableSidebar.json"))?;
    let container = pairs(&root["sidebar"]["containers"])
        .find(|(kind, _)| kind.get("global").is_some())
        .map(|(_, c)| c)
        .ok_or(ImportError::Unreadable("no global container"))?;

    let mut w = Walk {
        items: HashMap::new(),
        order: Vec::new(),
        layout: Layout::default(),
    };
    for (id, item) in pairs(&container["items"]) {
        if let Some(id) = id.as_str() {
            w.items.insert(id, item);
            w.order.push(id);
        }
    }

    let mut profiles = Vec::new();
    for (n, (_, s)) in pairs(&container["spaces"]).enumerate() {
        let profile = profile_name(&s["profile"]);
        let space = w.layout.spaces.len();
        w.layout.spaces.push(ImportedSpace {
            name: s["title"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Space {}", n + 1)),
            icon: space_icon(&s["customInfo"]),
            tint: space_tint(&s["customInfo"]["windowTheme"]),
            container: ContainerId::DEFAULT,
            profile: Some(profile.clone()),
        });
        profiles.push(profile);
        for (kind, place) in [
            ("pinned", Placement::Pinned),
            ("unpinned", Placement::Normal),
        ] {
            for id in marked_ids(s, kind) {
                w.walk(id, space, None, place, 0);
            }
        }
    }

    // Favorites are per profile; attach them to that profile's first space.
    let favourites = match &container["topAppsContainerIDs"] {
        v if v.is_array() => v,
        _ => &root["sidebarSyncState"]["container"]["value"]["topAppsContainerIDs"],
    };
    for (marker, id) in pairs(favourites) {
        let (profile, Some(id)) = (profile_name(marker), id.as_str()) else {
            continue;
        };
        let space = profiles.iter().position(|p| *p == profile).unwrap_or(0);
        w.walk(id, space, None, Placement::Essential, 0);
    }
    Ok(w.layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(l: &Layout) -> Vec<(usize, Placement, Option<usize>, &str)> {
        l.tabs
            .iter()
            .map(|t| (t.space, t.placement, t.folder, t.url.as_str()))
            .collect()
    }

    #[test]
    fn fixture() {
        let l = parse(include_bytes!("../tests/fixtures/arc_sidebar.json")).unwrap();
        assert_eq!(l.spaces.len(), 3);
        assert_eq!(l.spaces[0].name, "Personal");
        assert_eq!(l.spaces[0].icon.as_deref(), Some("🎨"));
        assert_eq!(l.spaces[0].tint, Some(Rgb(255, 128, 0)));
        assert_eq!(l.spaces[0].profile.as_deref(), Some("Default"));
        assert_eq!(l.spaces[1].name, "Space 2");
        assert_eq!(l.spaces[1].profile.as_deref(), Some("Profile 1"));
        assert_eq!(l.spaces[1].icon, None);
        assert_eq!(l.spaces[1].tint, None);
        assert_eq!(l.spaces[2].icon.as_deref(), Some("A"));

        let f: Vec<_> = l
            .folders
            .iter()
            .map(|f| (f.name.as_str(), f.space, f.parent))
            .collect();
        assert_eq!(f, [("Docs", 0, None), ("Inner", 0, Some(0))]);

        use Placement::*;
        assert_eq!(
            summary(&l),
            [
                (0, Pinned, Some(0), "https://docs.example/a"),
                (0, Pinned, Some(1), "https://docs.example/inner"),
                (0, Pinned, None, "https://split.example/1"),
                (0, Pinned, None, "https://split.example/2"),
                (0, Normal, None, "https://today.example/"),
                (1, Normal, None, "https://other.example/"),
                (0, Essential, None, "https://fav.example/"),
                (1, Essential, None, "https://fav2.example/"),
            ]
        );
        // rename wins; page title fallback; URL as last resort
        assert_eq!(l.tabs[0].title, "Docs A (renamed)");
        assert_eq!(l.tabs[1].title, "Inner page");
        assert_eq!(l.tabs[4].title, "https://today.example/");
    }

    #[test]
    fn marker_forms_and_parent_fallback() {
        let sidebar = json!({"sidebar": {"containers": [
            {"global": {}},
            {"items": [
                "pin", {"id": "pin"},
                "t1", {"id": "t1", "parentID": "pin", "data": {"tab": {"savedURL": "https://a/"}}},
                "t2", {"id": "t2", "parentID": "pin", "data": {"tab": {"savedURL": "https://b/"}}},
                "t3", {"id": "t3", "parentID": "other", "data": {"tab": {"savedURL": "https://c/"}}}
            ],
            "spaces": ["s", {"id": "s", "title": "S",
                "newContainerIDs": [{"pinned": {"_0": {}}}, "pin"]}]}
        ]}});
        let l = parse(sidebar.to_string().as_bytes()).unwrap();
        let urls: Vec<_> = l.tabs.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://a/", "https://b/"]);
    }

    #[test]
    fn non_web_urls_dropped() {
        let sidebar = json!({"sidebar": {"containers": [
            {"global": {}},
            {"items": [
                "p", {"id": "p", "childrenIds": ["a", "b", "c"]},
                "a", {"id": "a", "data": {"tab": {"savedURL": "javascript:alert(1)"}}},
                "b", {"id": "b", "data": {"tab": {"savedURL": "arc://boost"}}},
                "c", {"id": "c", "data": {"tab": {"savedURL": "https://ok/"}}}],
            "spaces": ["s", {"id": "s", "containerIDs": ["pinned", "p"]}]}
        ]}});
        let l = parse(sidebar.to_string().as_bytes()).unwrap();
        assert_eq!(l.tabs.len(), 1);
    }

    #[test]
    fn depth_is_capped_and_cycles_terminate() {
        // a folder that contains itself
        let sidebar = json!({"sidebar": {"containers": [
            {"global": {}},
            {"items": [
                "p", {"id": "p", "childrenIds": ["f"]},
                "f", {"id": "f", "title": "loop", "childrenIds": ["f"], "data": {"list": {}}}],
            "spaces": ["s", {"id": "s", "containerIDs": ["pinned", "p"]}]}
        ]}});
        let l = parse(sidebar.to_string().as_bytes()).unwrap();
        assert_eq!(l.folders.len(), MAX_DEPTH + 1);
    }

    #[test]
    fn little_arc_only_and_garbage_are_errors() {
        let little = json!({"sidebar": {"containers": [{"littleBrowser": {}}, {"items": []}]}});
        assert!(parse(little.to_string().as_bytes()).is_err());
        assert!(parse(b"nope").is_err());
        assert!(parse(b"{}").is_err());
    }

    #[test]
    fn colour_rules() {
        assert_eq!(
            rgb(&json!({"red": 1.0, "green": 0.0, "blue": 0.5, "alpha": 1.0})),
            Some(Rgb(255, 0, 128))
        );
        assert_eq!(
            rgb(&json!({"red": 1.0, "green": 0.0, "blue": 0.5, "alpha": 0.0})),
            None
        );
        let gradient = json!({"background": {"single": {"_0": {"style": {"color": {"_0":
            {"blendedGradient": {"_0": {"baseColors": [{"red": 0.0, "green": 1.0, "blue": 0.0}]}}}}}}}}});
        assert_eq!(space_tint(&gradient), Some(Rgb(0, 255, 0)));
        assert_eq!(space_tint(&json!({})), None);
    }
}
