//! Applying a `wubble_core::import::Imported` profile to the model and the database.
//!
//! The Model owns Space, folder, tab and container ids, and `save_session` rewrites every
//! session table from it, so Spaces and tabs cannot be inserted as rows: [`merge_spaces`]
//! (pure) appends them to a copy of the Model and the same transaction that persists it also
//! writes bookmarks and history. Idempotency: Spaces through the `import_marks` table,
//! bookmarks by (parent, url) and (parent, title), visits by (url, time). Design reference for
//! the natural-key merge: Firefox Places `insertMany` / Sync dedupe; no code copied.
//!
//! Mapping notes: Essentials become pinned tabs (core has one pinned list); core folders do not
//! nest, so a nested imported folder's tabs go into its outermost folder; tabs come back
//! Discarded; a Space is never merged into an existing one; the active Space is unchanged.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use turso::Connection;
use wubble_core::import::{BookmarkNode as Imp, Imported, Placement, Source};
use wubble_core::lifecycle::{Activity, Lifecycle, Pin as TabPin};
use wubble_core::model::Model;
use wubble_core::space::{
    Container, ContainerId, Folder, FolderId, Item, Space, SpaceIcon, SpaceId,
};
use wubble_core::tab::{Liveness, Tab, TabId};

use crate::bookmarks::{BookmarkKind, find_folder, find_link, insert_node, valid_link_url};
use crate::history::{recompute_url_counters, url_key};
use crate::session::write_model;
use crate::{Error, Result, Store, is_recordable, scalar};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Space,
    Folder,
    Tab,
    Bookmark,
    Visit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The Space was imported before (and still exists).
    AlreadyImported,
    /// Not a web page (history) or not bookmarkable.
    InvalidUrl,
    /// An index into the imported spaces or folders that points nowhere.
    BadReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub kind: ItemKind,
    /// A name or url, for the report shown to the user.
    pub what: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub containers_added: u32,
    pub spaces_added: u32,
    pub folders_added: u32,
    pub tabs_added: u32,
    /// Nested folders: core folders do not nest, their tabs went into the outermost folder.
    pub folders_flattened: u32,
    pub bookmark_folders_added: u32,
    pub bookmarks_added: u32,
    pub bookmarks_already_present: u32,
    pub visits_added: u32,
    pub visits_already_present: u32,
    pub skipped: Vec<Skipped>,
}

impl ImportSummary {
    fn skip(&mut self, kind: ItemKind, what: &str, reason: SkipReason) {
        self.skipped.push(Skipped {
            kind,
            what: what.to_string(),
            reason,
        });
    }
}

fn source_name(source: Source) -> &'static str {
    match source {
        Source::Zen => "Zen",
        Source::Firefox => "Firefox",
        Source::Arc => "Arc",
    }
}

/// Where a folder's tabs go: the Space (index into the imported spaces) and the core folder.
type Slot = Option<(usize, FolderId)>;

/// The result of [`merge_spaces`].
struct Merged {
    model: Model,
    /// (key, id) of each new Space, stored in `import_marks`.
    marks: Vec<(String, SpaceId)>,
    /// Container, Space, folder and tab counts and the skips so far.
    summary: ImportSummary,
}

