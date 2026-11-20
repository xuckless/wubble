//! Element refs for `read_page` (docs/research/mcp-server.md section 3, "Element refs"): an
//! opaque `e<doc>.<node>`, the tab's document number and Chromium's accessibility node id. A
//! document is numbered the first time one of its nodes is named, so a new document (a
//! navigation gives the page a new accessibility tree id) gives new refs and every older ref
//! stops resolving: "stale ref: take a new snapshot".

/// The documents one tab's refs have named, in the order they were first seen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefBook {
    docs: Vec<String>,
}

impl RefBook {
    /// The ref of node `node` of document `tree` (Chromium's tree id).
    pub fn name(&mut self, tree: &str, node: i32) -> String {
        let doc = match self.docs.iter().position(|d| d == tree) {
            Some(i) => i,
            None => {
                self.docs.push(tree.to_owned());
                self.docs.len() - 1
            }
        };
        format!("e{}.{node}", doc + 1)
    }

    /// The document and node a ref this book made names; `None` for anything else.
    pub fn resolve(&self, r: &str) -> Option<(&str, i32)> {
        let (doc, node) = r.strip_prefix('e')?.split_once('.')?;
        let doc: usize = doc.parse().ok()?;
        let tree = self.docs.get(doc.checked_sub(1)?)?;
        Some((tree.as_str(), node.parse().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_name_a_document_and_a_node() {
        let mut b = RefBook::default();
        assert_eq!(b.name("tree-a", 5), "e1.5");
        assert_eq!(b.name("tree-a", 9), "e1.9");
        assert_eq!(b.name("tree-b", 5), "e2.5");
        assert_eq!(b.resolve("e1.5"), Some(("tree-a", 5)));
        assert_eq!(b.resolve("e2.5"), Some(("tree-b", 5)));
        assert_eq!(b.resolve("e2.-3"), Some(("tree-b", -3)));
    }

    #[test]
    fn foreign_and_malformed_refs_resolve_to_nothing() {
        let mut b = RefBook::default();
        b.name("t", 1);
        for r in ["e3.1", "e0.1", "e1", "e1.x", "1.1", "", "e.1", "ex.1"] {
            assert_eq!(b.resolve(r), None, "{r}");
        }
    }
}
