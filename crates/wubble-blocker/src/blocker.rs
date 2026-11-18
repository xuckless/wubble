use std::collections::HashSet;
use std::path::Path;

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::resources::Resource;

use crate::decision::Decision;
use crate::site::Shields;
use crate::types::NetRequest;

#[derive(Debug, thiserror::Error)]
pub enum BlockerError {
    #[error("invalid resources json: {0}")]
    Resources(#[from] serde_json::Error),
    #[error("compiled cache is unreadable: {0:?}")]
    Cache(adblock::engine::DeserializationError),
    #[error("cache io: {0}")]
    Io(#[from] std::io::Error),
}

/// Cosmetic filtering data for one page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cosmetic {
    /// Selectors to hide, sorted.
    pub hide_selectors: Vec<String>,
    /// Procedural cosmetic filters, sorted. Executing them is the engine lane's job.
    pub procedural_actions: Vec<String>,
    /// Scriptlet source (empty unless resources are loaded).
    pub injected_script: String,
    /// Generic hiding is disabled on this page.
    pub generichide: bool,
    /// Exceptions for `generic_selectors`, which takes the whole `Cosmetic`.
    pub(crate) exceptions: HashSet<String>,
}

impl Cosmetic {
    /// One `sel{display:none!important}` rule per selector, so one bad selector
    /// cannot void the rest.
    pub fn css(&self) -> String {
        self.hide_selectors
            .iter()
            .map(|s| format!("{s}{{display:none!important}}\n"))
            .collect()
    }

    /// The script that applies this page's rules: a `<style>` with
    /// [`Self::css`] (added as soon as the document has a root element),
    /// then the scriptlets. `None` when there is nothing to apply.
    ///
    /// Brave applies the same data (`url_cosmetic_resources`: hide
    /// selectors as a stylesheet, then `injected_script`) from its cosmetic
    /// filters tab helper (brave-core `components/brave_shields`, MPL-2.0);
    /// no code copied. Generic class/id selectors and procedural filters
    /// are not applied yet.
    pub fn page_script(&self) -> Option<String> {
        let css = self.css();
        if css.is_empty() && self.injected_script.is_empty() {
            return None;
        }
        let mut script = String::new();
        if !css.is_empty() {
            let css = serde_json::to_string(&css).expect("a string always serialises");
            script.push_str(&format!(
                "(function(){{var css={css};function add(){{var s=document.createElement('style');\
                 s.textContent=css;(document.head||document.documentElement).appendChild(s);}}\
                 if(document.documentElement){{add();}}else{{document.addEventListener(\
                 'readystatechange',add,{{once:true}});}}}})();\n"
            ));
        }
        script.push_str(&self.injected_script);
        Some(script)
    }
}

/// Compiled filter engine. `Send + Sync`; holds no per-site state.
pub struct Blocker {
    engine: Engine,
}

impl Blocker {
    /// Compile filter list texts.
    pub fn from_lists(lists: &[&str]) -> Self {
        let mut set = FilterSet::new(false);
        for text in lists {
            set.add_filter_list((*text).to_owned(), ParseOptions::default());
        }
        Self {
            engine: Engine::new_with_filter_set(set),
        }
    }

    /// Load redirect/scriptlet resources (`Vec<Resource>` JSON, Brave adblock-resources).
    /// Must be re-applied after `from_bytes`/`load`: the cache does not store them.
    pub fn with_resources_json(mut self, json: &str) -> Result<Self, BlockerError> {
        self.use_resources_json(json)?;
        Ok(self)
    }

    /// [`Self::with_resources_json`] in place; on error nothing changes.
    pub fn use_resources_json(&mut self, json: &str) -> Result<(), BlockerError> {
        let resources: Vec<Resource> = serde_json::from_str(json)?;
        self.engine.use_resources(resources);
        Ok(())
    }

    /// Serialise the compiled filters (not resources). Only valid for the same adblock
    /// version: any `Err` from loading means "rebuild from list texts".
    pub fn to_bytes(&self) -> Vec<u8> {
        self.engine.serialize()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BlockerError> {
        let mut engine = Engine::default();
        engine.deserialize(bytes).map_err(BlockerError::Cache)?;
        Ok(Self { engine })
    }

    /// Atomic write: temp file in the same directory, then rename.
    pub fn save(&self, path: &Path) -> Result<(), BlockerError> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, self.to_bytes())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, BlockerError> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// Classify a request. Unparsable URLs and `Shields::Off` allow.
    pub fn decide(&self, req: NetRequest<'_>, shields: Shields) -> Decision {
        if shields == Shields::Off {
            return Decision::Allow;
        }
        match Request::new(
            req.url,
            req.source_url,
            req.resource_type.filter_type(),
            "GET",
        ) {
            Ok(r) => Decision::from_result(self.engine.check_network_request(&r)),
            Err(_) => Decision::Allow,
        }
    }