/// Append the imported Spaces, folders, tabs and containers to a copy of `model`. Returns the
/// new Model (a plain clone when nothing was added), the marks to store for the new Spaces,
/// and a summary with the Space-level counts filled in. `marks` maps a Space key to the id it
/// got when it was imported.
fn merge_spaces(model: &Model, imp: &Imported, marks: &HashMap<String, i64>) -> Result<Merged> {
    let mut sum = ImportSummary::default();

    // Containers: the Default container maps to Default, others by name, else appended.
    let mut containers = model.containers.clone();
    let mut next_container = containers.iter().map(|c| c.id.0).max().unwrap_or(0) + 1;
    let mut cmap: HashMap<ContainerId, ContainerId> = HashMap::new();
    for c in &imp.containers {
        let to = if c.id == ContainerId::DEFAULT {
            ContainerId::DEFAULT
        } else if let Some(e) = containers.iter().find(|e| e.name == c.name) {
            e.id
        } else {
            let id = ContainerId(next_container);
            next_container += 1;
            containers.push(Container {
                id,
                name: c.name.clone(),
            });
            sum.containers_added += 1;
            id
        };
        cmap.insert(c.id, to);
    }
    let container_of = |c: ContainerId| cmap.get(&c).copied().unwrap_or(ContainerId::DEFAULT);

    let mut next_space = model.spaces.iter().map(|s| s.id.0).max().unwrap_or(0) + 1;
    let mut next_tab = model.tabs.keys().map(|t| t.0).max().unwrap_or(0) + 1;
    let mut next_folder = model
        .spaces
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| match i {
            Item::Folder(f) => Some(f.id.0 + 1),
            Item::Tab(_) => None,
        })
        .max()
        .unwrap_or(0);
    let default_tint = Model::new(model.clock()).spaces[0].tint;

    // One new Space per imported Space, `None` when skipped.
    let mut seen_keys: HashMap<String, u32> = HashMap::new();
    let mut new_spaces: Vec<Option<(String, Space)>> = Vec::new();
    for sp in &imp.spaces {
        let base = format!(
            "{}\u{1f}{}\u{1f}{}",
            source_name(imp.source),
            sp.profile.as_deref().unwrap_or(""),
            sp.name
        );
        let n = seen_keys.entry(base.clone()).or_default();
        let key = if *n == 0 {
            base
        } else {
            format!("{base}\u{1f}{n}")
        };
        *n += 1;
        let imported_before = marks.get(&key).is_some_and(|&id| {
            // Space ids are reused after a delete: the name must match as well.
            model
                .spaces
                .iter()
                .any(|s| i64::from(s.id.0) == id && s.name == sp.name)
        });
        if imported_before {
            sum.skip(ItemKind::Space, &sp.name, SkipReason::AlreadyImported);
            new_spaces.push(None);
            continue;
        }
        let space = Space {
            id: SpaceId(next_space),
            name: sp.name.clone(),
            tint: sp.tint.unwrap_or(default_tint),
            icon: SpaceIcon::nth(next_space as usize),
            container: container_of(sp.container),
            pinned: vec![],
            items: vec![],
            active: None,
            split: None,
        };
        next_space += 1;
        new_spaces.push(Some((key, space)));
    }

    // Folders: top-level ones become core folders; nested ones share their outermost ancestor.
    let mut slots: Vec<Slot> = vec![None; imp.folders.len()];
    for (i, f) in imp
        .folders
        .iter()
        .enumerate()
        .filter(|(_, f)| f.parent.is_none())
    {
        match new_spaces.get_mut(f.space) {
            None => sum.skip(ItemKind::Folder, &f.name, SkipReason::BadReference),
            Some(None) => {} // its Space was imported before
            Some(Some((_, space))) => {
                let id = FolderId(next_folder);
                next_folder += 1;
                space.items.push(Item::Folder(Folder {
                    id,
                    name: f.name.clone(),
                    tabs: vec![],
                }));
                slots[i] = Some((f.space, id));
                sum.folders_added += 1;
            }
        }
    }
    for (i, f) in imp
        .folders
        .iter()
        .enumerate()
        .filter(|(_, f)| f.parent.is_some())
    {
        let mut top = i;
        let mut steps = 0;
        while let Some(p) = imp.folders.get(top).and_then(|f| f.parent) {
            top = p;
            steps += 1;
            if steps > imp.folders.len() {
                break; // a parent cycle
            }
        }
        let ok =
            steps <= imp.folders.len() && imp.folders.get(top).is_some_and(|t| t.space == f.space);
        if !ok {
            sum.skip(ItemKind::Folder, &f.name, SkipReason::BadReference);
        } else if let Some(slot) = slots[top] {
            slots[i] = Some(slot);
            sum.folders_flattened += 1;
        } // else: the top folder's Space was imported before, or already reported
    }

    // Tabs.
    let mut tabs: Vec<Tab> = model.tabs.values().cloned().collect();
    for t in &imp.tabs {
        let Some(slot) = new_spaces.get_mut(t.space) else {
            sum.skip(ItemKind::Tab, &t.url, SkipReason::BadReference);
            continue;
        };
        let Some((_, space)) = slot else { continue };
        if !valid_link_url(&t.url) {
            sum.skip(ItemKind::Tab, &t.url, SkipReason::InvalidUrl);
            continue;
        }
        let folder = match t.folder {
            None => None,
            Some(fi) => match imp.folders.get(fi).and_then(|_| slots[fi]) {
                Some((si, id)) if si == t.space => Some(id),
                _ => {
                    sum.skip(ItemKind::Tab, &t.url, SkipReason::BadReference);
                    continue;
                }
            },
        };
        let id = TabId(next_tab);
        next_tab += 1;
        let pinned = t.placement != Placement::Normal;
        tabs.push(Tab {
            id,
            url: t.url.clone(),
            title: t.title.clone(),
            lifecycle: Lifecycle {
                liveness: Liveness::Discarded,
                pin: if pinned {
                    TabPin::Pinned
                } else {
                    TabPin::Unpinned
                },
                activity: Activity::Idle,
                hidden_since: None,
            },
            container: container_of(t.container),
            opener: None,
        });
        match (pinned, folder) {
            (true, _) => space.pinned.push(id),
            (false, None) => space.items.push(Item::Tab(id)),
            (false, Some(f)) => space
                .folder_mut(f)
                .expect("slot folders were pushed to this space")
                .tabs
                .push(id),
        }
        sum.tabs_added += 1;
    }

    let mut spaces = model.spaces.clone();
    let mut new_marks = Vec::new();
    for (key, mut space) in new_spaces.into_iter().flatten() {
        // Core wants an active tab exactly when the Space has tabs: the first in display order.
        space.active = space.pinned.first().copied().or_else(|| {
            space.items.iter().find_map(|i| match i {
                Item::Tab(t) => Some(*t),
                Item::Folder(f) => f.tabs.first().copied(),
            })
        });
        new_marks.push((key, space.id));
        spaces.push(space);
        sum.spaces_added += 1;
    }
    if sum.spaces_added == 0 && sum.containers_added == 0 {
        // Nothing to add: hand back the very same model (rebuilding would restart its clock).
        return Ok(Merged {
            model: model.clone(),
            marks: new_marks,
            summary: sum,
        });
    }
    let merged = Model::from_parts(spaces, containers, tabs, model.active_space, model.clock())
        .map_err(Error::Corrupt)?;
    Ok(Merged {
        model: merged,
        marks: new_marks,
        summary: sum,
    })
}

