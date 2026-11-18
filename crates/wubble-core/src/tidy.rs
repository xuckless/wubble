//! Tab tidy: group a Space's tabs into folders by site, and find duplicate tabs.
//!
//! Pure planning over the model; `Model` applies it (`Command::TidyTabs`). Neither Zen's
//! folders (`ZenFolders.mjs` only groups the tabs the user selected) nor Firefox has a
//! group-by-site or an undo, so this is written for wubble. The site is the registrable
//! domain by the public suffix list (`psl`, MIT/Apache-2.0).

use std::collections::{BTreeMap, HashMap};

use crate::model::Model;
use crate::routing::host_key;
use crate::space::{ContainerId, Item, Space, SpaceId};
use crate::tab::{Tab, TabId};
use crate::url::{is_recordable, url_key};

/// Fewest unpinned top-level tabs of one site that make a folder.
pub const MIN_GROUP: usize = 2;

/// The tabs of one site and the folder name they get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub tabs: Vec<TabId>,
}

/// The site a web page belongs to: the registrable domain (`a.b.example.co.uk` is
/// `example.co.uk`), else the host (`localhost`, an IP address). `None` for pages without
/// a host and for internal pages.
pub fn site_of(url: &str) -> Option<String> {
    if !is_recordable(url) {
        return None;
    }
    let host = host_key(url)?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return Some(host);
    }
    Some(psl::domain_str(&host).map_or(host.clone(), str::to_owned))
}

/// The folders tidy would make: for each site with at least [`MIN_GROUP`] unpinned top-level
/// tabs (or one, when a top-level folder already bears the site's name), the site's tabs in
/// their order. Pinned tabs and tabs already in a folder are left alone. Groups come in the
/// order of each site's first tab.
pub fn plan_groups(space: &Space, tabs: &BTreeMap<TabId, Tab>) -> Vec<Group> {
    let mut groups: Vec<Group> = vec![];
    for item in &space.items {
        let Item::Tab(t) = item else { continue };
        let Some(site) = tabs.get(t).and_then(|t| site_of(&t.url)) else {
            continue;
        };
        match groups.iter_mut().find(|g| g.name == site) {
            Some(g) => g.tabs.push(*t),
            None => groups.push(Group {
                name: site,
                tabs: vec![*t],
            }),
        }
    }
    groups.retain(|g| g.tabs.len() >= MIN_GROUP || has_folder(space, &g.name));
    groups
}

/// The top-level folder named `name`, ignoring case.
pub fn has_folder(space: &Space, name: &str) -> bool {
    space
        .items
        .iter()
        .any(|i| matches!(i, Item::Folder(f) if f.name.eq_ignore_ascii_case(name)))
}