    /// Host-specific cosmetic rules for a page. Empty when shields are off.
    pub fn cosmetic(&self, page_url: &str, shields: Shields) -> Cosmetic {
        if shields == Shields::Off {
            return Cosmetic::default();
        }
        let r = self.engine.url_cosmetic_resources(page_url);
        let sorted = |s: HashSet<String>| {
            let mut v: Vec<String> = s.into_iter().collect();
            v.sort();
            v
        };
        Cosmetic {
            hide_selectors: sorted(r.hide_selectors),
            procedural_actions: sorted(r.procedural_actions),
            injected_script: r.injected_script,
            generichide: r.generichide,
            exceptions: r.exceptions,
        }
    }

    /// Generic selectors matching classes/ids reported by the page, sorted.
    pub fn generic_selectors(
        &self,
        classes: &[&str],
        ids: &[&str],
        cosmetic: &Cosmetic,
    ) -> Vec<String> {
        if cosmetic.generichide {
            return Vec::new();
        }
        let mut v = self
            .engine
            .hidden_class_id_selectors(classes, ids, &cosmetic.exceptions);
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResourceType::{self, *};

    fn req<'a>(url: &'a str, source: &'a str, ty: ResourceType) -> NetRequest<'a> {
        NetRequest {
            url,
            source_url: source,
            resource_type: ty,
        }
    }

    fn decide(b: &Blocker, url: &str, source: &str, ty: ResourceType) -> Decision {
        b.decide(req(url, source, ty), Shields::On)
    }

    const SITE: &str = "https://site.org/";

    #[test]
    fn send_sync() {
        fn check<T: Send + Sync>() {}
        check::<Blocker>();
    }

    #[test]
    fn block_exception_important() {
        let b = Blocker::from_lists(&["||ads.example^\n@@||ads.example/ok^\n"]);
        assert_eq!(
            decide(&b, "https://ads.example/x.js", SITE, Script),
            Decision::Block
        );
        assert_eq!(
            decide(&b, "https://ads.example/ok/x.js", SITE, Script),
            Decision::Allow
        );
        assert_eq!(
            decide(&b, "https://other.example/x.js", SITE, Script),
            Decision::Allow
        );

        let b = Blocker::from_lists(&["||ads.example^$important\n@@||ads.example/ok^\n"]);
        assert_eq!(
            decide(&b, "https://ads.example/ok/x.js", SITE, Script),
            Decision::Block
        );
    }

    #[test]
    fn third_party_option() {
        let b = Blocker::from_lists(&["||tracker.net^$third-party\n"]);
        let url = "https://tracker.net/p.js";
        assert_eq!(decide(&b, url, SITE, Script), Decision::Block);
        assert_eq!(
            decide(&b, url, "https://tracker.net/", Script),
            Decision::Allow
        );
        assert_eq!(
            decide(&b, url, "https://www.tracker.net/page", Script),
            Decision::Allow
        );
        assert_eq!(decide(&b, url, "garbage", Script), Decision::Block);
    }

    #[test]
    fn resource_type_options() {
        let b = Blocker::from_lists(&["||cdn.test^$image\n||s.test^$script\n"]);
        assert_eq!(
            decide(&b, "https://cdn.test/a", SITE, Image),
            Decision::Block
        );
        assert_eq!(
            decide(&b, "https://cdn.test/a", SITE, Script),
            Decision::Allow
        );
        assert_eq!(
            decide(&b, "https://s.test/a", SITE, Script),
            Decision::Block
        );
        assert_eq!(decide(&b, "https://s.test/a", SITE, Xhr), Decision::Allow);
    }

    const RES: &str = r#"[{"name":"1x1.gif","aliases":[],"kind":{"mime":"image/gif"},"content":"R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"}]"#;

    #[test]
    fn redirect_needs_resources() {
        let list = "||img.test^$image,redirect=1x1.gif\n";
        let b = Blocker::from_lists(&[list]);
        assert_eq!(
            decide(&b, "https://img.test/a.png", SITE, Image),
            Decision::Block
        );
        let b = b.with_resources_json(RES).unwrap();
        match decide(&b, "https://img.test/a.png", SITE, Image) {
            Decision::Redirect { data_url } => {
                assert!(data_url.starts_with("data:image/gif;base64,"))
            }
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn bad_resources_json_errs() {
        assert!(
            Blocker::from_lists(&[])
                .with_resources_json("nope")
                .is_err()
        );
    }

    #[test]
    fn removeparam_rewrites() {
        let b = Blocker::from_lists(&["||q.test^$removeparam=utm\n"]);
        assert_eq!(
            decide(&b, "https://q.test/?utm=1&a=2", SITE, Document),
            Decision::Rewrite {
                url: "https://q.test/?a=2".into()
            }
        );
    }

    #[test]
    fn invalid_url_allows() {
        let b = Blocker::from_lists(&["||ads.example^\n"]);
        assert_eq!(decide(&b, "garbage", SITE, Script), Decision::Allow);
    }

    #[test]
    fn shields_off_allows_and_hides_nothing() {
        let b = Blocker::from_lists(&["||ads.example^\nexample.com##.sponsor\n"]);
        let r = req("https://ads.example/x.js", SITE, Script);
        assert_eq!(b.decide(r, Shields::On), Decision::Block);
        assert_eq!(b.decide(r, Shields::Off), Decision::Allow);
        assert!(
            !b.cosmetic("https://example.com/", Shields::On)
                .hide_selectors
                .is_empty()
        );
        assert_eq!(
            b.cosmetic("https://example.com/", Shields::Off),
            Cosmetic::default()
        );
    }

    #[test]
    fn cosmetic_host_specific_sorted_with_css() {
        let b = Blocker::from_lists(&["example.com##.sponsor\nexample.com##.aaa\n"]);
        let c = b.cosmetic("https://example.com/", Shields::On);
        assert_eq!(c.hide_selectors, [".aaa", ".sponsor"]);
        assert_eq!(
            c.css(),
            ".aaa{display:none!important}\n.sponsor{display:none!important}\n"
        );
        assert!(
            b.cosmetic("https://other.org/", Shields::On)
                .hide_selectors
                .is_empty()
        );
    }

    #[test]
    fn generic_selectors_exception_and_generichide() {
        let b = Blocker::from_lists(&[
            "##.banner\nexample.com#@#.banner\n@@||nohide.org^$generichide\n",
        ]);
        let c = b.cosmetic("https://site.org/", Shields::On);
        assert!(c.hide_selectors.is_empty());
        assert_eq!(b.generic_selectors(&["banner"], &[], &c), [".banner"]);
        assert!(b.generic_selectors(&["other"], &[], &c).is_empty());

        let c = b.cosmetic("https://example.com/", Shields::On);
        assert!(b.generic_selectors(&["banner"], &[], &c).is_empty());

        let c = b.cosmetic("https://nohide.org/", Shields::On);
        assert!(c.generichide);
        assert!(b.generic_selectors(&["banner"], &[], &c).is_empty());
    }

    #[test]
    fn page_script_escapes_css_and_appends_scriptlets() {
        assert_eq!(Cosmetic::default().page_script(), None);
        let c = Cosmetic {
            hide_selectors: vec![r#"a[href="x\y"]"#.into()],
            ..Cosmetic::default()
        };
        let script = c.page_script().unwrap();
        assert!(
            script.contains(r#"var css="a[href=\"x\\y\"]{display:none!important}\n";"#),
            "{script}"
        );
        let c = Cosmetic {
            injected_script: "window.scriptlet=1;".into(),
            ..Cosmetic::default()
        };
        assert_eq!(c.page_script().as_deref(), Some("window.scriptlet=1;"));
    }

    #[test]
    fn page_script_from_host_rules() {
        let b = Blocker::from_lists(&["example.com##.sponsor\n"]);
        let script = b
            .cosmetic("https://example.com/", Shields::On)
            .page_script()
            .unwrap();
        assert!(script.contains(".sponsor{display:none!important}"));
        assert_eq!(
            b.cosmetic("https://other.org/", Shields::On).page_script(),
            None
        );
        assert_eq!(
            b.cosmetic("https://example.com/", Shields::Off)
                .page_script(),
            None
        );
    }

    const LIST: &str = "||ads.example^\n@@||ads.example/ok^\n||img.test^$image,redirect=1x1.gif\nexample.com##.sponsor\n";

    #[test]
    fn cache_round_trip_same_decisions() {
        let a = Blocker::from_lists(&[LIST]);
        let b = Blocker::from_bytes(&a.to_bytes()).unwrap();
        for (u, t) in [
            ("https://ads.example/x.js", Script),
            ("https://ads.example/ok/x.js", Script),
            ("https://img.test/a.png", Image),
            ("https://fine.test/", Document),
        ] {
            assert_eq!(decide(&a, u, SITE, t), decide(&b, u, SITE, t), "{u}");
        }
        assert_eq!(
            a.cosmetic("https://example.com/", Shields::On),
            b.cosmetic("https://example.com/", Shields::On)
        );
    }

    #[test]
    fn corrupt_cache_errs() {
        let mut bytes = Blocker::from_lists(&[LIST]).to_bytes();
        assert!(Blocker::from_bytes(b"garbage").is_err());
        bytes.truncate(bytes.len() - 3);
        assert!(Blocker::from_bytes(&bytes).is_err());
    }

    #[test]
    fn resources_reapplied_after_load() {
        let bytes = Blocker::from_lists(&[LIST])
            .with_resources_json(RES)
            .unwrap()
            .to_bytes();
        let b = Blocker::from_bytes(&bytes).unwrap();
        assert_eq!(
            decide(&b, "https://img.test/a.png", SITE, Image),
            Decision::Block
        );
        let b = b.with_resources_json(RES).unwrap();
        assert!(matches!(
            decide(&b, "https://img.test/a.png", SITE, Image),
            Decision::Redirect { .. }
        ));
    }

    #[test]
    fn save_load_file() {
        let dir = std::env::temp_dir().join(format!("wubble-blocker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("engine.bin");
        Blocker::from_lists(&[LIST]).save(&path).unwrap();
        assert!(!dir.join("engine.bin.tmp").exists());
        let b = Blocker::load(&path).unwrap();
        assert_eq!(
            decide(&b, "https://ads.example/x.js", SITE, Script),
            Decision::Block
        );
        assert!(Blocker::load(&dir.join("missing.bin")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