type Task<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

/// Merge `nodes` under `parent`: folders by title, links by url, both within `parent`.
fn merge_bookmarks<'a>(
    c: &'a Connection,
    parent: i64,
    nodes: &'a [Imp],
    now_ms: i64,
    sum: &'a mut ImportSummary,
) -> Task<'a> {
    Box::pin(async move {
        for node in nodes {
            match node {
                Imp::Folder { title, children } => {
                    let id = match find_folder(c, Some(parent), title).await? {
                        Some(id) => id,
                        None => {
                            sum.bookmark_folders_added += 1;
                            insert_node(
                                c,
                                Some(parent),
                                BookmarkKind::Folder,
                                title,
                                None,
                                now_ms,
                                now_ms,
                            )
                            .await?
                        }
                    };
                    merge_bookmarks(c, id, children, now_ms, sum).await?;
                }
                Imp::Link {
                    title,
                    url,
                    added_ms,
                } => {
                    if !valid_link_url(url) {
                        sum.skip(ItemKind::Bookmark, url, SkipReason::InvalidUrl);
                    } else if find_link(c, Some(parent), url).await?.is_some() {
                        sum.bookmarks_already_present += 1;
                    } else {
                        insert_node(
                            c,
                            Some(parent),
                            BookmarkKind::Link,
                            title,
                            Some(url),
                            *added_ms,
                            now_ms,
                        )
                        .await?;
                        sum.bookmarks_added += 1;
                    }
                }
            }
        }
        Ok(())
    })
}

/// Insert the visits with their original times; a visit already stored at the same
/// (url, time) is not added again. Counters of every touched url are recomputed.
async fn merge_history(c: &Connection, imp: &Imported, sum: &mut ImportSummary) -> Result<()> {
    let mut upsert = c
        .prepare(
            "INSERT INTO urls(url, url_key, title, last_visit_time) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(url) DO UPDATE SET title = CASE WHEN title = '' THEN excluded.title ELSE title END",
        )
        .await?;
    let mut add = c
        .prepare(
            "INSERT INTO visits(url_id, visit_time, transition) SELECT ?1, ?2, ?3 \
             WHERE NOT EXISTS (SELECT 1 FROM visits WHERE url_id = ?1 AND visit_time = ?2)",
        )
        .await?;
    let mut url_ids: HashMap<&str, i64> = HashMap::new();
    let mut touched = Vec::new();
    for v in &imp.history {
        if !is_recordable(&v.url) {
            sum.skip(ItemKind::Visit, &v.url, SkipReason::InvalidUrl);
            continue;
        }
        let url_id = match url_ids.get(v.url.as_str()) {
            Some(&id) => id,
            None => {
                upsert
                    .execute((v.url.as_str(), url_key(&v.url), v.title.as_str(), v.at_ms))
                    .await?;
                let id = scalar(c, "SELECT id FROM urls WHERE url = ?1", [v.url.as_str()])
                    .await?
                    .ok_or_else(|| {
                        Error::Corrupt(format!("url row missing after insert: {}", v.url))
                    })?;
                url_ids.insert(&v.url, id);
                touched.push(id);
                id
            }
        };
        if add.execute((url_id, v.at_ms, i64::from(v.typed))).await? > 0 {
            sum.visits_added += 1;
        } else {
            sum.visits_already_present += 1;
        }
    }
    recompute_url_counters(c, &touched).await?;
    Ok(())
}

