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

use std::collections::HashSet;

use crate::space::{Rgb, Space, SpaceIcon, SpaceId, glyph_id};
use crate::tabdrag::Rect;

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
    /// An agent session owns the Space: its tile draws the agent glyph ([`Row::glyph`]).
    pub agent: bool,
    /// The window's own Space (the row the popup marks).
    pub current: bool,
}

impl Row {
    /// What the row's tile draws (`space::glyph_id`).
    pub fn glyph(&self) -> &'static str {
        glyph_id(self.icon, self.agent)
    }
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

/// Where the open popup's scrolled list falls, in window px (the shell's SpacesMenu reports
/// it): its left edge and width, its top and visible height, how far it is scrolled (the
/// content's offset, 0 or less), and the heights of a row and a rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: f64,
    pub w: f64,
    pub list_top: f64,
    pub list_height: f64,
    pub scroll: f64,
    pub row: f64,
    pub rule: f64,
}

/// The popup for a window showing Space `current`, over the model's Spaces in order; `agents` are
/// the Spaces agent sessions own.
pub fn popup(spaces: &[Space], current: SpaceId, agents: &HashSet<SpaceId>) -> Popup {
    let rows: Vec<Row> = spaces
        .iter()
        .map(|s| Row {
            target: Target::Space(s.id),
            name: s.name.clone(),
            tint: s.tint,
            icon: s.icon,
            agent: agents.contains(&s.id),
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
    /// Where line `i` starts in the list, unscrolled: the rows and rules above it.
    pub fn line_top(&self, i: usize, g: &Geometry) -> f64 {
        self.lines()
            .iter()
            .take(i)
            .map(|l| match l {
                Line::Rule => g.rule,
                Line::Row(_) => g.row,
            })
            .sum()
    }

    /// A dragged tab's drop targets in the open popup: every Space row but the current Space's,
    /// as much of it as the list shows (a row scrolled out is none), with its line index in
    /// [`Popup::lines`] (the row the shell lights).
    pub fn drop_rows(&self, g: &Geometry) -> Vec<(usize, SpaceId, Rect)> {
        let (top, bottom) = (g.list_top, g.list_top + g.list_height);
        let mut y = g.list_top + g.scroll;
        let mut out = vec![];
        for (i, line) in self.lines().into_iter().enumerate() {
            let h = match line {
                Line::Rule => g.rule,
                Line::Row(_) => g.row,
            };
            if let Line::Row(r) = line {
                let (a, b) = (y.max(top), (y + h).min(bottom));
                let Target::Space(space) = r.target;
                if !r.current && b > a {
                    let rect = Rect {
                        x: g.x,
                        y: a,
                        w: g.w,
                        h: b - a,
                    };
                    out.push((i, space, rect));
                }
            }
            y += h;
        }
        out
    }

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
        let p = popup(&spaces, SpaceId(4), &HashSet::new());
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
    fn an_agent_s_space_row_draws_the_agent_glyph() {
        let spaces = [space(0, "Home"), space(1, "Session")];
        let p = popup(&spaces, SpaceId(0), &HashSet::from([SpaceId(1)]));
        let rows = &p.sections[0].rows;
        assert_eq!((rows[0].agent, rows[0].glyph()), (false, "personal"));
        assert_eq!((rows[1].agent, rows[1].glyph()), (true, "agent"));
        // the stored icon is untouched: only what is drawn changes
        assert_eq!(rows[1].icon, SpaceIcon::nth(1));
    }

    #[test]
    fn many_spaces_are_one_group_and_the_sticky_row_stays() {
        let spaces: Vec<_> = (0..12).map(|i| space(i, &format!("Space {i}"))).collect();
        let p = popup(&spaces, SpaceId(11), &HashSet::new());
        let lines = p.lines();
        assert_eq!(lines.len(), 12, "no rule inside one group");
        assert!(matches!(lines[11], Line::Row(r) if r.current));
        assert_eq!(p.sticky, Sticky::NewSpace);
    }

    #[test]
    fn no_spaces_no_group_but_new_space() {
        let p = popup(&[], SpaceId(0), &HashSet::new());
        assert!(p.sections.is_empty());
        assert!(p.lines().is_empty());
        assert_eq!(p.sticky, Sticky::NewSpace);
    }

    #[test]
    fn drop_rows_are_the_other_spaces_rows_the_list_shows() {
        let spaces: Vec<_> = (0..10).map(|i| space(i, "s")).collect();
        let p = popup(&spaces, SpaceId(1), &HashSet::new());
        let g = Geometry {
            x: 20.0,
            w: 224.0,
            list_top: 100.0,
            list_height: 7.0 * 36.0,
            scroll: -18.0,
            row: 36.0,
            rule: 8.0,
        };
        let rows = p.drop_rows(&g);
        // scrolled half a row: row 0 half shown, row 1 is the current Space, rows 2..=6 whole,
        // row 7 half shown, rows 8 and 9 scrolled out
        let ids: Vec<_> = rows.iter().map(|(i, s, _)| (*i, s.0)).collect();
        assert_eq!(
            ids,
            [(0, 0), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 7)]
        );
        let (_, _, first) = rows[0];
        assert_eq!(
            (first.x, first.y, first.w, first.h),
            (20.0, 100.0, 224.0, 18.0)
        );
        let (_, _, third) = rows[1];
        assert_eq!((third.y, third.h), (100.0 - 18.0 + 72.0, 36.0));
        let (_, _, last) = rows[6];
        assert_eq!(last.y + last.h, 100.0 + 7.0 * 36.0);
        assert_eq!(last.h, 18.0);
        assert_eq!(p.line_top(7, &g), 7.0 * 36.0);
        let mut two = p.clone();
        two.sections.push(p.sections[0].clone());
        assert_eq!(two.line_top(11, &g), 10.0 * 36.0 + 8.0, "past the rule");
    }

    #[test]
    fn a_second_group_is_set_off_by_a_rule() {
        // the Sessions group (MCP plan) will be a second section: the list puts a rule
        // between the two and none at either end
        let mut p = popup(&[space(0, "Home")], SpaceId(0), &HashSet::new());
        p.sections.push(p.sections[0].clone());
        let lines = p.lines();
        assert_eq!(lines.len(), 3);
        assert!(matches!(lines[0], Line::Row(_)));
        assert_eq!(lines[1], Line::Rule);
        assert!(matches!(lines[2], Line::Row(_)));
    }
}
