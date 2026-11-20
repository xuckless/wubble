//! The sidebar footer's Spaces popup (owner, 2026-11-19: the "^" beside the current Space's
//! icon "opens up popup with all the saved spaces and 'sessions (mcp plan)' and a sticky new
//! session inside that popup. That's how we use space!"): its rows as data.
//!
//! The popup is a list of groups, each a run of rows, and one sticky row under them all. Today
//! there is one group, the Spaces in their order with the current one marked, and the sticky
//! row is "New Space". The MCP plan's agent sessions (docs/research, "MCP server plan") become a
//! second group: a [`Group`] kind and a [`Target`] case, with no change to how the shell draws
//! the list (a rule between groups, the rows, the sticky row). Until then there is no Sessions
//! group at all (owner: hidden until the MCP server is built).

use crate::space::{Rgb, Space, SpaceIcon, SpaceId};

/// A group of rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Spaces,
}

/// What a row opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Space(SpaceId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub target: Target,
    pub name: String,
    pub tint: Rgb,
    pub icon: SpaceIcon,
    /// The window's own Space (the row the popup marks).
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub group: Group,
    pub rows: Vec<Row>,
}

/// The row pinned under the groups, outside their scroller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sticky {
    /// `Action::NewSpace`: a new Space, then shown.
    NewSpace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    /// Never an empty section.
    pub sections: Vec<Section>,
    pub sticky: Sticky,
}

/// One line of the scrolled list, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line<'a> {
    /// The separator between two groups.
    Rule,
    Row(&'a Row),
}

/// The popup for a window showing Space `current`, over the model's Spaces in order.
pub fn popup(spaces: &[Space], current: SpaceId) -> Popup {
    let rows: Vec<Row> = spaces
        .iter()
        .map(|s| Row {
            target: Target::Space(s.id),
            name: s.name.clone(),
            tint: s.tint,
            icon: s.icon,
            current: s.id == current,
        })
        .collect();
    let sections = (!rows.is_empty())
        .then_some(Section {
            group: Group::Spaces,
            rows,
        })
        .into_iter()
        .collect();
    Popup {
        sections,
        sticky: Sticky::NewSpace,
    }
}

impl Popup {
    /// The scrolled list: each group's rows, a rule between two groups.
    pub fn lines(&self) -> Vec<Line<'_>> {
        let mut out = vec![];
        for (i, s) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push(Line::Rule);
            }
            out.extend(s.rows.iter().map(Line::Row));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::ContainerId;

    fn space(id: u32, name: &str) -> Space {
        Space {
            id: SpaceId(id),
            name: name.into(),
            tint: Rgb(1, 2, 3),
            icon: SpaceIcon::nth(id as usize),
            container: ContainerId::DEFAULT,
            pinned: vec![],
            items: vec![],
            active: None,
            split: None,
            never_sleep: false,
        }
    }

    #[test]
    fn every_space_in_order_with_the_current_one_marked() {
        let spaces = [space(0, "Home"), space(4, "Work"), space(2, "Study")];
        let p = popup(&spaces, SpaceId(4));
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.sections[0].group, Group::Spaces);
        let rows = &p.sections[0].rows;
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Home", "Work", "Study"]);
        let current: Vec<_> = rows.iter().map(|r| r.current).collect();
        assert_eq!(current, [false, true, false]);
        assert_eq!(rows[1].target, Target::Space(SpaceId(4)));
        assert_eq!(rows[2].icon, SpaceIcon::nth(2));
        assert_eq!(p.sticky, Sticky::NewSpace);
    }

    #[test]
    fn many_spaces_are_one_group_and_the_sticky_row_stays() {
        let spaces: Vec<_> = (0..12).map(|i| space(i, &format!("Space {i}"))).collect();
        let p = popup(&spaces, SpaceId(11));
        let lines = p.lines();
        assert_eq!(lines.len(), 12, "no rule inside one group");
        assert!(matches!(lines[11], Line::Row(r) if r.current));
        assert_eq!(p.sticky, Sticky::NewSpace);
    }

    #[test]
    fn no_spaces_no_group_but_new_space() {
        let p = popup(&[], SpaceId(0));
        assert!(p.sections.is_empty());
        assert!(p.lines().is_empty());
        assert_eq!(p.sticky, Sticky::NewSpace);
    }

    #[test]
    fn a_second_group_is_set_off_by_a_rule() {
        // the Sessions group (MCP plan) will be a second section: the list puts a rule
        // between the two and none at either end
        let mut p = popup(&[space(0, "Home")], SpaceId(0));
        p.sections.push(p.sections[0].clone());
        let lines = p.lines();
        assert_eq!(lines.len(), 3);
        assert!(matches!(lines[0], Line::Row(_)));
        assert_eq!(lines[1], Line::Rule);
        assert!(matches!(lines[2], Line::Row(_)));
    }
}