impl Store {
    async fn import_marks(&self) -> Result<HashMap<String, i64>> {
        let mut rows = self
            .conn
            .query("SELECT key, space_id FROM import_marks", ())
            .await?;
        let mut marks = HashMap::new();
        while let Some(r) = rows.next().await? {
            marks.insert(r.get(0)?, r.get(1)?);
        }
        Ok(marks)
    }

    /// Merge `imported` into `model` and the database in one transaction; any error leaves
    /// both untouched. The caller replaces its model with the returned one.
    ///
    /// Spaces are appended as new Spaces (never merged into existing ones), bookmarks go under
    /// an `Imported from <source>` folder, visits keep their original times. Applying the same
    /// import again adds nothing and returns an equal model. The session is written only when
    /// Spaces or containers were added, from `model` (so persist any pending model changes
    /// through the same value you pass here).
    pub async fn apply_import(
        &mut self,
        model: &Model,
        imported: &Imported,
        now_ms: i64,
    ) -> Result<(Model, ImportSummary)> {
        model.check().map_err(Error::Corrupt)?;
        let marks = self.import_marks().await?;
        let Merged {
            model: merged,
            marks: new_marks,
            mut summary,
        } = merge_spaces(model, imported, &marks)?;
        let tx = self.conn.transaction().await?;
        if summary.spaces_added > 0 || summary.containers_added > 0 {
            write_model(&tx, &merged).await?;
            // Marks of Spaces that no longer exist must not suppress a later import.
            tx.execute(
                "DELETE FROM import_marks WHERE space_id NOT IN (SELECT id FROM spaces)",
                (),
            )
            .await?;
            for (key, id) in &new_marks {
                tx.execute(
                    "INSERT INTO import_marks(key, space_id) VALUES (?1, ?2) \
                     ON CONFLICT(key) DO UPDATE SET space_id = excluded.space_id",
                    (key.as_str(), i64::from(id.0)),
                )
                .await?;
            }
        }
        if !imported.bookmarks.is_empty() {
            let title = format!("Imported from {}", source_name(imported.source));
            let root = match find_folder(&tx, None, &title).await? {
                Some(id) => id,
                None => {
                    summary.bookmark_folders_added += 1;
                    insert_node(
                        &tx,
                        None,
                        BookmarkKind::Folder,
                        &title,
                        None,
                        now_ms,
                        now_ms,
                    )
                    .await?
                }
            };
            merge_bookmarks(&tx, root, &imported.bookmarks, now_ms, &mut summary).await?;
        }
        merge_history(&tx, imported, &mut summary).await?;
        tx.commit().await?;
        Ok((merged, summary))
    }
}

#[cfg(test)]
mod tests {
    use wubble_core::import::{ImportedFolder, ImportedSpace, ImportedTab, ImportedVisit};
    use wubble_core::model::{Command, apply};
    use wubble_core::space::Rgb;

    use super::*;

    const D: ContainerId = ContainerId::DEFAULT;

    fn run<F: Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    fn base_model() -> Model {
        apply(
            Model::new(std::time::Instant::now()),
            Command::NewTab {
                url: "https://mine.test".into(),
                container: None,
                opener: None,
            },
        )
        .0
    }

    fn space(name: &str, container: u32, tint: Option<Rgb>) -> ImportedSpace {
        ImportedSpace {
            name: name.into(),
            icon: None,
            tint,
            container: ContainerId(container),
            profile: None,
        }
    }

    fn folder(space: usize, parent: Option<usize>, name: &str) -> ImportedFolder {
        ImportedFolder {
            space,
            parent,
            name: name.into(),
            collapsed: false,
        }
    }

    fn tab(
        space: usize,
        folder: Option<usize>,
        p: Placement,
        container: u32,
        url: &str,
    ) -> ImportedTab {
        ImportedTab {
            space,
            folder,
            placement: p,
            container: ContainerId(container),
            url: url.into(),
            title: format!("T {url}"),
        }
    }

    fn link(title: &str, url: &str, added: i64) -> Imp {
        Imp::Link {
            title: title.into(),
            url: url.into(),
            added_ms: added,
        }
    }

