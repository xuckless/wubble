//! What CEF hands an `AccessibilityHandler`, as plain data. The engine converts its
//! `cef::Value` into [`AxValue`] (the only CEF-shaped step); everything after is pure.
//!
//! The shape is CEF's `libcef/browser/osr/osr_accessibility_util.cc` (chromiumembedded/cef,
//! BSD-3-Clause), read, not copied: a tree change is `{ax_tree_id, updates[], events[]}`, each
//! update `{has_tree_data, tree_data{focus_id, ...}, node_id_to_clear, root_id, nodes[]}`, each
//! node `{id, role, state[], offset_container_id, location{x,y,width,height}, transform,
//! child_ids[], actions[], attributes{}}`; roles, states, actions and enum attributes are
//! Chromium's `ui::ToString` names (`rootWebArea`, `focusable`, ...). A location change is a
//! list of `{id, ax_tree_id, new_location{offset_container_id, bounds{...}, transform}}`.
//! Keys CEF leaves out (it omits defaults) read as their default here.

use std::collections::BTreeMap;

/// A CEF value (`CefValue`: null, bool, int, double, string, list, dictionary; binary is
/// never sent for accessibility).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AxValue {
    #[default]
    Null,
    Bool(bool),
    Int(i32),
    Double(f64),
    String(String),
    List(Vec<AxValue>),
    Dict(BTreeMap<String, AxValue>),
}

impl AxValue {
    fn get(&self, key: &str) -> Option<&AxValue> {
        match self {
            AxValue::Dict(d) => d.get(key),
            _ => None,
        }
    }

    fn int(&self, key: &str) -> Option<i32> {
        match self.get(key)? {
            AxValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    fn number(&self, key: &str) -> f64 {
        match self.get(key) {
            Some(AxValue::Double(d)) => *d,
            Some(AxValue::Int(i)) => f64::from(*i),
            _ => 0.0,
        }
    }

    fn string(&self, key: &str) -> String {
        match self.get(key) {
            Some(AxValue::String(s)) => s.clone(),
            _ => String::new(),
        }
    }

    fn list(&self, key: &str) -> &[AxValue] {
        match self.get(key) {
            Some(AxValue::List(l)) => l,
            _ => &[],
        }
    }

    fn strings(&self, key: &str) -> Vec<String> {
        self.list(key)
            .iter()
            .filter_map(|v| match v {
                AxValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    fn ints(&self, key: &str) -> Vec<i32> {
        self.list(key)
            .iter()
            .filter_map(|v| match v {
                AxValue::Int(i) => Some(*i),
                _ => None,
            })
            .collect()
    }
}

/// A rectangle in CSS pixels, relative to its offset container.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    fn of(v: &AxValue) -> Bounds {
        Bounds {
            x: v.number("x"),
            y: v.number("y"),
            width: v.number("width"),
            height: v.number("height"),
        }
    }
}

/// Where a node is: its bounds and the node they are relative to (`None`: the page).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Location {
    pub offset_container: Option<i32>,
    pub bounds: Bounds,
}

/// One node of Chromium's tree (`ui::AXNodeData`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxNode {
    pub id: i32,
    pub role: String,
    pub states: Vec<String>,
    pub location: Location,
    pub child_ids: Vec<i32>,
    pub actions: Vec<String>,
    /// String, int, float, bool and list attributes by their Chromium name (`name`,
    /// `hierarchicalLevel`, `scrollX`, ...).
    pub attributes: BTreeMap<String, AxValue>,
}

impl AxNode {
    pub fn has_state(&self, state: &str) -> bool {
        self.states.iter().any(|s| s == state)
    }

    pub fn text(&self, attribute: &str) -> Option<&str> {
        match self.attributes.get(attribute) {
            Some(AxValue::String(s)) if !s.is_empty() => Some(s),
            _ => None,
        }
    }

    pub fn int(&self, attribute: &str) -> Option<i32> {
        match self.attributes.get(attribute) {
            Some(AxValue::Int(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn flag(&self, attribute: &str) -> bool {
        matches!(self.attributes.get(attribute), Some(AxValue::Bool(true)))
    }

    fn of(v: &AxValue) -> Option<AxNode> {
        let attributes = match v.get("attributes") {
            Some(AxValue::Dict(d)) => d.clone(),
            _ => BTreeMap::new(),
        };
        Some(AxNode {
            id: v.int("id")?,
            role: v.string("role"),
            states: v.strings("state"),
            location: Location {
                offset_container: v.int("offset_container_id").filter(|id| *id >= 0),
                bounds: v.get("location").map(Bounds::of).unwrap_or_default(),
            },
            child_ids: v.ints("child_ids"),
            actions: v.strings("actions"),
            attributes,
        })
    }
}

/// One `ui::AXTreeUpdate`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeChange {
    /// The node whose subtree is cleared before the nodes apply (`None`: nothing cleared).
    pub node_id_to_clear: Option<i32>,
    pub root_id: Option<i32>,
    /// The focused node, when the update carries tree data.
    pub focus_id: Option<i32>,
    pub nodes: Vec<AxNode>,
}

/// `OnAccessibilityTreeChange`'s value: one tree's updates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Updates {
    /// Chromium's `AXTreeID` as a string (one per document; a new one on navigation).
    pub tree_id: String,
    pub changes: Vec<TreeChange>,
}

/// Reads a tree change. Malformed nodes (no `id`) are skipped.
pub fn parse_updates(v: &AxValue) -> Updates {
    let changes = v
        .list("updates")
        .iter()
        .map(|u| TreeChange {
            node_id_to_clear: u.int("node_id_to_clear").filter(|id| *id != 0),
            root_id: u.int("root_id").filter(|id| *id != 0),
            focus_id: u.get("tree_data").and_then(|d| d.int("focus_id")),
            nodes: u.list("nodes").iter().filter_map(AxNode::of).collect(),
        })
        .collect();
    Updates {
        tree_id: v.string("ax_tree_id"),
        changes,
    }
}

/// One entry of `OnAccessibilityLocationChange`'s list.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationChange {
    pub tree_id: String,
    pub id: i32,
    pub location: Location,
}

/// Reads a location change list. Entries without an `id` are skipped.
pub fn parse_locations(v: &AxValue) -> Vec<LocationChange> {
    let AxValue::List(entries) = v else {
        return vec![];
    };
    entries
        .iter()
        .filter_map(|e| {
            let at = e.get("new_location")?;
            Some(LocationChange {
                tree_id: e.string("ax_tree_id"),
                id: e.int("id")?,
                location: Location {
                    offset_container: at.int("offset_container_id").filter(|id| *id >= 0),
                    bounds: at.get("bounds").map(Bounds::of).unwrap_or_default(),
                },
            })
        })
        .collect()
}