/// The tabs of `space` to close as duplicates: of the tabs with the same address (scheme,
/// `www.` and fragment do not count), all but one. The one kept is a pinned tab, else a tab on
/// screen, else the first. Pinned tabs, tabs in their own window and internal pages are never
/// closed.
pub fn duplicates(model: &Model, space: SpaceId) -> Vec<TabId> {
    let visible = model.visible_tabs();
    let mut by_key: HashMap<(ContainerId, String), Vec<TabId>> = HashMap::new();
    let mut order = vec![];
    for t in model.order(space) {
        let Some(tab) = model.tabs.get(&t) else {
            continue;
        };
        if !is_recordable(&tab.url) || model.windowed(t).is_some() {
            continue;
        }
        // containers have separate cookie jars: the same page in two is two sessions
        let key = (
            tab.container,
            url_key(tab.url.split('#').next().unwrap_or(&tab.url)),
        );
        let group = by_key.entry(key.clone()).or_default();
        if group.is_empty() {
            order.push(key);
        }
        group.push(t);
    }
    let mut close = vec![];
    for key in order {
        let group = &by_key[&key];
        let keep = group
            .iter()
            .find(|t| model.is_pinned(**t))
            .or_else(|| group.iter().find(|t| visible.contains(t)))
            .unwrap_or(&group[0]);
        close.extend(
            group
                .iter()
                .filter(|t| *t != keep && !model.is_pinned(**t) && !visible.contains(t)),
        );
    }
    close
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::model::{Command, Effect, apply};

    fn model(urls: &[&str]) -> (Model, Vec<TabId>) {
        let mut m = Model::new(Instant::now());
        let mut ids = vec![];
        for url in urls {
            let (m2, fx) = apply(
                m,
                Command::NewTab {
                    url: (*url).into(),
                    container: None,
                    opener: None,
                },
            );
            let Effect::CreateTab { tab, .. } = fx[0] else {
                panic!("no CreateTab")
            };
            m = m2;
            ids.push(tab);
        }
        (m, ids)
    }

    #[test]
    fn sites_by_public_suffix() {
        for (url, want) in [
            ("https://a.b.example.co.uk/x", Some("example.co.uk")),
            ("https://www.Example.com:8080/", Some("example.com")),
            ("http://localhost:3000/", Some("localhost")),
            ("http://127.0.0.1/", Some("127.0.0.1")),
            ("http://[::1]:80/", Some("[::1]")),
            ("wubble://settings", None),
            ("about:blank", None),
            ("file:///tmp/x", None),
            ("", None),
        ] {
            assert_eq!(site_of(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn groups_need_two_unpinned_top_level_tabs() {
        let (m, ids) = model(&[
            "https://a.x.com/",
            "https://solo.test/",
            "https://b.x.com/",
            "https://y.org/",
            "https://y.org/2",
            "https://p.org/",
            "https://p.org/2",
        ]);
        let (m, _) = apply(m, Command::PinTab(ids[4]));
        let (m, _) = apply(m, Command::PinTab(ids[6]));
        let groups = plan_groups(&m.spaces[0], &m.tabs);
        assert_eq!(
            groups,
            vec![Group {
                name: "x.com".into(),
                tabs: vec![ids[0], ids[2]]
            }]
        );
    }

    #[test]
    fn a_named_folder_is_reused_and_foldered_tabs_are_left_alone() {
        let (m, ids) = model(&["https://x.com/1", "https://x.com/2", "https://z.com/"]);
        let (m, _) = apply(
            m,
            Command::NewFolder {
                space: SpaceId(0),
                name: "Z.COM".into(),
            },
        );
        let Item::Folder(f) = m.spaces[0].items.last().unwrap().clone() else {
            panic!()
        };
        let (m, _) = apply(
            m,
            Command::MoveTab {
                tab: ids[1],
                to: crate::model::Dest::Folder {
                    folder: f.id,
                    index: 0,
                },
            },
        );
        // x.com has one top-level tab left: no group. z.com's lone tab joins its folder.
        assert_eq!(
            plan_groups(&m.spaces[0], &m.tabs),
            vec![Group {
                name: "z.com".into(),
                tabs: vec![ids[2]]
            }]
        );
    }

    #[test]
    fn duplicates_keep_the_pinned_then_the_visible_then_the_first() {
        let (m, ids) = model(&[
            "https://d.test/a",
            "https://www.d.test/a#frag",
            "https://d.test/a",
            "https://e.test/",
            "https://e.test/",
            "https://other.test/",
            "wubble://settings",
            "wubble://settings",
        ]);
        // e.test: the last one is visible (the active tab), so the first of them goes
        assert_eq!(m.visible_tab(), Some(ids[7]));
        let (m2, _) = apply(m.clone(), Command::ActivateTab(ids[4]));
        assert_eq!(duplicates(&m2, SpaceId(0)), vec![ids[1], ids[2], ids[3]]);
        // a pinned duplicate wins, and a pinned one is never closed
        let (m3, _) = apply(m2, Command::PinTab(ids[2]));
        let (m3, _) = apply(m3, Command::PinTab(ids[0]));
        assert_eq!(duplicates(&m3, SpaceId(0)), vec![ids[1], ids[3]]);
    }

    #[test]
    fn duplicates_in_other_containers_are_kept() {
        let (mut m, ids) = model(&["https://c.test/", "https://c.test/", "https://c.test/"]);
        m.tabs.get_mut(&ids[1]).unwrap().container = ContainerId(1);
        // ids[2] is visible; ids[0] shares its container, ids[1] does not
        assert_eq!(duplicates(&m, SpaceId(0)), vec![ids[0]]);
    }
}