    fn visit(url: &str, at: i64, typed: bool) -> ImportedVisit {
        ImportedVisit {
            url: url.into(),
            title: "V".into(),
            at_ms: at,
            typed,
        }
    }

    fn fixture() -> Imported {
        use Placement::*;
        Imported {
            source: Source::Zen,
            containers: vec![
                Container {
                    id: D,
                    name: "Default".into(),
                },
                Container {
                    id: ContainerId(3),
                    name: "Work".into(),
                },
            ],
            spaces: vec![
                space("Home", 3, Some(Rgb(1, 2, 3))),
                space("Reading", 0, None),
            ],
            folders: vec![
                folder(0, None, "Dev"),
                folder(0, Some(0), "Rust"), // nested
                folder(1, None, "News"),
            ],
            tabs: vec![
                tab(0, None, Essential, 3, "https://e.test"),
                tab(0, None, Pinned, 0, "https://p.test"),
                tab(0, Some(0), Normal, 0, "https://dev.test/1"),
                tab(0, Some(1), Normal, 0, "https://dev.test/2"), // in the nested folder
                tab(0, None, Normal, 0, "https://top.test"),
                tab(1, Some(2), Normal, 0, "https://news.test"),
                tab(1, None, Normal, 0, "https://n2.test"),
                tab(9, None, Normal, 0, "https://bad-space.test"),
                tab(0, Some(7), Normal, 0, "https://bad-folder.test"),
                tab(1, Some(0), Normal, 0, "https://wrong-space-folder.test"),
            ],
            bookmarks: vec![
                link("A", "https://a.test/", 10),
                Imp::Folder {
                    title: "Dev".into(),
                    children: vec![
                        link("B", "https://b.test/", 20),
                        Imp::Folder {
                            title: "Deep".into(),
                            children: vec![link("C", "https://c.test/", 30)],
                        },
                    ],
                },
                link("Script", "javascript:alert(1)", 40),
            ],
            history: vec![
                visit("https://h.test/a", 300, true),
                visit("https://h.test/a", 200, false),
                visit("https://h.test/b", 100, false),
                visit("file:///home/x", 50, false),
                visit("https://h.test/a", 200, false), // repeat inside the import
            ],
            warnings: vec![],
        }
    }

    /// Model equality without the clock.
    #[track_caller]
    fn assert_same(a: &Model, b: &Model) {
        assert_eq!(a.spaces, b.spaces);
        assert_eq!(a.containers, b.containers);
        assert_eq!(a.tabs, b.tabs);
        assert_eq!(a.active_space, b.active_space);
    }

    const TABLES: &[&str] = &[
        "containers",
        "spaces",
        "tab_folders",
        "tabs",
        "session",
        "import_marks",
        "urls",
        "visits",
        "bookmarks",
    ];

    async fn counts(s: &Store, tables: &[&str]) -> Vec<i64> {
        let mut out = vec![];
        for t in tables {
            out.push(
                scalar(&s.conn, &format!("SELECT COUNT(*) FROM {t}"), ())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        out
    }

    async fn consistent(s: &Store) {
        let p = s.check_consistency().await.unwrap();
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn spaces_folders_tabs_and_containers_are_mapped() {
        let model = base_model();
        let Merged {
            model: m,
            marks,
            summary: sum,
        } = merge_spaces(&model, &fixture(), &HashMap::new()).unwrap();
        m.check().unwrap();
        assert_eq!(m.active_space, model.active_space);
        assert_eq!(m.spaces[0], model.spaces[0]);
        assert_eq!(m.spaces.len(), 3);
        assert_eq!(marks.len(), 2);
        // New container appended with the next id; the Work Space uses it.
        assert_eq!(m.containers.len(), 2);
        assert_eq!(m.containers[1].name, "Work");
        let (home, reading) = (&m.spaces[1], &m.spaces[2]);
        assert_eq!(
            (home.name.as_str(), home.tint, home.container),
            ("Home", Rgb(1, 2, 3), ContainerId(1))
        );
        assert_eq!(
            reading.tint,
            Model::new(std::time::Instant::now()).spaces[0].tint
        );
        assert_eq!(reading.container, D);
        // Essential and Pinned both land in the pinned list, in source order.
        let url = |t: &TabId| m.tabs[t].url.as_str();
        assert_eq!(
            home.pinned.iter().map(url).collect::<Vec<_>>(),
            ["https://e.test", "https://p.test"]
        );
        assert_eq!(m.tabs[&home.pinned[0]].container, ContainerId(1));
        assert!(
            home.pinned
                .iter()
                .all(|t| m.tabs[t].lifecycle.pin == TabPin::Pinned)
        );
        // Folders first, then top-level tabs; the nested folder's tab is in the outer one.
        let Item::Folder(dev) = &home.items[0] else {
            panic!("folder first")
        };
        assert_eq!(dev.name, "Dev");
        assert_eq!(
            dev.tabs.iter().map(url).collect::<Vec<_>>(),
            ["https://dev.test/1", "https://dev.test/2"]
        );
        assert!(matches!(home.items[1], Item::Tab(t) if m.tabs[&t].url == "https://top.test"));
        assert_eq!(home.items.len(), 2);
        assert_eq!(home.active, Some(home.pinned[0]));
        let Item::Folder(news) = &reading.items[0] else {
            panic!("folder first")
        };
        assert_eq!(news.tabs.len(), 1);
        assert_eq!(reading.active, Some(news.tabs[0]));
        // No id collisions: from_parts checked; every tab is Discarded and has no opener.
        assert!(
            m.tabs
                .values()
                .filter(|t| t.url != "https://mine.test")
                .all(|t| { t.lifecycle.liveness == Liveness::Discarded && t.opener.is_none() })
        );
        assert_eq!(
            (
                sum.containers_added,
                sum.spaces_added,
                sum.folders_added,
                sum.tabs_added,
                sum.folders_flattened
            ),
            (1, 2, 2, 7, 1)
        );
        let reasons = |k| {
            sum.skipped
                .iter()
                .filter(|s| s.kind == k)
                .map(|s| (s.what.as_str(), s.reason))
                .collect::<Vec<_>>()
        };
        assert!(reasons(ItemKind::Folder).is_empty());
        assert_eq!(
            reasons(ItemKind::Tab),
            [
                ("https://bad-space.test", SkipReason::BadReference),
                ("https://bad-folder.test", SkipReason::BadReference),
                ("https://wrong-space-folder.test", SkipReason::BadReference),
            ]
        );
    }

    #[test]
    fn containers_map_by_name_and_unknown_ids_fall_back_to_default() {
        let model = apply(
            base_model(),
            Command::NewContainer {
                name: "Work".into(),
            },
        )
        .0;
        let mut imp = fixture();
        imp.containers[1].id = ContainerId(9);
        imp.spaces[0].container = ContainerId(9);
        imp.spaces[1].container = ContainerId(77); // not in the container list
        let Merged {
            model: m,
            summary: sum,
            ..
        } = merge_spaces(&model, &imp, &HashMap::new()).unwrap();
        assert_eq!(sum.containers_added, 0);
        assert_eq!(m.containers.len(), 2);
        assert_eq!(m.spaces[1].container, ContainerId(1));
        assert_eq!(m.spaces[2].container, D);
    }

    #[test]
    fn a_cyclic_folder_parent_is_skipped_not_followed() {
        let mut imp = fixture();
        imp.folders = vec![folder(0, Some(1), "X"), folder(0, Some(0), "Y")];
        imp.tabs = vec![tab(0, Some(0), Placement::Normal, 0, "https://x.test")];
        let Merged {
            model: m,
            summary: sum,
            ..
        } = merge_spaces(&base_model(), &imp, &HashMap::new()).unwrap();
        m.check().unwrap();
        assert_eq!(sum.folders_added, 0);
        assert_eq!(sum.tabs_added, 0);
        assert!(
            sum.skipped
                .iter()
                .all(|s| s.reason == SkipReason::BadReference)
        );
    }

    #[test]
    fn an_empty_import_returns_the_model_unchanged() {
        let model = base_model();
        let imp = Imported {
            spaces: vec![],
            folders: vec![],
            tabs: vec![],
            bookmarks: vec![],
            history: vec![],
            containers: vec![],
            ..fixture()
        };
        let Merged {
            model: m,
            marks,
            summary: sum,
        } = merge_spaces(&model, &imp, &HashMap::new()).unwrap();
        assert_eq!(m, model);
        assert!(marks.is_empty());
        assert_eq!(sum, ImportSummary::default());
    }

    #[test]
    fn apply_persists_everything_and_reports_exact_counts() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let model = base_model();
            s.save_session(&model).await.unwrap();
            let (m, sum) = s.apply_import(&model, &fixture(), 5_000).await.unwrap();
            assert_eq!(
                (
                    sum.bookmark_folders_added,
                    sum.bookmarks_added,
                    sum.bookmarks_already_present,
                    sum.visits_added,
                    sum.visits_already_present
                ),
                (3, 3, 0, 3, 1)
            );
            let skipped = |k| sum.skipped.iter().filter(|s| s.kind == k).count();
            assert_eq!(
                (
                    skipped(ItemKind::Bookmark),
                    skipped(ItemKind::Visit),
                    skipped(ItemKind::Space)
                ),
                (1, 1, 0)
            );
            // The stored session is the returned model.
            assert_same(
                &s.load_session(std::time::Instant::now()).await.unwrap(),
                &m,
            );
            consistent(&s).await;

            // Bookmarks: root folder, source order, original added time.
            let roots = s.bookmark_children(None).await.unwrap();
            assert_eq!(roots.len(), 1);
            assert_eq!(roots[0].title, "Imported from Zen");
            let kids = s.bookmark_children(Some(roots[0].id)).await.unwrap();
            assert_eq!(
                kids.iter().map(|k| k.title.as_str()).collect::<Vec<_>>(),
                ["A", "Dev"]
            );
            assert_eq!((kids[0].added_ms, kids[0].modified_ms), (10, 5_000));
            let dev = s.bookmark_children(Some(kids[1].id)).await.unwrap();
            assert_eq!(
                dev.iter().map(|k| k.title.as_str()).collect::<Vec<_>>(),
                ["B", "Deep"]
            );
            let deep = s.bookmark_children(Some(dev[1].id)).await.unwrap();
            assert_eq!((deep[0].title.as_str(), deep[0].added_ms), ("C", 30));

            // History keeps the original times and typed counts.
            let a = &s.query_history("h.test/a", 1_000, 10).await.unwrap()[0];
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (2, 1, 300));
            assert_eq!(a.title, "V");
            let times = scalar(&s.conn, "SELECT MIN(visit_time) FROM visits", ())
                .await
                .unwrap();
            assert_eq!(times, Some(100));
            let file_urls = scalar(
                &s.conn,
                "SELECT COUNT(*) FROM urls WHERE url LIKE 'file:%'",
                (),
            )
            .await
            .unwrap();
            assert_eq!(file_urls, Some(0));
        });
    }

    #[test]
    fn applying_the_same_import_twice_adds_nothing() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let model = base_model();
            s.save_session(&model).await.unwrap();
            let (m1, first) = s.apply_import(&model, &fixture(), 5_000).await.unwrap();
            let rows = counts(&s, TABLES).await;
            let (m2, second) = s.apply_import(&m1, &fixture(), 9_000).await.unwrap();
            assert_eq!(m2, m1);
            assert_eq!(counts(&s, TABLES).await, rows);
            consistent(&s).await;
            assert_eq!(
                (
                    second.containers_added,
                    second.spaces_added,
                    second.folders_added,
                    second.tabs_added
                ),
                (0, 0, 0, 0)
            );
            assert_eq!(
                (
                    second.bookmark_folders_added,
                    second.bookmarks_added,
                    second.bookmarks_already_present
                ),
                (0, 0, 3)
            );
            assert_eq!((second.visits_added, second.visits_already_present), (0, 4));
            assert_eq!(
                second
                    .skipped
                    .iter()
                    .filter(|s| s.reason == SkipReason::AlreadyImported)
                    .count(),
                2
            );
            assert_eq!(first.spaces_added, 2);
            assert_same(
                &s.load_session(std::time::Instant::now()).await.unwrap(),
                &m1,
            );
            // Counters untouched by the repeat.
            let a = &s.query_history("h.test/a", 1_000, 10).await.unwrap()[0];
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (2, 1, 300));
        });
    }

    #[test]
    fn a_deleted_imported_space_can_be_imported_again() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let model = base_model();
            let (m1, _) = s.apply_import(&model, &fixture(), 1).await.unwrap();
            // The user closes the "Reading" Space; its mark no longer points at a Space.
            let mut m = m1.clone();
            m.spaces.retain(|sp| sp.name != "Reading");
            let tabs = m
                .tabs
                .values()
                .filter(|t| t.url != "https://news.test" && t.url != "https://n2.test")
                .cloned()
                .collect();
            let now = m.clock();
            let m = Model::from_parts(m.spaces, m.containers, tabs, m.active_space, now).unwrap();
            s.save_session(&m).await.unwrap();
            let (m2, sum) = s.apply_import(&m, &fixture(), 2).await.unwrap();
            assert_eq!(sum.spaces_added, 1);
            assert_eq!(
                m2.spaces.iter().filter(|sp| sp.name == "Reading").count(),
                1
            );
            m2.check().unwrap();
            consistent(&s).await;
        });
    }

    #[test]
    fn a_reused_space_id_does_not_suppress_the_import() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let (m1, _) = s.apply_import(&base_model(), &fixture(), 1).await.unwrap();
            // "Reading" had the highest id; the user deletes it and a new Space takes that id.
            let reading = m1.spaces.iter().find(|sp| sp.name == "Reading").unwrap();
            let id = reading.id;
            let mut spaces = m1.spaces.clone();
            spaces.retain(|sp| sp.id != id);
            let mut fresh = reading.clone();
            fresh.name = "Fresh".into();
            fresh.pinned = vec![];
            fresh.items = vec![];
            fresh.active = None;
            spaces.push(fresh);
            let tabs = m1
                .tabs
                .values()
                .filter(|t| t.url != "https://news.test" && t.url != "https://n2.test")
                .cloned()
                .collect();
            let m = Model::from_parts(
                spaces,
                m1.containers.clone(),
                tabs,
                m1.active_space,
                m1.clock(),
            )
            .unwrap();
            s.save_session(&m).await.unwrap();
            let (m2, sum) = s.apply_import(&m, &fixture(), 2).await.unwrap();
            assert_eq!(sum.spaces_added, 1);
            assert!(m2.spaces.iter().any(|sp| sp.name == "Reading"));
            assert!(m2.spaces.iter().any(|sp| sp.name == "Fresh"));
            consistent(&s).await;
        });
    }

    #[test]
    fn tabs_with_script_urls_are_skipped() {
        let mut imp = fixture();
        imp.tabs
            .push(tab(0, None, Placement::Normal, 0, "javascript:alert(1)"));
        let m = merge_spaces(&base_model(), &imp, &HashMap::new()).unwrap();
        assert!(m.summary.skipped.iter().any(|k| k.kind == ItemKind::Tab
            && k.what == "javascript:alert(1)"
            && k.reason == SkipReason::InvalidUrl));
        assert!(
            m.model
                .tabs
                .values()
                .all(|t| !t.url.starts_with("javascript:"))
        );
    }

    #[test]
    fn history_dedupes_against_recorded_visits_at_the_same_millisecond() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.record_visit("https://h.test/a", "Mine", 200, crate::Transition::Link)
                .await
                .unwrap();
            let imp = Imported {
                history: fixture().history,
                bookmarks: vec![],
                ..fixture()
            };
            let (_, sum) = s
                .apply_import(&Model::new(std::time::Instant::now()), &imp, 1)
                .await
                .unwrap();
            assert_eq!((sum.visits_added, sum.visits_already_present), (2, 2));
            let a = &s.query_history("h.test/a", 1_000, 10).await.unwrap()[0];
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (2, 1, 300));
            assert_eq!(a.title, "Mine"); // an existing title wins
            consistent(&s).await;
        });
    }

    #[test]
    fn bookmarks_merge_into_an_existing_import_folder() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let root = s.add_folder(None, "Imported from Zen", 1).await.unwrap();
            s.add_bookmark(Some(root), "Mine", "https://mine.test", 1)
                .await
                .unwrap();
            s.add_bookmark(Some(root), "A", "https://a.test/", 1)
                .await
                .unwrap();
            let (_, sum) = s
                .apply_import(&Model::new(std::time::Instant::now()), &fixture(), 2)
                .await
                .unwrap();
            assert_eq!(
                (
                    sum.bookmark_folders_added,
                    sum.bookmarks_added,
                    sum.bookmarks_already_present
                ),
                (2, 2, 1)
            );
            let kids = s.bookmark_children(Some(root)).await.unwrap();
            assert_eq!(
                kids.iter().map(|k| k.title.as_str()).collect::<Vec<_>>(),
                ["Mine", "A", "Dev"]
            );
            assert_eq!(s.bookmark_children(None).await.unwrap().len(), 1);
            consistent(&s).await;
        });
    }

    #[test]
    fn a_failed_import_changes_nothing() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let model = base_model();
            s.save_session(&model).await.unwrap();
            // Everything but `visits`, which is gone below.
            let tables: Vec<&str> = TABLES.iter().copied().filter(|t| *t != "visits").collect();
            let before = counts(&s, &tables).await;
            // History is written last: with its table gone the whole import must roll back.
            s.conn.execute_batch("DROP TABLE visits").await.unwrap();
            assert!(s.apply_import(&model, &fixture(), 1).await.is_err());
            assert_eq!(counts(&s, &tables).await, before);
            assert_same(
                &s.load_session(std::time::Instant::now()).await.unwrap(),
                &model,
            );
        });
    }
}
